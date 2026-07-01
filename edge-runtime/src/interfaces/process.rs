//! `edge:process` — environment variables, command-line args, and exit.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Prefixes of environment variables that are blocked from the guest.
/// These typically contain secrets (credentials, keys, tokens) that should
/// not be exposed to guest WASM components.
/// Prefixes and names of environment variables blocked from the guest.
/// This is a best-effort defense-in-depth filter — not exhaustive.
/// Secrets should be managed via a secrets manager in production, not env vars.
const BLOCKED_ENV_PREFIXES: &[&str] = &[
    "AWS_",
    "AZURE_",
    "EDGE_SECRET",
    "EDGE_API_KEY",
    "DATABASE_URL",
    "GCP_",
    "NATS_CREDS",
    "NATS_TOKEN",
    "REDIS_PASSWORD",
    "REDIS_URL",
    "JWT_SECRET",
    "JWT_TOKEN",
    "AUTH_TOKEN",
    "BEARER_TOKEN",
    "STRIPE_",
    "TWILIO_",
    "SENDGRID_",
    "PASSWORD",
    "SECRET",
    "PRIVATE_KEY",
    "API_KEY",
    "ACCESS_TOKEN",
    "SESSION_TOKEN",
];

/// Returns true if an env var name should not be exposed to the guest.
fn is_blocked_env_key(key: &str) -> bool {
    BLOCKED_ENV_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Filter an iterator of (key, value) pairs, removing blocked env vars.
pub fn filter_env_vars<'a>(
    iter: impl Iterator<Item = (String, String)> + 'a,
) -> impl Iterator<Item = (String, String)> + 'a {
    iter.filter(|(k, _)| !is_blocked_env_key(k))
}

/// Process state — holds per-app environment variables when used in the worker supervisor.
pub struct Process {
    env: Arc<HashMap<String, String>>,
    /// Atomic flag set when the guest calls process.exit. Non-zero = exit code requested.
    /// This allows execute_app to distinguish "guest called exit" from "wasm trap".
    exit_code: Arc<AtomicU32>,
}

impl Process {
    /// Create a new Process with environment variables from the host,
    /// excluding sensitive vars (secrets, credentials, keys).
    pub fn new() -> Self {
        Self {
            env: Arc::new(filter_env_vars(std::env::vars()).collect()),
            exit_code: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Create a Process with a specific per-app environment map (pre-filtered by caller).
    pub fn with_env(env: Arc<HashMap<String, String>>) -> Self {
        Self {
            env,
            exit_code: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Create a Process with a specific environment map and exit code flag.
    pub fn with_env_and_exit_code(
        env: Arc<HashMap<String, String>>,
        exit_code: Arc<AtomicU32>,
    ) -> Self {
        Self { env, exit_code }
    }

    pub fn get_env(&self, key: &str) -> Option<String> {
        self.env.get(key).cloned()
    }

    pub fn get_all_env(&self) -> Vec<(String, String)> {
        self.env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn get_args(&self) -> Vec<String> {
        std::env::args().collect()
    }

    /// Returns the current working directory of the host process.
    pub fn get_cwd(&self) -> Result<String, String> {
        std::env::current_dir()
            .and_then(|p| {
                p.into_os_string().into_string().map_err(|os| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("current directory path is not valid UTF-8: {:?}", os),
                    )
                })
            })
            .map_err(|e| e.to_string())
    }

    /// Called by the guest WASM component via the `exit` host function.
    /// Stores the exit code in an atomic flag and returns normally — the wasmtime
    /// trap that follows will cause `call()` to return Err, which we distinguish
    /// from a real error by checking this flag after a successful call return.
    pub fn exit(&self, code: u32) {
        self.exit_code.store(code, Ordering::SeqCst);
    }

    /// Returns `Some(code)` if the guest called process.exit, `None` otherwise.
    pub fn exit_requested(&self) -> Option<u32> {
        let code = self.exit_code.load(Ordering::SeqCst);
        if code == 0 {
            None
        } else {
            Some(code)
        }
    }
}

impl Default for Process {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_env() -> Arc<HashMap<String, String>> {
        Arc::new(
            [
                ("FOO".into(), "bar".into()),
                ("EDGE_VAR".into(), "value".into()),
            ]
            .into_iter()
            .collect(),
        )
    }

    #[test]
    fn test_get_env_existing() {
        let process = Process::with_env(test_env());
        assert_eq!(process.get_env("FOO"), Some("bar".into()));
    }

    #[test]
    fn test_get_env_missing() {
        let process = Process::with_env(test_env());
        assert_eq!(process.get_env("DOES_NOT_EXIST"), None);
    }

    #[test]
    fn test_get_all_env() {
        let process = Process::with_env(test_env());
        let all = process.get_all_env();
        let keys: std::collections::HashSet<_> = all.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains("FOO"));
        assert!(keys.contains("EDGE_VAR"));
    }

    #[test]
    fn test_exit_stores_code() {
        let env = test_env();
        let exit_code = Arc::new(AtomicU32::new(0));
        let process = Process::with_env_and_exit_code(env, exit_code.clone());
        assert_eq!(process.exit_requested(), None);
        process.exit(42);
        assert_eq!(process.exit_requested(), Some(42));
    }

    #[test]
    fn test_exit_code_persists_across_calls() {
        let env = test_env();
        let exit_code = Arc::new(AtomicU32::new(0));
        let process = Process::with_env_and_exit_code(env, exit_code.clone());
        process.exit(1);
        process.exit(2); // second call should overwrite
        assert_eq!(process.exit_requested(), Some(2));
    }

    #[test]
    fn test_get_cwd_returns_absolute_path() {
        let process = Process::new();
        let cwd = process.get_cwd().expect("get_cwd should succeed");
        assert!(
            std::path::Path::new(&cwd).is_absolute(),
            "cwd should be an absolute path, got: {:?}",
            cwd
        );
    }

    #[test]
    fn test_get_cwd_succeeds_in_normal_envs() {
        // In normal test environments the cwd is always valid and readable.
        // The API is fallible (e.g., cwd deleted at runtime) but untestable in unit tests.
        let process = Process::new();
        assert!(process.get_cwd().is_ok());
    }

    // ── get_args ────────────────────────────────────────────────────────

    #[test]
    fn test_get_args_returns_at_least_binary_path() {
        let process = Process::new();
        let args = process.get_args();
        assert!(!args.is_empty(), "args should contain at least the binary path");
        // The first argument is the test binary path. It should exist
        // and be an absolute path in normal test environments.
        let first = &args[0];
        assert!(!first.is_empty(), "first arg (binary path) should not be empty");
    }

    // ── filter_env_vars blocklist ───────────────────────────────────────

    #[test]
    fn test_filter_env_vars_blocks_aws_prefix() {
        let raw = vec![
            ("AWS_ACCESS_KEY_ID".into(), "AKID123".into()),
            ("AWS_SECRET_KEY".into(), "sk_test".into()),
            ("SAFE_VAR".into(), "hello".into()),
        ];
        let filtered: HashMap<_, _> = filter_env_vars(raw.into_iter()).collect();
        assert!(!filtered.contains_key("AWS_ACCESS_KEY_ID"));
        assert!(!filtered.contains_key("AWS_SECRET_KEY"));
        assert_eq!(filtered.get("SAFE_VAR"), Some(&"hello".into()));
    }

    #[test]
    fn test_filter_env_vars_blocks_edge_secrets() {
        let raw = vec![
            ("EDGE_SECRET_KEY".into(), "secret".into()),
            ("EDGE_API_KEY".into(), "sk_test".into()),
        ];
        let filtered: Vec<_> = filter_env_vars(raw.into_iter()).collect();
        assert!(filtered.is_empty(), "EDGE_SECRET and EDGE_API_KEY should be blocked");
    }

    #[test]
    fn test_filter_env_vars_blocks_database_url() {
        let raw = vec![("DATABASE_URL".into(), "postgres://...".into())];
        let filtered: Vec<_> = filter_env_vars(raw.into_iter()).collect();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_env_vars_blocks_stripe_keys() {
        let raw = vec![("STRIPE_API_KEY".into(), "sk_live_xxx".into())];
        let filtered: Vec<_> = filter_env_vars(raw.into_iter()).collect();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_env_vars_blocks_secret_keyword() {
        // Vars whose name STARTS WITH "SECRET" are blocked (prefix match).
        let raw = vec![("SECRET_VALUE".into(), "classified".into())];
        let filtered: Vec<_> = filter_env_vars(raw.into_iter()).collect();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_env_vars_blocks_api_key_keyword() {
        let raw = vec![("API_KEY".into(), "abc123".into())];
        let filtered: Vec<_> = filter_env_vars(raw.into_iter()).collect();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_env_vars_passes_safe_vars() {
        let raw = vec![
            ("PATH".into(), "/usr/bin".into()),
            ("HOME".into(), "/root".into()),
            ("MY_APP_VAR".into(), "value".into()),
        ];
        let filtered: HashMap<_, _> = filter_env_vars(raw.into_iter()).collect();
        assert_eq!(filtered.len(), 3);
        assert!(filtered.contains_key("PATH"));
        assert!(filtered.contains_key("HOME"));
        assert!(filtered.contains_key("MY_APP_VAR"));
    }

    #[test]
    fn test_filter_env_vars_allows_non_blocked_secret_like_names() {
        // The blocklist uses starts_with, so vars that contain "SECRET"
        // but don't start with it are NOT blocked (defense-in-depth only).
        let raw = vec![("MY_SECRET_KEY".into(), "val".into())];
        let filtered: HashMap<_, _> = filter_env_vars(raw.into_iter()).collect();
        assert!(filtered.contains_key("MY_SECRET_KEY"));
    }
}
