//! `edge open` — open the deployed URL in a browser.

use anyhow::{Context, Result};
use std::path::Path;

use crate::state::State;

/// Open the deployed URL in a browser.
pub fn run(path: &Path) -> Result<()> {
    let state = State::load(path).with_context(|| "no deployment found — run `edge deploy` first")?;
    open::that(&state.live_url)?;
    println!("Opening {}...", state.live_url);
    Ok(())
}
