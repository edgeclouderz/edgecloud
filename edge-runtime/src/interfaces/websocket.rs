//! `edge:websocket` — inbound WebSocket connection management and frame I/O.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::{mpsc, oneshot};

use crate::edge::cloud::websocket::WsEvent;
use tokio_tungstenite::tungstenite::Message;

/// Decision passed from guest (via accept/reject) back to the HTTP server connection task.
pub enum UpgradeDecision {
    Accept {
        conn_id: u64,
        client_rx: mpsc::UnboundedReceiver<Message>,
        event_tx: mpsc::UnboundedSender<WsEvent>,
        active_connections: Arc<StdMutex<HashMap<u64, WsConnection>>>,
    },
    Reject {
        status: u16,
        reason: String,
    },
}

#[derive(Clone)]
pub struct WsConnection {
    pub tx: mpsc::UnboundedSender<Message>,
}

#[derive(Clone)]
pub struct WebSocketState {
    pub pending_upgrades: Arc<StdMutex<HashMap<u64, oneshot::Sender<UpgradeDecision>>>>,
    pub active_connections: Arc<StdMutex<HashMap<u64, WsConnection>>>,
    pub event_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<WsEvent>>>,
    pub event_tx: mpsc::UnboundedSender<WsEvent>,
    pub next_conn_id: Arc<AtomicU64>,
}

impl Default for WebSocketState {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSocketState {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            pending_upgrades: Arc::new(StdMutex::new(HashMap::new())),
            active_connections: Arc::new(StdMutex::new(HashMap::new())),
            event_rx: Arc::new(tokio::sync::Mutex::new(event_rx)),
            event_tx,
            next_conn_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn accept_websocket(&self, request_id: u64) -> Result<u64, String> {
        let upgrade_tx = self
            .pending_upgrades
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&request_id)
            .ok_or_else(|| "Pending upgrade not found or already handled".to_string())?;

        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let (client_tx, client_rx) = mpsc::unbounded_channel::<Message>();

        let decision = UpgradeDecision::Accept {
            conn_id,
            client_rx,
            event_tx: self.event_tx.clone(),
            active_connections: self.active_connections.clone(),
        };

        self.active_connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(conn_id, WsConnection { tx: client_tx });

        upgrade_tx
            .send(decision)
            .map_err(|_| "HTTP connection task dropped".to_string())?;

        Ok(conn_id)
    }

    pub fn reject_websocket(
        &self,
        request_id: u64,
        status: u32,
        reason: String,
    ) -> Result<(), String> {
        let upgrade_tx = self
            .pending_upgrades
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&request_id)
            .ok_or_else(|| "Pending upgrade not found or already handled".to_string())?;

        let decision = UpgradeDecision::Reject {
            status: status as u16,
            reason,
        };

        upgrade_tx
            .send(decision)
            .map_err(|_| "HTTP connection task dropped".to_string())?;

        Ok(())
    }

    pub fn ws_poll(&self) -> Result<Option<WsEvent>, String> {
        let mut rx = self
            .event_rx
            .try_lock()
            .map_err(|_| "Event queue lock contended".to_string())?;

        match rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                Err("Event queue disconnected".to_string())
            }
        }
    }

    pub fn ws_send_text(&self, conn_id: u64, data: String) -> Result<(), String> {
        let conn = self
            .active_connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&conn_id)
            .cloned()
            .ok_or_else(|| "Connection not found or closed".to_string())?;

        conn.tx
            .send(Message::Text(data.into()))
            .map_err(|_| "Connection channel closed".to_string())?;

        Ok(())
    }

    pub fn ws_send_binary(&self, conn_id: u64, data: Vec<u8>) -> Result<(), String> {
        let conn = self
            .active_connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&conn_id)
            .cloned()
            .ok_or_else(|| "Connection not found or closed".to_string())?;

        conn.tx
            .send(Message::Binary(data.into()))
            .map_err(|_| "Connection channel closed".to_string())?;

        Ok(())
    }

    pub fn ws_close(&self, conn_id: u64, code: u16, reason: String) -> Result<(), String> {
        let conn = self
            .active_connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&conn_id)
            .cloned()
            .ok_or_else(|| "Connection not found or closed".to_string())?;

        let close_frame = tokio_tungstenite::tungstenite::protocol::CloseFrame {
            code: code.into(),
            reason: reason.into(),
        };

        conn.tx
            .send(Message::Close(Some(close_frame)))
            .map_err(|_| "Connection channel closed".to_string())?;

        Ok(())
    }

    pub fn ws_connection_count(&self) -> u32 {
        self.active_connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len() as u32
    }
}
