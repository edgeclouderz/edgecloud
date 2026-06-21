use std::process::Command;

/// `edge-migrate --analyze-json <path>` should emit a single JSON
/// object (no trailing whitespace, no log lines) on stdout, parseable
/// as a `MigrationReport`. Used by the Go control-plane's
/// `MigrateTree` for per-file structured data.
#[test]
fn test_analyze_json_outputs_valid_migration_report_json() {
    let test_file = concat!(env!("CARGO_MANIFEST_DIR"), "/../testdata/http_client.c");

    let output = Command::new(env!("CARGO_BIN_EXE_edge-migrate"))
        .arg("--analyze-json")
        .arg(test_file)
        .output()
        .expect("failed to execute edge-migrate --analyze-json");

    assert!(
        output.status.success(),
        "edge-migrate --analyze-json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The output must be a single JSON object — no leading log lines.
    let trimmed = stdout.trim();
    assert!(trimmed.starts_with('{'), "expected JSON, got: {}", trimmed);
    assert!(
        trimmed.ends_with('}'),
        "expected JSON object end, got: {}",
        trimmed
    );

    // Parse and confirm key fields are present.
    let v: serde_json::Value = serde_json::from_str(trimmed).expect("parse JSON");
    assert!(v.get("status").is_some(), "missing status field");
    assert!(v.get("app_name").is_some(), "missing app_name field");
    let patterns_detected = v
        .get("patterns_detected")
        .and_then(|p| p.as_array())
        .expect("patterns_detected must be an array");
    // The http_client.c fixture has at least one POSIX pattern.
    assert!(
        !patterns_detected.is_empty(),
        "expected at least one pattern in http_client.c"
    );
}

/// `edge-migrate --analyze-json` on a file with no POSIX patterns
/// should still emit valid JSON with empty pattern arrays.
#[test]
fn test_analyze_json_handles_empty_source() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("edge_migrate_empty_{}.c", std::process::id()));
    std::fs::write(&path, "/* no patterns here */\n").expect("write temp");

    let output = Command::new(env!("CARGO_BIN_EXE_edge-migrate"))
        .arg("--analyze-json")
        .arg(&path)
        .output()
        .expect("run");

    let _ = std::fs::remove_file(&path);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse");
    assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("success"));
    assert_eq!(
        v.get("patterns_detected")
            .and_then(|p| p.as_array())
            .map(|a| a.len()),
        Some(0)
    );
}
