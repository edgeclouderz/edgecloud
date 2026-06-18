//! edge-migrate CLI.
//!
//! Accepts a C source file, analyzes it locally, and uploads to edgeCloud
//! for transformation and compilation.

mod report;

use anyhow::{Context, Result};
use clap::Parser;
use edge_migrate_lib::{analyzer::CAnalyzer, report::MigrationReport, transformer::Transformer};
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
        let mut analyzer = CAnalyzer::new();
        let matches = analyzer.analyze(&source);
        let result = Transformer::transform(&source, matches);
        print!("{}", result.transformed_source);
        return Ok(());
    }

    let file = args.file.as_ref().expect("FILE argument required when not using --transform");
    let source = read_file(file).await?;
    let app_name = derive_app_name(file);

    // Analyze locally
    let mut analyzer = CAnalyzer::new();
    let matches = analyzer.analyze(&source);
    let local_report = MigrationReport::from_pattern_matches(&app_name, matches);

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

/// Path to the user's edgeCloud config file (`~/.config/edgecloud/config.toml`
/// on Linux/macOS, platform equivalent elsewhere).
fn config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|p| p.join("edgecloud").join("config.toml"))
}

/// Read the API key from `EDGE_API_KEY` env var, falling back to
/// `~/.config/edgecloud/config.toml` `[default].api_key`. The error
/// message stays accurate after this fallback — `edge auth login` writes
/// that file.
fn read_api_key() -> Result<String> {
    if let Ok(k) = std::env::var("EDGE_API_KEY") {
        if !k.is_empty() {
            return Ok(k);
        }
    }
    if let Some(path) = config_path() {
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            #[derive(serde::Deserialize)]
            struct Cfg {
                default: DefaultSection,
            }
            #[derive(serde::Deserialize)]
            struct DefaultSection {
                api_key: Option<String>,
            }
            if let Ok(cfg) = toml::from_str::<Cfg>(&content) {
                if let Some(k) = cfg.default.api_key {
                    if !k.is_empty() {
                        return Ok(k);
                    }
                }
            }
        }
    }
    anyhow::bail!("EDGE_API_KEY not set — run `edge auth signup` or `edge auth login` first")
}

/// Read the API URL from `EDGE_API_URL` env var, falling back to
/// `~/.config/edgecloud/config.toml` `[default].api`, then to
/// [`DEFAULT_API_URL`].
fn read_api_url() -> String {
    if let Ok(u) = std::env::var("EDGE_API_URL") {
        if !u.is_empty() {
            return u;
        }
    }
    if let Some(path) = config_path() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            #[derive(serde::Deserialize)]
            struct Cfg {
                default: DefaultSection,
            }
            #[derive(serde::Deserialize)]
            struct DefaultSection {
                api: Option<String>,
            }
            if let Ok(cfg) = toml::from_str::<Cfg>(&content) {
                if let Some(u) = cfg.default.api {
                    if !u.is_empty() {
                        return u;
                    }
                }
            }
        }
    }
    DEFAULT_API_URL.to_string()
}

async fn upload_to_edgecloud(file_path: &str, source: &str) -> Result<MigrationReport> {
    let api_url = read_api_url();
    let api_key = read_api_key()?;

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
