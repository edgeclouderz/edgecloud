//! wasmtime Store creation.

use crate::RuntimeState;
use wasmtime::{Engine, Store};

/// Create a wasmtime Store with memory limits enforced via StoreLimits.
///
/// The limiter reads the `limits` field from RuntimeState.
pub fn create_store(engine: &Engine, _max_memory_mb: u64, data: RuntimeState) -> Store<RuntimeState> {
    let mut store = Store::new(engine, data);
    store.limiter(|state| &mut state.limits);
    store
}
