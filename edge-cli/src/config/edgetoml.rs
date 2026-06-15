//! edge.toml parsing.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// edge.toml project configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct EdgeToml {
    pub project: Project,
    pub deployment: Deployment,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub name: String,
    pub version: String,
    pub target: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Deployment {
    pub api: String,
}

impl EdgeToml {
    /// Read and parse edge.toml from the given directory.
    pub fn from_path(path: &Path) -> Result<Self> {
        let path = path.join("edge.toml");
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
    }
}
