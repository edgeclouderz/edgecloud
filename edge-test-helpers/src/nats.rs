//! NATS container + skip-predicate helpers shared by every integration test.
//!
//! `should_skip_integration_tests` and `nats_container` were previously
//! inlined as byte-identical copies in three test files
//! (`edge-worker/tests/integration_tests.rs`,
//! `edge-worker/tests/ingress_wire_integration.rs`,
//! `edge-ingress/tests/integration.rs` — the latter named it
//! `start_nats` instead of `nats_container`). Centralizing them means a
//! future change to the skip policy or the NATS startup contract lands
//! in one place.

use std::time::Duration;

use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerRequest;
use testcontainers::ImageExt;
use testcontainers_modules::nats::Nats;

/// Returns true if integration tests should be skipped.
///
/// Skip conditions (any one is sufficient):
/// - `SKIP_INTEGRATION_TESTS=1` is set in the environment
/// - `CI=1` is set (CI environments typically lack the Docker socket
///   the tests need, and even when present, container start-up is
///   unreliable in shared CI infrastructure)
/// - `/var/run/docker.sock` is absent (no Docker daemon reachable;
///   this is the primary skip path on developer laptops without Docker)
pub fn should_skip_integration_tests() -> bool {
    std::env::var("SKIP_INTEGRATION_TESTS").is_ok()
        || std::env::var("CI").is_ok()
        || !std::path::Path::new("/var/run/docker.sock").exists()
}

/// Start a NATS container (testcontainers) and return the container
/// handle plus a `host:port` URL suitable for `async_nats::connect`.
///
/// Caller is responsible for keeping the container alive — typically
/// by binding it to a struct field (e.g., `TestHarness._nats_container`)
/// that drops at end of test. If the container is dropped early, the
/// NATS server terminates and the `nats_url` becomes useless.
///
/// Implementation notes:
/// - `startup_timeout = 30s` bounds the total wait; without it, a
///   failed container start could hang the test indefinitely.
/// - `WaitFor::Duration { 5s }` is a coarse time-based wait used in
///   place of stderr matching, which is unreliable in CI where NATS
///   log messages may appear out of order with the test's stdout
///   capture.
pub async fn nats_container() -> (testcontainers::ContainerAsync<Nats>, String) {
    let container: testcontainers::ContainerAsync<Nats> = ContainerRequest::from(Nats::default())
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
