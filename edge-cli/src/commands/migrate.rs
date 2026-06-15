//! `edge migrate` — analyze source for WASI compatibility.

use anyhow::Result;
use std::path::Path;
use walkdir::WalkDir;

use crate::migrate::analyzer::Analyzer;

/// Analyze source for WASI compatibility.
pub fn run(path: &Path, auto: bool) -> Result<()> {
    let analyzer = Analyzer::new();

    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let ext = e.path().extension().and_then(|s| s.to_str());
            ext == Some("c") || ext == Some("rs")
        })
    {
        let file_path = entry.path();
        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        println!("\nAnalyzing {}...", file_path.display());
        let findings = analyzer.analyze(&source, file_path)?;

        for finding in findings {
            let prefix = match finding.severity {
                crate::migrate::analyzer::Severity::Warning => "⚠",
                crate::migrate::analyzer::Severity::Compatible => "✅",
            };
            println!("{} {}:{}", prefix, finding.location, finding.message);
            if let Some(suggestion) = &finding.suggestion {
                println!("  → {}", suggestion);
            }
        }
    }

    if auto {
        crate::migrate::transformer::apply_transforms(path)?;
        println!("\n✓ Auto-transform applied");
    }

    Ok(())
}
