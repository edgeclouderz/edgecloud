//! `edge activate` — activate a specific deployment.

use anyhow::{Context, Result};
use std::path::Path;

use crate::api::ApiClient;
use crate::config::EdgeToml;
use crate::output;
use crate::state::State;

/// Activate a specific deployment.  If --weight is given, performs a canary
/// activation (partial traffic split); weight=100 means atomic cutover.
#[cfg(feature = "network")]
pub fn run(path: &Path, deployment_id: &str, weight: Option<u8>) -> Result<()> {
    let state =
        State::load(path).with_context(|| "no deployment found — run `edge deploy` first")?;
    let edge_toml = EdgeToml::from_path(path)?;

    let client = ApiClient::new(edge_toml.api_url("https://api.edgecloud.dev"))?;
    client.activate(&state.app_name, deployment_id, weight)?;

    match weight {
        Some(w) if w < 100 => output::success(&format!(
            "Deployment {} activated with {}% traffic", deployment_id, w
        )),
        Some(100) | None => output::success(&format!("Deployment {} activated", deployment_id)),
        _ => output::success(&format!(
            "Deployment {} draining (0% traffic)", deployment_id
        )),
    }
    Ok(())
}

#[cfg(not(feature = "network"))]
pub fn run(_path: &Path, _deployment_id: &str, _weight: Option<u8>) -> Result<()> {
    anyhow::bail!("activate requires network support; rebuild with --features network")
}
