//! Per-deployment filesystem preopen policy for wasi:filesystem.
//!
//! Reads `EDGE_FS_SCRATCH_PATH` from the host environment and constructs a
//! per-deployment directory `{EDGE_FS_SCRATCH_PATH}/{tenant_id}/{deployment_id}/`
//! that the guest can access as its root via the WASI preopens mechanism.
//! If the env var is absent the guest sees an empty filesystem — the safe default.
//!
//! The directory is scoped per-deployment (not per-tenant) so that cleanup on
//! app stop does not affect other deployments of the same tenant, and so that
//! canary slots do not share mutable state.

use std::io;
use std::path::PathBuf;
use thiserror::Error;

const ENV_FS_SCRATCH_PATH: &str = "EDGE_FS_SCRATCH_PATH";
const ENV_FS_MAX_MB: &str = "EDGE_FS_MAX_MB";
const DEFAULT_MAX_MB: u64 = 512;

#[derive(Debug, Error)]
pub enum FilesystemError {
    #[error("invalid tenant_id {0:?} — contains path-traversal characters")]
    InvalidTenantId(String),
    #[error("invalid deployment_id {0:?} — contains path-traversal characters")]
    InvalidDeploymentId(String),
    #[error("EDGE_FS_SCRATCH_PATH must be an absolute path, got {0:?}")]
    RelativePath(String),
    #[error("tenant scratch dir exceeds EDGE_FS_MAX_MB={0} MB limit")]
    QuotaExceeded(u64),
    #[error("failed to create tenant scratch directory: {0}")]
    Io(#[from] io::Error),
}

/// Returns the per-deployment scratch directory path, creating it if absent.
/// Returns `Ok(None)` when `EDGE_FS_SCRATCH_PATH` is not set (no-FS mode).
/// Returns `Err` for an unsafe id, a relative base path, or IO failures.
pub fn scratch_dir_for_deployment(
    tenant_id: &str,
    deployment_id: &str,
) -> Result<Option<PathBuf>, FilesystemError> {
    if !super::is_safe_tenant_id(tenant_id) {
        return Err(FilesystemError::InvalidTenantId(tenant_id.to_string()));
    }
    if !super::is_safe_tenant_id(deployment_id) {
        return Err(FilesystemError::InvalidDeploymentId(deployment_id.to_string()));
    }
    let base_str = match std::env::var(ENV_FS_SCRATCH_PATH) {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };
    let base = PathBuf::from(&base_str);
    if !base.is_absolute() {
        return Err(FilesystemError::RelativePath(base_str));
    }
    let tenant_dir = base.join(tenant_id).join(deployment_id);
    std::fs::create_dir_all(&tenant_dir)?;

    // Pre-admission quota check: if the dir already exists and exceeds the
    // configured cap, refuse to open it so the guest cannot continue accumulating.
    // Full enforcement at write-time requires OS-level quotas.
    let max_mb = std::env::var(ENV_FS_MAX_MB)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_MB);
    let used_mb = dir_size_mb(&tenant_dir);
    if used_mb > max_mb {
        return Err(FilesystemError::QuotaExceeded(max_mb));
    }

    Ok(Some(tenant_dir))
}

/// Remove the per-deployment scratch directory. Called by the supervisor on
/// app stop to prevent cross-request data accumulation. Ignores `NotFound`.
pub fn cleanup_scratch_dir_for_deployment(tenant_id: &str, deployment_id: &str) {
    let base = match std::env::var(ENV_FS_SCRATCH_PATH) {
        Ok(p) => p,
        Err(_) => return,
    };
    let path = PathBuf::from(base).join(tenant_id).join(deployment_id);
    if let Err(e) = std::fs::remove_dir_all(&path) {
        if e.kind() != io::ErrorKind::NotFound {
            tracing::warn!("failed to clean scratch dir {:?}: {}", path, e);
        }
    }
}

fn dir_size_mb(path: &std::path::Path) -> u64 {
    let mut total: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total += meta.len();
                } else if meta.is_dir() {
                    total += dir_size_mb(&entry.path()) * 1024 * 1024;
                }
            }
        }
    }
    total / (1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    #[test]
    #[serial]
    fn returns_none_when_env_unset() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        assert!(scratch_dir_for_deployment("t_abc", "d_001").unwrap().is_none());
    }

    #[test]
    #[serial]
    fn rejects_path_traversal_tenant_id() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        assert!(matches!(
            scratch_dir_for_deployment("../evil", "d_001"),
            Err(FilesystemError::InvalidTenantId(_))
        ));
    }

    #[test]
    #[serial]
    fn rejects_path_traversal_deployment_id() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        assert!(matches!(
            scratch_dir_for_deployment("t_abc", "../evil"),
            Err(FilesystemError::InvalidDeploymentId(_))
        ));
    }

    #[test]
    #[serial]
    fn rejects_empty_tenant_id() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        assert!(matches!(
            scratch_dir_for_deployment("", "d_001"),
            Err(FilesystemError::InvalidTenantId(_))
        ));
    }

    #[test]
    #[serial]
    fn rejects_windows_reserved_names() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        for name in ["CON", "NUL", "PRN", "COM1", "LPT9"] {
            assert!(
                matches!(
                    scratch_dir_for_deployment(name, "d_001"),
                    Err(FilesystemError::InvalidTenantId(_))
                ),
                "{name} should be rejected"
            );
        }
    }

    #[test]
    #[serial]
    fn rejects_relative_scratch_path() {
        env::set_var(ENV_FS_SCRATCH_PATH, "relative/path");
        let result = scratch_dir_for_deployment("t_abc", "d_001");
        env::remove_var(ENV_FS_SCRATCH_PATH);
        assert!(matches!(result, Err(FilesystemError::RelativePath(_))));
    }

    #[test]
    #[serial]
    fn creates_per_deployment_dir_when_env_set() {
        let tmp = tempfile::tempdir().expect("tempdir");
        env::set_var(ENV_FS_SCRATCH_PATH, tmp.path());
        let result = scratch_dir_for_deployment("t_tenant1", "d_deploy1").unwrap();
        env::remove_var(ENV_FS_SCRATCH_PATH);
        let dir = result.expect("should return Some");
        assert!(dir.exists(), "deployment dir should be created");
        assert_eq!(dir, tmp.path().join("t_tenant1").join("d_deploy1"));
    }

    #[test]
    #[serial]
    fn cleanup_removes_deployment_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        env::set_var(ENV_FS_SCRATCH_PATH, tmp.path());
        scratch_dir_for_deployment("t_abc", "d_002").unwrap();
        let dir = tmp.path().join("t_abc").join("d_002");
        assert!(dir.exists());
        cleanup_scratch_dir_for_deployment("t_abc", "d_002");
        env::remove_var(ENV_FS_SCRATCH_PATH);
        assert!(!dir.exists(), "cleanup should remove the directory");
    }

    #[test]
    #[serial]
    fn cleanup_is_noop_when_env_unset() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        // Should not panic or error.
        cleanup_scratch_dir_for_deployment("t_abc", "d_003");
    }
}
