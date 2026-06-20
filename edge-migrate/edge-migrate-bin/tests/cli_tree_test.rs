use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path =
            std::env::temp_dir().join(format!("edge_migrate_treetest_{}_{}_{}", label, pid, id));
        fs::create_dir_all(&path).expect("create tempdir");
        Self { path }
    }
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn make_tree_with_patterns() -> TempDir {
    let dir = TempDir::new("ok");
    fs::write(
        dir.path().join("main.c"),
        "int main(){int fd = socket(2, 1, 0); (void)fd; return 0;}\n",
    )
    .unwrap();
    dir
}

/// `edge-migrate --tree <dir>` should print the local tree report and
/// reject the upload when the tree has untransformable patterns
/// (without `--force`). The exit code is 1.
#[test]
fn test_tree_flag_rejects_unmigratable_without_force() {
    // Create a dir with a poll() call (non-transformable).
    let dir = TempDir::new("unmigratable");
    fs::write(
        dir.path().join("broken.c"),
        "int use_poll(void){struct pollfd fds[1]; poll(fds, 1, 0); return 0;}\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_edge-migrate"))
        .arg("--tree")
        .arg(dir.path())
        .arg("--app-name")
        .arg("broken-app")
        .env("EDGE_API_URL", "http://localhost:1") // never called, but in case
        .env_remove("EDGE_API_KEY")
        .output()
        .expect("run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "expected non-zero exit. stdout: {}\nstderr: {}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("Manual review") || stderr.contains("untransformable"),
        "expected 'untransformable' message; stdout: {}\nstderr: {}",
        stdout,
        stderr
    );
}

/// `edge-migrate --tree <dir> --force --app-name bad-name` should
/// reject the upload up front because the app name doesn't match the
/// public-facing regex. The exit code is 1, and the failure message
/// references the regex.
#[test]
fn test_tree_flag_validates_app_name() {
    let dir = make_tree_with_patterns();
    let output = Command::new(env!("CARGO_BIN_EXE_edge-migrate"))
        .arg("--tree")
        .arg(dir.path())
        .arg("--app-name")
        .arg("../traversal")
        .output()
        .expect("run");

    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("invalid app name") || stderr.contains("invalid app name"),
        "expected 'invalid app name' error; stdout: {}\nstderr: {}",
        stdout,
        stderr
    );
}

/// `--tree` should not crash on an empty directory. Empty dirs have
/// zero `.c`/`.h` files; the CLI exits with a clear error before
/// trying to upload.
#[test]
fn test_tree_flag_handles_empty_directory() {
    let dir = TempDir::new("empty");
    let output = Command::new(env!("CARGO_BIN_EXE_edge-migrate"))
        .arg("--tree")
        .arg(dir.path())
        .arg("--app-name")
        .arg("empty-app")
        .output()
        .expect("run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "expected non-zero exit; stdout: {}\nstderr: {}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("no .c or .h") || stderr.contains("no .c or .h"),
        "expected 'no .c or .h files' message; stdout: {}\nstderr: {}",
        stdout,
        stderr
    );
}
