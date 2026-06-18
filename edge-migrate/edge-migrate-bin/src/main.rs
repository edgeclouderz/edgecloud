//! edge-migrate CLI.
//!
//! Accepts a C source file, analyzes it locally, and uploads to edgeCloud
//! for transformation and compilation.

mod report;

use anyhow::{Context, Result};
use clap::Parser;
use edge_migrate_lib::{
    analyzer::CAnalyzer,
    preprocessor::{Preprocessor, PreprocessorInfo},
    report::MigrationReport,
    transformer::Transformer,
};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

const DEFAULT_API_URL: &str = "https://api.edgecloud.dev";

#[derive(Parser, Debug)]
#[command(name = "edge-migrate")]
#[command(version)]
struct Args {
    /// The C source file to migrate.
    #[arg(value_name = "FILE")]
    file: Option<String>,

    /// Transform the source and write WASI C to stdout.
    /// Used by the edgeCloud control-plane to pipe output to wasi-sdk clang.
    #[arg(long, value_name = "SOURCE_FILE")]
    transform: Option<String>,

    /// Force upload even if the file has untransformable patterns.
    #[arg(short, long)]
    force: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // --transform: analyze + transform, output WASI C to stdout, exit immediately.
    // Used by the Go control-plane as: edge-migrate --transform <file>
    if let Some(ref source_path) = args.transform {
        let source = read_file(source_path).await?;
        // M1.C5: when clang is reachable, attach a preprocessor so
        // patterns hidden behind macros become visible. When clang is
        // missing, the analyzer falls back to the unexpanded source
        // silently — no user-visible error.
        let (mut analyzer, preprocessor_info) = build_analyzer_with_preprocessor(&source);
        let matches = analyzer.analyze(&source);
        let result = Transformer::transform(&source, matches, preprocessor_info);
        print!("{}", result.transformed_source);
        return Ok(());
    }

    let file = args.file.as_ref().expect("FILE argument required when not using --transform");
    let source = read_file(file).await?;
    let app_name = derive_app_name(file);

    // Analyze locally
    let (mut analyzer, preprocessor_info) = build_analyzer_with_preprocessor(&source);
    let matches = analyzer.analyze(&source);
    let local_report = match preprocessor_info {
        Some(pp) => MigrationReport::from_pattern_matches_with_preprocessor(
            &app_name,
            matches,
            pp,
        ),
        None => MigrationReport::from_pattern_matches(&app_name, matches),
    };

    // Display local analysis report
    report::print_analysis_report(&local_report);

    // Determine if we should upload
    let migratable = local_report.is_migratable();
    if !migratable && !args.force {
        println!();
        println!("❌ File contains untransformable patterns.");
        println!("  Run with --force to upload anyway.");
        std::process::exit(1);
    }

    // Upload to edgeCloud
    println!();
    println!("Uploading to edgeCloud for transformation...");
    match upload_to_edgecloud(file, &source).await {
        Ok(server_report) => {
            report::print_server_report(&server_report);
        }
        Err(e) => {
            eprintln!("Upload failed: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn read_file(path: &str) -> Result<String> {
    let mut file = File::open(path).await.context("Failed to open file")?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .await
        .context("Failed to read file")?;
    String::from_utf8(contents).context("File is not valid UTF-8")
}

fn derive_app_name(path: &str) -> String {
    let path = Path::new(path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("app");
    stem.to_string()
}

/// Build an analyzer with a preprocessor attached if one is reachable
/// on the system. Returns the analyzer + the `PreprocessorInfo` to
/// attach to the transform result (so the report can summarize macro
/// expansion). When no clang is found, the analyzer falls back to the
/// unexpanded source and `preprocessor_info` is `None`.
///
/// `source` is used to count `#define` directives so the report can
/// display an accurate macro count. The analyzer will re-run the
/// preprocessor internally during `analyze()`; the upfront count is
/// for the user-facing summary only.
fn build_analyzer_with_preprocessor(
    source: &str,
) -> (CAnalyzer, Option<PreprocessorInfo>) {
    match Preprocessor::discover() {
        Some(pp) => {
            // Count #define directives in the *original* source.
            // The analyzer will re-expand and produce an authoritative
            // count internally; this is the best estimate we can give
            // before invoking clang twice.
            let macros_expanded = source
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    t.starts_with("#define ") || t.starts_with("#define\t")
                })
                .count();
            let info = PreprocessorInfo {
                clang_version: pp.clang_version(),
                files_processed: 1,
                macros_expanded,
            };
            (CAnalyzer::with_preprocessor(pp), Some(info))
        }
        None => (CAnalyzer::new(), None),
    }
}

async fn upload_to_edgecloud(file_path: &str, source: &str) -> Result<MigrationReport> {
    let api_url = std::env::var("EDGE_API_URL")
        .unwrap_or_else(|_| DEFAULT_API_URL.to_string());
    let api_key = std::env::var("EDGE_API_KEY")
        .context("EDGE_API_KEY not set — run `edge auth login` first")?;

    let client = reqwest::Client::new();
    let form = reqwest::multipart::Form::new()
        .text("filename", file_path.to_string())
        .text("language", "c".to_string())
        .part(
            "file",
            reqwest::multipart::Part::text(source.to_string())
                .file_name(file_path.to_string()),
        );

    let response = client
        .post(format!("{}/api/migrate", api_url))
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .context("Failed to send request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Server returned {}: {}", status, body);
    }

    let report: MigrationReport = response
        .json()
        .await
        .context("Failed to parse server response")?;

    Ok(report)
}
