//! The engine's watchdog of spec section 7, independent of the worker client.
//!
//! Each `arm` starts one generation, served by its own `watchdog` thread (§3.4) that waits on the injected `Clock`
//! and on nothing else: no worker, no `engine-main`, no channel. At the street deadline it records a street
//! violation if no first-attempt terminal has arrived; at `final delivery - 100 ms` it delivers the request's one
//! `Final`: the retained payload (a `Provisional` or an earlier `best_so_far`) when there is one, else the fallback
//! `Unsupported{DeadlineExceeded{stage}}` with the stage the request reached by then. The `Final` goes out whatever
//! the worker is doing, and the cancellation, kill or restart of a busy worker proceeds independently (§7, §12).
//!
//! Generations. Only the most recent generation is live: `arm` retires the one before it, and `disarm` retires the
//! live one. A retired generation does nothing at its times, not even the street-violation record, and its thread
//! ends at its next wake-up. The `Clock` has no way to interrupt a wait, so a retired thread still sleeps until its
//! next deadline (at most the flop's `5 s + flop_budget_s`) before it ends.
//!
//! One `Final` per request. `delivered` is shared with the engine's own delivery path: whichever side swaps it from
//! false to true first delivers, and the other stays silent. A request whose `Final` was already delivered is never
//! armed, nor is a decision identity the watchdog already fired for (both asserted).
//!
//! Locking. Waiting holds no lock. The generation lock is taken to check liveness and, at the fire, held from the
//! liveness check through the emission, so `arm` and `disarm` are linearized with a fire: once either returns, no
//! earlier generation emits anything. Lock order: generation, then `retained`, then `stage`, then the sink. A caller
//! must therefore never call `arm` or `disarm` while holding the sink, `retained` or `stage` lock of an armed request.

use crate::clock::Clock;
use crate::EventSink;
use proto::{Coverage, DecisionIdentity, Phase, Recommendation, RecommendationEvent, UnsupportedReason};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// The sink of one request's events, shared by `engine-main`, the `fast-path` thread and the watchdog.
pub type SharedSink = Arc<Mutex<Box<dyn EventSink>>>;

/// One request as the watchdog sees it.
pub struct Armed {
    /// The decision the `Final` answers.
    pub identity: DecisionIdentity,
    /// `t0 + street budget`: a first attempt without a terminal by then is a street violation.
    pub street_deadline_ms: u64,
    /// `final delivery - WATCHDOG_LEAD_MS` (`Deadlines::watchdog_fire_ms`).
    pub fire_ms: u64,
    /// A `Provisional` or an earlier `best_so_far` of this decision (plan 4 fills it); taken at the fire.
    pub retained: Arc<Mutex<Option<Recommendation>>>,
    /// `Unsupported{DeadlineExceeded{stage}}` for this decision, with the equity so far; the stage is filled in at the fire.
    pub fallback: Recommendation,
    /// The furthest stage the request has reached (`EngineCore::stage`).
    pub stage: Arc<Mutex<String>>,
    pub sink: SharedSink,
    /// Set by whichever side delivers the request's `Final`.
    pub delivered: Arc<AtomicBool>,
    /// Set by the engine when the first attempt's terminal arrives.
    pub terminal_seen: Arc<AtomicBool>,
    /// Set by the watchdog when the street deadline passed without a first-attempt terminal.
    pub street_violation: Arc<AtomicBool>,
}

/// The live generation, and the decision the watchdog last fired for.
struct Generations {
    live: u64,
    fired: Option<DecisionIdentity>,
}

/// §7: independent of the worker client. At the street deadline it records a violation when no first-attempt terminal
/// arrived; at `final delivery - 100 ms` it emits `Final` with the retained payload or `DeadlineExceeded`. `disarm`
/// retires the armed generation; a retired thread wakes at its times and does nothing.
pub struct Watchdog {
    clock: Arc<dyn Clock>,
    generations: Arc<Mutex<Generations>>,
}

/// The watchdog is the delivery of last resort: a panic elsewhere while one of these locks was held must not stop the
/// `Final`. Every value behind them stays consistent at every point a panic could interrupt it (a counter, an
/// `Option` taken or not, a string replaced whole, a sink call).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Watchdog {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, generations: Arc::new(Mutex::new(Generations { live: 0, fired: None })) }
    }

    /// Starts a new generation for `a`, retiring the previous one, on a thread of its own.
    pub fn arm(&self, a: Armed) {
        assert!(
            !a.delivered.load(Ordering::SeqCst),
            "watchdog armed for decision {:?} after its Final was delivered",
            a.identity
        );
        assert!(
            a.street_deadline_ms <= a.fire_ms,
            "watchdog armed with the street deadline {} ms after its fire time {} ms",
            a.street_deadline_ms,
            a.fire_ms
        );
        assert!(
            a.fallback.identity == a.identity,
            "watchdog armed for decision {:?} with the fallback of decision {:?}",
            a.identity,
            a.fallback.identity
        );
        assert!(
            matches!(a.fallback.coverage, Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { .. }, .. }),
            "watchdog fallback must be Unsupported{{DeadlineExceeded}}, got {:?}",
            a.fallback.coverage
        );
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
        std::thread::Builder::new()
            .name("watchdog".into())
            .spawn(move || watch(clock.as_ref(), &generations, generation, a))
            .expect("spawn the watchdog thread");
    }

    /// Retires the live generation: it emits nothing and records nothing from now on. Once `disarm` returns, no fire
    /// of an earlier generation is in progress.
    pub fn disarm(&self) {
        let mut g = lock(&self.generations);
        g.live = g.live.checked_add(1).expect("watchdog generation counter overflowed u64");
    }
}

/// The body of one generation's thread.
fn watch(clock: &dyn Clock, generations: &Mutex<Generations>, generation: u64, a: Armed) {
    clock.wait_until(a.street_deadline_ms);
    {
        let g = lock(generations);
        if g.live != generation {
            return;
        }
        if !a.terminal_seen.load(Ordering::SeqCst) {
            a.street_violation.store(true, Ordering::SeqCst);
        }
    }
    clock.wait_until(a.fire_ms);
    // Held through the emission: see "Locking" above.
    let mut g = lock(generations);
    if g.live != generation || a.delivered.swap(true, Ordering::SeqCst) {
        return;
    }
    g.fired = Some(a.identity.clone());
    let mut rec = lock(&a.retained).take().unwrap_or_else(|| a.fallback.clone());
    rec.phase = Phase::Final;
    if let Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } = &mut rec.coverage {
        *stage = lock(&a.stage).clone();
    }
    lock(&a.sink).emit(RecommendationEvent::Final(rec));
    drop(g);
}
