//! API key management.

use anyhow::Result;
use serde::Deserialize;
use std::env;

/// API key — loaded from EDGE_API_KEY env var or .edge/config.toml.
#[derive(Debug, Clone)]
pub struct ApiKey(pub String);

impl ApiKey {
    /// Load API key: first EDGE_API_KEY env var, then config file.
    pub fn load() -> Result<Self> {
        if let Ok(key) = env::var("EDGE_API_KEY") {
            if !key.is_empty() {
                return Ok(Self(key));
            }
        }

        if let Some(config_path) = dirs::config_dir() {
            let path = config_path.join("edgecloud").join("config.toml");
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                if let Ok(config) = toml::from_str::<TomlConfig>(&content) {
                    if let Some(key) = config.default.api_key {
                        if !key.is_empty() {
                            return Ok(Self(key));
                        }
                    }
                }
            }
        }

        anyhow::bail!(
            "API key not found: set EDGE_API_KEY env var or create ~/.config/edgecloud/config.toml"
        )
    }
}

#[derive(Debug, Deserialize)]
struct TomlConfig {
    default: DefaultSection,
}

#[derive(Debug, Deserialize)]
struct DefaultSection {
    api_key: Option<String>,
    #[allow(dead_code)]
    api: Option<String>,
}
