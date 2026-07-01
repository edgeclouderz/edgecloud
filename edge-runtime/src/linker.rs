//! Linker factories — one per WIT world.
//!
//! Two worlds, two factories. Both share the `edge:cloud@0.2.0` package
//! but differ in their wasi:* surface (handler omits `wasi:sockets`,
//! adds an `incoming-handler` export).
//!
//! ## Wiring strategy (Phase C)
//!
//! See the module-level comment in `lib.rs`. In short: we do NOT let
//! bindgen auto-register the wasi:* Host impls (which would require
//! `RuntimeState` to implement 100+ `wasmtime_wasi::bindings::...::Host`
//! methods directly — `wit-bindgen 0.51` doesn't auto-wrap in
//! `WasiImpl`). Instead, the linker is built up in three explicit
//! passes:
//!
//! 1. **`wasmtime_wasi::add_to_linker_async`** — registers every
//!    `wasi:cli/command` import (`wasi:io/*`, `wasi:clocks/*`,
//!    `wasi:filesystem/*`, `wasi:random/*`, `wasi:sockets/*`,
//!    `wasi:cli/*`) via the canonical `WasiImpl<&mut T>` wrapper.
//!    Requires `T: WasiView` (`RuntimeState` has this; see
//!    `runtime.rs`).
//! 2. **`wasmtime_wasi_http::add_only_http_to_linker_async`** — adds
//!    `wasi:http/{outgoing-handler,types}` via `WasiHttpImpl<&mut T>`.
//!    `add_only_http` (NOT `add_to_linker_async`) because the latter
//!    double-registers `wasi:io`, `wasi:clocks`, `wasi:random` with
//!    step 1. Requires `T: WasiHttpView` (Phase C-3 adds this to
//!    `RuntimeState`).
//! 3. **bindgen-generated per-interface `add_to_linker_get_host` for
//!    each `edge:cloud/*` Host impl** — `RuntimeState` directly
//!    implements these (in `runtime.rs`). We invoke each individually
//!    so we don't pull in wasi:* registration accidentally.
//!
//! `allow_shadowing(true)` was removed because it was a defensive hack
//! from the v0.1 days; with wasi: + edge: in disjoint namespaces it is
//! both unnecessary and a footgun (silent overloads of canonical
//! interfaces).

use crate::RuntimeState;
use anyhow::Result;
use wasmtime::component::Linker as ComponentLinker;
use wasmtime::Engine;

/// Create a linker for the `edge-runtime` (long-running) world.
///
/// Long-running components implement `_start` and self-host any TCP
/// servers they need via `wasi:sockets` (registered via step 1).
pub fn create_component_linker_long_running(
    engine: &Engine,
) -> Result<ComponentLinker<RuntimeState>> {
    let mut linker: ComponentLinker<RuntimeState> = ComponentLinker::new(engine);

    // Step 1: wasi:cli/command (io, clocks, filesystem, random, sockets, cli).
    // Requires `RuntimeState: WasiView` — implemented in runtime.rs.
    wasmtime_wasi::add_to_linker_async(&mut linker)?;

    // Step 2: wasi:http/{outgoing-handler, types}. Components can make
    // outbound HTTP calls. Egress enforcement is wired via
    // RuntimeState's WasiHttpView::send_request override.
    // Requires `RuntimeState: WasiHttpView` — implemented in runtime.rs.
    wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;

    // Step 3: edge:cloud/* — registers each Host impl individually.
    // RuntimeState implements all six below in runtime.rs.
    edge_cloud_add_to_linker_get_host(&mut linker)?;

    Ok(linker)
}

/// Create a linker for the `edge-runtime-handler` (FaaS) world.
///
/// Handler components export `wasi:http/incoming-handler` and the host
/// owns the TCP listener. The wasi:* surface is identical to the
/// long-running world (handler guests CAN use wasi:sockets to make
/// outbound TCP — but in practice they shouldn't need to).
pub fn create_component_linker_handler(engine: &Engine) -> Result<ComponentLinker<RuntimeState>> {
    let mut linker: ComponentLinker<RuntimeState> = ComponentLinker::new(engine);

    // Same three steps as long_running (handler has the same wasi:cli/
    // command include; the only difference is the incoming-handler
    // EXPORT, which ProxyPre routes through at instantiation time).
    wasmtime_wasi::add_to_linker_async(&mut linker)?;
    wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;
    edge_cloud_add_to_linker_get_host(&mut linker)?;

    Ok(linker)
}

/// Register every `edge:cloud@0.2.0/*` Host impl on the linker.
///
/// Each `add_to_linker` is bindgen-generated per-interface and has the
/// shape `fn add_to_linker<T, U>(linker, get: impl Fn(&mut T) -> &mut U)`
/// requiring `T: Send` and `U: <iface>::Host + Send`. We pass a plain
/// function pointer instead of a closure so the lifetime annotations
/// are unambiguous (a `Fn(&'a mut T) -> &'a mut U` closure type is what
/// the trait wants; function pointers have explicit `'a` lifetimes).
///
/// `RuntimeState` implements every `edge:cloud::*:Host` in `runtime.rs`
/// and is Send (every field is `Send` via Arc/ParkingLot RwLock
/// patterns), so `U = RuntimeState` works for every interface.
///
/// Both worlds share the `edge:cloud@0.2.0` package, so the bindgens
/// generate the SAME interface module paths under
/// `edge_runtime_long::edge::cloud::*` and
/// `edge_runtime_handler::edge::cloud::*`. We register each interface
/// once via the long_running bindgen's path (equivalent to the handler
/// one's — same underlying Host impl since the WIT bodies are
/// identical; the bindgens differ only in their generated trait
/// namespaces, not the host impls).
fn host_getter(state: &mut RuntimeState) -> &mut RuntimeState {
    state
}

fn edge_cloud_add_to_linker_get_host(linker: &mut ComponentLinker<RuntimeState>) -> Result<()> {
    use crate::edge_runtime_long::edge::cloud as long_cloud;

    macro_rules! register_host {
        ($mod:ident) => {{
            long_cloud::$mod::add_to_linker(linker, host_getter)?;
        }};
    }

    register_host!(cache);
    register_host!(kv_store);
    register_host!(observe);
    register_host!(time);
    register_host!(scheduling);
    register_host!(process);

    Ok(())
}
