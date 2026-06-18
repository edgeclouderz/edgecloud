//! Worker configuration loaded from environment variables.

use anyhow::Context;
use std::path::PathBuf;

// `max_memory_mb`, `epoch_tick_ms`, and `epoch_deadline_ticks` are read from
// env vars and consumed by the supervisor (PR #64 follow-up). They plumb
// per-app wasmtime limits: StoreLimits for memory, and an epoch ticker +
// deadline for CPU budgets. The previous PR deferred the wiring; this PR
// closes the loop and removes the dead_code allow.
#[derive(Debug, Clone)]
pub struct Config {
    pub worker_id: String,
    pub region: String,
    pub nats_url: String,
    pub control_plane_url: String,
    pub cache_dir: PathBuf,
    pub heartbeat_interval_secs: u64,
    pub health_check_timeout_secs: u64,
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
            health_check_timeout_secs: std::env::var("EDGE_HEALTH_CHECK_TIMEOUT_SECS")
                .unwrap_or_else(|_| "60".into())
                .parse()
                .unwrap_or(60),
            port_cooldown_secs: 60,
            starting_port: 8081,
            max_memory_mb: parse_env_u64("APP_MAX_MEMORY_MB", 256)?,
            epoch_tick_ms: parse_env_u64("EPOCH_TICK_MS", 10)?,
            epoch_deadline_ticks: parse_env_u64("EPOCH_DEADLINE_TICKS", 100)?,
        })
    }
}

/// Parse an integer-valued environment variable, falling back to `default`
/// when unset. Returns an error (rather than silently using the default) when
/// the variable is set but not a valid non-negative integer — operators
/// debugging a misconfiguration prefer a startup failure over a mystery
/// default.
fn parse_env_u64(name: &str, default: u64) -> anyhow::Result<u64> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(s) => s
            .parse::<u64>()
            .with_context(|| format!("{} must be a non-negative integer (got {:?})", name, s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes env-mutating tests. The Rust test runner executes tests in
    /// parallel by default; without this lock concurrent tests would stomp on
    /// each other's env-var values and produce flaky failures.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that sets an env var for the duration of a test and
    /// restores its previous value on Drop. Holds `ENV_LOCK` for the test's
    /// lifetime so env mutations don't race.
    struct EnvGuard {
        key: String,
        prev: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var(key).ok();
            // Safety: serialized via ENV_LOCK above.
            unsafe { std::env::set_var(key, value) };
            Self {
                key: key.to_string(),
                prev,
                _lock: lock,
            }
        }

        fn unset(key: &str) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var(key).ok();
            unsafe { std::env::remove_var(key) };
            Self {
                key: key.to_string(),
                prev,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => unsafe { std::env::set_var(&self.key, v) },
                None => unsafe { std::env::remove_var(&self.key) },
            }
        }
    }

    #[test]
    fn parse_env_u64_returns_default_when_unset() {
        let _g = EnvGuard::unset("EDGE_TEST_VAR");
        assert_eq!(parse_env_u64("EDGE_TEST_VAR", 42).unwrap(), 42);
    }

    #[test]
    fn parse_env_u64_parses_valid_value() {
        let _g = EnvGuard::set("EDGE_TEST_VAR", "1024");
        assert_eq!(parse_env_u64("EDGE_TEST_VAR", 42).unwrap(), 1024);
    }

    #[test]
    fn parse_env_u64_parses_zero() {
        let _g = EnvGuard::set("EDGE_TEST_VAR", "0");
        assert_eq!(parse_env_u64("EDGE_TEST_VAR", 42).unwrap(), 0);
    }

    #[test]
    fn parse_env_u64_errors_on_non_integer() {
        let _g = EnvGuard::set("EDGE_TEST_VAR", "hello");
        let err = parse_env_u64("EDGE_TEST_VAR", 42).unwrap_err();
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("EDGE_TEST_VAR"),
            "error should name the var: {}",
            msg
        );
        assert!(
            msg.contains("hello"),
            "error should include the bad value: {}",
            msg
        );
    }

    #[test]
    fn parse_env_u64_errors_on_negative_string() {
        let _g = EnvGuard::set("EDGE_TEST_VAR", "-1");
        let err = parse_env_u64("EDGE_TEST_VAR", 42).unwrap_err();
        // u64 can't represent -1, so we expect a parse error.
        assert!(format!("{:#}", err).contains("EDGE_TEST_VAR"));
    }

    /// `Config::from_env` requires WORKER_ID, REGION, and CONTROL_PLANE_URL
    /// to be set. Tests that exercise the full `from_env` path need to set
    /// all three; missing any of them produces a clear error.
    ///
    /// These tests set env vars directly under a single manual ENV_LOCK
    /// acquisition. The existing EnvGuard helper takes the lock internally
    /// and is non-reentrant, so creating more than one EnvGuard per test
    /// deadlocks. Direct mutation under a held lock is the only safe
    /// pattern for tests that need several env vars.
    fn lock_and_set(vars: &[(&str, Option<&str>)]) -> std::sync::MutexGuard<'static, ()> {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for (k, v) in vars {
            match v {
                Some(s) => unsafe { std::env::set_var(k, s) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
        lock
    }

    /// `Config::from_env` reads APP_MAX_MEMORY_MB and passes it to the
    /// supervisor's create_store call. Without this test, the field could
    /// regress to a hardcoded 256 (the previous behavior) and the
    /// env-var knob would become decorative.
    #[test]
    fn config_from_env_reads_max_memory_mb() {
        let _g = lock_and_set(&[
            ("WORKER_ID", Some("w_test_abc")),
            ("REGION", Some("fra")),
            ("CONTROL_PLANE_URL", Some("http://localhost:8080")),
            ("APP_MAX_MEMORY_MB", Some("64")),
        ]);
        let cfg = Config::from_env().expect("from_env");
        assert_eq!(cfg.max_memory_mb, 64, "APP_MAX_MEMORY_MB should be 64");
    }

    /// EPOCH_TICK_MS and EPOCH_DEADLINE_TICKS together define the per-app
    /// CPU budget. The supervisor spawns a ticker at EPOCH_TICK_MS and
    /// sets a deadline of EPOCH_DEADLINE_TICKS — defaults of 10 ms and
    /// 100 ticks yield a ~1 s budget per call.
    #[test]
    fn config_from_env_reads_epoch_fields() {
        let _g = lock_and_set(&[
            ("WORKER_ID", Some("w_test_abc")),
            ("REGION", Some("fra")),
            ("CONTROL_PLANE_URL", Some("http://localhost:8080")),
            ("EPOCH_TICK_MS", Some("5")),
            ("EPOCH_DEADLINE_TICKS", Some("50")),
        ]);
        let cfg = Config::from_env().expect("from_env");
        assert_eq!(cfg.epoch_tick_ms, 5, "EPOCH_TICK_MS should be 5");
        assert_eq!(
            cfg.epoch_deadline_ticks, 50,
            "EPOCH_DEADLINE_TICKS should be 50"
        );
    }

    /// When the env vars are unset, the defaults (256 / 10 / 100) take
    /// effect. Pinning the defaults in a test catches accidental
    /// regressions where a future refactor changes the fallback.
    #[test]
    fn config_from_env_uses_defaults_when_unset() {
        let _g = lock_and_set(&[
            ("WORKER_ID", Some("w_test_abc")),
            ("REGION", Some("fra")),
            ("CONTROL_PLANE_URL", Some("http://localhost:8080")),
            ("APP_MAX_MEMORY_MB", None),
            ("EPOCH_TICK_MS", None),
            ("EPOCH_DEADLINE_TICKS", None),
        ]);
        let cfg = Config::from_env().expect("from_env");
        assert_eq!(cfg.max_memory_mb, 256, "default max_memory_mb is 256");
        assert_eq!(cfg.epoch_tick_ms, 10, "default epoch_tick_ms is 10");
        assert_eq!(
            cfg.epoch_deadline_ticks, 100,
            "default epoch_deadline_ticks is 100"
        );
    }
}
