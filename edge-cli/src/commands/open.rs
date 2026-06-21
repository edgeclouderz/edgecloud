//! `edge open` — open the deployed URL in a browser.
//!
//! Before opening, this command checks the deployment status via
//! `GET /api/status/<deployment_id>` (the same endpoint `edge status`
//! uses). If the status is `crashed`, the command exits non-zero and
//! prints a hint pointing at `edge rollback` and `--force`. `--force`
//! skips the preflight and opens the URL anyway — useful when a user
//! knows the deployment is broken and just wants to see the error
//! page directly.

use anyhow::{Context, Result};
use std::path::Path;

#[cfg(feature = "network")]
use crate::api::ApiClient;
#[cfg(feature = "network")]
use crate::config::EdgeToml;
#[cfg(feature = "network")]
use crate::output;
use crate::state::State;

/// Open the deployed URL in a browser.
///
/// `force`: skip the crashed-deployment preflight. False by default so
/// users get a clear recovery hint instead of opening a known-broken URL.
#[cfg(feature = "network")]
pub fn run(path: &Path, force: bool) -> Result<()> {
    let state =
        State::load(path).with_context(|| "no deployment found — run `edge deploy` first")?;

    // Skip the preflight entirely when (a) --force was passed, or
    // (b) we have no live URL to open. The latter case covers the
    // legacy state.json from issue #74 step 2 where live_url is empty
    // (the URL staleness follow-up).
    if !force && !state.live_url.is_empty() {
        if let Err(e) = preflight(path, &state) {
            // Preflight errors are already user-facing — print the
            // error and the recovery hints, then return Err so main()
            // exits non-zero. Other commands (rollback, activate,
            // deploy) follow the same Result<()> contract; calling
            // std::process::exit here would skip destructors and
            // bypass main's exit-code path.
            output::error(&format!("{e:#}"));
            output::hint("run `edge rollback` to roll back to the last good deployment");
            output::hint("or run `edge open --force` to open anyway");
            return Err(e).context("preflight failed");
        }
    }

    open::that(&state.live_url)?;
    println!("Opening {}...", state.live_url);
    Ok(())
}

#[cfg(feature = "network")]
fn preflight(path: &Path, state: &State) -> Result<()> {
    let edge_toml = EdgeToml::from_path(path)
        .with_context(|| "edge open requires edge.toml with [deployment] api = \"<url>\"")?;
    let client = ApiClient::new(edge_toml.api_url("https://api.edgecloud.dev"))?;
    let status = client.status(&state.deployment_id)?;

    if status.status == "crashed" {
        anyhow::bail!(
            "deployment {} has crashed — opening it would show an error page",
            state.deployment_id
        );
    }
    Ok(())
}

#[cfg(not(feature = "network"))]
pub fn run(path: &Path, _force: bool) -> Result<()> {
    // No preflight is possible without the network feature; just open.
    let state =
        State::load(path).with_context(|| "no deployment found — run `edge deploy` first")?;
    open::that(&state.live_url)?;
    println!("Opening {}...", state.live_url);
    Ok(())
}
