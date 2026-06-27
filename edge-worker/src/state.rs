//! App state tracking for running instances.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use edge_runtime::RequestMeter;
use tokio::sync::Mutex;
use wasmtime::component::InstancePre;
use wasmtime::Engine;

/// Status of a running app instance.
#[derive(Debug, Clone, PartialEq)]
pub enum AppInstanceStatus {
    #[allow(dead_code)]
    Starting,
    Running,
    #[allow(dead_code)]
    Stopping,
    Crashed {
        restart_count: u32,
    },
    /// App did not return from execute_app within the health check timeout.
    Hung,
}

/// A single running app instance.
#[allow(dead_code)]
pub struct AppInstance {
    pub deployment_id: String,
    pub app_name: String,
    pub tenant_id: String,
    pub port: u16,
    pub status: AppInstanceStatus,
    pub meter: Arc<RequestMeter>,
    /// Channel to signal graceful shutdown to the app task. Wrapped in Option so
    /// it can be taken out of the locked struct to call send().
    pub shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Pre-compiled component for fast instantiation on restart.
    pub instance_pre: InstancePre<edge_runtime::RuntimeState>,
    /// Handle to the spawned app task — used to propagate panics on stop.
    /// Wrapped in Arc so it can be cloned without taking ownership.
    pub handle: Option<std::sync::Arc<tokio::task::JoinHandle<()>>>,
    /// Handle to the epoch ticker that advances the wasmtime engine clock.
    /// The ticker is aborted on app stop; without it the engine epoch would
    /// never advance, and the Store-level deadline would never fire.
    /// Wrapped in Option so stop_app can take it out of the locked struct.
    pub ticker: Option<tokio::task::JoinHandle<()>>,
}

/// Shared worker state — protected by a tokio RwLock.
/// Apps are stored behind Arc<Mutex<>> so individual fields can be mutated
/// (e.g., status update to Crashed) without replacing the Arc entry.
pub struct WorkerState {
    /// Currently running app instances: app_name -> AppInstance (Arc-wrapped for
    /// cheap clone, with Mutex for interior mutability of status/fields).
    pub apps: HashMap<String, Arc<Mutex<AppInstance>>>,
    /// Shared wasmtime Engine (for compilation caching across apps)
    pub engine: Engine,
    /// Operational counters surfaced through heartbeats. Kept separate
    /// from `RequestMeter` (which tracks per-deployment billing data) so
    /// the two concerns don't share an atomic. Phase 5 will surface
    /// these counters in `AppStatus`.
    pub stats: RuntimeStats,
}

/// Operational counters — atomic, no lock required for hot-path
/// increments. Counters are intentionally NOT reset between heartbeats:
/// like `RequestMeter::request_count`, they are monotonically increasing
/// gauges that operators rate against time. Subtract-on-publish
/// semantics (Phase 5's `reset_meters_after`) only apply to billing
/// data.
#[derive(Debug, Default)]
pub struct RuntimeStats {
    /// Number of times the port pool returned `None` and a `start_app`
    /// call returned `Err`. Spikes indicate the worker is over-
    /// subscribed or apps are leaking ports (look at the cooldown
    /// accounting in `PortPool`).
    pub port_pool_exhausted_total: AtomicU64,
}

impl RuntimeStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the counters for heartbeat serialization. The returned
    /// values are point-in-time atomics; a separate increment between
    /// the snapshot and the publish is acceptable — heartbeats are not
    /// a transactional read.
    #[allow(dead_code)] // Phase 5 surfaces this in AppStatus.
    pub fn snapshot(&self) -> RuntimeStatsSnapshot {
        RuntimeStatsSnapshot {
            port_pool_exhausted_total: self.port_pool_exhausted_total.load(Ordering::Relaxed),
        }
    }
}

/// Serializable snapshot of `RuntimeStats`. Cheap to clone, safe to
/// embed in `HeartbeatMessage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Phase 5 surfaces this in AppStatus.
pub struct RuntimeStatsSnapshot {
    pub port_pool_exhausted_total: u64,
}

impl WorkerState {
    pub fn new(engine: Engine) -> Self {
        Self {
            apps: HashMap::new(),
            engine,
            stats: RuntimeStats::new(),
        }
    }
}
