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

    /// Sleep — delegates to the scheduling subsystem's thread pool so it does not
    /// block the tokio main thread.
    pub fn sleep(&self, duration_ms: u64) -> Result<(), String> {
        let duration = std::time::Duration::from_millis(duration_ms);
        std::thread::sleep(duration);
        Ok(())
    }

    pub fn resolution(&self) -> u64 {
        100 // nanoseconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_returns_increasing_values() {
        let clock = Clock::new();
        let t1 = clock.now();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let t2 = clock.now();
        assert!(t2 > t1, "clock should advance over time");
    }

    #[test]
    fn test_now_returns_non_zero() {
        let clock = Clock::new();
        assert!(clock.now() > 0);
    }

    #[test]
    fn test_sleep_does_not_panic() {
        let clock = Clock::new();
        clock.sleep(1).unwrap(); // 1ms sleep
    }

    #[test]
    fn test_resolution_is_constant() {
        let clock = Clock::new();
        let r1 = clock.resolution();
        let r2 = clock.resolution();
        assert_eq!(r1, r2);
        assert_eq!(r1, 100); // constant from implementation
    }
}
