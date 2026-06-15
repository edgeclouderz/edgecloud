# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

edgeCloud is a managed WebAssembly edge computing platform. This repository contains the **edge-runtime** Rust library — a Wasmtime-based runtime that exposes WASI Preview 2 host interfaces (`edge:*`) to Wasm modules running on the platform.

## Build Commands

```bash
# Build
cargo build --manifest-path edge-runtime/Cargo.toml
cargo build --manifest-path edge-runtime/Cargo.toml --release

# Test
cargo test --manifest-path edge-runtime/Cargo.toml
cargo test --manifest-path edge-runtime/Cargo.toml -- <test-name>  # single test

# Lint
cargo clippy --all-targets --all-features --manifest-path edge-runtime/Cargo.toml -- -D warnings
cargo fmt --check --manifest-path edge-runtime/Cargo.toml
```

CI pipeline: `.gitlab-ci.yml` — runs fmt, clippy, audit, test, build:debug, build:release on MRs and main.

## Architecture

### Core Components

The runtime is structured around a single WIT world defined **inline** in `src/lib.rs` (the `edge-runtime` world). The `wasmtime::component::bindgen!` macro generates Rust bindings from this inline definition at compile time.

**Key modules:**

| File | Role |
|------|------|
| `src/lib.rs` | WIT world definition + public exports (`create_engine`, `create_store`, `RuntimeState`) |
| `src/engine.rs` | wasmtime `Engine` creation with security-hardened config (no threads, no reference types, SIMD enabled, component model enabled, epoch interruption enabled) |
| `src/runtime.rs` | `RuntimeState` struct implementing all WIT Host traits via delegation to sub-components |
| `src/linker.rs` | `create_linker` (core wasm/P1) and `create_component_linker` (WASI P2) |
| `src/store.rs` | wasmtime `Store` creation |
| `src/memory.rs` | Host-to-wasm memory access helpers: `read_string`, `write_string`, `read_bytes`, `write_bytes`, `allocate`, `get_memory` |
| `src/limits.rs` | `StoreLimits` configuration via `StoreLimitsBuilder` |
| `src/interfaces/` | Per-interface host implementations (feature-gated) |

### Interfaces

Each `edge:*` interface lives in its own feature-gated module under `src/interfaces/`:

- `http-client` — outbound HTTP via `reqwest`
- `kv-store` — key-value persistence
- `cache` — in-memory LRU (size-capped)
- `observe` — metrics and logging
- `time` — monotonic clock
- `scheduling` — delayed/repeating tasks via `tokio`
- `process` — env vars, args, exit
- `networking` — DNS resolution
- `http-server` — inbound HTTP serving

The `RuntimeState` in `src/runtime.rs` holds one instance of each and delegates WIT trait calls to them.

### Memory Access Pattern

All host function implementations follow a consistent pattern for crossing the wasm boundary:
1. Get the `memory` export from `Caller`
2. Read args from wasm linear memory via `read_string` / `read_bytes`
3. Perform the operation
4. Write results back via `write_string` / `write_bytes` if needed

The `get_memory` helper must be called **after** any wasm execution — the `Memory` handle is invalidated by `memory.grow()`.

### Feature Flags

Interfaces are conditionally compiled via Cargo features in `Cargo.toml`. The `default` feature set enables all interfaces.

### WIT World Definition

The WIT world is defined inline in `src/lib.rs` under the `package edge:cloud@0.1.0` namespace. If you add a new interface, add it to this inline WIT definition and run `cargo build` to regenerate bindings — no external `wit` tool is required since everything is inline.