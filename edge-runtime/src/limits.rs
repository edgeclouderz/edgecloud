//! Memory enforcement via wasmtime's StoreLimitsBuilder.

use wasmtime::StoreLimitsBuilder;

/// Create a MemoryLimits configured for a given max memory in MB.
pub fn new_memory_limits(max_memory_mb: u64) -> wasmtime::StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size((max_memory_mb * 1024 * 1024) as usize)
        .table_elements(100_000)
        // WASI Preview 2 components embed multiple core wasm instances
        // internally (one per WASI interface). An instance limit of 1
        // was correct for v0.1 (single core module) but blocks v0.2
        // components. 10 is a safe upper bound — typical WASI P2
        // components use 3-5 instances.
        .instances(10)
        .memories(1)
        .build()
}
