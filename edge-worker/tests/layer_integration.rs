//! Phase E — L1–L10 integration tests.
//!
//! These tests exercise the end-to-end FaaS dispatch path:
//!
//!   * The handler fixture (`edge-worker/tests/fixtures/handler.wasm`)
//!     is parsed through the runtime linker and instantiated.
//!   * A `HandlerDispatch` binds an HTTP/1 server on a free port.
//!   * `reqwest::Client` makes real HTTP requests against it.
//!
//! Each test is gated on the fixture file being present (skip with a
//! clear message otherwise) and on the runtime being buildable. They
//! do **not** require Docker / NATS / the supervisor — the harness
//! instantiates `HandlerDispatch` directly so we exercise the proxy
//! path without spinning up the full worker.
//!
//! ## Layer index
//!
//! - L1–L4: linker-state tests, in `edge-runtime/tests/v0_2_smoke.rs`
//! - L5: dispatch round-trip — `l5_handler_dispatch_round_trip` (this file)
//! - L6: body cap — deferred (fixture path not yet implemented)
//! - L7: per-request timeout — `l7_per_request_timeout_returns_500` (this file)
//! - L8: long-running self-host — deferred (long_running fixture not yet built)
//! - L9: tenant filesystem isolation — deferred (fixture fs paths not yet impl)
//! - L10: deadline interrupts outbound — deferred (outgoing-handler path pending)
//!
//! ## Skip policy
//!
//! Set `SKIP_INTEGRATION_TESTS=1` (or `CI=1`) to skip all tests in this
//! file without taking down the rest of the suite. Tests also self-skip
//! when the handler fixture is missing.
//!
//! Run with:
//!   cargo test --manifest-path edge-worker/Cargo.toml --test layer_integration
//! Skip:
//!   SKIP_INTEGRATION_TESTS=1 cargo test --manifest-path edge-worker/Cargo.toml

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use edge_runtime::interfaces::observe::{AppLogContext, LogRecord, LogSink};
use edge_runtime::{
    create_component_linker_handler, create_engine, EgressPolicy, RequestMeter, RuntimeState,
};
use edge_worker::dispatch::{HandlerConfig, HandlerDispatch};
use edge_worker::port_pool::PortPool;
use reqwest::StatusCode;
use wasmtime::component::{Component, InstancePre};

/// Skip predicate for the layer integration tests. Unlike the supervisor
/// integration tests, these don't need Docker — but they're skipped when
/// the fixture is missing or CI is set.
fn should_skip_layer_tests() -> bool {
    if std::env::var("SKIP_INTEGRATION_TESTS").is_ok() {
        return true;
    }
    if std::env::var("CI").is_ok() {
        return true;
    }
    let candidates = [
        "tests/fixtures/handler.wasm",
        "edge-worker/tests/fixtures/handler.wasm",
    ];
    !candidates.iter().any(|p| PathBuf::from(p).exists())
}

/// Locate the pre-built `handler.wasm` fixture. Returns `None` if the
/// fixture is missing in any of the expected paths.
fn handler_fixture_path() -> Option<PathBuf> {
    [
        "tests/fixtures/handler.wasm",
        "edge-worker/tests/fixtures/handler.wasm",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// No-op log sink for tests — the L5/L7 tests don't observe log output
/// but the HandlerConfig requires an `Arc<dyn LogSink>`.
struct NullSink;

impl LogSink for NullSink {
    fn push(&self, _record: LogRecord, _ctx: AppLogContext) {}
}

/// Per-test harness. Holds the engine + linker + instance_pre + the
/// spawned dispatch server + its shutdown handle.
///
/// Drop tears the server down via the broadcast channel.
struct LayerHarness {
    url_base: String,
    client: reqwest::Client,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
    /// Wall-clock start of the most recent request, used by `elapsed()`.
    request_started: StdMutex<Option<Instant>>,
}

impl LayerHarness {
    /// Spin up a new handler dispatch on a free port using the pre-built
    /// `handler.wasm` fixture. The returned harness owns a reqwest
    /// client ready to fire requests.
    async fn spawn() -> anyhow::Result<Self> {
        let path = handler_fixture_path().context("handler.wasm fixture missing")?;

        let engine = create_engine().context("create_engine")?;
        let linker =
            create_component_linker_handler(&engine).context("create_component_linker_handler")?;

        let bytes = std::fs::read(&path).context("read handler.wasm")?;
        let component =
            Component::from_binary(&engine, &bytes).context("Component::from_binary")?;

        // Pre-compile the component into an InstancePre. The per-request
        // path rebuilds its own store+state; this pre-compilation step
        // is what makes HandlerDispatch::serve fast on the hot path.
        let instance_pre: InstancePre<RuntimeState> = linker
            .instantiate_pre(&component)
            .context("linker.instantiate_pre")?;

        // Port allocation: prefer 8192+ (ephemeral range, less likely
        // to clash with a developer's local services).
        let mut pool = PortPool::new(8192, 0);
        let port = pool.acquire().context("port pool exhausted")?;

        let config = HandlerConfig {
            tenant_id: "test-tenant".to_string(),
            egress: Arc::new(EgressPolicy::allow_all()),
            log_sink: Arc::new(NullSink),
            app_ctx: AppLogContext {
                app_name: "l5".to_string(),
                tenant_id: "test-tenant".to_string(),
                deployment_id: "l5-deployment".to_string(),
            },
            meter: Arc::new(RequestMeter::new(
                "test-tenant".to_string(),
                "l7-deployment".to_string(),
            )),
            env: HashMap::new(),
        };

        let dispatch = Arc::new(
            HandlerDispatch::new(instance_pre, port, 1_000, 1, config)
                .context("HandlerDispatch::new")?,
        );

        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let dispatch_for_serve = dispatch.clone();
        tokio::spawn(async move {
            if let Err(e) = dispatch_for_serve.serve(shutdown_rx).await {
                tracing::error!(err = %e, "HandlerDispatch serve failed");
            }
        });

        // The handler is HTTP/1 plain text. Give the listener a moment
        // to bind; `accept()` will fail if the bind raced.
        tokio::time::sleep(Duration::from_millis(50)).await;

        Ok(Self {
            url_base: format!("http://127.0.0.1:{port}"),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .context("reqwest::Client::builder")?,
            shutdown_tx,
            request_started: StdMutex::new(None),
        })
    }

    /// Fire a GET against the harness and return (status, body).
    async fn get(&self, path: &str) -> anyhow::Result<(StatusCode, String)> {
        let url = format!("{}{}", self.url_base, path);
        if let Ok(mut guard) = self.request_started.lock() {
            *guard = Some(Instant::now());
        }
        let resp = self.client.get(&url).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        Ok((status, body))
    }

    #[allow(dead_code)]
    fn elapsed(&self) -> Duration {
        self.request_started
            .lock()
            .ok()
            .and_then(|t| *t)
            .map(|started| started.elapsed())
            .unwrap_or(Duration::ZERO)
    }
}

impl Drop for LayerHarness {
    fn drop(&mut self) {
        // Best-effort shutdown. The dispatch's `serve` loop exits on
        // the next select! poll.
        let _ = self.shutdown_tx.send(());
    }
}

// ---- L5: handler dispatch round-trip ------------------------------------

/// L5: a Handler component is dispatched through `HandlerDispatch` and
/// the guest's `handle(req, out)` produces a real HTTP response that
/// matches the contract documented in
/// `edge-worker/tests/fixtures/README.md`.
#[tokio::test(flavor = "multi_thread")]
async fn l5_handler_dispatch_round_trip() {
    if should_skip_layer_tests() {
        eprintln!(
            "SKIPPED: layer integration tests disabled (CI=1 / \
             SKIP_INTEGRATION_TESTS=1 / handler.wasm fixture missing)"
        );
        return;
    }

    let harness = LayerHarness::spawn().await.expect("LayerHarness::spawn");

    let (status, body) = harness
        .get("/")
        .await
        .expect("GET / against the handler dispatch");
    assert_eq!(status, StatusCode::OK, "GET / status was {body}");
    let parsed: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("body {body:?} not JSON: {e}"));
    assert_eq!(parsed["hello"], "handler");
    assert_eq!(parsed["path"], "/");
}

/// L5b: paths the fixture doesn't implement must return 404 (not 200,
/// not a connection drop). The fixture's documented contract is "all
/// other paths return 404". This catches a regression where the guest
/// panics and the dispatch returns 500 instead.
#[tokio::test(flavor = "multi_thread")]
async fn l5b_handler_dispatch_unknown_path_returns_404() {
    if should_skip_layer_tests() {
        return;
    }
    let harness = LayerHarness::spawn().await.expect("LayerHarness::spawn");
    let (status, _body) = harness
        .get("/does-not-exist")
        .await
        .expect("GET /does-not-exist");
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unknown path should return 404"
    );
}

// ---- L7: per-request timeout -------------------------------------------

/// L7: a handler that exceeds the per-request epoch deadline returns
/// 500 to the client. The fixture's `/busy` path busy-loops a counter
/// for ~5s of Wasm execution; the harness sets a 100ms request budget
/// and asserts the response returns well before the busy loop would
/// naturally complete.
#[tokio::test(flavor = "multi_thread")]
async fn l7_per_request_timeout_returns_500() {
    if should_skip_layer_tests() {
        return;
    }

    // Build a harness with a tight 100ms budget instead of the
    // 1000ms default. We can't change `LayerHarness::spawn`'s
    // hardcoded budget without exposing another constructor, so
    // duplicate the spawn path here.
    let path = handler_fixture_path().expect("handler.wasm fixture missing");
    let engine = create_engine().expect("create_engine");
    let linker = create_component_linker_handler(&engine).expect("create_component_linker_handler");
    let bytes = std::fs::read(&path).expect("read handler.wasm");
    let component = Component::from_binary(&engine, &bytes).expect("Component::from_binary");

    let instance_pre: InstancePre<RuntimeState> = linker
        .instantiate_pre(&component)
        .expect("linker.instantiate_pre");

    let mut pool = PortPool::new(8192, 0);
    let port = pool.acquire().expect("port pool exhausted");

    let config = HandlerConfig {
        tenant_id: "test-tenant".to_string(),
        egress: Arc::new(EgressPolicy::allow_all()),
        log_sink: Arc::new(NullSink),
        app_ctx: AppLogContext {
            app_name: "l7".to_string(),
            tenant_id: "test-tenant".to_string(),
            deployment_id: "l7-deployment".to_string(),
        },
        meter: Arc::new(RequestMeter::new(
            "test-tenant".to_string(),
            "l7-deployment".to_string(),
        )),
        env: HashMap::new(),
    };

    let dispatch = Arc::new(
        HandlerDispatch::new(
            instance_pre,
            port,
            /* request_budget_ms */ 100,
            1,
            config,
        )
        .expect("HandlerDispatch::new"),
    );

    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let shutdown_rx = shutdown_tx.subscribe();
    let dispatch_for_serve = dispatch.clone();
    tokio::spawn(async move {
        let _ = dispatch_for_serve.serve(shutdown_rx).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest::Client::builder");

    let url = format!("http://127.0.0.1:{port}/busy");
    let started = Instant::now();
    let resp = client.get(&url).send().await.expect("GET /busy");
    let elapsed = started.elapsed();

    // The dispatch's `Err(_dropped)` arm wraps the guest trap into a
    // 500. hyper's `serve_connection` returns the error to the client
    // as a 500-class response.
    assert!(
        resp.status().is_server_error() || resp.status() == StatusCode::INTERNAL_SERVER_ERROR,
        "/busy should return 5xx after deadline; got {}",
        resp.status()
    );

    // Budget is 100ms. The busy loop would otherwise run for ~5s. If
    // the deadline fired correctly the request returned in well under
    // 1s.
    assert!(
        elapsed < Duration::from_secs(2),
        "request should have been interrupted at ~100ms, not run for \
         the full busy loop (elapsed: {elapsed:?})"
    );

    let _ = shutdown_tx.send(());
}
