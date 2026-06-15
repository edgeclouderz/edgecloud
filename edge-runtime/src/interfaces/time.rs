//! `edge:time` — monotonic clock and sleep.

#[derive(Default)]
pub struct Clock;

impl Clock {
    pub fn new() -> Self {
        Self {}
    }

    pub fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }

    /// Sleep — uses tokio's timer so it integrates with the async runtime.
    /// Note: this still blocks the calling thread (inherent to sync sleep).
    pub fn sleep(&self, duration_ms: u64) -> Result<(), String> {
        let rt = tokio::runtime::Handle::current();
        let duration = std::time::Duration::from_millis(duration_ms);
        rt.block_on(tokio::time::sleep(duration));
        Ok(())
    }

    pub fn resolution(&self) -> u64 {
        100 // nanoseconds
    }
}
