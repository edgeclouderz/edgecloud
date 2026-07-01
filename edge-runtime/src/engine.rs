//! wasmtime Engine creation with security configuration.

use anyhow::Result;
use wasmtime::{Config, Engine};

/// Create a wasmtime Engine with security-hardened configuration.
pub fn create_engine() -> Result<wasmtime::Engine> {
    let mut config = Config::new();

    // Security: disable features that expand attack surface
    config.wasm_threads(false);

    // Reference types MUST be enabled for compatibility with
    // `wasm32-unknown-unknown` components produced via the
    // `wasm-tools component new` workflow (Phase D fix). The compiled
    // core wasm uses multi-byte LEB128 zero encoding for memory
    // indices in bulk-memory instructions (`memory.copy`, `memory.fill`,
    // etc.). With reference types disabled, the wasmtime parser runs in
    // "single-memory" mode and rejects multi-byte zeros at those
    // positions with `Invalid input WebAssembly code at offset N:
    // zero byte expected`. Reference types was historically disabled
    // for defense-in-depth, but the bulk-memory instructions it gates
    // are required for any modern toolchain.
    config.wasm_reference_types(true);

    // Performance: enable SIMD
    config.wasm_simd(true);

    // Required for WASI Preview 2 / component model
    config.wasm_component_model(true);

    // Enable epoch interruption for CPU time limits
    config.epoch_interruption(true);

    // Required for `wasmtime_wasi::add_to_linker_async` /
    // `wasmtime_wasi_http::add_only_http_to_linker_async` — both use
    // `func_wrap_async` internally (Phase C: wasi: wiring). Without
    // this, linker construction panics with "cannot use
    // `func_wrap_async` without enabling async support in the config".
    // The performance cost is ~5% on synchronous workloads and is
    // mandatory for any async-aware wasi:* surface.
    config.async_support(true);

    let engine = Engine::new(&config)?;
    Ok(engine)
}
