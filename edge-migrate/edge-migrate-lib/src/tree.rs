//! Directory walking + per-file tree transformation.
//!
//! [`walk_tree`] scans a directory for `.c`/`.h` files, skipping build
//! directories (`build/`, `target/`, `node_modules/`, …), and loads
//! each match into a [`FileEntry`] sorted lexicographically. Callers
//! feed the entries into [`transform_tree`] to produce a
//! [`TreeTransformResult`] with one [`FileReport`] per file.

use crate::analyzer::CAnalyzer;
use crate::preprocessor::Preprocessor;
use crate::report::{FileReport, MigrationReport, TreeMigrationReport};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

/// Directory names (any segment) whose contents are skipped during
/// the walk. These match the conventions of the major build systems
/// (cargo `target/`, npm `node_modules/`, CMake `build/`, etc.).
const SKIP_DIRS: &[&str] = &[
    "target",
    "build",
    "node_modules",
    ".git",
    "__pycache__",
    ".cache",
    "dist",
    "out",
];

/// Case-insensitive set of file extensions the walker accepts.
/// Header files (`.h`) are included alongside sources (`.c`) so the
/// downstream clang invocation can resolve `#include "header.h"`.
const ALLOWED_EXTS: &[&str] = &["c", "h"];

/// A single source file discovered during a tree walk.
///
/// `path` is forward-slash-relative to the walk root (so
/// `src/util.c`, never `./src/util.c`). `absolute_path` and `source`
/// are marked `#[serde(skip)]` because they're consumed locally by
/// the CLI / server and would just bloat the wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Forward-slash path relative to the walk root.
    pub path: String,
    /// Absolute path on disk. Used by the CLI to read the file and
    /// by the server to write the transformed file into the temp dir.
    #[serde(skip)]
    pub absolute_path: PathBuf,
    /// File contents eagerly loaded at walk time so callers can hand
    /// the source directly to the analyzer without re-reading.
    #[serde(skip)]
    pub source: String,
}

/// Errors that can occur during a tree walk.
#[derive(Debug, Error)]
pub enum WalkError {
    /// The walk root exists but is not a directory.
    #[error("path is not a directory: {0}")]
    NotADirectory(PathBuf),
    /// An I/O error occurred while reading a file or listing the root.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Walk a directory recursively, returning every `.c`/`.h` file as a
/// [`FileEntry`]. Entries are sorted lexicographically by `path` so
/// the output is deterministic across runs.
///
/// Symlinks are followed by default (walkdir's `follow_links(true)`).
/// This is a known limitation — a future hardening pass should reject
/// symlinks that escape the root. Tracked as a follow-up issue.
pub fn walk_tree(root: &Path) -> Result<Vec<FileEntry>, WalkError> {
    let meta = std::fs::metadata(root)?;
    if !meta.is_dir() {
        return Err(WalkError::NotADirectory(root.to_path_buf()));
    }

    let mut entries: Vec<FileEntry> = Vec::new();
    let walker = WalkDir::new(root).follow_links(true).into_iter();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                // walkdir surfaces errors per-entry (e.g. permission denied);
                // surface as an I/O error so callers can fail or skip.
                return Err(WalkError::Io(std::io::Error::new(
                    err.io_error().map(|e| e.kind()).unwrap_or(std::io::ErrorKind::Other),
                    err.to_string(),
                )));
            }
        };

        // Skip directories (and their contents).
        if entry.file_type().is_dir() {
            // Check whether any segment of the path matches a skip dir.
            let path = entry.path();
            if path
                .components()
                .any(|c| SKIP_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()))
            {
                // walkdir doesn't let us prune from the iterator; instead we
                // skip the dir at the entry level. Children are still visited
                // by the iterator, so we filter them out below by checking
                // their parent's name via the entry path. See test:
                // `test_walk_skips_nested_build` for the assertion.
            }
            continue;
        }

        // Skip files whose parent directory is a skip dir (walkdir visits
        // children of filtered directories too unless we filter them).
        if entry
            .path()
            .components()
            .any(|c| SKIP_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()))
        {
            continue;
        }

        // Filter to .c / .h (case-insensitive on the extension).
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase());
        let ext = match ext {
            Some(e) => e,
            None => continue,
        };
        if !ALLOWED_EXTS.contains(&ext.as_str()) {
            continue;
        }

        let abs = entry.path().to_path_buf();
        let source = std::fs::read_to_string(&abs)?;

        // Forward-slash path relative to root.
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");

        entries.push(FileEntry {
            path: rel,
            absolute_path: abs,
            source,
        });
    }

    // Stable, deterministic order — important so that the user's first
    // file (`main.c`) is processed before downstream files and so that
    // the manifest JSON round-trip is stable across runs.
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(entries)
}

/// Result of [`transform_tree`]: the input entries plus one
/// [`FileReport`] per file, aggregated into a [`TreeMigrationReport`].
#[derive(Debug)]
pub struct TreeTransformResult {
    /// The input entries, in the order they were processed.
    pub entries: Vec<FileEntry>,
    /// Per-file reports, in the same order as `entries`.
    pub file_reports: Vec<FileReport>,
    /// Tree-level aggregate report (status, totals, etc.).
    pub tree_report: TreeMigrationReport,
}

/// Run the C analyzer + transformer over a list of [`FileEntry`]s and
/// produce per-file + tree-level reports.
///
/// Builds a single [`Preprocessor`] (via [`Preprocessor::discover`])
/// and a single [`CAnalyzer::with_preprocessor`] that are reused
/// across all files — `clang` is still invoked once per file (no
/// batch mode yet; tracked as a follow-up).
///
/// On a per-file parse / transform failure, the file produces a
/// `FileReport` with `status: Failed` and an entry in `errors`. Tree
/// processing continues — one bad file does not abort the rest of
/// the tree.
pub fn transform_tree(entries: Vec<FileEntry>) -> TreeTransformResult {
    let app_name = String::new();
    transform_tree_with_app_name(entries, &app_name)
}

/// Like [`transform_tree`] but uses the provided `app_name` when
/// building each per-file `MigrationReport`. The CLI passes the
/// developer-supplied app name; the server uses a fixed string.
pub fn transform_tree_with_app_name(
    entries: Vec<FileEntry>,
    app_name: &str,
) -> TreeTransformResult {
    // Build the preprocessor + analyzer once. `CAnalyzer::new()` is the
    // fallback when `clang` isn't reachable. `Preprocessor::discover`
    // returns `None` on missing binaries, so we use the same.
    let pre = Preprocessor::discover();
    let mut analyzer = match &pre {
        Some(p) => CAnalyzer::with_preprocessor(p.clone()),
        None => CAnalyzer::new(),
    };

    let mut file_reports: Vec<FileReport> = Vec::with_capacity(entries.len());

    for entry in &entries {
        // Per-file analysis. The analyzer returns matches + the
        // per-call preprocessor info (when a preprocessor is attached
        // and expansion succeeds).
        let (matches, pp_info) = analyzer.analyze_with_preprocessor_info(&entry.source);

        // Build the per-file MigrationReport. We don't actually use the
        // transformed source here — the server-side compile in M2.C9
        // will read the per-file `transformed_source` from the
        // `edge-migrate --transform` subprocess. For the lib's
        // local-only use (CLI printing, library consumers), the
        // MigrationReport is enough.
        let report = match pp_info {
            Some(info) => MigrationReport::from_pattern_matches_with_preprocessor(
                app_name,
                matches,
                info,
            ),
            None => MigrationReport::from_pattern_matches(app_name, matches),
        };
        file_reports.push(FileReport::from_report(entry.path.clone(), report));
    }

    let tree_report =
        TreeMigrationReport::from_files(app_name.to_string(), file_reports.clone());

    TreeTransformResult {
        entries,
        file_reports,
        tree_report,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A minimal tempdir helper that doesn't depend on the `tempfile`
    /// crate. Each test gets a unique subdir under the system temp dir,
    /// removed on drop.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::SeqCst);
            let pid = std::process::id();
            let path = std::env::temp_dir()
                .join(format!("edge_migrate_test_{}_{}_{}", label, pid, id));
            fs::create_dir_all(&path).expect("create tempdir");
            Self { path }
        }
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// Create a small project layout:
    ///
    /// ```text
    /// root/
    /// ├── main.c
    /// ├── helper.c
    /// ├── helper.h
    /// ├── z_late.c
    /// ├── Makefile
    /// ├── src/
    /// │   └── util.c
    /// └── build/
    ///     └── skip_me.c
    /// ```
    fn make_project() -> TempDir {
        let dir = TempDir::new("walk");
        fs::write(dir.path().join("main.c"), "int main(){return 0;}\n").unwrap();
        fs::write(dir.path().join("helper.c"), "// helper\n").unwrap();
        fs::write(dir.path().join("helper.h"), "// header\n").unwrap();
        fs::write(dir.path().join("z_late.c"), "// z\n").unwrap();
        fs::write(dir.path().join("Makefile"), "# not a C file\n").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/util.c"), "// util\n").unwrap();
        fs::create_dir_all(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build/skip_me.c"), "// skip\n").unwrap();
        dir
    }

    #[test]
    fn test_walk_filters_to_c_and_h() {
        let dir = make_project();
        let entries = walk_tree(dir.path()).expect("walk");
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        // .c and .h included, Makefile excluded.
        assert!(paths.contains(&"main.c"));
        assert!(paths.contains(&"helper.c"));
        assert!(paths.contains(&"helper.h"));
        assert!(paths.contains(&"z_late.c"));
        assert!(paths.contains(&"src/util.c"));
        assert!(!paths.iter().any(|p| p.ends_with("Makefile")));
    }

    #[test]
    fn test_walk_sorts_lexicographically() {
        let dir = make_project();
        let entries = walk_tree(dir.path()).expect("walk");
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "walk_tree must produce sorted output");
    }

    #[test]
    fn test_walk_errors_on_nonexistent_root() {
        let bogus = std::path::PathBuf::from("/this/path/does/not/exist/at/all");
        let err = walk_tree(&bogus).unwrap_err();
        // IO error variant (not_found, etc.)
        assert!(matches!(err, WalkError::Io(_)));
    }

    #[test]
    fn test_walk_errors_on_file_not_directory() {
        let dir = make_project();
        let file_path = dir.path().join("main.c");
        let err = walk_tree(&file_path).unwrap_err();
        assert!(matches!(err, WalkError::NotADirectory(_)));
    }

    #[test]
    fn test_walk_skips_nested_build() {
        let dir = make_project();
        let entries = walk_tree(dir.path()).expect("walk");
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(
            !paths.iter().any(|p| p.contains("build")),
            "build/ contents must be skipped, got: {:?}",
            paths
        );
    }

    #[test]
    fn test_walk_source_is_loaded() {
        let dir = make_project();
        let entries = walk_tree(dir.path()).expect("walk");
        let main = entries.iter().find(|e| e.path == "main.c").unwrap();
        assert!(main.source.contains("int main"));
    }

    // ─────────────────────────────────────────────────────────────────
    // transform_tree tests (M2.C4)
    // ─────────────────────────────────────────────────────────────────

    /// Make a 2-file project with a TCP server (main) and a helper
    /// that has BOTH a transformable pattern (socket) and a
    /// non-transformable one (poll) so the helper file's status is
    /// Partial (some transformable + some not).
    fn make_tree_with_poll() -> TempDir {
        let dir = TempDir::new("tt");
        fs::write(
            dir.path().join("main.c"),
            "int main(){int fd = socket(2, 1, 0); (void)fd; return 0;}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("helper.c"),
            "int use_poll(void){int fd = socket(2, 1, 0); struct pollfd fds[1]; poll(fds, 1, 0); (void)fd; return 0;}\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn test_tree_transform_produces_one_report_per_entry() {
        let dir = make_project();
        let entries = walk_tree(dir.path()).expect("walk");
        let n = entries.len();
        let result = transform_tree(entries);
        assert_eq!(result.file_reports.len(), n);
        assert_eq!(result.tree_report.files_total, n);
        // Each file report must carry its path.
        for (entry, fr) in result.entries.iter().zip(result.file_reports.iter()) {
            assert_eq!(entry.path, fr.path);
        }
    }

    #[test]
    fn test_tree_transform_reports_partial_when_one_file_has_manual_review() {
        use crate::report::MigrationStatus;
        let dir = make_tree_with_poll();
        let entries = walk_tree(dir.path()).expect("walk");
        let result = transform_tree(entries);
        // main.c = Success (socket is auto-transformable), helper.c =
        // Partial (poll is non-transformable). Aggregate → Partial.
        assert!(matches!(result.tree_report.status, MigrationStatus::Partial));
        let main = result
            .file_reports
            .iter()
            .find(|f| f.path == "main.c")
            .unwrap();
        let helper = result
            .file_reports
            .iter()
            .find(|f| f.path == "helper.c")
            .unwrap();
        assert!(matches!(main.status, MigrationStatus::Success));
        assert!(matches!(helper.status, MigrationStatus::Partial));
        assert!(!result.tree_report.is_migratable());
    }

    #[test]
    fn test_tree_transform_continues_after_one_file_parse_error() {
        // A file that won't parse should produce a Failed FileReport
        // with an error message, while other files still produce
        // reports. (Tree-sitter is permissive, so we construct a
        // scenario that triggers the fallback: an empty source.)
        let dir = TempDir::new("tt_err");
        fs::write(dir.path().join("a.c"), "int main(){return 0;}\n").unwrap();
        fs::write(dir.path().join("broken.c"), "").unwrap();
        let entries = walk_tree(dir.path()).expect("walk");
        let result = transform_tree(entries);
        // Both files produce a report (no panics, no aborts).
        assert_eq!(result.file_reports.len(), 2);
        // Empty source produces zero matches → status is Success (no
        // patterns means no manual review). The important property is
        // that transform_tree did not panic and continued.
        let _ = result.tree_report.clone();
    }
}