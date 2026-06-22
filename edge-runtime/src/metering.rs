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

    /// Get the current request count.
    pub fn get_count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Get the current outbound byte total.
    pub fn get_outbound_bytes(&self) -> u64 {
        self.outbound_bytes.load(Ordering::Relaxed)
    }

    /// Reset both counters. Called after reporting to the control plane so
    /// each heartbeat interval carries a delta rather than a cumulative total.
    pub fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
        self.outbound_bytes.store(0, Ordering::Relaxed);
    }

    /// Get a snapshot of the meter state for reporting.
    pub fn snapshot(&self) -> MeterSnapshot {
        MeterSnapshot {
            tenant_id: self.tenant_id.clone(),
            deployment_id: self.deployment_id.clone(),
            request_count: self.get_count(),
            outbound_bytes: self.get_outbound_bytes(),
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
    fn reset_zeroes_both_counters() {
        let m = RequestMeter::new("t_test".into(), "d_test".into());
        m.record_request();
        m.record_outbound_bytes(4096);
        m.reset();
        let snap = m.snapshot();
        assert_eq!(snap.request_count, 0);
        assert_eq!(snap.outbound_bytes, 0);
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
