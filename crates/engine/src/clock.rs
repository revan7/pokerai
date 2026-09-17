//! The engine's single time source.
//!
//! Every deadline, budget and progress timestamp in the engine is measured through a `Clock`, so a
//! test can substitute a driven fake (Task 19, `testing` feature) and get deterministic behaviour
//! instead of racing wall time. Nothing in the engine calls `Instant::now` or `sleep` directly.

use std::time::{Duration, Instant};

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
    /// Blocks until `now_ms() >= t_ms` (returns at once when already past).
    fn wait_until(&self, t_ms: u64);
}

/// Monotonic wall-clock time, counted in milliseconds from the moment this clock was created.
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self { origin: Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        // Narrowing precondition: `as_millis` is `u128`, so the value is checked in the wide type
        // before it becomes a `u64` rather than being truncated or clamped. `u64` milliseconds span
        // ~5.8e8 years of process uptime, so this can only fire on a broken monotonic clock.
        let elapsed_ms = self.origin.elapsed().as_millis();
        u64::try_from(elapsed_ms).expect("process uptime in milliseconds must fit in u64")
    }

    fn wait_until(&self, t_ms: u64) {
        let now = self.now_ms();
        if t_ms > now {
            std::thread::sleep(Duration::from_millis(t_ms - now));
        }
    }
}
