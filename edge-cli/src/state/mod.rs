//! Deployment state management.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// State file persisted after a successful deploy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub deployment_id: String,
    pub app_name: String,
    pub live_url: String,
}

impl State {
    /// Read state from .edge/state.json in the given project directory.
    pub fn load(path: &Path) -> Result<Self> {
        let path = path.join(".edge").join("state.json");
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Write state to .edge/state.json in the given project directory.
    pub fn save(&self, path: &Path) -> Result<()> {
        let dir = path.join(".edge");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
        let path = dir.join("state.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}
