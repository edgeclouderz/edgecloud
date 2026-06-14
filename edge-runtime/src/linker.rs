//! Linker setup for both core wasm and component model.

use anyhow::Result;
use wasmtime::{Linker, Engine};
use wasmtime::component::Linker as ComponentLinker;
use crate::RuntimeState;
use crate::EdgeRuntime;

/// Create a linker for core wasm modules (WASI Preview 1).
pub fn create_linker(engine: &Engine) -> Result<Linker<()>> {
    let linker: Linker<()> = Linker::new(engine);
    Ok(linker)
}

/// Create a linker for WASI Preview 2 components.
pub fn create_component_linker(engine: &Engine) -> Result<ComponentLinker<RuntimeState>> {
    let mut linker: ComponentLinker<RuntimeState> = ComponentLinker::new(engine);
    EdgeRuntime::add_to_linker(&mut linker, |state: &mut RuntimeState| state)?;
    Ok(linker)
}
