//! Per-tenant filesystem preopen policy for wasi:filesystem.
//!
//! Reads `EDGE_FS_SCRATCH_PATH` from the host environment and constructs a
//! per-tenant directory `{EDGE_FS_SCRATCH_PATH}/{tenant_id}/` that the guest
//! can access as its root via the WASI preopens mechanism.  If the env var is
//! absent the guest sees an empty filesystem — the safe default.

use std::io;
use std::path::PathBuf;
use thiserror::Error;

const ENV_FS_SCRATCH_PATH: &str = "EDGE_FS_SCRATCH_PATH";

#[derive(Debug, Error)]
pub enum FilesystemError {
    #[error("invalid tenant_id {0:?} — contains path-traversal characters")]
    InvalidTenantId(String),
    #[error("failed to create tenant scratch directory: {0}")]
    Io(#[from] io::Error),
}

/// Returns `Ok(None)` when `EDGE_FS_SCRATCH_PATH` is not set (ephemeral / no-FS mode).
/// Returns `Ok(Some(path))` with the created per-tenant directory otherwise.
/// Returns `Err` for unsafe `tenant_id` or IO failures.
pub fn scratch_dir_for_tenant(tenant_id: &str) -> Result<Option<PathBuf>, FilesystemError> {
    if !super::is_safe_tenant_id(tenant_id) {
        return Err(FilesystemError::InvalidTenantId(tenant_id.to_string()));
    }
    let base = match std::env::var(ENV_FS_SCRATCH_PATH) {
        Ok(p) => PathBuf::from(p),
        Err(_) => return Ok(None),
    };
    let tenant_dir = base.join(tenant_id);
    std::fs::create_dir_all(&tenant_dir)?;
    Ok(Some(tenant_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn returns_none_when_env_unset() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        assert!(scratch_dir_for_tenant("t_abc").unwrap().is_none());
    }

    #[test]
    fn rejects_path_traversal_tenant_id() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        assert!(matches!(
            scratch_dir_for_tenant("../evil"),
            Err(FilesystemError::InvalidTenantId(_))
        ));
    }

    #[test]
    fn rejects_empty_tenant_id() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        assert!(matches!(
            scratch_dir_for_tenant(""),
            Err(FilesystemError::InvalidTenantId(_))
        ));
    }

    #[test]
    fn rejects_windows_reserved_names() {
        env::remove_var(ENV_FS_SCRATCH_PATH);
        for name in ["CON", "NUL", "PRN", "COM1", "LPT9"] {
            assert!(
                matches!(
                    scratch_dir_for_tenant(name),
                    Err(FilesystemError::InvalidTenantId(_))
                ),
                "{name} should be rejected"
            );
        }
    }

    #[test]
    fn creates_tenant_dir_when_env_set() {
        let tmp = tempfile::tempdir().expect("tempdir");
        env::set_var(ENV_FS_SCRATCH_PATH, tmp.path());
        let result = scratch_dir_for_tenant("t_tenant1").unwrap();
        assert!(result.is_some());
        let dir = result.unwrap();
        assert!(dir.exists(), "tenant dir should be created");
        assert_eq!(dir, tmp.path().join("t_tenant1"));
        env::remove_var(ENV_FS_SCRATCH_PATH);
    }
}
