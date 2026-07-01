//! Detect the execution model of a WASI Preview 2 component.
//!
//! The worker supports two execution models:
//!
//! * **LongRunning** — the guest implements `_start` and is responsible
//!   for hosting its own TCP server (typically via `wasi:sockets`). The
//!   supervisor spawns `run_app_loop` to drive the guest.
//!
//! * **Handler (FaaS)** — the guest implements
//!   `wasi:http/incoming-handler` and is invoked once per HTTP request.
//!   The supervisor hosts an axum server; each request goes through a
//!   `wasmtime_wasi_http::ProxyPre` that calls the guest's
//!   `handle(request, response-out)` function.
//!
//! Detection is purely structural — we inspect the component's exported
//! interface list without instantiating it. That makes the choice cheap
//! and lets us pick the right linker factory before
//! `linker.instantiate_pre(&component)` is attempted.

use wasmtime::component::Component;

/// Which execution model a component expects.
///
/// Maps directly to (a) which linker factory the supervisor picks and
/// (b) which task the supervisor spawns in `start_app`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionModel {
    /// Component implements `_start` and hosts its own TCP server via
    /// `wasi:sockets`. Spawned via `run_app_loop`.
    LongRunning,
    /// Component implements `wasi:http/incoming-handler`. The worker
    /// owns the HTTP listener and dispatches each request through a
    /// `wasmtime_wasi_http::ProxyPre`. Wired up in Phase C.
    Handler,
}

/// Inspect a component's exported interfaces to decide which execution
/// model it expects.
///
/// We treat any component exporting `wasi:http/incoming-handler` (with
/// or without a version suffix, e.g. `@0.2.0`) as `Handler`. LongRunning
/// is the default — `_start` is canonical for WASI Preview 2 components
/// and we don't require any specific signature beyond that.
pub fn detect_execution_model(component: &Component) -> ExecutionModel {
    let ty = component.component_type();
    // `ComponentType::exports` needs the engine because canonical-ABI
    // type lookups inside the component are engine-scoped. The component
    // already holds its engine internally; we just borrow it back.
    let engine = component.engine();
    for (name, _item) in ty.exports(engine) {
        // Match the bare interface name and the version-suffixed form.
        // The bindgen normalizes across versions, so a `wasi:http/incoming-handler@0.2.0`
        // export is still valid for our Handler linker.
        if name.starts_with("wasi:http/incoming-handler") {
            return ExecutionModel::Handler;
        }
    }
    ExecutionModel::LongRunning
}

#[cfg(test)]
mod tests {
    //! Unit tests for `detect_execution_model`.
    //!
    //! Building a real `wasmtime::component::Component` requires byte
    //! input — the test fixtures (long-running.c, handler.c) live in
    //! Phase D. End-to-end coverage of detection against real binaries
    //! lands there.

    use super::*;

    /// Sanity-check the enum variants are distinct and copyable.
    #[test]
    fn execution_model_variants_distinct() {
        assert_ne!(ExecutionModel::LongRunning, ExecutionModel::Handler);
        let a = ExecutionModel::Handler;
        let b = a; // Copy
        assert_eq!(a, b);
    }
}
