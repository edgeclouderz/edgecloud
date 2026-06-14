//! edge-runtime: WASI Preview 2 host interfaces for edge computing.

pub mod engine;
pub mod limits;
pub mod linker;
pub mod memory;
pub mod metering;
pub mod store;

pub mod interfaces;

pub use engine::create_engine;
pub use store::create_store;
pub use metering::RequestMeter;