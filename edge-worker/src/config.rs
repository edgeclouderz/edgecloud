//! Worker configuration loaded from environment variables.

use anyhow::Context;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub worker_id: String,
    pub region: String,
    pub nats_url: String,
    pub control_plane_url: String,
    pub cache_dir: PathBuf,
    pub heartbeat_interval_secs: u64,
    pub port_cooldown_secs: u64,
    pub starting_port: u16,
    /// Per-app memory cap in MiB, applied via wasmtime StoreLimits.
    /// Default 256 MiB. Tune via `APP_MAX_MEMORY_MB`.
    pub max_memory_mb: u64,
    /// How often (ms) the worker advances the wasmtime epoch. Default 10 ms.
    /// Tune via `EPOCH_TICK_MS`.
    pub epoch_tick_ms: u64,
    /// Number of epoch ticks an app call may consume before being interrupted.
    /// With the default tick of 10 ms and deadline of 100, each call has a
    /// ~1 s CPU budget. Tune via `EPOCH_DEADLINE_TICKS`.
    pub epoch_deadline_ticks: u64,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Required env vars:
    /// - `WORKER_ID` (e.g., `w_fra_abc123`)
    /// - `REGION` (e.g., `fra`)
    /// - `CONTROL_PLANE_URL` (e.g., `https://api.edgecloud.dev`)
    ///
    /// Optional env vars:
    /// - `NATS_URL` (default: `nats://localhost:4222`)
    /// - `CACHE_DIR` (default: `.worker-cache`)
    /// - `APP_MAX_MEMORY_MB` (default: 256)
    /// - `EPOCH_TICK_MS` (default: 10)
    /// - `EPOCH_DEADLINE_TICKS` (default: 100)
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Config {
            worker_id: std::env::var("WORKER_ID").context("WORKER_ID not set")?,
            region: std::env::var("REGION").context("REGION not set")?,
            nats_url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into()),
            control_plane_url: std::env::var("CONTROL_PLANE_URL")
                .context("CONTROL_PLANE_URL not set")?,
            cache_dir: std::env::var("CACHE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(".worker-cache")),
            heartbeat_interval_secs: 30,
            port_cooldown_secs: 60,
            starting_port: 8081,
            max_memory_mb: parse_env_u64("APP_MAX_MEMORY_MB", 256),
            epoch_tick_ms: parse_env_u64("EPOCH_TICK_MS", 10),
            epoch_deadline_ticks: parse_env_u64("EPOCH_DEADLINE_TICKS", 100),
        })
    }
}

fn parse_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
