//! Worker configuration loaded from environment variables.

use anyhow::Context;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub worker_id: String,
    pub region: String,
    /// The address the public ingress should reverse-proxy to in order to reach
    /// apps on this worker (e.g. `203.0.113.10` or `worker-fra-1.internal:8080`).
    /// Required: the worker fails to start without it. Operators in private VPCs
    /// must set this to a routable IP or domain (Cloud NAT EIP, internal LB, etc.).
    pub worker_addr: String,
    pub nats_url: String,
    pub control_plane_url: String,
    pub cache_dir: PathBuf,
    pub heartbeat_interval_secs: u64,
    pub health_check_timeout_secs: u64,
    pub port_cooldown_secs: u64,
    pub starting_port: u16,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Required env vars:
    /// - `WORKER_ID` (e.g., `w_fra_abc123`)
    /// - `REGION` (e.g., `fra`)
    /// - `CONTROL_PLANE_URL` (e.g., `https://api.edgecloud.dev`)
    /// - `EDGE_WORKER_ADDR` (e.g., `203.0.113.10`) — the routable address of
    ///   this worker for the public ingress. Required: silent defaults have
    ///   produced every past "URL works for me but not for users" incident.
    ///
    /// Optional env vars:
    /// - `NATS_URL` (default: `nats://localhost:4222`)
    /// - `CACHE_DIR` (default: `.worker-cache`)
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Config {
            worker_id: std::env::var("WORKER_ID").context("WORKER_ID not set")?,
            region: std::env::var("REGION").context("REGION not set")?,
            worker_addr: std::env::var("EDGE_WORKER_ADDR").context("EDGE_WORKER_ADDR not set")?,
            nats_url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into()),
            control_plane_url: std::env::var("CONTROL_PLANE_URL")
                .context("CONTROL_PLANE_URL not set")?,
            cache_dir: std::env::var("CACHE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(".worker-cache")),
            heartbeat_interval_secs: 30,
            health_check_timeout_secs: std::env::var("EDGE_HEALTH_CHECK_TIMEOUT_SECS")
                .unwrap_or_else(|_| "60".into())
                .parse()
                .unwrap_or(60),
            port_cooldown_secs: 60,
            starting_port: 8081,
        })
    }
}
