//! CLI report formatting.
//!
//! Formats migration reports for terminal display. M3 added a
//! `language: &str` parameter to every entry point so the section
//! headers and the "no patterns detected" message reflect whether
//! the report describes POSIX (C) or std (Rust) patterns.

use edge_migrate_lib::report::{MigrationReport, MigrationStatus, TreeMigrationReport};

/// Section header for the per-file analysis report's pattern list.
/// Varies by language so a developer reading the output can tell at a
/// glance which source language is being summarized.
fn pattern_section_label(language: &str) -> &'static str {
    match language {
        "rust" => "Rust std patterns",
        // Default to the C wording; anything we don't recognize is
        // treated as C because that's the historical default.
        _ => "POSIX patterns",
    }
}

/// Print the local analysis report (before upload).
pub fn print_analysis_report(report: &MigrationReport, language: &str) {
    println!();
    println!("=== edge-migrate Analysis Report ===");
    println!();
    println!("App name: {}", report.app_name);
    println!();

    if report.patterns_detected.is_empty() {
        println!("No {} detected.", pattern_section_label(language));
    } else {
        println!("Patterns detected: {}", report.patterns_detected.len());
        println!();

        if !report.patterns_transformed.is_empty() {
            println!(
                "Auto-transformable ({}):",
                report.patterns_transformed.len()
            );
            for p in &report.patterns_transformed {
                println!(
                    "  ✅ Line {}: {} → {}",
                    p.line, p.pattern, p.wasi_equivalent
                );
            }
            println!();
        }

        if !report.patterns_manual_review.is_empty() {
            println!(
                "Manual review required ({}):",
                report.patterns_manual_review.len()
            );
            for p in &report.patterns_manual_review {
                println!("  ⚠️  Line {}: {}", p.line, p.pattern);
                println!("      WASI equivalent: {}", p.wasi_equivalent);
            }
            println!();
        }
    }

    // The preprocessor summary is useful even when no patterns
    // are detected — it tells the developer that macros were
    // expanded (and how many) so they're not surprised by a
    // count that differs from "manual count of #define". C-only;
    // Rust has no preprocessor in v1, so the block is hidden.
    if let Some(pp) = &report.preprocessor {
        println!(
            "Preprocessor: {} files processed, {} macros expanded",
            pp.files_processed, pp.macros_expanded
        );
        if let Some(v) = &pp.clang_version {
            println!("  ({})", v);
        }
        println!();
    }
}

/// Print the server response report (after upload).
pub fn print_server_report(report: &MigrationReport) {
    println!();
    match report.status {
        MigrationStatus::Success => {
            println!("✅ Migration successful!");
            println!();
            println!(
                "Binary stored. Run `edge deploy {} --id {}` to go live.",
                report.app_name,
                report.deployment_id.as_deref().unwrap_or("<id>")
            );
        }
        MigrationStatus::Partial => {
            println!("⚠️  Migration partially successful.");
            println!();
            println!("Binary stored, but some patterns require manual review:");
            for p in &report.patterns_manual_review {
                println!(
                    "  ⚠️  Line {}: {} — {}",
                    p.line, p.pattern, p.wasi_equivalent
                );
            }
            println!();
            println!(
                "Run `edge deploy {} --id {}` to deploy anyway.",
                report.app_name,
                report.deployment_id.as_deref().unwrap_or("<id>")
            );
        }
        MigrationStatus::Failed => {
            println!("❌ Migration failed.");
            println!();
            println!("The following patterns could not be auto-transformed:");
            for p in &report.patterns_manual_review {
                println!(
                    "  ❌ Line {}: {} — {}",
                    p.line, p.pattern, p.wasi_equivalent
                );
            }
            println!();
            println!("Fix these issues and re-run `edge-migrate`.");
        }
    }

    if !report.errors.is_empty() {
        println!();
        println!("Errors:");
        for e in &report.errors {
            println!("  ❌ Line {}: {}", e.line, e.message);
        }
    }
}

/// Print the local tree-level analysis report (before upload).
/// `language` switches the section header so the developer can tell
/// at a glance whether the report covers a C or Rust tree.
pub fn print_tree_report(report: &TreeMigrationReport, language: &str) {
    println!();
    println!(
        "=== edge-migrate Tree Analysis ({}) ===",
        pattern_section_label(language)
    );
    println!();
    println!("App name: {}", report.app_name);
    println!(
        "Files: {} total, {} with auto-transformations, {} requiring manual review",
        report.files_total, report.files_transformed, report.files_manual_review
    );
    println!();

    for f in &report.files {
        let marker = match f.status {
            MigrationStatus::Success => "✅",
            MigrationStatus::Partial => "⚠️ ",
            MigrationStatus::Failed => "❌",
        };
        println!(
            "{}  {} ({} auto, {} review, {} errors)",
            marker,
            f.path,
            f.transformations.len(),
            f.manual_review.len(),
            f.errors.len()
        );
        for p in &f.manual_review {
            println!(
                "      ⚠️  Line {}: {} — {}",
                p.line, p.pattern, p.wasi_equivalent
            );
        }
        for e in &f.errors {
            println!("      ❌ Line {}: {}", e.line, e.message);
        }
    }
    println!();
}
