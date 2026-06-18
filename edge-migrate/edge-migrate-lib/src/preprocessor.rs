//! C preprocessor expansion via `clang -E`.
//!
//! When the analyzer has a preprocessor attached, source is first expanded
//! with `clang -E -P -nostdinc` before tree-sitter analysis. This catches
//! POSIX patterns hidden behind macros like
//! `#define socket(x) make_socket(x)`.
//!
//! Silent fallback: when `clang` is not reachable, `Preprocessor::discover`
//! returns `None` and the analyzer uses the unexpanded source as-is.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors from the preprocessor.
#[derive(Debug, Error)]
pub enum PreprocessError {
    /// `clang` returned non-zero exit status.
    #[error("clang -E failed: {0}")]
    ClangFailed(String),

    /// `clang` stdout could not be decoded as UTF-8.
    #[error("clang output is not valid UTF-8")]
    NotUtf8,

    /// I/O error spawning `clang`.
    #[error("failed to spawn clang: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of a successful expansion.
#[derive(Debug, Clone)]
pub struct ExpandedSource {
    /// The fully expanded C source (one logical line per output line).
    pub text: String,
    /// Maps each expanded line (0-indexed) to the original source line
    /// (1-indexed). `line_map[i]` is the original line that produced
    /// expanded line `i + 1`. When `# <lineno> "<file>"` markers are
    /// absent or cannot be parsed, the entry is `i + 1` (identity).
    pub line_map: Vec<u32>,
    /// Number of macro substitutions observed (heuristic, may be 0).
    pub macros_expanded: usize,
}

/// Summary of preprocessing metadata, attached to reports.
#[derive(Debug, Clone)]
pub struct PreprocessorInfo {
    /// Version reported by `clang --version` (best-effort).
    pub clang_version: Option<String>,
    /// Number of files the preprocessor was invoked on.
    pub files_processed: usize,
    /// Total macros expanded across all invocations.
    pub macros_expanded: usize,
}

/// Wrapper around a `clang -E` invocation.
#[derive(Debug, Clone)]
pub struct Preprocessor {
    /// Path to the `clang` binary in use.
    clang_path: PathBuf,
}

impl Preprocessor {
    /// Locate `clang` on the system. Returns `None` if not found.
    ///
    /// Search order:
    /// 1. `which clang` (PATH lookup)
    /// 2. `$WASI_SDK_PATH/bin/clang` — the wasi-sdk ships a `clang` binary;
    ///    with `-nostdinc` the bundled sysroot is irrelevant for
    ///    preprocessor-only mode, so we can reuse it for expansion.
    pub fn discover() -> Option<Self> {
        Self::discover_with(
            |name| which::which(name).ok(),
            std::env::var("WASI_SDK_PATH").ok(),
        )
    }

    /// Internal testable seam for `discover`.
    ///
    /// `which_fn` looks up a binary by name (mimics the real `which::which`).
    /// `wasi_sdk_path` is the value of the `WASI_SDK_PATH` env var, if any.
    fn discover_with<F>(which_fn: F, wasi_sdk_path: Option<String>) -> Option<Self>
    where
        F: Fn(&str) -> Option<PathBuf>,
    {
        if let Some(p) = which_fn("clang") {
            return Some(Self { clang_path: p });
        }
        if let Some(sdk) = wasi_sdk_path {
            let candidate = Path::new(&sdk).join("bin").join("clang");
            if candidate.is_file() {
                return Some(Self {
                    clang_path: candidate,
                });
            }
        }
        None
    }

    /// Construct from an explicit path. Mostly for tests.
    pub fn new(clang_path: PathBuf) -> Self {
        Self { clang_path }
    }

    /// Path to the `clang` binary in use.
    pub fn clang_path(&self) -> &Path {
        &self.clang_path
    }

    /// Run `clang -E -P -nostdinc` on the given source and return the
    /// expanded output plus a line-mapping table.
    ///
    /// `filename_hint` is used in `# <lineno> "<file>"` markers so the
    /// line map points back at the original source.
    ///
    /// **Status:** the implementation lands in M1 commit 2. This stub
    /// returns `PreprocessError::ClangFailed` so the analyzer can be wired
    /// up against a stable signature first.
    pub fn expand(
        &self,
        source: &str,
        filename_hint: &str,
    ) -> Result<ExpandedSource, PreprocessError> {
        let _ = (source, filename_hint);
        Err(PreprocessError::ClangFailed(
            "expand() not yet implemented — see M1 commit 2".to_string(),
        ))
    }

    /// Best-effort `clang --version` probe. Used for `PreprocessorInfo`.
    pub fn clang_version(&self) -> Option<String> {
        let output = Command::new(&self.clang_path).arg("--version").output().ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8(output.stdout).ok()?;
        // First line of `clang --version` is the version string.
        Some(stdout.lines().next()?.to_string())
    }
}

use std::process::Command;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_with_returns_none_when_clang_missing() {
        let result = Preprocessor::discover_with(|_| None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_discover_with_returns_none_when_only_wasi_sdk_path_set_but_no_file() {
        // Even with WASI_SDK_PATH set, if the clang binary doesn't exist
        // at the expected path, discover returns None.
        let result = Preprocessor::discover_with(
            |_| None,
            Some("/this/path/does/not/exist".to_string()),
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_discover_with_finds_clang_in_path() {
        let result = Preprocessor::discover_with(
            |name| {
                assert_eq!(name, "clang");
                Some(PathBuf::from("/usr/bin/clang"))
            },
            None,
        );
        let p = result.expect("expected Some");
        assert_eq!(p.clang_path(), Path::new("/usr/bin/clang"));
    }

    #[test]
    fn test_discover_with_falls_back_to_wasi_sdk_path() {
        // PATH lookup returns nothing, but $WASI_SDK_PATH/bin/clang exists
        // (we use the system clang to create a real file for the test).
        let system_clang = which::which("clang").expect("system clang must exist for this test");
        let sdk_dir = system_clang
            .parent()
            .and_then(|p| p.parent())
            .expect("clang's grandparent dir")
            .to_path_buf();
        let result = Preprocessor::discover_with(
            |_| None,
            Some(sdk_dir.to_string_lossy().to_string()),
        );
        let p = result.expect("expected fallback to WASI_SDK_PATH");
        assert!(p.clang_path().starts_with(&sdk_dir));
        assert!(p.clang_path().ends_with("clang"));
    }

    #[test]
    fn test_path_lookup_wins_over_wasi_sdk_path() {
        // When both PATH and WASI_SDK_PATH are set, PATH wins.
        let result = Preprocessor::discover_with(
            |_| Some(PathBuf::from("/from/path/clang")),
            Some("/should/be/ignored".to_string()),
        );
        let p = result.expect("expected Some");
        assert_eq!(p.clang_path(), Path::new("/from/path/clang"));
    }

    #[test]
    fn test_new_and_clang_path() {
        let p = Preprocessor::new(PathBuf::from("/opt/clang-19/bin/clang"));
        assert_eq!(p.clang_path(), Path::new("/opt/clang-19/bin/clang"));
    }
}
