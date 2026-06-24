//! Domain poller: pulls `GET /api/internal/domains` from the control
//! plane every `cfg.domain_poll_interval` and applies the result to
//! the shared `RoutingTable` (via `apply_poll_snapshot`).
//!
//! This is the only path that mutates the FQDN table on the ingress
//! side. The heartbeat path mutates the upstream table; this path
//! mutates the FQDN table. They are decoupled: a heartbeat can arrive
//! between two domain polls and the FQDN route is still rendered
//! (looked up from `by_app` at render time).
//!
//! Failure mode: any HTTP / decode error is logged and the loop
//! continues to the next tick. We do NOT abort the task — a 503 from
//! the control plane for one cycle is recoverable on the next 30s
//! tick, and aborting would mean losing domain state on transient
//! outages.
//!
//! The function is exposed as `pub async fn run` so the caller can
//! spawn it with its own backoff loop, mirroring `heartbeats::run`'s
//! shape. In `main.rs` we just `tokio::spawn(async move { run(...) })`
//! because domain polling is fire-and-forget — there's no reconnect
//! semantics worth re-invoking like there are with NATS.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::sync::Notify;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::routing::RoutingTable;

/// Number of consecutive 401/403 responses after which the poller
/// gives up and returns Err. Three 30s ticks = ~90s of downtime
/// before the operator's alerting has had a chance to fire. The
/// `main.rs` task wrapper turns the returned Err into a process
/// exit, so the orchestrator restarts the ingress with a fresh
/// token from the (operator-copied) ingest token file.
const MAX_CONSECUTIVE_AUTH_ERRORS: u32 = 3;

/// Run the domain poller until the process exits. The renderer's
/// `Notify` is signalled on every successful apply so Caddy reloads
/// pick up the new FQDN routes. Errors are logged and the loop
/// continues; the function only returns Err if the reqwest client
/// itself fails to build (which is unrecoverable) OR the control
/// plane has rejected our token repeatedly (rotated JWT secret,
/// revoked ingest token, etc.) — see `MAX_CONSECUTIVE_AUTH_ERRORS`.
pub async fn run(cfg: Config, table: Arc<RoutingTable>, render_notify: Arc<Notify>) -> Result<()> {
    if cfg.control_plane_url.is_empty() {
        info!("CONTROL_PLANE_URL unset; domain poller disabled");
        return Ok(());
    }

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("building reqwest client for domain poller")?;

    let url = format!("{}/api/internal/domains", cfg.control_plane_url);
    let mut ticker = interval(cfg.domain_poll_interval);
    // Skip the first immediate tick — we want a deterministic
    // poll AFTER the NATS bring-up, not a race against it.
    ticker.tick().await;

    info!(
        %url,
        interval_secs = cfg.domain_poll_interval.as_secs(),
        "domain poller started"
    );

    let mut consecutive_auth_errors: u32 = 0;
    loop {
        ticker.tick().await;
        match fetch_and_apply(&http, &url, &cfg.service_token, &table).await {
            Ok((added, removed)) => {
                // Reset on any success — the previous auth failures
                // (if any) were transient, the token is working now.
                consecutive_auth_errors = 0;
                if !added.is_empty() || !removed.is_empty() {
                    info!(
                        added = added.len(),
                        removed = removed.len(),
                        "domain table updated"
                    );
                    debug!(?added, ?removed, "domain table diff");
                    render_notify.notify_one();
                } else {
                    debug!("domain poll: no changes");
                }
            }
            Err(e) => {
                // Distinguish auth errors (token rotated, revoked) from
                // transient errors (5xx, network). The former fail-fast
                // to surface "your token is no good" to the operator;
                // the latter retry on the next tick as before. The
                // `fetch_and_apply` error chain includes the HTTP
                // status code, so substring-matching is reliable.
                let msg = e.to_string();
                let is_auth = msg.contains("401") || msg.contains("403");
                if is_auth {
                    consecutive_auth_errors += 1;
                    if consecutive_auth_errors >= MAX_CONSECUTIVE_AUTH_ERRORS {
                        error!(
                            err = %e,
                            count = consecutive_auth_errors,
                            "domain poller got {MAX_CONSECUTIVE_AUTH_ERRORS} consecutive 401/403 — failing fast (likely rotated INGRESS_SERVICE_TOKEN); restart with the new token from the control plane's ingest token file"
                        );
                        return Err(e);
                    }
                    warn!(
                        err = %e,
                        count = consecutive_auth_errors,
                        max = MAX_CONSECUTIVE_AUTH_ERRORS,
                        "domain poll auth error; will retry"
                    );
                } else {
                    // Transient error — reset the auth-error counter
                    // so a single 503 doesn't burn the budget.
                    consecutive_auth_errors = 0;
                    warn!(err = %e, "domain poll failed; will retry on next tick");
                }
            }
        }
    }
}

/// Fetch the current domain list and apply it to the routing table.
/// Returns the `(added, removed)` diff so the caller can log churn.
///
/// This is the only function in the module that does I/O; tests
/// exercise it directly with a wiremock `MockServer`.
pub async fn fetch_and_apply(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    table: &RoutingTable,
) -> Result<(Vec<String>, Vec<String>)> {
    let resp = http
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("control plane returned {status} for {url}: {body}"));
    }

    let domains: Vec<crate::routing::Domain> = resp
        .json()
        .await
        .with_context(|| format!("decoding {url} body as Vec<Domain>"))?;

    let (added, removed) = table.apply_poll_snapshot(domains).await;
    Ok((added, removed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::Domain;
    use wiremock::matchers::{bearer_token, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[allow(dead_code)] // available for future tests; currently unused
    fn test_domain(id: &str, tenant_id: &str, app_name: &str, fqdn: &str) -> Domain {
        Domain {
            id: id.to_string(),
            tenant_id: tenant_id.to_string(),
            app_name: app_name.to_string(),
            fqdn: fqdn.to_string(),
        }
    }

    /// Happy path: wiremock serves a single domain; after
    /// `fetch_and_apply` the routing table carries that one FQDN.
    /// Without this we'd only know the poller is broken at runtime.
    #[tokio::test]
    async fn fetch_and_apply_populates_routing_table() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/internal/domains"))
            .and(bearer_token("test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": "d_1",
                    "tenant_id": "t_a",
                    "app_name": "api",
                    "fqdn": "api.acme.com"
                }
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let url = format!("{}/api/internal/domains", server.uri());
        let table = RoutingTable::new();
        let (added, removed) = fetch_and_apply(&http, &url, "test-token", &table)
            .await
            .unwrap();

        assert_eq!(added, vec!["api.acme.com".to_string()]);
        assert!(removed.is_empty());

        let snap = table.fqdn_snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].fqdn, "api.acme.com");
        assert_eq!(snap[0].tenant_id, "t_a");
        assert_eq!(snap[0].app_name, "api");
    }

    /// 503 from the control plane must surface as an Err so the
    /// poller logs a "will retry" message instead of silently
    /// carrying an empty table. (If the token is wrong the operator
    /// needs to know — empty-by-silence is the failure mode that
    /// makes for a 6-hour debugging session.)
    #[tokio::test]
    async fn fetch_and_apply_returns_err_on_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/internal/domains"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let url = format!("{}/api/internal/domains", server.uri());
        let table = RoutingTable::new();
        let err = fetch_and_apply(&http, &url, "test-token", &table)
            .await
            .expect_err("503 must surface as Err");
        // The error chain must mention the status code so the warn
        // log line tells the operator which HTTP code to look for.
        assert!(
            err.to_string().contains("503"),
            "err chain should mention 503, got: {err}"
        );
    }

    /// After two polls with different bodies, the second poll must
    /// produce a clean diff: only the FQDN that actually changed
    /// appears in `added` / `removed`. This is the regression test
    /// for the "diff vs full replace" choice — a full-replace impl
    /// would mark every FQDN as added on every tick, defeating the
    /// churn-logging design.
    #[tokio::test]
    async fn fetch_and_apply_second_poll_only_diff() {
        let server = MockServer::start().await;

        // First poll: two FQDNs.
        Mock::given(method("GET"))
            .and(path("/api/internal/domains"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "d_1", "tenant_id": "t_a", "app_name": "api", "fqdn": "api.acme.com"},
                {"id": "d_2", "tenant_id": "t_b", "app_name": "web", "fqdn": "web.acme.com"}
            ])))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let url = format!("{}/api/internal/domains", server.uri());
        let table = RoutingTable::new();

        fetch_and_apply(&http, &url, "test-token", &table)
            .await
            .unwrap();

        // Second poll: same two + one new.
        Mock::given(method("GET"))
            .and(path("/api/internal/domains"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "d_1", "tenant_id": "t_a", "app_name": "api", "fqdn": "api.acme.com"},
                {"id": "d_2", "tenant_id": "t_b", "app_name": "web", "fqdn": "web.acme.com"},
                {"id": "d_3", "tenant_id": "t_c", "app_name": "blog", "fqdn": "blog.acme.com"}
            ])))
            .mount(&server)
            .await;

        let (added, removed) = fetch_and_apply(&http, &url, "test-token", &table)
            .await
            .unwrap();
        assert_eq!(added, vec!["blog.acme.com".to_string()]);
        assert!(removed.is_empty());
    }

    /// The bearer-token gate: the control plane's
    /// `WorkerAuth` middleware checks the JWT before serving
    /// `/api/internal/domains`. A token-mismatch produces 401.
    /// This test pins that we DO send the configured token (not, e.g.,
    /// no Authorization header at all) and the assertion lives on
    /// the wiremock side via the `bearer_token` matcher.
    #[tokio::test]
    async fn fetch_and_apply_sends_configured_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/internal/domains"))
            .and(bearer_token("ingest-fra-1y"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .expect(1)
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let url = format!("{}/api/internal/domains", server.uri());
        let table = RoutingTable::new();
        fetch_and_apply(&http, &url, "ingest-fra-1y", &table)
            .await
            .unwrap();
    }

    /// Malformed JSON (e.g. a 200 with an HTML error page) must
    /// surface as a decode error, NOT silently zero out the table.
    /// Without this test, a future refactor that logs and returns
    /// Ok(()) on decode failure would erase tenant domains from the
    /// routing table on every misbehaving 200.
    #[tokio::test]
    async fn fetch_and_apply_returns_err_on_malformed_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/internal/domains"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let url = format!("{}/api/internal/domains", server.uri());
        let table = RoutingTable::new();
        let err = fetch_and_apply(&http, &url, "test-token", &table)
            .await
            .expect_err("malformed JSON must surface as Err");
        assert!(
            err.to_string().contains("decoding"),
            "err chain should mention decoding, got: {err}"
        );
    }

    /// `run` with `control_plane_url` empty returns Ok immediately
    /// (default-only mode), and `run` would normally loop forever —
    /// we don't have a test for the happy loop because it requires a
    /// controllable ticker, which `tokio::time::pause()` and a fake
    /// interval would need. The fetch_and_apply tests above cover
    /// the I/O path; this just pins the "empty URL → no-op" branch.
    #[tokio::test]
    async fn run_returns_ok_when_control_plane_url_empty() {
        let cfg = Config {
            nats_url: "nats://localhost:4222".into(),
            caddy_admin_url: "http://127.0.0.1:2019".into(),
            region: "test".into(),
            cert_file: "/tmp/c.pem".into(),
            key_file: "/tmp/k.pem".into(),
            listen_http: ":80".into(),
            listen_https: ":443".into(),
            refresh_debounce_ms: 1000,
            http_to_https: true,
            admin_token: None,
            control_plane_url: String::new(),
            service_token: "ignored".into(),
            domain_poll_interval: Duration::from_secs(30),
        };
        let table = std::sync::Arc::new(RoutingTable::new());
        let notify = std::sync::Arc::new(Notify::new());
        run(cfg, table, notify).await.unwrap();
    }

    /// Three consecutive 401/403 responses from the control plane
    /// must cause `run` to fail-fast (return Err) so the operator
    /// notices the rotated `INGRESS_SERVICE_TOKEN`. Without this,
    /// a stale token would silently keep the FQDN table empty for
    /// hours — the routes would just stop resolving, with no
    /// error in the logs beyond a recurring `warn!`.
    ///
    /// We use a real-time 50ms interval (not `tokio::time::pause`)
    /// because `pause` + `tokio::spawn` + `advance` interactions are
    /// racy: the spawned `interval` is on a different scheduling
    /// path that the test's `advance` calls don't always wake in
    /// time. 50ms × 3 ticks = 150ms wall clock, which is acceptable
    /// for a unit test.
    #[tokio::test]
    async fn run_fails_fast_after_three_consecutive_401s() {
        // The wiremock server returns 401 on every call. `.expect(3)`
        // is a tight pin on the budget constant — if
        // `MAX_CONSECUTIVE_AUTH_ERRORS` changes, this test signals
        // the change to the next reader.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/internal/domains"))
            .respond_with(ResponseTemplate::new(401))
            .expect(3)
            .mount(&server)
            .await;

        let cfg = Config {
            nats_url: "nats://localhost:4222".into(),
            caddy_admin_url: "http://127.0.0.1:2019".into(),
            region: "test".into(),
            cert_file: "/tmp/c.pem".into(),
            key_file: "/tmp/k.pem".into(),
            listen_http: ":80".into(),
            listen_https: ":443".into(),
            refresh_debounce_ms: 1000,
            http_to_https: true,
            admin_token: None,
            control_plane_url: server.uri(),
            service_token: "stale-token".into(),
            // Short interval for fast test. 50ms × 3 ticks = 150ms.
            domain_poll_interval: Duration::from_millis(50),
        };
        let table = std::sync::Arc::new(RoutingTable::new());
        let notify = std::sync::Arc::new(Notify::new());

        // Spawn `run` and wait up to 2s for it to return Err.
        let run_handle = tokio::spawn(async move { run(cfg, table, notify).await });
        let result = tokio::time::timeout(Duration::from_secs(2), run_handle)
            .await
            .expect("run loop didn't return within 2s")
            .expect("run task panicked");
        let err = result.expect_err("run must return Err on 3 consecutive 401s");
        assert!(
            err.to_string().contains("401"),
            "err chain should mention 401, got: {err}"
        );
    }
}
