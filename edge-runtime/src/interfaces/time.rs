//! `edge:time` — monotonic clock and sleep.

#[cfg(feature = "time")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "time")]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

#[cfg(feature = "time")]
pub fn resolution() -> u64 {
    100
}

#[cfg(feature = "time")]
pub fn sleep(_caller: &mut wasmtime::Caller<'_, ()>, duration_ms: u64) -> Result<(), String> {
    std::thread::sleep(std::time::Duration::from_millis(duration_ms));
    Ok(())
}