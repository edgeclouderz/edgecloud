//! App state tracking for running instances.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use edge_runtime::{MetricsAccumulator, RequestMeter};
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
    /// Shared metrics accumulator. Written by the Observer inside
    /// RuntimeState on every `edge:observe` counter/gauge/histogram call;
    /// read by `build_heartbeat` to produce the `observer_metrics` field
    /// shipped to the control plane.
    pub metrics: Arc<MetricsAccumulator>,
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
    /// Currently running app instances: (app_name, deployment_id) -> AppInstance (Arc-wrapped for
    /// cheap clone, with Mutex for interior mutability of status/fields).
    /// The composite key allows multiple deployment IDs for the same app_name
    /// to run concurrently (canary/blue-green).
    pub apps: HashMap<(String, String), Arc<Mutex<AppInstance>>>,
    /// Shared wasmtime Engine (for compilation caching across apps)
    pub engine: Engine,
    /// Wall-clock instant of the most recent TaskMessage (any variant)
    /// parsed by handle_task_message. Read by the heartbeat-loop
    /// watchdog to decide when to fall back to the HTTP /sync endpoint
    /// (issue #53). Seeded to `Some(Instant::now())` at construction so
    /// the first heartbeat tick after boot doesn't fire /sync on its
    /// own — the periodic control-plane sweep (RECONCILE_INTERVAL,
    /// 5min default) is the durable safety net for a worker that
    /// never hears from NATS.
    pub last_task_received_at: Mutex<Option<Instant>>,
}

impl WorkerState {
    pub fn new(engine: Engine) -> Self {
        Self {
            apps: HashMap::new(),
            engine,
            // Seed with the construction instant so the heartbeat-loop
            // watchdog's first measurement (at T≈heartbeat_interval
            // after boot) sees "time since boot" rather than "infinitely
            // stale". Without the seed, every worker that boots at the
            // same moment would fetch /sync on its first heartbeat tick
            // — a thundering herd against the CP at fleet scale.
            //
            // The first NATS TaskMessage that arrives will overwrite
            // this seed via handle_task_message's bump (commit C).
            last_task_received_at: Mutex::new(Some(Instant::now())),
        }
    }
}
