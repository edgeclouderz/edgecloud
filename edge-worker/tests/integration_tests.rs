//! Integration tests for the edge-worker supervisor.
//!
//! These tests use testcontainers to spin up a real NATS server and exercise
//! the full Supervisor lifecycle: start_app → run_app_loop → stop_app.
//!
//! Run with: cargo test --manifest-path edge-worker/Cargo.toml
//!
//! Prerequisites: Docker must be running for testcontainers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::nats::Nats;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use edge_worker::config::Config;
use edge_worker::downloader::Downloader;
use edge_worker::messages::{AppSpec, HeartbeatMessage, TaskMessage};
use edge_worker::nats::{NatsClient as NatsClientTrait, NatsClientImpl};
use edge_worker::port_pool::PortPool;
use edge_worker::state::{AppInstanceStatus, WorkerState};
use edge_worker::supervisor::Supervisor;

/// Test WASM component bytes — a minimal component that exports `handle` and `_start`.
fn test_component_bytes() -> &'static [u8] {
    include_bytes!("fixtures/test-handle.wasm")
}

/// Build a test Supervisor with real NATS and a mock Downloader.
async fn build_test_supervisor(nats_url: &str) -> Supervisor {
    let config = Config {
        worker_id: "test-worker".to_string(),
        region: "test-region".to_string(),
        nats_url: nats_url.to_string(),
        control_plane_url: "http://localhost:9999".to_string(),
        cache_dir: PathBuf::from("/tmp/edge-worker-test-cache"),
        heartbeat_interval_secs: 30,
        port_cooldown_secs: 60,
        starting_port: 18_000,
    };

    let engine = edge_runtime::create_engine().expect("create engine");
    let state = Arc::new(tokio::sync::RwLock::new(WorkerState::new(engine)));
    let downloader = Arc::new(Downloader::new(
        config.control_plane_url.clone(),
        config.cache_dir.clone(),
    ));
    let port_pool = Arc::new(TokioMutex::new(PortPool::new(
        config.starting_port,
        config.port_cooldown_secs,
    )));

    let nats = Arc::new(NatsClientImpl::connect(nats_url).await.expect("connect nats"))
        as Arc<dyn NatsClientTrait>;

    Supervisor {
        config,
        state,
        downloader,
        port_pool,
        nats,
    }
}

/// Helper: subscribe to heartbeats and collect the first one.
async fn subscribe_heartbeats(nats_url: &str, region: &str) -> HeartbeatMessage {
    let client = async_nats::connect(nats_url).await.expect("connect nats");
    let subject = format!("edgecloud.heartbeats.{}", region);
    let mut sub = client.subscribe(subject).await.expect("subscribe");
    let msg = sub.next().await.expect("no heartbeat");
    serde_json::from_slice(&msg.payload).expect("parse heartbeat")
}

/// Helper: wait for an app to appear in state with Running status.
async fn wait_for_app_running(
    supervisor: &Supervisor,
    app_name: &str,
    timeout_secs: u64,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    while tokio::time::Instant::now() < deadline {
        let state = supervisor.state.read().await;
        if let Some(inst) = state.apps.get(app_name) {
            let inst = inst.lock().await;
            if matches!(inst.status, AppInstanceStatus::Running) {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Helper: wait for an app to disappear from state.
async fn wait_for_app_gone(supervisor: &Supervisor, app_name: &str, timeout_secs: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    while tokio::time::Instant::now() < deadline {
        let state = supervisor.state.read().await;
        if !state.apps.contains_key(app_name) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Start a NATS container and return the URL string.
async fn nats_container() -> String {
    let container = Nats::default()
        .start()
        .await
        .expect("start NATS container");
    let host = container.get_host().await.expect("get host");
    let port = container.get_host_port_ipv4(4222).await.expect("get NATS port");
    let url = format!("{}:{}", host, port);
    // Keep container alive for the test — it's dropped at the end of the test.
    // We rely on Drop to clean up the container.
    std::mem::forget(container);
    url
}

#[tokio::test]
async fn test_app_lifecycle() {
    // Setup: start NATS and mock HTTP server
    let nats_url = nats_container().await;

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/internal/download/d_deploy_001"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(test_component_bytes()),
        )
        .mount(&mock_server)
        .await;

    let config = Config {
        worker_id: "test-worker".to_string(),
        region: "test-region".to_string(),
        nats_url: nats_url.clone(),
        control_plane_url: mock_server.uri(),
        cache_dir: PathBuf::from("/tmp/edge-worker-test-cache"),
        heartbeat_interval_secs: 30,
        port_cooldown_secs: 60,
        starting_port: 18_000,
    };

    let engine = edge_runtime::create_engine().expect("create engine");
    let state = Arc::new(tokio::sync::RwLock::new(WorkerState::new(engine)));
    let downloader = Arc::new(Downloader::new(
        config.control_plane_url.clone(),
        config.cache_dir.clone(),
    ));
    let port_pool = Arc::new(TokioMutex::new(PortPool::new(
        config.starting_port,
        config.port_cooldown_secs,
    )));

    let nats =
        Arc::new(NatsClientImpl::connect(&nats_url).await.expect("connect nats"))
            as Arc<dyn NatsClientTrait>;

    let supervisor = Arc::new(Supervisor {
        config,
        state,
        downloader,
        port_pool,
        nats,
    });

    // Step 1: send TaskMessage to start an app
    let spec = AppSpec {
        deployment_id: "d_deploy_001".to_string(),
        deployment_hash: "abc123".to_string(),
        env: HashMap::new(),
        allowlist: vec![],
    };

    let msg = TaskMessage::TaskUpdate {
        timestamp: "2026-06-15T00:00:00Z".to_string(),
        tenant_id: "t_test".to_string(),
        apps: HashMap::from([("test-app".to_string(), spec)]),
    };

    supervisor
        .handle_task_message(msg)
        .await
        .expect("handle_task_message");

    // Step 2: app should be Running
    let running = wait_for_app_running(&supervisor, "test-app", 10).await;
    assert!(
        running,
        "app should be Running within 10s (check NATS connectivity and component compilation)"
    );

    // Step 3: heartbeat should include the app
    let heartbeat = supervisor.build_heartbeat().await;
    assert!(
        heartbeat.apps.contains_key("test-app"),
        "heartbeat should contain test-app"
    );
    let app_status = heartbeat.apps.get("test-app").unwrap();
    assert_eq!(app_status.status, "running", "app status should be 'running'");
    assert_eq!(
        app_status.deployment_id, "d_deploy_001",
        "deployment_id should match"
    );

    // Step 4: send empty TaskMessage to stop the app
    let stop_msg = TaskMessage::TaskUpdate {
        timestamp: "2026-06-15T00:00:01Z".to_string(),
        tenant_id: "t_test".to_string(),
        apps: HashMap::new(),
    };
    supervisor
        .handle_task_message(stop_msg)
        .await
        .expect("handle_task_message");

    // Step 5: app should be removed from state
    let gone = wait_for_app_gone(&supervisor, "test-app", 10).await;
    assert!(gone, "app should be removed from state after stop");
}

#[tokio::test]
async fn test_heartbeat_published() {
    let nats_url = nats_container().await;

    let supervisor = build_test_supervisor(&nats_url).await;

    // Build and publish a heartbeat manually
    let heartbeat = supervisor.build_heartbeat().await;
    supervisor
        .nats
        .publish_heartbeat(&supervisor.config.region, &heartbeat)
        .await
        .expect("publish heartbeat");

    // Subscribe and receive it
    let received = timeout(
        Duration::from_secs(5),
        subscribe_heartbeats(&nats_url, "test-region"),
    )
    .await
    .expect("heartbeat should arrive within 5s");

    assert_eq!(received.worker_id, "test-worker");
    assert_eq!(received.region, "test-region");
}

#[tokio::test]
async fn test_stop_all_apps() {
    let nats_url = nats_container().await;

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/internal/download/d_deploy_001"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(test_component_bytes()),
        )
        .mount(&mock_server)
        .await;

    let config = Config {
        worker_id: "test-worker".to_string(),
        region: "test-region".to_string(),
        nats_url: nats_url.clone(),
        control_plane_url: mock_server.uri(),
        cache_dir: PathBuf::from("/tmp/edge-worker-test-cache"),
        heartbeat_interval_secs: 30,
        port_cooldown_secs: 60,
        starting_port: 18_000,
    };

    let engine = edge_runtime::create_engine().expect("create engine");
    let state = Arc::new(tokio::sync::RwLock::new(WorkerState::new(engine)));
    let downloader = Arc::new(Downloader::new(
        config.control_plane_url.clone(),
        config.cache_dir.clone(),
    ));
    let port_pool = Arc::new(TokioMutex::new(PortPool::new(
        config.starting_port,
        config.port_cooldown_secs,
    )));

    let nats =
        Arc::new(NatsClientImpl::connect(&nats_url).await.expect("connect nats"))
            as Arc<dyn NatsClientTrait>;

    let supervisor = Arc::new(Supervisor {
        config,
        state,
        downloader,
        port_pool,
        nats,
    });

    // Start two apps
    for i in 0..2 {
        let spec = AppSpec {
            deployment_id: format!("d_deploy_{:03}", i),
            deployment_hash: "abc123".to_string(),
            env: HashMap::new(),
            allowlist: vec![],
        };
        let msg = TaskMessage::TaskUpdate {
            timestamp: "2026-06-15T00:00:00Z".to_string(),
            tenant_id: "t_test".to_string(),
            apps: HashMap::from([(format!("app-{}", i), spec)]),
        };
        supervisor
            .handle_task_message(msg)
            .await
            .expect("handle_task_message");
    }

    // Both should be running
    tokio::time::sleep(Duration::from_secs(2)).await;
    let state = supervisor.state.read().await;
    assert_eq!(state.apps.len(), 2, "two apps should be running");

    // stop_all_apps
    supervisor.stop_all_apps().await;

    let state = supervisor.state.read().await;
    assert!(state.apps.is_empty(), "all apps should be stopped");
}
