//! The engine's watchdog of spec section 7, independent of the worker client.
//!
//! Each `arm` starts one generation, served by its own `watchdog` thread (§3.4) that waits on the injected `Clock`
//! and on nothing else: no worker, no `engine-main`, no channel. At the street deadline it records that the live
//! generation reached it; at `final delivery - 100 ms`, if the request's decision is still the active one, it delivers
//! the request's one `Final`: the retained payload (a `Provisional` or an earlier `best_so_far`) when there is one, else
//! the fallback `Unsupported{DeadlineExceeded{stage}}` with the stage the request reached by then. The `Final` goes out
//! whatever the worker is doing, and the cancellation, kill or restart of a busy worker proceeds independently (§7,
//! §12).
//!
//! Street deadline. Whether the first attempt met its street deadline is judged from the engine-clock time its
//! terminal arrived, which the engine publishes in the request's `StreetDeadline`, never from when the watchdog thread
//! happens to resume (ruling 20-I1): a terminal 1 ms late is a violation even when it is published before the
//! watchdog wakes, and one on time is not a violation even when it is published after. The watchdog's own record, that
//! the live generation reached the deadline, decides only while no terminal has been published.
//!
//! Generations. Only the most recent generation is live: `arm` retires the one before it, and `disarm` retires the
//! live one. A retired generation does nothing at its times, not even its street-deadline record, and its thread ends
//! at its next wake-up. The `Clock` has no way to interrupt a wait, so a retired thread still sleeps until its next
//! deadline (at most the flop's `5 s + flop_budget_s`) before it ends.
//!
//! One `Final` per request. `delivered` is shared with the engine's own delivery path: whichever side swaps it from
//! false to true first delivers, and the other stays silent. A request whose `Final` was already delivered is never
//! armed, nor is a decision identity the watchdog already fired for (both asserted).
//!
//! Identity at the fire (follow-up P2.W3; spec 12, a stale event is discarded by `engine-main`; spec 13.3). The fire
//! itself is the guard against a superseded decision: under the session's identity lock (`Armed::identity_state`, the
//! engine's `EngineCore::identity`) it checks that the armed decision is still the active one (`IdentityState::is_active`,
//! the check `serve_request` makes) and only then claims `delivered`, in one step, the step `serve_request`'s own
//! acceptance makes under the same lock. A decision no longer active emits nothing, records nothing (neither `fired`
//! nor the identity the watchdog fired for), leaves `delivered` unclaimed, and its thread ends: the watchdog has retired
//! for it. Supersession therefore needs no retirement hook. A newer request's `arm` retires the older generation as
//! before, and a supersession without a new arm (an undo, a newer decision whose request still waits behind its
//! predecessor's 1.5 s cancel window, any other invalidating call of `IdentityState`) is linearized with the fire by the
//! identity lock: once the invalidating call has returned, no fire of an earlier decision claims or emits anything; a
//! fire that claimed first delivers the `Final` of a decision active at its claim, as `serve_request`'s own delivery
//! does when a mutation lands between its acceptance and its sink callback (the UI refuses that one by identity). A
//! decision still active behaves exactly as before. The identity lock is released before the `Final` is handed to the
//! sink (ruling 28-I1: no engine lock is held during a sink callback), so the callback may read the identity or
//! supersede the decision it receives.
//!
//! Locking. Waiting holds no lock. The generation lock is taken to check liveness and, at the fire, held from the
//! liveness check through the emission, so `arm` and `disarm` are linearized with a fire: once either returns, no
//! earlier generation emits anything. Lock order: generation, then the identity lock (taken alone under the generation
//! lock for the check and the claim, and released before any other lock is taken), then `retained`, then `fallback`,
//! then `stage`, then `fired`, then the sink; the street-deadline state is taken alone or under the generation lock
//! alone, and no other lock is taken while it is held. The generation lock is the watchdog's own, not one of the
//! engine's (identity, stage, snapshot store, config, range source), none of which is held during the callback. A
//! caller must therefore never call `arm` or `disarm` while holding the identity lock, nor the sink, `retained`,
//! `fallback`, `stage` or `fired` lock of an armed request (a sink callback runs under the sink lock, so it never arms
//! or disarms).
//!
//! What was delivered. A fire records the `Final` it emits, and the engine-clock time it read just before, in the
//! request's `fired` slot, under the generation lock and before the emission: once `disarm` returns, the engine finds
//! there the `Final` the watchdog delivered, to log it as the request's (ruling 28-I2).
//!
//! Test seam. With the `testing` feature (or in this crate's unit tests) a `Watchdog` also counts its ended threads, so
//! a test waits for a fire or a retirement to be over (`wait_for_ended_threads`, or `ended_threads` from a test double
//! running while a request holds the engine) instead of yielding or sleeping (ruling 20-I2), and can see that a fire
//! holds the generation lock (`retirement_would_block`). Nothing of it is compiled into a build without the feature.

use crate::clock::Clock;
use crate::identity::IdentityState;
use crate::EventSink;
use proto::{Coverage, DecisionIdentity, Phase, Recommendation, RecommendationEvent, UnsupportedReason};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// The sink of one request's events, shared by `engine-main`, the `fast-path` thread and the watchdog.
pub type SharedSink = Arc<Mutex<Box<dyn EventSink>>>;

/// The street deadline of one request (§7): `t0 + street budget`, when the first attempt's terminal arrived, and
/// whether the deadline was met. The engine publishes the terminal's arrival (`terminal_arrived`); the watchdog records
/// that its live generation reached the deadline. Both write this one coordinated state (ruling 20-I1).
///
/// The verdict (`violated`) is judged from the terminal's engine-clock arrival time, never from when a thread runs:
/// once a terminal is published, the deadline was violated exactly when that terminal arrived after `deadline_ms`
/// (one arriving at `deadline_ms` is on time), whether it was published before or after the watchdog reached the
/// deadline, and whether or not that watchdog generation is still live. With no terminal published, the deadline
/// counts as violated once the live watchdog generation has reached it; a terminal published later with an arrival
/// at or before the deadline was on time after all, and the verdict then says so.
pub struct StreetDeadline {
    deadline_ms: u64,
    state: Mutex<StreetState>,
}

struct StreetState {
    /// The engine-clock time the first attempt's terminal arrived, once published.
    arrival_ms: Option<u64>,
    /// The live watchdog generation has reached the deadline.
    reached: bool,
}

impl StreetDeadline {
    /// The street deadline at `deadline_ms` (`Deadlines::street_deadline_ms`), with no terminal and not yet reached.
    pub fn new(deadline_ms: u64) -> Self {
        Self { deadline_ms, state: Mutex::new(StreetState { arrival_ms: None, reached: false }) }
    }

    pub fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }

    /// Publishes `at_ms`, the engine-clock time at which the first attempt's terminal arrived. That is the time read
    /// when the terminal arrived, not the time of this call: an engine that publishes after further work passes the
    /// arrival time it kept. The first publication stands; a later one is ignored.
    pub fn terminal_arrived(&self, at_ms: u64) {
        let mut s = lock(&self.state);
        if s.arrival_ms.is_none() {
            s.arrival_ms = Some(at_ms);
        }
    }

    /// The first attempt's terminal's arrival time, once published.
    pub fn terminal_arrival_ms(&self) -> Option<u64> {
        lock(&self.state).arrival_ms
    }

    /// Whether the first attempt's terminal has been published (the plan's boolean `terminal_seen`).
    pub fn terminal_seen(&self) -> bool {
        lock(&self.state).arrival_ms.is_some()
    }

    /// Whether the first attempt missed the street deadline (see the type's doc).
    pub fn violated(&self) -> bool {
        let s = lock(&self.state);
        match s.arrival_ms {
            Some(at_ms) => at_ms > self.deadline_ms,
            None => s.reached,
        }
    }

    /// The live watchdog generation has reached the deadline.
    fn reach(&self) {
        lock(&self.state).reached = true;
    }
}

/// One request as the watchdog sees it.
pub struct Armed {
    /// The decision the `Final` answers.
    pub identity: DecisionIdentity,
    /// `t0 + street budget` with the first-attempt terminal's arrival and the verdict, shared with the engine.
    pub street_deadline: Arc<StreetDeadline>,
    /// `final delivery - WATCHDOG_LEAD_MS` (`Deadlines::watchdog_fire_ms`).
    pub fire_ms: u64,
    /// A `Provisional` or an earlier `best_so_far` of this decision (plan 4 fills it); taken at the fire.
    pub retained: Arc<Mutex<Option<Recommendation>>>,
    /// `Unsupported{DeadlineExceeded{stage}}` for this decision, with the equity so far; the stage is filled in at the fire.
    /// Shared so the engine can refresh it as the request learns more (the range source's reasons, the assumptions:
    /// ruling 28-I6); read at the fire. The engine keeps it this decision's `DeadlineExceeded` fallback, as `arm` checks.
    pub fallback: Arc<Mutex<Recommendation>>,
    /// The furthest stage the request has reached (`EngineCore::stage`).
    pub stage: Arc<Mutex<String>>,
    pub sink: SharedSink,
    /// Set by whichever side delivers the request's `Final`.
    pub delivered: Arc<AtomicBool>,
    /// The `Final` this watchdog delivered and when (ruling 28-I2); empty unless it fired and delivered.
    pub fired: Arc<Mutex<Option<Fired>>>,
    /// The session's decision identity (`EngineCore::identity`), where the fire checks that `identity` is still the
    /// active decision (follow-up P2.W3; see "Identity at the fire" above).
    pub identity_state: Arc<Mutex<IdentityState>>,
}

/// The `Final` a fire delivered and the engine-clock time read immediately before its emission.
#[derive(Debug, Clone, PartialEq)]
pub struct Fired {
    pub at_ms: u64,
    pub rec: Recommendation,
}

/// The live generation, and the decision the watchdog last fired for.
struct Generations {
    live: u64,
    fired: Option<DecisionIdentity>,
}

/// §7: independent of the worker client. At the street deadline it records that the live generation reached it (the
/// verdict is the `StreetDeadline`'s); at `final delivery - 100 ms` it emits `Final` with the retained payload or
/// `DeadlineExceeded`, for a decision still active (follow-up P2.W3). `disarm` retires the armed generation; a retired
/// thread wakes at its times and does nothing.
pub struct Watchdog {
    clock: Arc<dyn Clock>,
    generations: Arc<Mutex<Generations>>,
    #[cfg(any(test, feature = "testing"))]
    ends: Arc<seam::Ends>,
}

/// The watchdog is the delivery of last resort: a panic elsewhere while one of these locks was held must not stop the
/// `Final`. Every value behind them stays consistent at every point a panic could interrupt it (a counter, an
/// `Option` taken or not, a string replaced whole, a sink call, a time set once, a flag; the identity state's calls
/// panic, on a counter overflow, before they change anything `IdentityState::is_active` reads, all the fire reads).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Watchdog {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            generations: Arc::new(Mutex::new(Generations { live: 0, fired: None })),
            #[cfg(any(test, feature = "testing"))]
            ends: Arc::new(seam::Ends::default()),
        }
    }

    /// Starts a new generation for `a`, retiring the previous one, on a thread of its own.
    pub fn arm(&self, a: Armed) {
        assert!(
            !a.delivered.load(Ordering::SeqCst),
            "watchdog armed for decision {:?} after its Final was delivered",
            a.identity
        );
        assert!(lock(&a.fired).is_none(), "watchdog armed for decision {:?} with a Final already recorded as delivered", a.identity);
        assert!(
            a.street_deadline.deadline_ms() <= a.fire_ms,
            "watchdog armed with the street deadline {} ms after its fire time {} ms",
            a.street_deadline.deadline_ms(),
            a.fire_ms
        );
        {
            let fallback = lock(&a.fallback);
            assert!(
                fallback.identity == a.identity,
                "watchdog armed for decision {:?} with the fallback of decision {:?}",
                a.identity,
                fallback.identity
            );
            assert!(
                matches!(fallback.coverage, Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { .. }, .. }),
                "watchdog fallback must be Unsupported{{DeadlineExceeded}}, got {:?}",
                fallback.coverage
            );
        }
        let generation = {
            let mut g = lock(&self.generations);
            assert!(
                g.fired.as_ref() != Some(&a.identity),
                "watchdog armed for decision {:?}, which it already fired for",
                a.identity
            );
            g.live = g.live.checked_add(1).expect("watchdog generation counter overflowed u64");
            g.live
        };
        let (clock, generations) = (self.clock.clone(), self.generations.clone());
        #[cfg(any(test, feature = "testing"))]
        let ends = self.ends.clone();
        std::thread::Builder::new()
            .name("watchdog".into())
            .spawn(move || {
                // Counts this thread's end once `watch` has returned (or unwound): after its last action.
                #[cfg(any(test, feature = "testing"))]
                let _end = seam::End(&ends);
                watch(clock.as_ref(), &generations, generation, a)
            })
            .expect("spawn the watchdog thread");
    }

    /// Retires the live generation: it emits nothing and records nothing from now on. Once `disarm` returns, no fire
    /// of an earlier generation is in progress.
    pub fn disarm(&self) {
        let mut g = lock(&self.generations);
        g.live = g.live.checked_add(1).expect("watchdog generation counter overflowed u64");
    }
}

/// Test seam (ruling 20-I2): acknowledgements a test waits for instead of yielding, spinning or sleeping.
#[cfg(any(test, feature = "testing"))]
impl Watchdog {
    /// Blocks until `n` of this watchdog's generation threads have ended. A thread ends right after its last action:
    /// the emission of its `Final`, or its return on finding its generation retired or its `Final` already delivered,
    /// so a test asserts what a fire or a retirement did, or did not do, only after this acknowledgement. It waits on
    /// a condition variable that each thread's end notifies, never on time.
    pub fn wait_for_ended_threads(&self, n: u64) {
        self.ends.wait_for(n);
    }

    /// Whether `arm` and `disarm` would block now because a thread holds the generation lock, as a fire does from its
    /// liveness check through its emission. Never true while every generation thread is waiting on the clock.
    pub fn retirement_would_block(&self) -> bool {
        matches!(self.generations.try_lock(), Err(std::sync::TryLockError::WouldBlock))
    }

    /// A handle on this watchdog's count of ended generation threads that does not borrow the watchdog, for a test double
    /// that runs on `engine-main` while a request holds the `EngineCore` (a worker link, a clock, a `serve_request`
    /// seam). It acknowledges a fire that emits nothing, a superseded decision's (follow-up P2.W3), which no sink can.
    pub fn ended_threads(&self) -> EndedThreads {
        EndedThreads(self.ends.clone())
    }
}

/// A watchdog's count of ended generation threads (`Watchdog::ended_threads`).
#[cfg(any(test, feature = "testing"))]
#[derive(Clone)]
pub struct EndedThreads(Arc<seam::Ends>);

#[cfg(any(test, feature = "testing"))]
impl EndedThreads {
    /// Blocks until `n` of the watchdog's generation threads have ended (as `Watchdog::wait_for_ended_threads`), and
    /// fails naming that acknowledgement after `testing::ACK_LIVENESS` of wall time (ruling 20-A: a test-liveness
    /// allowance, never a condition on the engine's time).
    pub fn wait_for(&self, n: u64) {
        self.0.wait_for_within(n, crate::testing::ACK_LIVENESS);
    }
}

#[cfg(any(test, feature = "testing"))]
mod seam {
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};

    /// How many generation threads have ended, and the condition variable each end notifies.
    #[derive(Default)]
    pub(super) struct Ends {
        count: Mutex<u64>,
        ended: Condvar,
    }

    impl Ends {
        pub(super) fn wait_for(&self, n: u64) {
            let mut count = super::lock(&self.count);
            while *count < n {
                count = self.ended.wait(count).unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }

        /// `wait_for`, failing after `bound` of wall time.
        pub(super) fn wait_for_within(&self, n: u64, bound: Duration) {
            let deadline = Instant::now().checked_add(bound).expect("the liveness bound fits an Instant");
            let mut count = super::lock(&self.count);
            while *count < n {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    let ended = *count;
                    drop(count);
                    panic!("Watchdog: {n} ended generation thread(s) awaited, {ended} ended, no acknowledgement within the {bound:?} liveness bound");
                }
                count = self.ended.wait_timeout(count, left).unwrap_or_else(|poisoned| poisoned.into_inner()).0;
            }
        }
    }

    /// Counts one ended thread when dropped, on every way out of the thread's body.
    pub(super) struct End<'a>(pub(super) &'a Ends);

    impl Drop for End<'_> {
        fn drop(&mut self) {
            *super::lock(&self.0.count) += 1;
            self.0.ended.notify_all();
        }
    }
}

/// The fire's decision (see "Identity at the fire" above): under the identity lock, whether `a.identity` is still the
/// active decision and, if it is, the request's once-only `Final` claimed. True when this fire won the claim. The
/// identity lock is released on return, before anything is emitted.
fn claim_if_active(a: &Armed) -> bool {
    let ids = lock(&a.identity_state);
    ids.is_active(&a.identity) && !a.delivered.swap(true, Ordering::SeqCst)
}

/// The body of one generation's thread.
fn watch(clock: &dyn Clock, generations: &Mutex<Generations>, generation: u64, a: Armed) {
    clock.wait_until(a.street_deadline.deadline_ms());
    {
        let g = lock(generations);
        if g.live != generation {
            return;
        }
        // Only that the live generation reached the deadline: the verdict is judged from the terminal's arrival time
        // (`StreetDeadline::violated`), not from when this thread resumed.
        a.street_deadline.reach();
    }
    clock.wait_until(a.fire_ms);
    // Held through the emission: see "Locking" above.
    let mut g = lock(generations);
    if g.live != generation || !claim_if_active(&a) {
        return;
    }
    g.fired = Some(a.identity.clone());
    let mut rec = lock(&a.retained).take().unwrap_or_else(|| lock(&a.fallback).clone());
    rec.phase = Phase::Final;
    if let Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } = &mut rec.coverage {
        *stage = lock(&a.stage).clone();
    }
    // Recorded before the emission, under the generation lock: see "What was delivered" above.
    *lock(&a.fired) = Some(Fired { at_ms: clock.now_ms(), rec: rec.clone() });
    lock(&a.sink).emit(RecommendationEvent::Final(rec));
    drop(g);
}
