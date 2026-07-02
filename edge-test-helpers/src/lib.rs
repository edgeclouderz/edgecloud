//! Shared test harness helpers for edge-worker, edge-ingress, and future
//! crates that need to spin up a NATS container or a `Supervisor` in
//! integration tests.
//!
//! Three helpers, all gated behind test-only usage by convention (this
//! crate is added as a `dev-dependency` by callers — nothing in
//! `edge-worker`'s or `edge-ingress`'s release binary links against it):
//!
//! - [`should_skip_integration_tests`] returns `true` when Docker isn't
//!   available or the test is running in a CI environment that should
//!   skip container-based tests.
//! - [`start_nats`] starts a fresh testcontainers-managed NATS server
//!   and returns the container (which the caller MUST keep alive for
//!   the duration of the test) plus the host:port URL.
//! - [`build_supervisor_with`] constructs an `edge_worker::supervisor::Supervisor`
//!   from a caller-provided `Config`. The returned `SupervisorGuard`
//!   owns the NATS container — drop it to stop the container.
//!
//! The supervisor wiring is intentionally identical to what production
//! `edge-worker/src/main.rs` does: a real engine, a real `WorkerState`,
//! a real `Downloader` (pointed at whatever `control_plane_url` you put
//! in the config — typically a wiremock URL for HTTP endpoints), the
//! real NATS client, a real `LogForwarder`, and a real `reqwest::Client`.
//! The only knob that varies between tests is the `Config` fields (in
//! particular `worker_id`, `region`, `queue_group`, `consumer_name`,
//! `starting_port`, `cache_dir`, and `worker_tenant_id`).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ContainerRequest, ImageExt};
use testcontainers_modules::nats::Nats;
use tokio::sync::Mutex as TokioMutex;

use edge_runtime::create_engine;
use edge_worker::auth::WorkerJwtSigner;
use edge_worker::config::Config;
use edge_worker::downloader::Downloader;
use edge_worker::log_forwarder::LogForwarder;
use edge_worker::nats::{NatsClient as NatsClientTrait, NatsClientImpl};
use edge_worker::port_pool::PortPool;
use edge_worker::state::WorkerState;
use edge_worker::supervisor::Supervisor;

/// Returns `true` if integration tests should be skipped.
///
/// Tests are skipped when:
///   - `SKIP_INTEGRATION_TESTS` is set (manual override), or
///   - `/var/run/docker.sock` is absent (Docker unavailable).
///
/// The `CI` env var is **not** checked — GitHub Actions runners have
/// Docker available, so removing that gate lets integration tests run
/// in CI.
pub fn should_skip_integration_tests() -> bool {
    std::env::var("SKIP_INTEGRATION_TESTS").is_ok()
        || !std::path::Path::new("/var/run/docker.sock").exists()
}

// Lazily-initialized NATS URL shared across all tests in the same
// binary. The container handle is leaked so it lives for the process
// lifetime. Prevents each test from starting its own container.
static GLOBAL_NATS_URL: OnceLock<String> = OnceLock::new();

/// Returns a shared NATS URL, starting a NATS container on first call.
///
/// Subsequent calls return the same URL — the container is started once
/// per test-binary invocation and leaked for the process lifetime.
/// This avoids starting a separate NATS container per test (the worker
/// suite has ~18 tests that previously each started their own container).
pub async fn shared_nats_url() -> &'static str {
    if let Some(url) = GLOBAL_NATS_URL.get() {
        return url;
    }
    let (container, url) = start_nats().await;
    // Leak the container handle so it lives for the entire test-binary
    // lifetime. The handle is !Sync but once leaked it's never
    // dereferenced again — only the URL string is used by callers.
    Box::leak(Box::new(container));
    GLOBAL_NATS_URL.set(url).unwrap_or_else(|_| unreachable!());
    GLOBAL_NATS_URL.get().unwrap()
}

/// Spawn a NATS container via `testcontainers`. Returns the live
/// container handle (which the caller MUST keep alive — dropping it
/// stops the container and NATS connections will fail) plus the
/// `host:port` URL the worker should connect to.
///
/// Uses a duration-based ready-condition (5s) rather than the
/// built-in `WaitFor::Log` matcher — the latter can match stderr output
/// that arrives before the listener is actually accepting connections,
/// especially in CI where container I/O can be reordered.
pub async fn start_nats() -> (ContainerAsync<Nats>, String) {
    let container: ContainerAsync<Nats> = ContainerRequest::from(Nats::default())
        .with_startup_timeout(Duration::from_secs(30))
        .with_ready_conditions(vec![WaitFor::Duration {
            length: Duration::from_secs(5),
        }])
        .start()
        .await
        .expect("start NATS container");
    let host = container.get_host().await.expect("get host");
    let port = container
        .get_host_port_ipv4(4222)
        .await
        .expect("get NATS port");
    (container, format!("{}:{}", host, port))
}

/// RAII guard bundling a freshly-built `Supervisor` with the NATS
/// container it was wired to use. Dropping the guard stops the
/// container. Created by [`build_supervisor_with`] and returned to the
/// test; the typical usage is to bind it to a struct field for the
/// lifetime of the test (see how `edge-worker/tests/integration_tests.rs`
/// uses `TestHarness._nats_container`).
pub struct SupervisorGuard {
    pub supervisor: Arc<Supervisor>,
    /// The `host:port` URL the supervisor is connected to. Cached so
    /// tests don't have to query the container again.
    pub nats_url: String,
    /// Held so the container is alive for the lifetime of the
    /// Supervisor. Most callers don't read this field directly; the
    /// important property is "this is in scope while supervisor is".
    pub _nats_container: ContainerAsync<Nats>,
}

/// Build a Supervisor pointed at a freshly-started NATS container. The
/// caller controls every interesting test parameter via `config` — in
/// particular `worker_id`, `region`, `queue_group`, `consumer_name`,
/// `starting_port`, `cache_dir`, `worker_tenant_id`, and
/// `control_plane_url`. The `nats_url` field is overwritten with the
/// fresh container's URL before construction.
///
/// Returns a [`SupervisorGuard`] that owns both the Supervisor and the
/// NATS container; dropping the guard stops the container.
pub async fn build_supervisor_with(config: Config) -> SupervisorGuard {
    let (nats_container, nats_url) = start_nats().await;
    let mut config = config;
    config.nats_url = nats_url.clone();
    let supervisor = build_supervisor_inner(&config)
        .await
        .expect("build supervisor");
    SupervisorGuard {
        supervisor,
        nats_url,
        _nats_container: nats_container,
    }
}

/// Build a Supervisor that connects to an externally-managed NATS URL.
/// Use this when a test already started its own NATS container (e.g.,
/// because it needs direct NATS subscription without going through a
/// `SupervisorGuard`).
pub async fn build_supervisor_from_url(
    nats_url: &str,
    config: Config,
) -> anyhow::Result<Arc<Supervisor>> {
    let mut config = config;
    config.nats_url = nats_url.to_string();
    build_supervisor_inner(&config).await
}

/// Default cache directory used by tests that don't care about cache
/// isolation. A single-process run can share the same on-disk directory
/// across tests; the cache-poisoning tests use [`tempfile::TempDir`]
/// instead so they don't leak state across the suite.
pub fn default_cache_dir() -> PathBuf {
    PathBuf::from("/tmp/edge-worker-test-cache")
}

/// Build the Supervisor struct itself — the same wiring as production
/// `edge-worker/src/main.rs`, minus the JetStream stream-creation step
/// (the worker process asserts that streams already exist on startup;
/// tests don't create the streams so they can run without a real NATS
/// cluster).
async fn build_supervisor_inner(config: &Config) -> anyhow::Result<Arc<Supervisor>> {
    let engine = create_engine()?;
    let state = Arc::new(tokio::sync::RwLock::new(WorkerState::new(engine)));
    let jwt_signer = WorkerJwtSigner::new(
        config.worker_jwt_secret.clone(),
        config.worker_jwt_issuer.clone(),
        config.worker_id.clone(),
        config.region.clone(),
        config.worker_tenant_id.clone(),
    );
    let downloader = Arc::new(Downloader::new(
        config.control_plane_url.clone(),
        config.cache_dir.clone(),
        jwt_signer.clone(),
    ));
    let port_pool = Arc::new(TokioMutex::new(PortPool::new(
        config.starting_port,
        config.port_cooldown_secs,
    )));
    let nats =
        Arc::new(NatsClientImpl::connect(&config.nats_url).await?) as Arc<dyn NatsClientTrait>;
    let log_forwarder = LogForwarder::new(
        config.control_plane_url.clone(),
        config.worker_id.clone(),
        config.region.clone(),
        jwt_signer.clone(),
    );
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    Ok(Arc::new(Supervisor {
        config: config.clone(),
        state,
        downloader,
        port_pool,
        nats,
        log_forwarder,
        jwt_signer,
        http,
    }))
}

/// Convenience: replace `cache_dir` with a per-test tempdir. Use this
/// in tests where cache state must NOT leak across runs (the
/// cache-poisoning regression tests in `edge-worker/tests/integration_tests.rs`).
pub fn config_with_per_test_cache(mut base: Config) -> (Config, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("create cache tempdir");
    base.cache_dir = dir.path().to_path_buf();
    (base, dir)
}
