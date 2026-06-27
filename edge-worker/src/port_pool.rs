//! Port allocation with sequential assignment and cooldown.

use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Port pool for allocating TCP ports to apps.
///
/// Ports are allocated sequentially starting at `starting_port`.
/// When a port is released, it enters a cooldown period before being
/// re-available, preventing address reuse conflicts with TIME_WAIT connections.
pub struct PortPool {
    next_port: u16,
    starting_port: u16,
    cooldown_secs: u64,
    /// Ports available for immediate allocation (populated as ports leave cooldown).
    available: HashSet<u16>,
    /// Ports currently in cooldown: (port, release_time).
    cooling_down: Vec<(u16, Instant)>,
}

impl PortPool {
    /// Create a new port pool.
    ///
    /// - `starting_port`: first port to allocate (e.g., 8081)
    /// - `cooldown_secs`: seconds before a released port is re-available
    pub fn new(starting_port: u16, cooldown_secs: u64) -> Self {
        let mut pool = Self {
            next_port: starting_port,
            starting_port,
            cooldown_secs,
            available: HashSet::new(),
            cooling_down: Vec::new(),
        };
        // Pre-populate with a range of available ports for fast O(1) allocation.
        for port in starting_port..(starting_port + 100) {
            pool.available.insert(port);
        }
        pool
    }

    /// Acquire a port for an app. Returns `None` if the pool is exhausted.
    pub fn acquire(&mut self) -> Option<u16> {
        self.reap_cooled_ports();

        // Fast path: try pre-populated available ports.
        if let Some(port) = self.available.iter().copied().next() {
            self.available.remove(&port);
            return Some(port);
        }

        // Sequential fallback: find the next port not currently in cooldown.
        // Caps at 1000 iterations to prevent infinite loops if all ports are in
        // cooldown (e.g., during a burst of restarts).
        let mut attempts = 0u32;
        while attempts < 1000 {
            let port = self.next_port;
            self.next_port = self.next_port.saturating_add(1);
            if self.next_port == u16::MAX {
                self.next_port = self.starting_port;
            }
            attempts += 1;

            // Skip ports currently in cooldown.
            if !self.cooling_down.iter().any(|(p, _)| *p == port) {
                return Some(port);
            }
        }

        // Exhausted: all ports are in cooldown.
        None
    }

    /// Release a port back into cooldown.
    /// Guard against double-release: if the port is already cooling down, this
    /// is a no-op.
    pub fn release(&mut self, port: u16) {
        if self.cooling_down.iter().any(|(p, _)| *p == port) {
            return; // already cooling down
        }
        let release_time = Instant::now() + Duration::from_secs(self.cooldown_secs);
        self.cooling_down.push((port, release_time));
    }

    /// Move cooled ports back into the available set.
    fn reap_cooled_ports(&mut self) {
        let now = Instant::now();
        self.cooling_down.retain(|(port, release_time)| {
            if now >= *release_time {
                self.available.insert(*port);
                false // remove from cooling_down
            } else {
                true // keep in cooling_down
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acquire_and_release() {
        let mut pool = PortPool::new(8081, 60);
        let port = pool.acquire();
        assert!(port.is_some());
        pool.release(port.unwrap());
    }

    #[test]
    fn test_cooldown() {
        let mut pool = PortPool::new(8081, 0); // 0-second cooldown for testing
        let port = pool.acquire().unwrap();
        pool.release(port);
        // With 0-second cooldown, port should be immediately available
        let next = pool.acquire().unwrap();
        assert_eq!(port, next);
    }

    #[test]
    fn test_double_release_ignored() {
        let mut pool = PortPool::new(8081, 60);
        let port = pool.acquire().unwrap();
        pool.release(port);
        pool.release(port); // second release should be a no-op
                            // Port should only be in cooling_down once; acquire returns it after cooldown.
                            // With 0 cooldown it would be immediately available, but with 60s it stays
                            // in cooling_down so acquire falls back to sequential.
                            // Verify by checking the port is NOT in available (since cooldown hasn't passed).
        let next = pool.acquire();
        assert_ne!(next, Some(port));
    }

    #[test]
    fn test_sequential_fallback_skips_cooldown() {
        let mut pool = PortPool::new(8081, 0); // 0-second cooldown so ports are immediately reusable
                                               // Exhaust the pre-populated ports
        let mut ports = Vec::new();
        for _ in 0..100 {
            ports.push(pool.acquire().unwrap());
        }
        // Now fallback kicks in — sequential should work
        let port = pool.acquire().unwrap();
        assert!(port >= 8081);
        // Release it and acquire again (cooldown is 0 so it's immediately available)
        pool.release(port);
        let next = pool.acquire().unwrap();
        assert_eq!(port, next);
    }

    /// When every reachable port is in cooldown, `acquire` must return `None`
    /// rather than block or panic. The sequential fallback caps at 1000
    /// iterations and returns `None` when every checked port is cooling
    /// down.
    ///
    /// To exercise this, we pick `starting_port = u16::MAX - 100` so the
    /// sequential search wraps within its 1000-iteration budget: the
    /// pre-populated 100-port range becomes the only range the fallback
    /// can ever check. Drain those 100 ports and release them — they
    /// enter cooldown with the 60s default — then a fresh acquire must
    /// return `None`.
    #[test]
    fn acquire_returns_none_after_exhaustion() {
        const START: u16 = u16::MAX - 100; // 65435
        let mut pool = PortPool::new(START, 60);

        // Drain the 100 pre-populated ports (range START..START+100).
        let mut drained: Vec<u16> = Vec::with_capacity(100);
        for _ in 0..100 {
            drained.push(
                pool.acquire()
                    .expect("pre-populated ports should drain cleanly"),
            );
        }
        // Release them all → each enters cooldown.
        for port in &drained {
            pool.release(*port);
        }

        // Now `available` is empty, and the sequential fallback's entire
        // 1000-iteration budget lands on ports in cooldown (the pre-populated
        // range + the wrap to itself). acquire must return None.
        assert!(
            pool.acquire().is_none(),
            "pool should be exhausted once every reachable port is in cooldown"
        );
    }
}
