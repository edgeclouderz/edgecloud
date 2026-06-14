//! `edge:http-server` — inbound HTTP serving.

use crate::metering::RequestMeter;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

#[derive(Debug, Clone)]
pub struct IncomingRequest {
    pub id: u64,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct HttpServer {
    port: Option<u16>,
    tx: Arc<RwLock<Option<mpsc::Sender<IncomingRequest>>>>,
    rx: Arc<RwLock<Option<mpsc::Receiver<IncomingRequest>>>>,
    pub meter: Option<Arc<RequestMeter>>,
    #[allow(dead_code)]
    next_id: Arc<std::sync::atomic::AtomicU64>,
}

impl HttpServer {
    pub fn new() -> Self {
        Self {
            port: None,
            tx: Arc::new(RwLock::new(None)),
            rx: Arc::new(RwLock::new(None)),
            meter: None,
            next_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    pub async fn start(&mut self, port: u16, host: Option<String>) -> Result<(), String> {
        let addr = format!("{}:{}", host.as_deref().unwrap_or("0.0.0.0"), port);
        let _listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| format!("failed to bind {}: {}", addr, e))?;

        self.port = Some(port);
        let (tx, rx) = mpsc::channel::<IncomingRequest>(100);
        *self.tx.write().await = Some(tx);
        *self.rx.write().await = Some(rx);

        tracing::info!(addr = %addr, "http-server listening");
        Ok(())
    }

    pub async fn poll(&mut self) -> Result<Option<IncomingRequest>, String> {
        let mut rx = self.rx.write().await;
        if let Some(rx) = rx.as_mut() {
            match rx.try_recv() {
                Ok(request) => {
                    if let Some(ref meter) = self.meter {
                        meter.record_request();
                    }
                    Ok(Some(request))
                }
                Err(_) => Ok(None),
            }
        } else {
            Err("http-server not started".to_string())
        }
    }

    pub async fn respond(
        &self,
        _req_id: u64,
        _status: u16,
        _headers: Vec<(String, String)>,
        _body: Vec<u8>,
    ) -> Result<(), String> {
        Ok(())
    }
}