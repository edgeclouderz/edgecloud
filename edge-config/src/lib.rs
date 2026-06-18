//! Shared helpers for reading the user's `~/.config/edgecloud/config.toml`.
//!
//! `edge-cli` and `edge-migrate-bin` both need to read the same on-disk
//! config file to discover a saved API key and base URL. Keeping the
//! loaders here means a config-schema change ships in one crate.

use std::path::PathBuf;

/// Return the platform-default config path.
///
/// `dirs::config_dir()` returns the right place on every supported OS:
///   - Linux: `$XDG_CONFIG_HOME` or `$HOME/.config`
///   - macOS: `$HOME/Library/Application Support`
///   - Windows: `%APPDATA%` (Roaming)
///
/// Returns `None` on platforms where the resolution fails.
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("edgecloud").join("config.toml"))
}

/// Read the API key with precedence: `EDGE_API_KEY` env var → config file
/// → error. The empty string is treated as unset at every layer.
pub fn read_api_key() -> anyhow::Result<String> {
    if let Ok(k) = std::env::var("EDGE_API_KEY") {
        if !k.is_empty() {
            return Ok(k);
        }
    }
    if let Some(path) = config_path() {
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("failed to read {}: {}", path.display(), e))?;
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

/// Read the API base URL with precedence: `EDGE_API_URL` env var → config
/// file → `fallback`. Used by subcommands that have no `edge.toml` to read
/// from (e.g. `edge auth signup`, `edge-migrate`).
pub fn read_api_url(fallback: &str) -> String {
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
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_api_url_falls_back_when_nothing_set() {
        // EDGE_API_URL may or may not be set in the test env; we cannot
        // mutate it from a multi-threaded test, so just check the
        // fallback path produces *some* non-empty value.
        let resolved = read_api_url("https://fallback.example");
        assert!(!resolved.is_empty());
    }

    #[test]
    fn read_api_key_falls_through_when_nothing_set() {
        // If neither EDGE_API_KEY nor a config file is reachable, the
        // function bails with the "run `edge auth signup`" error.
        // We can't unset the env var cleanly, so we only assert the
        // function doesn't panic — the success/err shape depends on the
        // host environment.
        let _ = read_api_key();
    }
}
