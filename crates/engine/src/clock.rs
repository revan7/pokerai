//! The engine's single time source.
//!
//! Every deadline, budget and progress timestamp in the engine is measured through a `Clock`, so a
//! test can substitute a driven fake (Task 19, `testing` feature) and get deterministic behaviour
//! instead of racing wall time. Nothing in the engine calls `Instant::now` or `sleep` directly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
    /// Blocks until `now_ms() >= t_ms` (returns at once when already past).
    fn wait_until(&self, t_ms: u64);
    /// `wait_until`, ended early by a stop (ruling 29-I2: the engine's teardown wakes and joins every thread it
    /// started): returns `true` when it returned because `stop` was set, `false` when the time came. The stopper sets
    /// `stop` and then calls `wake_waiters`; a stop already set when the wait starts returns at once. The engine's two
    /// clocks (`SystemClock`, the fake of `testing`) end the wait as soon as they are woken; this default, for a test
    /// clock that wraps one, cannot be interrupted: it waits for the time and then reports the stop.
    fn wait_until_or_stopped(&self, t_ms: u64, stop: &AtomicBool) -> bool {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        self.wait_until(t_ms);
        stop.load(Ordering::SeqCst)
    }
    /// Wakes every thread in `wait_until_or_stopped` to check its stop again (the stopper has set it first).
    fn wake_waiters(&self) {}
}

/// Monotonic wall-clock time, counted in milliseconds from the moment this clock was created.
pub struct SystemClock {
    origin: Instant,
    /// Where `wait_until_or_stopped` waits, so `wake_waiters` can end it (a `sleep` cannot be interrupted).
    waiters: Mutex<()>,
    woken: Condvar,
}

impl SystemClock {
    pub fn new() -> Self {
        Self { origin: Instant::now(), waiters: Mutex::new(()), woken: Condvar::new() }
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

    /// Waits on a condition variable, timed by this clock, and checks `stop` under its lock, which `wake_waiters` takes
    /// before it notifies: a stop set before the check is seen, and one set after it wakes the wait.
    fn wait_until_or_stopped(&self, t_ms: u64, stop: &AtomicBool) -> bool {
        let mut guard = self.waiters.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if stop.load(Ordering::SeqCst) {
                return true;
            }
            let now = self.now_ms();
            if now >= t_ms {
                return false;
            }
            guard = self.woken.wait_timeout(guard, Duration::from_millis(t_ms - now)).unwrap_or_else(|poisoned| poisoned.into_inner()).0;
        }
    }

    fn wake_waiters(&self) {
        let _guard = self.waiters.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        self.woken.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A wait whose time has come reports the time (`false`); one whose stop is already set returns at once (`true`).
    #[test]
    fn a_due_wait_reports_the_time_and_a_stopped_one_returns_at_once() {
        let clock = SystemClock::new();
        let stop = AtomicBool::new(false);
        assert!(!clock.wait_until_or_stopped(0, &stop));
        stop.store(true, Ordering::SeqCst);
        assert!(clock.wait_until_or_stopped(u64::MAX, &stop));
    }

    /// A thread waiting for a far time is woken by its stop (the flag set, then `wake_waiters`) and reports it, whether
    /// the wake lands before or while it waits. Its far time is the tests' liveness allowance, so a wait that the stop
    /// failed to end returns `false` after it rather than hanging.
    #[test]
    fn a_wait_is_ended_by_its_stop() {
        let clock = Arc::new(SystemClock::new());
        let stop = Arc::new(AtomicBool::new(false));
        let far = clock.now_ms() + u64::try_from(crate::testing::ACK_LIVENESS.as_millis()).unwrap();
        let waiter = { let (clock, stop) = (clock.clone(), stop.clone()); std::thread::spawn(move || clock.wait_until_or_stopped(far, &stop)) };
        stop.store(true, Ordering::SeqCst);
        clock.wake_waiters();
        assert!(waiter.join().unwrap(), "the wait ended because of its stop");
    }
}
