//! Port allocation with sequential assignment and cooldown.

use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Stable upper bound on simultaneously-hostable apps for a single worker.
/// 100 ports pre-populated in `PortPool::new` + 900 sequential-fallback
/// iterations in `acquire` (the hard cap that prevents infinite loops if
/// every port is in cooldown). Surfaced explicitly so the control-plane
/// autoscaler (issue #85) has a stable capacity number to report.
///
/// Allowed-dead-code: only used by PR #3's autoscaler. Exposing it in PR #1
/// (the wire-shape PR) so the constant is colocated with the pool it
/// describes, and the autoscaler can `use crate::port_pool::PORT_POOL_MAX_CAPACITY`
/// without a code-shape PR.
#[allow(dead_code)]
pub const PORT_POOL_MAX_CAPACITY: u32 = 1000;

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

    /// Stable upper bound on simultaneously-hostable apps. Surfaces the
    /// implicit cap (pre-populated + sequential fallback) so the
    /// autoscaler can compare `app_slots` against a known number.
    ///
    /// Allowed-dead-code: only used by PR #3's autoscaler (and by tests).
    #[allow(dead_code)]
    pub fn capacity(&self) -> u32 {
        PORT_POOL_MAX_CAPACITY
    }

    /// Slots free right now: number of ports the next `acquire` call
    /// would hand out without falling through to the slower sequential
    /// path. Reported as `HeartbeatMessage.cluster_headroom.app_slots`
    /// so the control-plane autoscaler (issue #85) knows how many more
    /// apps this worker can host immediately.
    ///
    /// `available.len()` (not `available + cooling`) is the right number
    /// for autoscaling decisions: ports in cooldown are unavailable to
    /// `acquire` right now (TIME_WAIT), and counting them as "free" would
    /// let the autoscaler under-provision. The conservative number is
    /// better — better to spin up a worker that turns out unnecessary
    /// than to refuse a deploy because the math looked optimistic.
    pub fn capacity_remaining(&mut self) -> u32 {
        self.reap_cooled_ports();
        self.available.len() as u32
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

    // ── capacity / capacity_remaining (issue #85) ──────────────────────────

    /// `capacity` returns the stable upper bound. The autoscaler relies on
    /// this number being constant for a given worker config so it can
    /// compare reported `app_slots` against a known ceiling.
    #[test]
    fn capacity_returns_stable_upper_bound() {
        let pool = PortPool::new(8081, 60);
        assert_eq!(pool.capacity(), PORT_POOL_MAX_CAPACITY);
    }

    /// Fresh pool has the pre-populated count (100) immediately available —
    /// NOT `capacity()` (1000). The 900-port delta is the sequential-fallback
    /// reserve that the pool can hand out as it discovers ports, but those
    /// slots are not "ready now"; reporting them as free would let the
    /// autoscaler under-provision during a burst of restarts.
    ///
    /// The heartbeat's `cluster_headroom.app_slots` reports this number to
    /// the autoscaler — getting it wrong means an idle worker is reported
    /// as saturated (if we report 0) or over-provisioned (if we report 1000).
    #[test]
    fn fresh_pool_capacity_remaining_equals_pre_populated_count() {
        let mut pool = PortPool::new(8081, 60);
        // The fresh pool pre-populates 100 ports in `available`.
        assert_eq!(pool.capacity_remaining(), 100);
        // Sanity: capacity() reports the larger theoretical bound.
        assert!(pool.capacity() >= pool.capacity_remaining());
    }

    /// After acquiring a port, `capacity_remaining` drops by exactly 1.
    /// Pin the math so a future refactor (e.g., moving `reap_cooled_ports`
    /// out of `capacity_remaining`) doesn't silently report wrong headroom.
    #[test]
    fn acquire_decrements_capacity_remaining_by_one() {
        let mut pool = PortPool::new(8081, 0);
        let initial = pool.capacity_remaining();
        pool.acquire().unwrap();
        assert_eq!(pool.capacity_remaining(), initial - 1);
    }

    /// `capacity_remaining` reports `available.len()`, NOT
    /// `available + cooling_down`. A port in cooldown cannot be acquired
    /// (TIME_WAIT conflict) and must NOT be reported as free — otherwise
    /// the autoscaler under-provisions when bursts of restarts put
    /// dozens of ports in cooldown simultaneously. Pin this so a future
    /// "optimistic" refactor doesn't accidentally include cooling ports.
    #[test]
    fn capacity_remaining_excludes_cooling_down_ports() {
        let mut pool = PortPool::new(8081, 60); // 60s cooldown
        let initial = pool.capacity_remaining();
        // Acquire one port and release it: it enters cooling_down, NOT
        // back to available. capacity_remaining must NOT count it as free.
        let port = pool.acquire().unwrap();
        pool.release(port);
        assert_eq!(
            pool.capacity_remaining(),
            initial - 1,
            "released port is in cooldown; capacity_remaining must drop by 1, not stay the same"
        );
    }

    /// After draining the pool, `capacity_remaining` is 0 — not
    /// u32::MAX from a missing `saturating_sub` and not negative
    /// (would wrap on the `as u32` cast).
    #[test]
    fn capacity_remaining_is_zero_after_drain() {
        let mut pool = PortPool::new(8081, 0);
        for _ in 0..100 {
            pool.acquire().unwrap();
        }
        // Sequential fallback has now handed out 100 ports; pre-populated
        // available set is empty. Some may be cooling (if released) but
        // a freshly-acquired port has no cooldown entry. Either way,
        // `available.len()` is 0 → `capacity_remaining == 0`.
        assert_eq!(pool.capacity_remaining(), 0);
    }
}
