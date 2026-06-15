//! `edge build` — compile the project to WebAssembly.

use anyhow::Result;
use std::path::Path;
use std::process::Command;

use crate::config::EdgeToml;
use crate::output;

/// Compile the project to WebAssembly.
pub fn run(path: &Path) -> Result<()> {
    let edge_toml = EdgeToml::from_path(path)?;
    let project_name = &edge_toml.project.name;

    output::info(&format!(
        "Building '{}' (target: {})...",
        project_name, edge_toml.project.target
    ));

    // Run cargo build for the wasm target
    let status = Command::new("cargo")
        .args(["build", "--target", "wasm32-wasip2", "--release"])
        .current_dir(path)
        .spawn()?
        .wait()?;

    if !status.success() {
        anyhow::bail!("cargo build failed");
    }

    let artifact = path
        .join("target")
        .join("wasm32-wasip2")
        .join("release")
        .join(format!("{}.wasm", project_name));

    if !artifact.exists() {
        anyhow::bail!("artifact not found at {}", artifact.display());
    }

    output::success("Built successfully");
    println!("  Artifact: {}", artifact.display());
    Ok(())
}
