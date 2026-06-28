//! Shared integration-test helpers for edge-worker (and friends).
//!
//! Extracted from `edge-worker/tests/integration_tests.rs` and
//! `edge-worker/tests/ingress_wire_integration.rs` so the byte-identical
//! `should_skip_integration_tests` + `nats_container` helpers and the
//! `Supervisor`-wiring logic live in one place. The crate is `publish = false`
//! and intended only for test consumption via `[dev-dependencies]`.
//!
//! Out of scope (separate follow-up): migrating `edge-ingress/tests/integration.rs`,
//! whose `Config` is `edge_ingress::config::Config` (a different type than
//! `edge_worker::config::Config`). Only `should_skip_integration_tests` and
//! `nats_container` apply there.

// Public surface — re-exports so consumers can write
// `use edge_test_helpers::{Config, Supervisor, build_supervisor, test_config, ...};`
// in one line.
pub mod reexports;

pub mod nats;
pub mod supervisor;

// Convenience re-exports at the crate root.
pub use reexports::{Config, Supervisor};
pub use supervisor::{build_supervisor, test_config};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes env-mutating tests so concurrent test threads don't
    /// stomp on each other's env-var values. Same pattern as
    /// `edge-worker/src/config.rs::tests::ENV_LOCK`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// `should_skip_integration_tests` checks three things:
    /// `SKIP_INTEGRATION_TESTS=1`, `CI=1`, and the docker socket file.
    /// We can only safely mutate the env vars in this test; the docker
    /// socket depends on the host and is left alone.
    #[test]
    fn should_skip_integration_tests_returns_bool() {
        // Save and clear both env vars to get a clean baseline.
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_skip = std::env::var("SKIP_INTEGRATION_TESTS").ok();
        let prev_ci = std::env::var("CI").ok();
        // SAFETY: serialized by ENV_LOCK above.
        unsafe {
            std::env::remove_var("SKIP_INTEGRATION_TESTS");
            std::env::remove_var("CI");
        }

        // No skip vars set: result depends on whether docker socket
        // exists on this host. We don't assert a specific value — we
        // only assert the function is callable and returns a bool.
        let _result: bool = nats::should_skip_integration_tests();

        // With SKIP_INTEGRATION_TESTS=1 the function must return true
        // regardless of docker socket.
        // SAFETY: serialized by ENV_LOCK above.
        unsafe { std::env::set_var("SKIP_INTEGRATION_TESTS", "1") };
        assert!(
            nats::should_skip_integration_tests(),
            "SKIP_INTEGRATION_TESTS=1 must force skip=true"
        );

        // With CI=1 the function must also return true.
        // SAFETY: serialized by ENV_LOCK above.
        unsafe { std::env::set_var("CI", "1") };
        assert!(
            nats::should_skip_integration_tests(),
            "CI=1 must force skip=true"
        );

        // Restore.
        // SAFETY: serialized by ENV_LOCK above.
        match prev_skip {
            Some(v) => unsafe { std::env::set_var("SKIP_INTEGRATION_TESTS", v) },
            None => unsafe { std::env::remove_var("SKIP_INTEGRATION_TESTS") },
        }
        match prev_ci {
            Some(v) => unsafe { std::env::set_var("CI", v) },
            None => unsafe { std::env::remove_var("CI") },
        }
    }

    /// `test_config` populates fields the test doesn't override with
    /// sensible defaults. This pins those defaults so a future
    /// regression that changes them shows up in CI.
    #[test]
    fn test_config_uses_sensible_defaults() {
        let cfg = test_config(
            "w_smoke",
            "fra",
            "nats://localhost:4222".to_string(),
            "http://localhost:9999".to_string(),
        );
        assert_eq!(cfg.worker_id, "w_smoke");
        assert_eq!(cfg.region, "fra");
        assert_eq!(cfg.nats_url, "nats://localhost:4222");
        assert_eq!(cfg.control_plane_url, "http://localhost:9999");
        // Defaults that the original `build_supervisor` in
        // integration_tests.rs used; pinning them prevents accidental
        // regression.
        assert_eq!(cfg.health_check_timeout_secs, 60);
        assert_eq!(cfg.port_cooldown_secs, 60);
        assert_eq!(cfg.max_memory_mb, 256);
        assert_eq!(cfg.epoch_tick_ms, 10);
        assert_eq!(cfg.epoch_deadline_ticks, 100);
        assert_eq!(cfg.starting_port, 18_000);
        assert_eq!(cfg.heartbeat_interval_secs, 30);
        assert_eq!(cfg.worker_jwt_issuer, "edgecloud");
        assert_eq!(cfg.worker_tenant_id, "t_test");
        assert_eq!(cfg.nats_max_deliver, 20);
        // worker_addr is a placeholder; the test can override before
        // passing to build_supervisor.
        assert!(!cfg.worker_addr.is_empty());
    }

    /// R4: `spool_dir` is per-process (suffixed with `process::id`)
    /// so parallel test workers don't cross-contaminate their spools.
    /// Within a single process the suffix is constant, so two
    /// consecutive `test_config` calls produce the same path —
    /// what matters is that two distinct processes would produce
    /// distinct paths (which we can't easily test in-process).
    /// We assert the suffix is present and well-formed.
    #[test]
    fn test_config_spool_dir_is_per_process() {
        let cfg = test_config(
            "w_smoke",
            "fra",
            "nats://localhost:4222".to_string(),
            "http://localhost:9999".to_string(),
        );
        let pid = std::process::id();
        let expected_suffix = format!("edge-worker-test-spool-{pid}");
        let spool_path = cfg.spool_dir.to_string_lossy().into_owned();
        assert!(
            spool_path.ends_with(&expected_suffix),
            "spool_dir must end with per-process suffix; got {spool_path}, expected suffix {expected_suffix}"
        );
        // Sanity: the path is under temp_dir (not the prior
        // hardcoded /tmp/edge-worker-test-spool).
        let temp_root = std::env::temp_dir().to_string_lossy().into_owned();
        assert!(
            spool_path.starts_with(&temp_root),
            "spool_dir must live under std::env::temp_dir(); got {spool_path}, root {temp_root}"
        );
    }

    /// R5: `build_supervisor` honors `Config.worker_bootstrap_psk`.
    /// Pre-R5, the helper silently constructed `WorkerJwtSigner::new`
    /// (static secret path) regardless of whether `worker_bootstrap_psk`
    /// was set. This test introspects the resulting `Supervisor`'s
    /// `log_forwarder.jwt_signer` and verifies the signer's
    /// `TokenSource` is the Callback variant when a PSK is configured.
    ///
    /// The supervisor's `log_forwarder` field is private; the test
    /// uses `build_signer_for_config` (pub(crate)) directly to
    /// exercise the same code path that `build_supervisor` takes.
    /// The signer doesn't fire the callback here — the test only
    /// verifies the constructor chose the right path.
    ///
    /// `sign()` returns Err when the callback fails (no wiremock
    /// running), Ok(token) when the static-secret path succeeds.
    /// The test asserts `Err` to prove the Callback path was
    /// chosen.
    ///
    /// Implementation note: tokio's "Cannot start a runtime from
    /// within a runtime" panic means we must run on a thread with
    /// no active tokio context. `cargo test`'s default test
    /// harness doesn't install one, but sibling tests may leak
    /// context. We spawn a fresh OS thread and build a runtime
    /// there; the closure itself also builds its own runtime
    /// (see `supervisor.rs::build_signer_for_config`), so the
    /// nested-runtime panic doesn't apply.
    #[test]
    fn build_signer_for_config_uses_callback_when_psk_set() {
        let mut cfg = test_config(
            "w_smoke",
            "fra",
            "nats://localhost:4222".to_string(),
            "http://localhost:9999".to_string(),
        );
        cfg.worker_bootstrap_psk = Some("test-psk-32-bytes-long-aaaaaaaaaaaa".into());

        let signer = supervisor::build_signer_for_config(&cfg);

        // The closure builds its own runtime (no `try_current`
        // gymnastics), so we can call sign() from any context,
        // including the test thread's. No thread spawn needed.
        let result = signer.sign();
        assert!(
            result.is_err(),
            "signer must use the callback path when worker_bootstrap_psk is set; got Ok"
        );
    }

    /// R5 (negative case): when `worker_bootstrap_psk` is None,
    /// the signer uses the legacy static-secret path and `sign()`
    /// succeeds (returning a valid JWT).
    #[test]
    fn build_signer_for_config_uses_static_when_psk_unset() {
        let cfg = test_config(
            "w_smoke",
            "fra",
            "nats://localhost:4222".to_string(),
            "http://localhost:9999".to_string(),
        );
        assert!(cfg.worker_bootstrap_psk.is_none());
        let signer = supervisor::build_signer_for_config(&cfg);
        let token = signer.sign().expect("static-secret signer must succeed");
        assert!(
            !token.is_empty() && token.contains('.'),
            "static-secret signer must produce a JWT-shaped token (3 dot-separated segments); got: {token:?}"
        );
    }
}
