//! Request metering for per-request and per-byte billing.
//!
//! Tracks HTTP request counts and outbound byte totals for an app instance.
//! Both counters are read by the Worker Supervisor during heartbeat reporting
//! and sent to the control plane for billing aggregation and quota enforcement.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Request meter for tracking billable requests and outbound bytes per deployment.
#[derive(Debug, Clone)]
pub struct RequestMeter {
    /// Atomic request counter.
    count: Arc<AtomicU64>,
    /// Atomic outbound byte counter. Accumulates response bytes from
    /// http-client fetches and response bodies written by http-server.
    outbound_bytes: Arc<AtomicU64>,
    /// Atomic count of incoming WebSocket messages.
    ws_messages_in: Arc<AtomicU64>,
    /// Atomic count of outgoing WebSocket messages.
    ws_messages_out: Arc<AtomicU64>,
    /// Atomic count of currently active WebSocket connections.
    ws_connections_active: Arc<AtomicU64>,
    /// Tenant ID for reporting.
    pub tenant_id: String,
    /// Deployment ID for reporting.
    pub deployment_id: String,
}

impl RequestMeter {
    /// Create a new meter for a deployment.
    pub fn new(tenant_id: String, deployment_id: String) -> Self {
        Self {
            count: Arc::new(AtomicU64::new(0)),
            outbound_bytes: Arc::new(AtomicU64::new(0)),
            ws_messages_in: Arc::new(AtomicU64::new(0)),
            ws_messages_out: Arc::new(AtomicU64::new(0)),
            ws_connections_active: Arc::new(AtomicU64::new(0)),
            tenant_id,
            deployment_id,
        }
    }

    /// Record a single request. Called by http-server on each incoming request.
    pub fn record_request(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record outbound bytes. Called after each http-client response is received
    /// and after each http-server response body is written to the caller.
    pub fn record_outbound_bytes(&self, n: u64) {
        self.outbound_bytes.fetch_add(n, Ordering::Relaxed);
    }

    /// Record an incoming WebSocket message.
    pub fn record_ws_message_in(&self) {
        self.ws_messages_in.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an outgoing WebSocket message.
    pub fn record_ws_message_out(&self) {
        self.ws_messages_out.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a new active WebSocket connection.
    pub fn record_ws_connection_connect(&self) {
        self.ws_connections_active.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a closed WebSocket connection.
    pub fn record_ws_connection_close(&self) {
        self.ws_connections_active.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get the current request count.
    pub fn get_count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Get the current outbound byte total.
    pub fn get_outbound_bytes(&self) -> u64 {
        self.outbound_bytes.load(Ordering::Relaxed)
    }

    /// Get the current incoming WebSocket message count.
    pub fn get_ws_messages_in(&self) -> u64 {
        self.ws_messages_in.load(Ordering::Relaxed)
    }

    /// Get the current outgoing WebSocket message count.
    pub fn get_ws_messages_out(&self) -> u64 {
        self.ws_messages_out.load(Ordering::Relaxed)
    }

    /// Get the current active WebSocket connection count.
    pub fn get_ws_connections_active(&self) -> u64 {
        self.ws_connections_active.load(Ordering::Relaxed)
    }

    /// Subtract previously-snapshotted values from the counters. Called after a
    /// successful heartbeat publish so only the delta not yet reported remains.
    /// Using fetch_sub rather than store(0) preserves any bytes recorded after
    /// the snapshot was taken — those will appear in the next heartbeat interval.
    pub fn subtract_delta(
        &self,
        count_delta: u64,
        bytes_delta: u64,
        ws_in_delta: u64,
        ws_out_delta: u64,
    ) {
        self.count.fetch_sub(count_delta, Ordering::Relaxed);
        self.outbound_bytes
            .fetch_sub(bytes_delta, Ordering::Relaxed);
        self.ws_messages_in
            .fetch_sub(ws_in_delta, Ordering::Relaxed);
        self.ws_messages_out
            .fetch_sub(ws_out_delta, Ordering::Relaxed);
    }

    /// Get a snapshot of the meter state for reporting.
    pub fn snapshot(&self) -> MeterSnapshot {
        MeterSnapshot {
            tenant_id: self.tenant_id.clone(),
            deployment_id: self.deployment_id.clone(),
            request_count: self.get_count(),
            outbound_bytes: self.get_outbound_bytes(),
            ws_messages_in: self.get_ws_messages_in(),
            ws_messages_out: self.get_ws_messages_out(),
            ws_connections_active: self.get_ws_connections_active() as u32,
        }
    }
}

/// A snapshot of metering state for a reporting interval.
#[derive(Debug, Clone)]
pub struct MeterSnapshot {
    pub tenant_id: String,
    pub deployment_id: String,
    pub request_count: u64,
    /// Total outbound bytes since the last reset (heartbeat interval delta).
    pub outbound_bytes: u64,
    pub ws_messages_in: u64,
    pub ws_messages_out: u64,
    pub ws_connections_active: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_request_increments_count() {
        let m = RequestMeter::new("t_test".into(), "d_test".into());
        m.record_request();
        m.record_request();
        assert_eq!(m.snapshot().request_count, 2);
    }

    #[test]
    fn record_outbound_bytes_accumulates() {
        let m = RequestMeter::new("t_test".into(), "d_test".into());
        m.record_outbound_bytes(1024);
        m.record_outbound_bytes(512);
        assert_eq!(m.snapshot().outbound_bytes, 1536);
    }

    #[test]
    fn subtract_delta_removes_only_snapshotted_values() {
        let m = RequestMeter::new("t_test".into(), "d_test".into());
        m.record_request();
        m.record_request();
        m.record_outbound_bytes(4096);
        let snap = m.snapshot();
        // Simulate a new request arriving after the snapshot but before reset.
        m.record_request();
        m.record_outbound_bytes(100);
        m.subtract_delta(
            snap.request_count,
            snap.outbound_bytes,
            snap.ws_messages_in,
            snap.ws_messages_out,
        );
        // Only the post-snapshot delta should remain.
        let after = m.snapshot();
        assert_eq!(after.request_count, 1);
        assert_eq!(after.outbound_bytes, 100);
    }

    #[test]
    fn clone_shares_counters() {
        let m = RequestMeter::new("t_test".into(), "d_test".into());
        let m2 = m.clone();
        m.record_outbound_bytes(100);
        m2.record_outbound_bytes(50);
        // Both clones share the same Arc, so the total is 150.
        assert_eq!(m.snapshot().outbound_bytes, 150);
    }
}
