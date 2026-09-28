//! The engine's single time source.
//!
//! Every deadline, budget and progress timestamp in the engine is measured through a `Clock`, so a
//! test can substitute a driven fake (Task 19, `testing` feature) and get deterministic behaviour
//! instead of racing wall time. Nothing in the engine calls `Instant::now` or `sleep` directly.
//!
//! Bounded slices (final review M1; spec 12, suspend/resume during a request). Every timed wait in the engine re-reads
//! its clock after each wake and never trusts a single OS timeout: Windows excludes the time a machine spends suspended
//! from wait timeouts while the monotonic clock goes on counting it, so a waiter given its whole remaining time as one
//! OS timeout would resume that long after its deadline. `SystemClock`'s waits (and so the watchdog's), the worker
//! link's receives (`worker::process`) and the solve client's receives (`solve`) each wait at most `WAIT_SLICE_MS` at a
//! time and then compare the clock with their deadline again; after a resume a waiter is at most one slice late.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// The longest single OS wait of any timed wait in the engine (see "Bounded slices" above): also the longest a receive
/// of the solve client lasts, so a supersession is noticed within it (final review I1).
pub const WAIT_SLICE_MS: u64 = 100;

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
    /// Unit tests only: a suspend's effect on the clock (see `tests`).
    #[cfg(test)]
    seam: tests::Seam,
}

impl SystemClock {
    pub fn new() -> Self {
        Self { origin: Instant::now(), waiters: Mutex::new(()), woken: Condvar::new(), #[cfg(test)] seam: tests::Seam::default() }
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
        let now = u64::try_from(elapsed_ms).expect("process uptime in milliseconds must fit in u64");
        #[cfg(test)]
        let now = now + self.seam.skew_ms.load(Ordering::SeqCst);
        now
    }

    /// Sleeps in slices of at most `WAIT_SLICE_MS`, re-reading the clock after each (see "Bounded slices" above).
    fn wait_until(&self, t_ms: u64) {
        loop {
            let now = self.now_ms();
            if now >= t_ms {
                return;
            }
            std::thread::sleep(Duration::from_millis((t_ms - now).min(WAIT_SLICE_MS)));
        }
    }

    /// Waits on a condition variable, timed by this clock in slices of at most `WAIT_SLICE_MS` (see "Bounded slices"
    /// above), and checks `stop` under its lock, which `wake_waiters` takes before it notifies: a stop set before the
    /// check is seen, and one set after it wakes the wait.
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
            #[cfg(test)]
            self.seam.note_wait();
            let slice = Duration::from_millis((t_ms - now).min(WAIT_SLICE_MS));
            guard = self.woken.wait_timeout(guard, slice).unwrap_or_else(|poisoned| poisoned.into_inner()).0;
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
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;

    /// A suspend as the engine sees it: the monotonic clock jumps (`skew_ms` is added to every reading) while no OS wait
    /// timeout moves and nothing is woken. `note_wait` acknowledges each OS wait a waiter enters, with the waiters' lock
    /// held, so a test that has seen the count and then takes that lock knows the waiter is inside its wait.
    #[derive(Default)]
    pub(super) struct Seam {
        pub(super) skew_ms: AtomicU64,
        waits: Mutex<u64>,
        entered: Condvar,
    }

    impl Seam {
        pub(super) fn note_wait(&self) {
            *self.waits.lock().unwrap() += 1;
            self.entered.notify_all();
        }

        /// Blocks until `n` waits were entered; fails after the tests' liveness allowance.
        fn wait_for_waits(&self, n: u64) {
            let deadline = Instant::now() + crate::testing::ACK_LIVENESS;
            let mut waits = self.waits.lock().unwrap();
            while *waits < n {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "SystemClock: {n} wait(s) awaited, {} entered, no acknowledgement within the liveness bound", *waits);
                waits = self.entered.wait_timeout(waits, left).unwrap().0;
            }
        }
    }

    /// Final review M1 (spec 12's suspend row): Windows excludes the time a machine is suspended from wait timeouts,
    /// while the monotonic clock goes on counting it, so a waiter that trusted one OS timeout would resume long after its
    /// time. A waiter re-reads the clock after every bounded slice. Here the waiter's time is ten liveness allowances
    /// away; once it is inside its OS wait, the clock jumps past that time without waking anyone (a suspend): the waiter
    /// notices at its next slice and reports the time, well within one liveness allowance.
    #[test]
    fn a_wait_rereads_the_clock_after_each_bounded_slice() {
        let clock = Arc::new(SystemClock::new());
        let span = 10 * u64::try_from(crate::testing::ACK_LIVENESS.as_millis()).unwrap();
        let far = clock.now_ms() + span;
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let clock = clock.clone();
            std::thread::spawn(move || { let _ = tx.send(clock.wait_until_or_stopped(far, &AtomicBool::new(false))); });
        }
        clock.seam.wait_for_waits(1);
        drop(clock.waiters.lock().unwrap()); // the waiter holds this lock until it is inside its OS wait
        clock.seam.skew_ms.store(span, Ordering::SeqCst);
        let stopped = rx.recv_timeout(crate::testing::ACK_LIVENESS).expect("the waiter noticed the clock's jump within one slice, not at its OS timeout");
        assert!(!stopped, "the wait ended because its time came");
    }

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
