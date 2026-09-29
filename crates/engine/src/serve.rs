//! Spec §5 steps 4-10 for one live request on `engine-main` (Task 28): the river, turn and (plan 4 Task 10) flop decision
//! path, and (plan 3 Task 17) the preflop decision path, which `crate::preflop::serve_preflop` answers at the
//! `Classification::Preflop` arm, after the degraded-engine check, through the request's own claim (`settle`) and never
//! with a solve.
//!
//! Admission (final review I1, orchestrator ruling F-I1; spec 7, a watchdog independent of the worker client; spec 5
//! step 4). `Engine::recommend` admits a request through `admit` the moment it allocates its decision, before
//! `engine-main` gets to it: every request at a decision point is armed on the shared watchdog there, with its street
//! deadline, its once-only `Final` claim, the record of what the watchdog delivered, its `DeadlineExceeded` fallback
//! (built from what admission knows: the legal intervals, hero's combo, the big blind and a pending equity) and a
//! stage slot of its own (`"queued"` until `engine-main` starts it). The `LiveRequest` carries all of it (`Watched`)
//! into `serve_request`, which refreshes the fallback as it learns more and never arms anything. So a request waiting
//! behind another (a hung solve, a cancel window, a restart, a sink that blocks) still gets its `Final` at its fire.
//! A request at no decision point is armed with nothing: it answers `NoDecision`, never a `Final`.
//!
//! One request. `serve_request` classifies the decision (§6) and answers at once every row the classifier settles
//! alone; a degraded engine (a worker whose `ready` was refused at startup, spec 12, ruling 29-I4) answers every
//! decision that way too, with the version mismatch. For a heads-up flop, turn or river decision it reads the public
//! root ranges from the range source (§9: the only provider, which owns their validation, rulings 27-D3/D4; in
//! production the replay's, plan 3 Task 18), emits `Fast` (§5 step 5), starts the `fast-path` equity thread (§3.4), asks
//! the cache on the flop and the turn (below), solves through the solve client (`solve::run_solve`, Tasks 22-23),
//! assembles the `Final` (§4.4, §5 step 7), registers a validated solution as a snapshot (§9.2), stores a flop or turn
//! solution in the cache and logs the decision (§5 step 10). A heads-up postflop decision that ends with no solution (a
//! refused range, a tree that does not build, a failed solve with nothing retained, the watchdog's `Final`, an unserved
//! retirement or a panic containment) records its miss instead, with the engine's cause, under the same identity rule
//! (plan 3 Task 18, spec 9.3): a later replay names that cause for a street left without a snapshot, and records the
//! delivered cause as the street's miss (ruling 18-I2).
//!
//! The multiway row (plan 4 Task 11; spec 6). A decision with three or more pot-eligible players is answered with the
//! classifier's `Unsupported{MultiwayEv}` row as before, through the request's claim (`settle`), and beside it, never in
//! its `actions`, the `experimental` block of the synthetic-root surrogate (`with_experimental`, `crate::experimental`):
//! the seats' street-root public ranges from the range source, the opponent by range-vs-range equity, one isolated
//! solve through the solve client's shared transport inside the request's own street deadline. It never asks or stores
//! the cache, registers no snapshot and records no miss; a surrogate that cannot answer leaves a note naming why the
//! block is absent. The watchdog is never armed again: the multiway `Final` is retained for it (`set_retained`) before
//! the surrogate starts and once it has answered.
//!
//! The cache (plan 4 Task 10; spec 5 step 7, 7, 10.4, 10.5). After the `Fast`, a flop or turn decision probes the cache
//! (`cache_phase`): on the flop the pre-solver's template first and then the live template the flop policy picks (Task
//! 9: `flop_min_v1` only for an admitted single-raised pot), when it differs; on the turn its one template. The probes
//! run on `engine-main`, inside the request's own budget: one absolute cutoff, `CACHE_BUDGET_MS` from the start of the
//! phase or the street deadline when that comes first, with the time left read again immediately before each lookup
//! and no probe started once the cutoff has passed (ruling 10-pre1: the shared 500 ms cache budget of spec 7; review
//! P4T10-I4). The watchdog never asks the cache, and a lookup that holds `engine-main` to its bound cannot delay the
//! watchdog's `Final`. The suit permutation of every key is `cache_bridge::canonical_perm`'s over the public root ranges
//! (hero's cards enter no key).
//!
//! A hit at the request's raw target is its `Final` (`cache_exact` or `cache_approximate` snapshot, registered through
//! `finish` like a live one), with no live solve. An above-target hit is its `Provisional` while a live solve refines
//! it; its coverage discloses its accuracy shortfall (`DeadlineBestSoFar` at the stored raw accuracy against the
//! request's target, review P4T10-I1). Every cache result lists the translations and mappings of its complete coverage
//! in its assumptions (P4T10-I3). The `Provisional` is accepted under the identity lock like any event, and in that
//! same accepted delivery its `cache_provisional` snapshot is registered (`replay_bridge::register_accepted`: a
//! `Provisional` never replaces a `Final`) and its payload, promoted to `Final`, is retained for the watchdog
//! (`set_retained`), so a watchdog `Final` delivers exactly it. A decision already superseded, or whose `Final` the
//! watchdog already delivered, gets no `Provisional` and registers nothing. The live refinement's `Final` replaces the
//! `Provisional` unless the retained hit is more accurate (raw) or the refinement failed: that `Final` is then the
//! retained payload with its own coverage (P4T10-I2: nothing of the discarded live solve), its note disclosing both
//! accuracies. A lookup miss changes nothing but the label; only a lookup that ran out of its budget, or was never made
//! because the budget was spent, is disclosed in the notes. The live terminal of a flop or turn solve (the tree
//! actually solved: the `_min` retry's when it answered) is stored through the non-blocking `Cache::store` once its
//! `Final` is delivered; an entry the cache refuses is logged (`EngineCore::log_cache_reject`) and never affects
//! delivery. River solves are never stored.
//!
//! Identity (ruling 28-I1). Every event goes through `deliver` (`emit` for the crate): it is accepted under the
//! identity lock, where a decision no longer active is refused and a `Final` claims the request's once-only delivery,
//! and an accepted event is handed to the sink only after that lock is released, so this path holds no engine lock
//! during a sink callback and a sink may re-enter the engine (read the identity, mutate the hand). The watchdog's fire
//! makes the same decision under the same lock (follow-up P2.W3, `watchdog`'s "Identity at the fire"): a decision no
//! longer active gets no watchdog `Final` and its claim stays untaken, with no retirement hook needed on supersession;
//! the fire releases the identity lock before its sink callback and keeps only its own generation lock. A mutation that
//! lands between the acceptance and the callback lets that one event through; the UI refuses it by identity (§5 step
//! 9). A request whose decision is no longer active, when it is served or once its solve returns, ends with no `Final`
//! of the engine's and no snapshot, and its watchdog generation is retired; it logs nothing, unless its watchdog had
//! claimed and delivered its `Final` while the decision was still active and the supersession came after that claim:
//! that delivered `Final` is then the request's, and it is logged exactly once, with the delivery time and the deadline
//! verdicts the fire recorded (`retire_stale`, ruling W3-I1; spec 5 step 10). The same holds for a request that is never
//! served (dropped from the depth-1 queue by a newer one, or still queued at shutdown: `retire_unserved`), and records
//! the delivered cause as the street's miss (ruling 18-I2). Once the solve returns, the identity and the watchdog's
//! claim are read in two steps; a supersession and a fire landing between them leave the claim untaken, so the engine's
//! own claim then finds the decision stale (re-review observation O4).
//!
//! One `Final` (§7, ruling 28-I2). The engine's `Final` and the watchdog's race for one claim (`Armed::delivered`); the
//! fire records what it delivered and when (`Armed::fired`). The engine claims its candidate in `finish`: the snapshot
//! of a solved candidate is registered under the identity lock as part of the accepted claim (§9.2, by the one
//! registration rule, `replay_bridge::register_accepted`), and the `Final` delivered is the one logged. When the
//! watchdog won, during the solve or while the candidate was prepared, the candidate is discarded, registers nothing
//! (and, won during the solve, no candidate or analytic fallback is even built; a heads-up postflop decision records the
//! watchdog's cause as its miss, superseded since or not), and the decision log records the watchdog's `Final` with its
//! delivery time. Until the fire the watchdog's fallback is refreshed as the request learns more, so a watchdog `Final`
//! keeps the range source's reasons and the assumptions known by then (ruling 28-I6; spec 6). Nothing but an `Equity`
//! of the request follows its delivered `Final` (ruling 28-N1; spec 7, spec 4.4): the request's `Fast` carries its
//! claim, and so does the solve client's `Progress` (`SolvePlan::final_claim`, re-review observation O5), and `deliver`
//! drops either, under the sink lock, once the watchdog has delivered.
//!
//! Panics (final review I3, ruling F-I3). `engine-main` serves each request inside `catch_unwind`. A panic (an always-on
//! assert of an internal invariant) is contained at that boundary by `contain_panic`: the request's `Final`, unless the
//! watchdog already delivered it, is a non-retryable `Unsupported{EngineError("internal: ..")}` delivered through the
//! request's claim; its watchdog generation is retired, the `Final` delivered is logged once and records the delivered
//! cause as the street's miss (ruling 18-I2), the worker (which may be running the request's job) is killed, for the
//! next solve to relaunch, and `engine-main` goes on serving. A panic after `engine-main` took the claim for its own
//! `Final` and before it handed that `Final` over (final fix round 2, ruling F2-I3) is answered the same way: the
//! claim is `engine-main`'s (`Watched::claimed_by_engine`, set under the
//! identity lock as the claim is taken), so the containment delivers the internal-error `Final` without claiming again;
//! a panic after the handover logs the `Final` handed over (`Watched::handed`) if it is not logged yet. Every engine
//! lock survives a panic (ruling N2): a poisoned lock is used as it stands, its value consistent at every panic point.
//!
//! Street verdict (ruling 20-I1). A request has one `StreetDeadline`, shared by its watchdog (`Armed::street_deadline`)
//! and the solve client (`SolvePlan::street_deadline`), which publishes the first attempt's terminal arrival to it at
//! receipt. The logged verdict is the client's (`SolveOutcome::street_violation`), judged from that arrival and never
//! from when processing finished; a `best_so_far` (the street deadline reached) is logged as a violation too, except on
//! a single-raised-pot flop cache miss, the designed outcome (`flop::is_street_violation`; §7, §10.6). A request that
//! ends before any solve logs no verdict when answered before the fire, and the shared street deadline's once it
//! expired (ruling 28-O3). A row the classifier settles solves no street and logs no street verdict.
//!
//! After the `Final` (§7, §12, rulings 23-I1 and 28-I3). At the watchdog's fire the client stops and cleans nothing up,
//! and reports whether it left a sent job that may still be running, or a failed link, behind
//! (`SolveOutcome::outstanding_job`). Only then is the worker killed, once, after the `Final` is out, whoever delivered
//! it; an idle worker (nothing sent, the job's terminal received, the worker's own `no_iteration`) is left alone. The
//! cleanup never relaunches the worker (final fix round 2, ruling F2-Q1: a relaunch of up to two startup timeouts on
//! `engine-main` would delay the next request's `Fast`): the next request's solve relaunches it once before its send,
//! its `ready` validated again (`run_solve`). The kill is recorded in the diagnostics log (final review M2).
//!
//! Equity (§3.4, §7, ruling 28-I4). Each request's `fast-path` equity runs with its own cancellation token, polled by
//! the equity routine between its units of work. It is set on supersession: by the next request as it starts
//! (`EngineCore::equity_cancel` holds the last request's token), by `Engine` when a public call supersedes the decision
//! (in the same identity-lock hold, rulings 29-I1 and 28-I4), when this request finds its decision no longer active,
//! and by the fast path itself when the decision is already stale as it starts; never by the request's own `Final`,
//! since a late `Equity` of the active decision still enriches it. The `fast-path` thread is the core's
//! (`EngineCore::tasks`), joined at the engine's teardown (ruling 29-I2).
//!
//! Locks. The identity lock is taken before the snapshot store and released after it, never the other way round, and
//! neither is held during a sink callback. The snapshot store, the config, the range source and the fallback slot are
//! each locked for one step, released before any emission or solve. `admit` arms the watchdog holding no engine lock
//! (the watchdog's lock order: its threads, then its generation, then the identity lock alone).

use crate::allin::{facing_allin, AllInAnswer, AllInInput};
use crate::assemble::{self, AssemblyCtx};
use crate::cache_bridge::{canonical_perm, entry_from_solution, make_cache_query};
use crate::clock::Clock;
use crate::core::EngineCore;
use crate::coverage::{classify, decision_point, seat_index, Classification};
use crate::deadline::Deadlines;
use crate::equity::{equity_summary_with_clock, pending_summary, EQUITY_BUDGET_MS};
use crate::experimental::{Skipped, SurrogateRequest};
use crate::flop::{cacheable, choose_cache_route, is_street_violation, preflop_wagers, CacheRoute, ProvisionalHit, PRESOLVER_TEMPLATE};
use crate::identity::IdentityState;
use crate::log::{DecisionRecord, InputRecord};
use crate::ranges::RootRanges;
use crate::replay_bridge::{miss_cause, miss_cause_of_final, miss_for, register_accepted, snapshot_from_solution, snapshot_note};
use crate::snapshots::StreetSnapshot;
use crate::solve::{run_solve, SolveOutcome, SolvePlan, Terminal};
use crate::tree::{build_tree_full, tree_signature, TemplateSelection, Templates};
use crate::watchdog::{Armed, Fired, SharedSink, StreetDeadline, Watchdog};
use crate::EngineError;
use cache::lookup::{CacheHit, Lookup, MissReason};
use core_iso::SuitPerm;
use core_model::derive;
use core_ranges::hash_scaled;
use core_replay::SnapshotMiss;
use proto::worker::SOLVER_COMMIT;
use proto::{
    combo_index, ApproxReason, Assumptions, Card, Coverage, DecisionIdentity, Derived, EffectiveTree, EquitySummary, ExperimentalHu, GameConfig, HandState,
    LegalAction, Phase, Range1326, Recommendation, RecommendationEvent, Seat, SolveInput, Street, StreetRootSnapshot, UnsupportedReason,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// One live request: the decision it answers, the hand state at that decision, its monotonic admission time `t0` on
/// the engine's clock (§5 step 4, §7), the sink its events go to, the session config at its admission, and, for a
/// request at a decision point, what its admission armed (`admit`).
pub struct LiveRequest {
    pub identity: DecisionIdentity,
    pub state: HandState,
    pub t0_ms: u64,
    pub sink: SharedSink,
    /// The session config in force at admission: the solver's target and flop budget of this request. A config change
    /// after it applies from the next hand (§4.2, spec 12), never to this request.
    pub config: GameConfig,
    /// What admission armed; `None` for a request at no decision point (it answers `NoDecision`).
    pub watch: Option<Watched>,
}

/// A request's side of its watchdog, armed at admission (final review I1): the generation to retire, the street and
/// deadlines it was armed with, the shared street deadline, the once-only `Final` claim, the record of what the
/// watchdog delivered, the fallback the watchdog delivers (refreshed by `serve_request`), the payload it delivers in the
/// fallback's place (`set_retained`), the request's own stage slot, and whether the `Final` delivered has been logged.
pub struct Watched {
    pub generation: u64,
    pub street: Street,
    pub deadlines: Deadlines,
    pub street_deadline: Arc<StreetDeadline>,
    pub delivered: Arc<AtomicBool>,
    pub fired: Arc<Mutex<Option<Fired>>>,
    pub fallback: Arc<Mutex<Recommendation>>,
    /// The watchdog's retained slot (`Armed::retained`, the same `Arc`): written through `set_retained` only (final fix
    /// round 2, ruling F2-N3).
    pub(crate) retained: Arc<Mutex<Option<Recommendation>>>,
    pub stage: Arc<Mutex<String>>,
    /// Set once the `Final` delivered is logged (§5 step 10), so a panic's containment never logs it twice.
    pub logged: Arc<AtomicBool>,
    /// Set under the identity lock the moment `engine-main` takes the request's claim for its own `Final` (`finish`):
    /// a panic before the handover leaves the claim `engine-main`'s, and the containment delivers the `Final` then
    /// (final fix round 2, ruling F2-I3).
    pub(crate) claimed_by_engine: AtomicBool,
    /// The `Final` `engine-main` handed to the sink, the engine-clock time it did and whether it is a `best_so_far`,
    /// recorded immediately before the handover: a panic after it logs that `Final` (ruling F2-I3).
    pub(crate) handed: Mutex<Option<(u64, Recommendation, bool)>>,
    /// The validated heads-up street root of the request's decision, set by `serve_request` as soon as the classifier
    /// has derived it (plan 3 Task 18, fix round 1, ruling 18-I2): kept on the request, not in a local of the frame that
    /// may panic, so a panic's containment records the delivered cause as the street's miss at that root. `None` for a
    /// request served at no heads-up postflop root, or not served at all.
    pub(crate) root: Mutex<Option<proto::StreetRootSnapshot>>,
}

/// Retains `rec` for `req`'s watchdog (final fix round 2, ruling F2-N3; spec 7): at the fire the watchdog delivers the
/// retained payload as the request's `Final` in place of its `DeadlineExceeded` fallback (`watchdog`'s fire). The
/// handle a validated `Provisional` (or an earlier `best_so_far`) is written through, for a request armed at admission:
/// its caller never arms the watchdog itself (ruling F-Q2). Plan 4 Task 10's cache `Provisional` is its first caller
/// (`deliver_provisional`): the promoted payload is retained inside the `Provisional`'s accepted delivery, under the
/// identity lock (the watchdog's lock order puts the identity lock before this slot, and the watchdog never waits on
/// the identity lock while it holds this slot), so a watchdog that claims the `Final` after that acceptance delivers
/// exactly it. Plan 4 Task 11's multiway row retains its `Final` through it too (`with_experimental`), from
/// `engine-main` holding no engine lock. A later call replaces the payload; one landing after the fire changes nothing delivered. Takes the
/// retained slot's lock alone, one step: never call it holding the sink, the fallback or the stage slot (`watchdog`'s
/// lock order). `req` must be armed (a request at a decision point) and `rec` must answer its decision: either is a
/// caller bug.
pub(crate) fn set_retained(req: &LiveRequest, rec: Recommendation) {
    let watch = req.watch.as_ref().unwrap_or_else(|| panic!("set_retained for decision {:?}: a request at no decision point has no watchdog", req.identity));
    assert!(rec.identity == req.identity, "set_retained for decision {:?}: a payload of decision {:?}", req.identity, rec.identity);
    *lock(&watch.retained) = Some(rec);
}

/// Admits a request (final review I1, ruling F-I1): what `Engine::recommend` does the moment it allocates `identity`,
/// before `engine-main` gets to the request. A state at a decision point (§2) is armed on `watchdog` at once, with the
/// request's deadlines from `t0_ms` and `config` (§7), a new claim, a `DeadlineExceeded` fallback built from what is
/// known now (the legal intervals, hero's combo, the big blind, a pending equity, `config`'s target) and a stage slot
/// of its own, `"queued"` until `engine-main` starts it. A state at no decision point is armed with nothing. Refused
/// (`EngineError`, nothing armed) once the watchdog is stopped: `engine-main` has ended. Takes no engine lock but the
/// watchdog's own; the caller holds none. `state` must replay (`core_model::derive`'s precondition).
pub fn admit(watchdog: &Watchdog, identity_state: &Arc<Mutex<IdentityState>>, config: GameConfig, identity: DecisionIdentity, state: HandState, t0_ms: u64,
    sink: SharedSink) -> Result<LiveRequest, EngineError> {
    let d = derive(&state);
    if decision_point(&state, &d).is_err() {
        return Ok(LiveRequest { identity, state, t0_ms, sink, config, watch: None });
    }
    let deadlines = Deadlines::for_request(t0_ms, d.street, config.solver.flop_budget_s);
    let hero_combo = state.hero_cards.map(|h| combo_index(h[0], h[1]));
    let ctx = AssemblyCtx { identity: identity.clone(), legal: d.legal.clone(), hero_combo, bb_chips: state.config.bb_chips, equity: pending_summary(&[]) };
    let fallback = Arc::new(Mutex::new(deadline_fallback(&ctx, &[], &request_assumptions(&config))));
    let street_deadline = Arc::new(StreetDeadline::new(deadlines.street_deadline_ms));
    let (delivered, fired): (Arc<AtomicBool>, Arc<Mutex<Option<Fired>>>) = (Arc::default(), Arc::default());
    let stage = Arc::new(Mutex::new("queued".to_string()));
    let retained: Arc<Mutex<Option<Recommendation>>> = Arc::default();
    let armed = Armed { identity: identity.clone(), street_deadline: street_deadline.clone(), fire_ms: deadlines.watchdog_fire_ms(), retained: retained.clone(),
        fallback: fallback.clone(), stage: stage.clone(), sink: sink.clone(), delivered: delivered.clone(), fired: fired.clone(), identity_state: identity_state.clone() };
    let generation = watchdog.try_arm(armed).map_err(|_| EngineError::Message("the engine is not running: its watchdog is stopped".into()))?;
    let watch = Watched { generation, street: d.street, deadlines, street_deadline, delivered, fired, fallback, retained, stage, logged: Arc::default(),
        claimed_by_engine: AtomicBool::new(false), handed: Mutex::new(None), root: Mutex::new(None) };
    Ok(LiveRequest { identity, state, t0_ms, sink, config, watch: Some(watch) })
}

impl LiveRequest {
    /// `admit` on `core`'s watchdog, identity and session config: a request admitted for `serve_request` without an
    /// `Engine` (the engine's tests, a harness). Panics once the core's watchdog is stopped (its teardown ran).
    pub fn admitted(core: &EngineCore, identity: DecisionIdentity, state: HandState, t0_ms: u64, sink: SharedSink) -> LiveRequest {
        admit(&core.watchdog, &core.identity, core.config(), identity, state, t0_ms, sink).unwrap_or_else(|e| panic!("admitting a request on a torn-down core: {e}"))
    }
}

/// The decision an event answers.
fn event_identity(ev: &RecommendationEvent) -> &DecisionIdentity {
    match ev {
        RecommendationEvent::Fast(r) | RecommendationEvent::Provisional(r) | RecommendationEvent::Final(r) => &r.identity,
        RecommendationEvent::Equity { identity, .. } | RecommendationEvent::Progress { identity, .. } | RecommendationEvent::NoDecision { identity, .. } => identity,
    }
}

/// A lock that survives a panic elsewhere (final review I3; final fix round 2, ruling N2): the identity state, a sink, a
/// request's watchdog slots, the equity token slot and the range source slot (replaced whole), and the snapshot store
/// (each entry whole: `Vec::retain` keeps the entries it had not examined when a panic interrupts it) stay consistent
/// at every point a panic could interrupt them (see `watchdog`'s `lock`); the panic's containment must still deliver
/// the request's `Final` through them, and `engine-main` must go on serving with them.
fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// How an event's acceptance ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// Accepted: the event is handed to the sink.
    Accepted,
    /// The decision is no longer active: nothing is emitted.
    Stale,
    /// The request's `Final` was already claimed (by the watchdog): a second `Final`, or a `Fast`, `Provisional` or
    /// `Progress` after it, is not emitted (ruling 28-N1).
    AlreadyDelivered,
}

/// Accepts an event of decision `identity` under the identity lock (ruling 28-I1): stale unless the decision is active,
/// and, when `final_claim` is given, the request's once-only `Final` claimed there. `on_accept` runs under the same
/// lock once the event is accepted, immediately after the claim (the engine's record of its claim and the snapshot
/// registration of an accepted `Final`, §9.2). The lock is released on return, before anything is handed to a sink.
fn accept(ids: &Mutex<IdentityState>, identity: &DecisionIdentity, final_claim: Option<&AtomicBool>, on_accept: impl FnOnce(&DecisionIdentity)) -> Delivery {
    let ids = lock(ids);
    if !ids.is_active(identity) {
        return Delivery::Stale;
    }
    let active = ids.active().expect("the active decision was just checked");
    if final_claim.is_some_and(|d| d.swap(true, Ordering::SeqCst)) {
        return Delivery::AlreadyDelivered;
    }
    on_accept(active);
    Delivery::Accepted
}

/// Accepts `ev`, an event of decision `identity`, then hands an accepted event to `sink` with no engine lock held, so
/// the sink may re-enter the engine from its callback (ruling 28-I1). With `delivered`, the request's once-only `Final`
/// flag shared with its watchdog: a `Final` claims it at acceptance; a `Fast`, `Provisional` or `Progress` is dropped once
/// the request's `Final` was delivered (ruling 28-N1; spec 7, `Final` once and last; spec 4.4, only `Equity` enriches a
/// delivered `Final`). That check is made under the sink lock, in the same hold as the emission: the watchdog swaps
/// `delivered` before it takes the sink lock to emit its `Final`, so either the event precedes that `Final` or it is
/// dropped. `Equity` and `NoDecision` never take it. Crate-visible: the solve client forwards the worker's progress
/// through it, with the claim its `SolvePlan::final_claim` carries (re-review observation O5).
pub(crate) fn deliver(ids: &Mutex<IdentityState>, identity: &DecisionIdentity, sink: &SharedSink, delivered: Option<&AtomicBool>, ev: RecommendationEvent) -> Delivery {
    assert!(event_identity(&ev) == identity, "an event of decision {:?} emitted for decision {identity:?}", event_identity(&ev));
    let (claim, not_after_final) = match &ev {
        RecommendationEvent::Final(_) => (delivered, None),
        RecommendationEvent::Fast(_) | RecommendationEvent::Provisional(_) | RecommendationEvent::Progress { .. } => (None, delivered),
        RecommendationEvent::Equity { .. } | RecommendationEvent::NoDecision { .. } => (None, None),
    };
    let verdict = accept(ids, identity, claim, |_| {});
    if verdict != Delivery::Accepted {
        return verdict;
    }
    let mut sink = lock(sink);
    if not_after_final.is_some_and(|d| d.load(Ordering::SeqCst)) {
        return Delivery::AlreadyDelivered;
    }
    sink.emit(ev);
    Delivery::Accepted
}

/// Crate-visible: plan 3 Task 17's `preflop.rs` imports this same helper (`use crate::serve::emit;`) rather than
/// creating a second event-emission path, so the identity check stays in one place. `delivered`, when given, is the
/// request's once-only `Final` flag, shared with its watchdog: it is claimed by a `Final` and keeps a `Fast`,
/// `Provisional` or `Progress` from following the delivered `Final` (see `deliver`).
pub(crate) fn emit(core: &EngineCore, req: &LiveRequest, delivered: Option<&AtomicBool>, ev: RecommendationEvent) {
    deliver(&core.identity, &req.identity, &req.sink, delivered, ev);
}

fn engine_error(message: &str) -> UnsupportedReason { UnsupportedReason::EngineError { message: message.into(), retryable: false } }

/// The assumptions a request starts from: nothing measured yet, the session's target (§4.4).
fn request_assumptions(config: &GameConfig) -> Assumptions {
    let mut assumptions = assemble::empty_assumptions("");
    assumptions.target_bp = config.solver.target_bp;
    assumptions
}

/// Every reason `coverage` lists: an `Approximate`'s reasons, an `Unsupported`'s `partial`, none when `Exact`.
fn coverage_reasons(coverage: &Coverage) -> Vec<ApproxReason> {
    match coverage {
        Coverage::Exact => vec![],
        Coverage::Approximate { reasons } => reasons.clone(),
        Coverage::Unsupported { partial, .. } => partial.clone(),
    }
}

/// The watchdog's `Unsupported{DeadlineExceeded}` fallback for this decision with what the request knows so far: the
/// reasons inherited to here and the assumptions (§7; spec 6, inherited reasons survive an `Unsupported` result). The
/// stage is the watchdog's to fill in at the fire.
pub(crate) fn deadline_fallback(ctx: &AssemblyCtx, inherited: &[ApproxReason], assumptions: &Assumptions) -> Recommendation {
    assemble::unsupported(ctx, UnsupportedReason::DeadlineExceeded { stage: String::new() }, inherited.to_vec(), assumptions.clone())
}

/// The fast-path equity routine (§3.4, §7): `equity::equity_summary_with_clock` in production. It polls `cancel` between
/// its units of work and stops once it is set (ruling 28-I4).
pub type EquityRoutine = Arc<dyn Fn(&dyn Clock, Option<[Card; 2]>, &Range1326, &[(Seat, Range1326)], &[Card], Duration, &AtomicBool) -> EquitySummary + Send + Sync>;

/// What `serve` runs that a test may replace (`serve_request_with`); production is `Hooks::production()`. Crate-visible
/// for the preflop path (`crate::preflop::serve_preflop`), which runs the same equity routine and seams.
pub(crate) struct Hooks {
    pub(crate) equity: EquityRoutine,
    /// Runs on `engine-main` with a candidate `Final` assembled, immediately before its delivery is claimed.
    before_claim: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Runs on `engine-main` right after the `Fast` was handed over (the sink released): on the river and turn before the
    /// tree is built, on the preflop path before the `Final` is assembled.
    pub(crate) after_fast: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Runs on `engine-main` once the solve has returned and its decision was found still active, before the watchdog's
    /// claim is consulted.
    after_active_check: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Runs on `engine-main` inside the accepted claim of the engine's own `Final`, under the identity lock, immediately
    /// before a solved candidate's snapshot is registered (never before a miss is recorded).
    at_registration: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Runs on `engine-main` immediately before each flop or turn cache lookup (plan 4 Task 10), where a lookup holds
    /// `engine-main` for up to its bound.
    before_lookup: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Hooks {
    fn production() -> Self {
        Self { equity: Arc::new(equity_summary_with_clock), before_claim: None, after_fast: None, after_active_check: None, at_registration: None, before_lookup: None }
    }
}

/// Test seams of `serve_request` (plan 2 Task 28 fix round 1), compiled for this crate's tests and with the `testing`
/// feature only: `equity` replaces the fast-path equity routine (ruling 28-I4's acknowledged runner); `before_claim` runs
/// on `engine-main` with the candidate `Final` assembled, immediately before its delivery is claimed (ruling 28-I2: a
/// watchdog fire landing there); `after_fast` runs on `engine-main` right after the `Fast` was handed over, the sink
/// released, before the tree is built (ruling 28-O1b: a fire between the watchdog fallback's two refreshes);
/// `after_active_check` runs on `engine-main` once the solve has returned and its decision was found still active,
/// between that identity read and the read of the watchdog's claim (follow-up P2.W3, re-review observation O4: a
/// supersession and a fire landing between the two); `at_registration` runs on `engine-main` inside the accepted claim
/// of the engine's own `Final`, under the identity lock, immediately before a solved candidate's snapshot is registered
/// (final fix round 2, ruling F2-I3: the re-review's probe P7 site, a panic after the claim and before the handover);
/// `before_lookup` runs on `engine-main` immediately before each flop or turn cache lookup (plan 4 Task 10: a lookup
/// that holds `engine-main` to its bound, a supersession or a watchdog fire landing while the cache is asked).
/// `None` keeps production behaviour. A seam that panics exercises `engine-main`'s containment (final review I3).
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Default)]
pub struct ServeSeams {
    pub equity: Option<EquityRoutine>,
    pub before_claim: Option<Arc<dyn Fn() + Send + Sync>>,
    pub after_fast: Option<Arc<dyn Fn() + Send + Sync>>,
    pub after_active_check: Option<Arc<dyn Fn() + Send + Sync>>,
    pub at_registration: Option<Arc<dyn Fn() + Send + Sync>>,
    pub before_lookup: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[cfg(any(test, feature = "testing"))]
impl ServeSeams {
    fn hooks(&self) -> Hooks {
        let production = Hooks::production();
        Hooks { equity: self.equity.clone().unwrap_or(production.equity), before_claim: self.before_claim.clone(), after_fast: self.after_fast.clone(),
            after_active_check: self.after_active_check.clone(), at_registration: self.at_registration.clone(), before_lookup: self.before_lookup.clone() }
    }
}

/// `serve_request` with test seams (see `ServeSeams`).
#[cfg(any(test, feature = "testing"))]
pub fn serve_request_with(core: &mut EngineCore, req: LiveRequest, seams: ServeSeams) {
    serve(core, &req, &seams.hooks());
}

/// §5 steps 4-10 for one request admitted by `admit` (see the module doc). `req.state` must replay
/// (`core_model::derive`'s precondition) and `req.t0_ms` must not lie ahead of the engine's clock.
pub fn serve_request(core: &mut EngineCore, req: LiveRequest) {
    serve(core, &req, &Hooks::production());
}

/// `serve_request` on a request `engine-main` keeps, to contain a panic of it afterwards (final review I3).
pub(crate) fn serve_admitted(core: &mut EngineCore, req: &LiveRequest) {
    serve(core, req, &Hooks::production());
}

/// `serve_request_with` on a request `engine-main` keeps (see `serve_admitted`).
#[cfg(any(test, feature = "testing"))]
pub(crate) fn serve_admitted_with(core: &mut EngineCore, req: &LiveRequest, seams: &ServeSeams) {
    serve(core, req, &seams.hooks());
}

fn serve(core: &mut EngineCore, req: &LiveRequest, hooks: &Hooks) {
    // Ruling 28-I4: this request's equity cancellation token. A newer request supersedes the one served before it (§5
    // step 4), whose equity is cancelled before anything of this one starts.
    let equity_cancel = Arc::new(AtomicBool::new(false));
    let older = lock(&core.equity_cancel).replace(equity_cancel.clone());
    if let Some(older) = older {
        older.store(true, Ordering::SeqCst);
    }
    let d = derive(&req.state);
    let class = classify(&req.state);
    // §5 step 4: a point that is no decision answers `NoDecision`, never a `Final`. Admission armed nothing for it, or,
    // when only the street root shows it is no decision, armed a generation that is retired here; if that generation
    // already delivered a `Final` (the request waited past its fire), the `Final` is logged and nothing follows it.
    if let Classification::NoDecision { reason } = class {
        if let Some(watch) = &req.watch {
            log_watchdog_final(core, req, watch, &Logged::unsolved(watch.street, None, vec![]));
            if watch.delivered.load(Ordering::SeqCst) {
                return;
            }
        }
        emit(core, req, None, RecommendationEvent::NoDecision { identity: req.identity.clone(), reason });
        return;
    }
    let watch = req.watch.as_ref().unwrap_or_else(|| panic!("decision {:?}: admission found no decision point, the classifier found {class:?}", req.identity));
    // Plan 3 Task 18 (fix round 1, ruling 18-I2): the validated heads-up street root, kept on the request from here on,
    // so a panic's containment can record the `Final` it delivers as the street's miss.
    if let Classification::HuStreet { root, .. } = &class {
        *lock(&watch.root) = Some(root.clone());
    }
    // This request's own stage slot, shared with its watchdog since admission ("queued" until now): the solve client
    // advances it from here (final review I1).
    core.stage = watch.stage.clone();
    core.reset_stage("fast");
    let config = &req.config;
    let hero_combo = req.state.hero_cards.map(|h| combo_index(h[0], h[1]));
    let mut ctx = AssemblyCtx { identity: req.identity.clone(), legal: d.legal.clone(), hero_combo, bb_chips: req.state.config.bb_chips, equity: pending_summary(&[]) };
    let mut assumptions = request_assumptions(config);
    let claim = Claim { watch, equity_cancel: &equity_cancel };
    // Spec 12 (ruling 29-I4): a worker whose `ready` was refused at startup (a degraded engine, `WorkerLink::refused`)
    // answers every decision with the non-retryable version mismatch, before anything else runs and without launching
    // the refused build. A heads-up postflop decision records that engine error as its miss (plan 3 Task 18).
    if let Some(refusal) = core.worker.refused().map(|refusal| format!("worker/proto version mismatch: {refusal}")) {
        let reason = engine_error(&refusal);
        let rec = assemble::unsupported(&ctx, reason.clone(), vec![], assumptions);
        let candidate = match &class {
            Classification::HuStreet { root, .. } => Candidate::missed(rec, miss_for(&req.identity, root, miss_cause(&reason))),
            _ => Candidate::unsolved(rec),
        };
        finish(core, req, hooks, &claim, candidate, Logged::unsolved(d.street, None, vec![]));
        return;
    }
    let (root, inherited, facing_allin_flag, opponent) = match class {
        Classification::NoDecision { .. } => unreachable!("a point that is no decision was answered above"),
        // Plan 3 Task 17 (spec 5 step 6): replay, the store lookup over the replayed branches, the `Final` through this
        // request's claim, then its equity. No solve, no arming.
        Classification::Preflop => {
            crate::preflop::serve_preflop(core, req, hooks, &claim, &equity_cancel, &ctx, assumptions);
            return;
        }
        Classification::Multiway { pot_eligible } => {
            // Plan 4 Task 11 (spec 6): the multiway row as before, with the `experimental` block beside it when the
            // synthetic-root surrogate answers (`with_experimental`), through the request's own claim.
            let rec = assemble::unsupported(&ctx, UnsupportedReason::MultiwayEv { pot_eligible }, vec![], assumptions);
            let (rec, outstanding_job) = with_experimental(core, req, watch, &d, rec, &equity_cancel);
            settle(core, req, hooks, &claim, d.street, rec);
            // Ruling 28-I3: a surrogate job left running at the watchdog's fire is killed once the `Final` is out.
            if outstanding_job {
                kill_the_busy_worker(core);
            }
            return;
        }
        Classification::Unsupported(reason) => {
            settle(core, req, hooks, &claim, d.street, assemble::unsupported(&ctx, reason, vec![], assumptions));
            return;
        }
        Classification::HuStreet { root, reasons, facing_allin, opponent } => (root, reasons, facing_allin, opponent),
    };
    // Plan 4 Task 10: the flop is served like the turn and the river, through the cache phase below (the plan-2 flop guard
    // is gone).
    assert!(matches!(root.street, Street::Flop | Street::Turn | Street::River), "a heads-up street root on {:?}", root.street);
    ctx.equity = pending_summary(&[opponent]);
    // Ruling 28-I6: the fallback armed at admission now carries the classifier's reasons and the opponent's pending
    // equity; it is refreshed again once the range source and the tree have answered.
    *lock(&watch.fallback) = deadline_fallback(&ctx, &inherited, &assumptions);
    let deadlines = watch.deadlines;
    let street_deadline = watch.street_deadline.clone();

    // Fast phase (§5 step 5). The range source's lock is released at the end of this statement, before any emission. In
    // production the source is the replay's (plan 3 Task 18: `replay_bridge::ReplayRanges`, installed by `Engine::new`),
    // which owns the ranges' validation; nothing here validates them again (rulings 27-D3/D4).
    let ranges = lock(&core.range_source).ranges_at_root(&req.state, &root);
    let ranges = match ranges {
        Ok(r) => r,
        Err(reason) => {
            let logged = Logged::unsolved(root.street, Some(street_deadline.clone()), vec![]);
            let miss = miss_for(&req.identity, &root, miss_cause(&reason));
            finish(core, req, hooks, &claim, Candidate::missed(assemble::unsupported(&ctx, reason, inherited, assumptions), miss), logged);
            return;
        }
    };
    // The public ranges handed to the solve, OOP then IP as in `ranges_used`; hero's cards are in neither (CLAUDE.md 6).
    let range_hashes = vec![hex::encode(hash_scaled(&ranges.oop)), hex::encode(hash_scaled(&ranges.ip))];
    let inherited: Vec<ApproxReason> = inherited.into_iter().chain(ranges.reasons.iter().cloned()).collect();
    assumptions.ranges_used = ranges.ranges_used.clone();
    // Plan 3 Task 18 (fix round 1, ruling 18-I3; spec 9.3): every snapshot a prior street was conditioned through is
    // disclosed, with its origin, in this request's assumptions from here on, the `Fast`, the watchdog's fallback and every
    // `Final` alike. The solve's own `source` and `cache` stay this solve's.
    assumptions.notes.extend(ranges.snapshots_used.iter().map(|(street, provenance)| snapshot_note(*street, provenance)));
    // Plan-3 final review F-I1 (spec 4.4's `translations` and `mappings`; spec 8.4: every non-exact mapping is recorded
    // in `assumptions.translations` with its deviation): the inherited reasons' translations and mappings are listed in
    // the assumptions, before the fallback is refreshed, so the `Fast`, the watchdog's `Final` and every `Final` carry
    // both lists. One classification of a mapping, the preflop path's (`preflop::is_mapping`).
    assumptions.translations = inherited.iter().filter(|r| matches!(r, ApproxReason::BetTranslation { .. })).cloned().collect();
    assumptions.mappings = inherited.iter().filter(|r| crate::preflop::is_mapping(r)).cloned().collect();
    // Ruling 28-I6: from here on a watchdog `Final` keeps the range source's reasons and the ranges used (spec 6).
    *lock(&watch.fallback) = deadline_fallback(&ctx, &inherited, &assumptions);
    emit(core, req, Some(&*watch.delivered), RecommendationEvent::Fast(assemble::fast(&ctx, assemble::accumulate(Coverage::Exact, inherited.clone()), assumptions.clone())));
    if let Some(after_fast) = &hooks.after_fast {
        after_fast();
    }
    let hero_is_oop = root.oop == req.state.hero;
    let (hero_public, opp_public) = if hero_is_oop { (&ranges.oop, &ranges.ip) } else { (&ranges.ip, &ranges.oop) };
    spawn_equity(core, req, hero_public.clone(), vec![(opponent, opp_public.clone())], root.board.clone(), hooks.equity.clone(), equity_cancel.clone());

    // The template (§10.1): the river's and the turn's own; on the flop, the policy's (plan 4 Task 9), classified from the
    // preflop history, never the ranges. The request's target travels with it (captured once, at admission).
    let template = live_template(core, &req.state, root.street);
    let target_bp = config.solver.target_bp;
    let hero_actor = if hero_is_oop { "oop" } else { "ip" };

    // The cache phase (§5 step 7, §10.4, §10.5; plan 4 Task 10, see "The cache" above): flop and turn only.
    let mut perm = SuitPerm::IDENTITY;
    let mut retained: Option<Retained> = None;
    if matches!(root.street, Street::Flop | Street::Turn) {
        perm = canonical_perm(&root.board, &ranges.oop, &ranges.ip);
        let (route, probes, unasked) = cache_phase(core, hooks, &root, [&ranges.oop, &ranges.ip], &d, &req.state, &inherited, &perm, template, target_bp,
            deadlines.street_deadline_ms);
        assumptions.cache = cache_label_for(&probes);
        // A lookup that ran out of its budget, or was never made because the phase's budget was spent (review P4T10-I4),
        // is the one miss disclosed: the cache was too slow, not empty.
        let spent = probes.iter().filter(|p| p.result == Lookup::Miss { reason: MissReason::BudgetExhausted }).map(|p| p.template_id.as_str()).chain(unasked.iter().map(String::as_str));
        assumptions.notes.extend(spent.map(|t| {
            format!("cache lookup of {t} missed: the cache phase's budget ({CACHE_BUDGET_MS} ms, never past the street deadline) was spent")
        }));
        *lock(&watch.fallback) = deadline_fallback(&ctx, &inherited, &assumptions);
        let cached = Cached { root: &root, ranges: &ranges, ctx: &ctx, hero_public, hero_actor, target_bp };
        match route {
            CacheRoute::Final(hit) => {
                // A hit at the request's raw target: the `Final`, no live solve. Its snapshot is registered only as part of
                // the `Final`'s accepted delivery (`finish`, the one registration rule).
                let probe = probe_of(&probes, &hit);
                assumptions.template_id = probe.template_id.clone();
                assumptions.tree_signature = probe.signature.clone();
                let label = hit.coverage.clone().unwrap_or_else(|| panic!("decision {:?}: an at-target cache hit carries its coverage", req.identity));
                let origin = if label == Coverage::Exact { "cache_exact" } else { "cache_approximate" };
                let rec = cached.recommendation(core, req, &hit, assemble::accumulate(label, inherited.clone()), &assumptions, Phase::Final);
                let snapshot = cached.snapshot(req, &hit, origin, coverage_reasons(&rec.coverage));
                let logged = Logged::unsolved(root.street, Some(street_deadline.clone()), range_hashes);
                finish(core, req, hooks, &claim, Candidate { rec, record: Record::Snapshot(snapshot), best_so_far: false }, logged);
                return;
            }
            CacheRoute::Refine { retained: Some(ProvisionalHit { hit, reasons }) } => {
                // An above-target hit: the `Provisional`, labelled here, refined by the live solve below. Its raw accuracy
                // missed the request's target (the lookup's raw comparison), so its coverage is never `Exact`: the lookup's
                // own reasons, the inherited ones, and the shortfall itself (review P4T10-I1, ruling 10-Q2), which the
                // cache label layer never synthesizes.
                let probe = probe_of(&probes, &hit);
                let mut provisional = assumptions.clone();
                provisional.template_id = probe.template_id.clone();
                provisional.tree_signature = probe.signature.clone();
                let incurred = inherited.iter().cloned().chain([accuracy_shortfall(hit.raw_exploitability_over_p, target_bp)]).collect();
                let rec = cached.recommendation(core, req, &hit, assemble::accumulate(Coverage::Approximate { reasons }, incurred), &provisional, Phase::Provisional);
                let snapshot = cached.snapshot(req, &hit, "cache_provisional", coverage_reasons(&rec.coverage));
                let mut promoted = rec.clone();
                promoted.phase = Phase::Final;
                if deliver_provisional(core, req, rec, snapshot, promoted.clone()) {
                    retained = Some(Retained { hit, promoted });
                }
            }
            CacheRoute::Refine { retained: None } => {}
        }
    }

    // Tree and solve (§5 step 7): each street is rooted at its street root.
    let build = match build_tree_full(&root, &TemplateSelection::from_history(template, &root.history)) {
        Ok(b) => b,
        Err(reason) => {
            let logged = Logged::unsolved(root.street, Some(street_deadline.clone()), range_hashes);
            // A retained cache payload is a validated solution: it answers when no live refinement can start.
            let candidate = match &retained {
                Some(r) => Cached { root: &root, ranges: &ranges, ctx: &ctx, hero_public, hero_actor, target_bp }.retained_final(core, req, r,
                    format!("live refinement could not start ({}); the retained cache payload {:.6} is delivered", miss_cause(&reason), r.hit.raw_exploitability_over_p), false),
                None => Candidate::missed(assemble::unsupported(&ctx, reason.clone(), inherited, assumptions), miss_for(&req.identity, &root, miss_cause(&reason))),
            };
            finish(core, req, hooks, &claim, candidate, logged);
            return;
        }
    };
    assumptions.template_id = template.into();
    assumptions.tree_signature = tree_signature(&build.tree, build.pot);
    *lock(&watch.fallback) = deadline_fallback(&ctx, &inherited, &assumptions);
    let input = SolveInput { root: root.clone(), ranges: [ranges.oop.clone(), ranges.ip.clone()], tree: build.tree.clone(), target_bp };
    // `background: false`: a live decision (final review M5: the wire flag only; plan 4's pre-solver jobs need an
    // executor of their own).
    let plan = SolvePlan { identity: req.identity.clone(), deadlines, street_deadline: street_deadline.clone(), template_id: template.into(),
        retry_template_id: Templates::min_variant(template).map(String::from), rake: req.state.config.rake, hero_actor: hero_actor.into(), background: false,
        final_claim: Some(watch.delivered.clone()) };
    let out = run_solve(core, &input, &plan, &req.sink);
    let returned_ms = core.clock.now_ms();
    // The client published what it returned: the first attempt's terminal arrival, which both verdicts read.
    assert!(street_deadline.terminal_arrival_ms() == out.first_terminal_ms, "the shared street deadline holds the first terminal at {:?} ms, the solve returned {:?} ms",
        street_deadline.terminal_arrival_ms(), out.first_terminal_ms);
    // Ruling 28-I3: the client's liveness provenance, not the clock, says whether a job was left running at the fire.
    let kill_after_final = out.outstanding_job;

    let logged = Logged { street: root.street, verdict: StreetVerdict::Judged(out.street_violation), range_hashes };
    let active = core.identity_active(&req.identity);
    if let (true, Some(after_active_check)) = (active, &hooks.after_active_check) {
        after_active_check();
    }
    if !active {
        // Superseded while it ran (§4.4): no candidate `Final` and no snapshot for a decision that is no longer active,
        // and its equity is cancelled (ruling 28-I4); a `Final` its watchdog delivered while it was still active is
        // logged (ruling W3-I1), and is the decision's outcome, its miss (plan 3 Task 18); nothing else is recorded.
        retire_stale(core, req, &claim, &logged);
        if let Some(cause) = watchdog_cause(watch) {
            record_delivered_miss(core, miss_for(&req.identity, &root, cause));
        }
    } else if watch.delivered.load(Ordering::SeqCst) {
        // The watchdog won the delivery during the solve (ruling 28-I2): nothing of a candidate is built, no analytic
        // fallback runs, and the `Final` it delivered is the one logged. The decision registers no solution: its miss is
        // the watchdog's cause (plan 3 Task 18).
        core.watchdog.disarm(watch.generation);
        let fired = watchdog_final(&watch.fired);
        record_delivered_miss(core, miss_for(&req.identity, &root, miss_cause_of_final(&fired.rec)));
        log_final(core, req, watch, Delivered { at_ms: fired.at_ms, rec: &fired.rec, by_watchdog: true, best_so_far: false }, &logged);
    } else {
        assumptions.elapsed_ms = elapsed_ms(req.t0_ms, returned_ms);
        assumptions.reached_bp = out.reached_bp;
        assumptions.source = format!("solver-worker@{SOLVER_COMMIT}");
        // §4.4 vocabulary `"unverified" | "exploitability <= x"`, in bp (review m14) and a true bound (ruling 28-I5).
        assumptions.source_accuracy = out.solution.as_ref()
            .map_or_else(|| "unverified".to_string(), |sol| format!("exploitability <= {} bp", accuracy_bound_bp(sol.exploitability_chips, build.pot)));
        let best_so_far = out.terminal == Terminal::BestSoFar;
        // Plan 4 Task 10: a retained cache payload stays the `Final` when it is more accurate than the live refinement (raw,
        // over each one's own pot) or when the refinement has no solution. It keeps its own coverage (review P4T10-I2,
        // ruling 10-Q4): nothing of the discarded live solve is its reason; the note names both accuracies, and the
        // deadline log keeps the refinement's own verdict.
        let live_over_p = out.solution.as_ref().map(|sol| f64::from(sol.exploitability_chips) / f64::from(build.pot));
        let keep = match (&retained, live_over_p) {
            (Some(r), Some(live)) if r.hit.raw_exploitability_over_p < live => Some(r),
            (Some(r), None) => Some(r),
            (Some(_), Some(_)) | (None, _) => None,
        };
        let candidate = if let Some(r) = keep {
            let cached = Cached { root: &root, ranges: &ranges, ctx: &ctx, hero_public, hero_actor, target_bp };
            let raw = r.hit.raw_exploitability_over_p;
            match (&out.terminal, &out.solution) {
                (Terminal::Ok | Terminal::BestSoFar, Some(_)) => {
                    let note = format!("live refinement reached raw {:.6}; the retained cache payload {raw:.6} is better", live_over_p.unwrap_or_default());
                    cached.retained_final(core, req, r, note, best_so_far)
                }
                (Terminal::Failed(reason), _) => {
                    let note = format!("live refinement failed ({}); the retained cache payload {raw:.6} is delivered", miss_cause(reason));
                    cached.retained_final(core, req, r, note, false)
                }
                (terminal, _) => unreachable!("run_solve: a {terminal:?} outcome without its solution"),
            }
        } else {
            match &out.terminal {
                Terminal::Ok | Terminal::BestSoFar => {
                    let sol = out.solution.as_ref().expect("run_solve: an Ok or BestSoFar outcome carries its validated solution");
                    // The template actually solved (the `_min` retry's when it answered) and its tree.
                    assumptions.template_id = out.template_used.clone();
                    assumptions.tree_signature = tree_signature(&out.tree, build.pot);
                    let coverage = assemble::coverage_for_solve(sol.exploitability_chips, build.pot, target_bp, best_so_far, inherited.clone());
                    let requested = sol.requested as usize;
                    let reach = assemble::hero_reach(&sol.nodes, &out.ordinal_paths, requested, hero_public, hero_actor);
                    // Registered only as part of this `Final`'s accepted delivery (§9.2, ruling 28-I2), by the one registration
                    // rule (plan 3 Task 18: `replay_bridge::register_accepted`). Built by `snapshot_from_solution` from the input
                    // actually solved: the public root ranges, which are the replay's own published ranges (so a later replay
                    // finds the snapshot by their hashes, Task 15 Q4; hero's cards are in neither), the street root and its
                    // history (the solved prefix, the projected root's for a projection), and the tree the answering attempt
                    // solved (the `_min` retry's when it answered), whose materialized nodes the solve client resolved every
                    // wire chip path against (`solve::validate`, §2's single rule) into `out.ordinal_paths`: the nodes the
                    // worker exported, never reconstructed. Its reasons are this solve's full coverage reasons (fix round 1,
                    // ruling 18-I1; spec 6, reasons accumulate; spec 9.3): every reason this decision inherited and the solve's
                    // own, a `best_so_far`'s `DeadlineBestSoFar` included, carried into every later result the snapshot
                    // conditions. Built before `assumptions` moves into the `Final`. The key's config revision is the
                    // identity's, the hand's (ruling F-I4).
                    let solved = SolveInput { root: input.root.clone(), ranges: input.ranges.clone(), tree: out.tree.clone(), target_bp: input.target_bp };
                    let snapshot = snapshot_from_solution(&req.identity, &solved, sol, out.ordinal_paths.clone(), assumptions.tree_signature.clone(), "live",
                        coverage_reasons(&coverage));
                    let rec = assemble::final_from_solution(&ctx, &sol.nodes[requested], &reach, coverage, assumptions);
                    Candidate { rec, record: Record::Snapshot(snapshot), best_so_far }
                }
                Terminal::Failed(reason) => {
                    // §5 step 7 / §6: facing an all-in with the worker failing, the analytic fallback answers.
                    let worker_failed = matches!(reason, UnsupportedReason::EngineError { .. } | UnsupportedReason::DeadlineExceeded { .. });
                    let analytic = if facing_allin_flag && worker_failed { analytic_allin(req, &d, hero_public, opp_public, &equity_cancel) } else { None };
                    // No solution to register: the worker's failure is the street's miss (plan 3 Task 18), analytic answer or not.
                    let miss = miss_for(&req.identity, &root, miss_cause(reason));
                    Candidate::missed(match analytic {
                        Some(a) => {
                            let mut rec = assemble::unsupported(&ctx, reason.clone(), vec![], assumptions);
                            rec.coverage = assemble::accumulate(Coverage::Approximate { reasons: vec![ApproxReason::UnconditionedCurrentStreet] }, inherited.clone());
                            rec.actions = a.actions;
                            rec.assumptions.notes.push(format!("analytic all-in fallback: equity {:.4}, W {}, R {:.2}, EV(call) {:.2} chips; headline: highest EV",
                                a.equity, a.w, a.r, a.ev_call_chips));
                            rec
                        }
                        None => assemble::unsupported(&ctx, reason.clone(), inherited.clone(), assumptions),
                    }, miss)
                }
            }
        };
        // §5 step 7: the live terminal of a flop or turn solve is stored once its `Final` is out (non-blocking; a refused
        // entry is logged and never affects delivery). A candidate discarded because its decision was superseded
        // stores nothing.
        if finish(core, req, hooks, &claim, candidate, logged) != Delivery::Stale {
            store_live_terminal(core, req, &root, &ranges, &out, build.pot, &inherited, &perm, target_bp);
        }
    }
    if kill_after_final {
        kill_the_busy_worker(core);
    }
}

/// §10.1: the live template of a heads-up street root: `river_std_v1` and `turn_std_v1`, and on the flop the policy's
/// (`EngineCore::flop_policy`, loaded once at startup, plan 4 Task 9) from the preflop history's wager count.
fn live_template(core: &EngineCore, state: &HandState, street: Street) -> &'static str {
    match street {
        Street::River => "river_std_v1",
        Street::Turn => "turn_std_v1",
        Street::Flop => core.flop_policy.live_template(preflop_wagers(state)),
        Street::Preflop => unreachable!("a heads-up street root is postflop"),
    }
}

/// Spec 6's multiway row (plan 4 Task 11): `rec`, the `Unsupported{MultiwayEv}` `Final` exactly as the classifier's row
/// answers it, with the `experimental` block of the synthetic-root surrogate beside it (`experimental`'s module doc),
/// or, when the surrogate is skipped, with a note naming why the block is absent and nothing else changed. Returns that
/// `Final` and whether the surrogate's solve left a job running at the watchdog's fire (ruling 28-I3).
///
/// The request's watchdog, armed at admission, is never armed again: before the surrogate starts, the multiway `Final`
/// (block absent, with the note that it was not ready) is retained for it (`set_retained`), so a fire during the
/// surrogate delivers the multiway row rather than its `DeadlineExceeded` fallback; once the surrogate has answered,
/// the `Final` about to be claimed is retained in its place. The cache, the snapshot store and the main result's
/// assumptions are never touched, and no engine lock is held across the surrogate's solve.
fn with_experimental(core: &mut EngineCore, req: &LiveRequest, watch: &Watched, d: &Derived, mut rec: Recommendation, equity_cancel: &AtomicBool) -> (Recommendation, bool) {
    let mut pending = rec.clone();
    pending.assumptions.notes.push(crate::experimental::absent_note("the surrogate had not answered by the final delivery"));
    set_retained(req, pending);
    let outstanding_job = match surrogate_block(core, req, watch, d, equity_cancel) {
        Ok(block) => {
            rec.experimental = Some(block);
            false
        }
        Err(Skipped { why, outstanding_job }) => {
            rec.assumptions.notes.push(crate::experimental::absent_note(&why));
            outstanding_job
        }
    };
    set_retained(req, rec.clone());
    (rec, outstanding_job)
}

/// The surrogate of a multiway decision (spec 6): the street-root public ranges of every seat in the pot from the range
/// source (`RangeSource::seat_ranges`: the replay's, never hero-conditioned), the opponent by range-vs-range equity
/// within the equity phase's own budget (`EQUITY_BUDGET_MS`, never past the street deadline) and the request's equity
/// token (set on supersession, ruling 28-I4), the synthetic root on the street's template (`live_template`), and the
/// isolated solve (`experimental::run_surrogate`) with the hand's big blind and rake, the request's target, deadlines,
/// street deadline and claim. Hero's cards reach only the advice row.
fn surrogate_block(core: &mut EngineCore, req: &LiveRequest, watch: &Watched, d: &Derived, equity_cancel: &AtomicBool) -> Result<ExperimentalHu, Skipped> {
    let skip = |why: String| Skipped { why, outstanding_job: false };
    let state = &req.state;
    if watch.delivered.load(Ordering::SeqCst) {
        return Err(skip("the request's Final was delivered at the final delivery before the surrogate started".into()));
    }
    let hero_cards = state.hero_cards.ok_or_else(|| skip("hero's cards are not entered".into()))?;
    // Every seat in the pot (all-in seats included, spec 6), in postflop order.
    let seats: Vec<Seat> = core_model::postflop_order(state.button, &state.dealt).into_iter().filter(|s| !d.folded[usize::from(s.0)]).collect();
    // The range source's lock is released at the end of this statement, before any equity or solve.
    let ranges = lock(&core.range_source).seat_ranges(state, &seats);
    let ranges = ranges.map_err(|reason| skip(format!("no street-root public ranges of the seats in the pot ({})", miss_cause(&reason))))?;
    let hero_public = ranges.iter().find(|(s, _)| *s == state.hero).map(|(_, r)| r.clone()).ok_or_else(|| skip("no street-root public range of hero's".into()))?;
    let others: Vec<(Seat, Range1326)> = ranges.into_iter().filter(|(s, _)| *s != state.hero).collect();
    let budget_ms = watch.deadlines.street_deadline_ms.saturating_sub(core.clock.now_ms()).min(EQUITY_BUDGET_MS);
    let opponent = crate::experimental::choose_opponent(state.hero, &hero_public, &others, &state.board, Duration::from_millis(budget_ms), equity_cancel)
        .ok_or_else(|| {
            skip(if equity_cancel.load(Ordering::SeqCst) {
                "the request's equity was cancelled before an opponent was chosen".into()
            } else {
                "the equity phase overran: no seat's range-vs-range equity was computed within its budget".into()
            })
        })?;
    let template = live_template(core, state, d.street);
    let input = crate::experimental::synthetic_root(d, state, state.hero, opponent, d.street, template).map_err(skip)?;
    let opp_public = others.into_iter().find(|(s, _)| *s == opponent).map(|(_, r)| r).expect("the chosen opponent is one of the other seats in the pot");
    let ranges = if input.hero_role == "oop" { [hero_public, opp_public] } else { [opp_public, hero_public] };
    let request = SurrogateRequest { identity: req.identity.clone(), deadlines: watch.deadlines, street_deadline: watch.street_deadline.clone(),
        final_claim: Some(watch.delivered.clone()) };
    crate::experimental::run_surrogate(core, &input, ranges, &state.board, hero_cards, state.config.bb_chips, &state.config.rake, req.config.solver.target_bp, &request,
        &req.sink)
}

/// §7: the whole decision may spend at most this long in cache lookups, every probe together.
pub const CACHE_BUDGET_MS: u64 = 500;

/// One cache probe of a decision (plan 4 Task 10): the template asked, its effective tree at the street root and that
/// tree's signature and pot, and the lookup's outcome.
#[derive(Clone, Debug)]
pub struct Probe {
    pub template_id: String,
    pub tree: EffectiveTree,
    pub signature: String,
    pub pot: u32,
    pub result: Lookup,
}

/// One probe: `template`'s effective tree at `root` (as the live solve would build it), the §10.4 query of the decision
/// over the public root `ranges` (OOP then IP) with the request's inherited reasons, the shared suit permutation `perm`
/// and the request's captured `target_bp`, answered by `Cache::lookup` within what is left, on the engine clock, until
/// the phase's absolute `cutoff_ms` (review P4T10-I4): read again immediately before the lookup, so the time the tree,
/// signature and query took, or a stall of `engine-main`, is never given back to it (`Cache::lookup` further bounds
/// it by `cache::lookup::LOOKUP_BOUND`; nothing left is a `BudgetExhausted` miss decided at once, before anything is
/// posted). `None` when the template does not build at this root or the query cannot be formed (the live solve then
/// answers as it would without the cache).
#[allow(clippy::too_many_arguments)]
pub(crate) fn probe_cache(core: &EngineCore, hooks: &Hooks, root: &StreetRootSnapshot, ranges: [&Range1326; 2], d: &Derived, state: &HandState,
    inherited: &[ApproxReason], perm: &SuitPerm, template: &str, target_bp: u16, cutoff_ms: u64) -> Option<Probe> {
    let build = build_tree_full(root, &TemplateSelection::from_history(template, &root.history)).ok()?;
    let signature = tree_signature(&build.tree, build.pot);
    let input = SolveInput { root: root.clone(), ranges: [ranges[0].clone(), ranges[1].clone()], tree: build.tree.clone(), target_bp };
    let mut query = make_cache_query(&input, d, state.config.bb_chips, &state.config.rake, inherited, &signature, perm, target_bp, Duration::ZERO).ok()?;
    if let Some(before_lookup) = &hooks.before_lookup {
        before_lookup();
    }
    query.budget = Duration::from_millis(cutoff_ms.saturating_sub(core.clock.now_ms()));
    let result = core.cache.lookup(&query);
    Some(Probe { template_id: template.into(), tree: build.tree, signature, pot: build.pot, result })
}

/// §5 step 7 / §10.5: the decision's probes, the pre-solver's template first on the flop and then the live template when
/// it is another, stopping at the first hit at target. The phase has one absolute cutoff on the engine clock (review
/// P4T10-I4): `CACHE_BUDGET_MS` from its start, or the street deadline when that comes first. A probe is never started
/// once the cutoff has passed (no tree, signature or query is built for it), and each lookup gets only what is left
/// until the cutoff when it is made. Returns the route (`flop::choose_cache_route`), the probes made and the templates
/// never asked because the phase was spent. Runs on `engine-main` (ruling 10-pre1), holding no engine lock.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cache_phase(core: &EngineCore, hooks: &Hooks, root: &StreetRootSnapshot, ranges: [&Range1326; 2], d: &Derived, state: &HandState,
    inherited: &[ApproxReason], perm: &SuitPerm, live_template: &str, target_bp: u16, street_deadline_ms: u64) -> (CacheRoute, Vec<Probe>, Vec<String>) {
    let mut order: Vec<&str> = Vec::new();
    if root.street == Street::Flop {
        order.push(PRESOLVER_TEMPLATE);
    }
    if !order.contains(&live_template) {
        order.push(live_template);
    }
    let cutoff_ms = core.clock.now_ms().saturating_add(CACHE_BUDGET_MS).min(street_deadline_ms);
    let (mut probes, mut unasked) = (Vec::new(), Vec::new());
    for template in order {
        if core.clock.now_ms() >= cutoff_ms {
            unasked.push(template.to_string());
            continue;
        }
        let Some(probe) = probe_cache(core, hooks, root, ranges, d, state, inherited, perm, template, target_bp, cutoff_ms) else { continue };
        let at_target = matches!(probe.result, Lookup::Exact { .. } | Lookup::Approximate { .. });
        probes.push(probe);
        if at_target {
            break;
        }
    }
    let route = choose_cache_route(probes.iter().map(|p| p.result.clone()).collect());
    (route, probes, unasked)
}

/// §4.4 `Assumptions.cache` (`miss | exact | approximate | provisional`, review m13) of a decision's probes: the label
/// of the hit served, `provisional` when only above-target hits were found, `miss` when nothing was.
pub fn cache_label_for(probes: &[Probe]) -> String {
    let mut label = "miss";
    for p in probes {
        label = match (&p.result, label) {
            (Lookup::Exact { .. }, _) => "exact",
            (Lookup::Approximate { .. }, "exact") => "exact",
            (Lookup::Approximate { .. }, _) => "approximate",
            (Lookup::Provisional { .. }, "miss") => "provisional",
            _ => label,
        };
    }
    label.into()
}

/// §4.4 display: a raw exploitability over the pot in basis points, rounded, shown as `u16::MAX` above it (the bounded
/// display conversion `assemble::coverage_for_solve` and the solve client use). Never compared with a target.
fn display_bp(raw_over_p: f64) -> u16 {
    let bp = (raw_over_p * 10_000.0).round();
    if bp <= f64::from(u16::MAX) { bp as u16 } else { u16::MAX }
}

/// Review P4T10-I1 (ruling 10-Q2; spec 2, 7): the coverage reason of a cache hit served above the request's target, the
/// stored raw accuracy it reached (`display_bp`) against the request's own target. Only the engine's delivery of a
/// `Provisional` adds it; the cache's label layer never does.
fn accuracy_shortfall(raw_over_p: f64, target_bp: u16) -> ApproxReason {
    ApproxReason::DeadlineBestSoFar { reached_bp: display_bp(raw_over_p), target_bp }
}

/// The probe a route's hit came from: the one whose tree signature the hit carries (each probe asks a template of its
/// own, and a template's signature names it).
fn probe_of<'a>(probes: &'a [Probe], hit: &CacheHit) -> &'a Probe {
    probes.iter().find(|p| p.signature == hit.tree_signature).unwrap_or_else(|| panic!("a cache hit of signature {} that no probe asked for", hit.tree_signature))
}

/// A `Provisional` that was accepted: the hit, and its payload promoted to `Final` (retained for the watchdog).
struct Retained {
    hit: CacheHit,
    promoted: Recommendation,
}

/// What serving a cache hit reads of the request: its street root, its public root ranges (OOP then IP: the ranges the
/// query was keyed by, and the snapshot is), its assembly context, hero's public range and role, and its target.
struct Cached<'a> {
    root: &'a StreetRootSnapshot,
    ranges: &'a RootRanges,
    ctx: &'a AssemblyCtx,
    hero_public: &'a Range1326,
    hero_actor: &'a str,
    target_bp: u16,
}

impl Cached<'_> {
    /// The result a cache hit answers with (§4.4, §10.4): hero's frequencies and EVs at the requested node of the hit's
    /// solution (rebuilt in the query's chips and suits), labelled `coverage`, in `phase`. The assumptions are `base`
    /// with the cache as the source, the hit's raw accuracy as an upper bound in bp and rounded for display, the hit's
    /// notes (the realized menu, the source storage mode), the elapsed time, and (review P4T10-I3; plan-3 F-I1, spec
    /// 4.4, 10.4) `translations` and `mappings` rebuilt from the complete `coverage`, the stored entry's inherited reasons
    /// included, with the one classification of a mapping (`preflop::is_mapping`), each reason once.
    fn recommendation(&self, core: &EngineCore, req: &LiveRequest, hit: &CacheHit, coverage: Coverage, base: &Assumptions, phase: Phase) -> Recommendation {
        let requested = hit.solution.requested as usize;
        let raw = hit.raw_exploitability_over_p;
        assert!(raw.is_finite() && raw >= 0.0, "decision {:?}: a cache hit's raw accuracy {raw}", req.identity);
        let mut a = base.clone();
        a.source = format!("cache@{SOLVER_COMMIT}");
        // §4.4 vocabulary `"exploitability <= x"`, a true bound in bp over the raw stored value (ruling 28-I5).
        a.source_accuracy = format!("exploitability <= {} bp", (raw * 10_000.0).ceil() + 0.0);
        a.reached_bp = Some(display_bp(raw));
        a.elapsed_ms = elapsed_ms(req.t0_ms, core.clock.now_ms());
        let reasons = coverage_reasons(&coverage);
        a.translations = reasons.iter().filter(|r| matches!(r, ApproxReason::BetTranslation { .. })).cloned().collect();
        a.mappings = reasons.iter().filter(|r| crate::preflop::is_mapping(r)).cloned().collect();
        a.notes.extend(hit.notes.iter().cloned());
        let reach = assemble::hero_reach(&hit.solution.nodes, &hit.covered_paths, requested, self.hero_public, self.hero_actor);
        let mut rec = assemble::final_from_solution(self.ctx, &hit.solution.nodes[requested], &reach, coverage, a);
        rec.phase = phase;
        rec
    }

    /// The §9.1 snapshot of a cache hit (plan-3 carry (d)): `replay_bridge::snapshot_from_solution` over the input the hit
    /// answers (the street root, the public root ranges, the query tree the hit was rebuilt on), its solution, its
    /// ordinal paths and signature, `origin` (one of `replay_bridge::ORIGINS`) and the result's full coverage reasons
    /// (ruling 18-I1).
    fn snapshot(&self, req: &LiveRequest, hit: &CacheHit, origin: &str, reasons: Vec<ApproxReason>) -> StreetSnapshot {
        let input = SolveInput { root: self.root.clone(), ranges: [self.ranges.oop.clone(), self.ranges.ip.clone()], tree: hit.tree.clone(), target_bp: self.target_bp };
        snapshot_from_solution(&req.identity, &input, &hit.solution, hit.covered_paths.clone(), hit.tree_signature.clone(), origin, reasons)
    }

    /// The `Final` of a retained cache payload (plan 4 Task 10): the promoted `Provisional` with its own coverage (review
    /// P4T10-I2, ruling 10-Q4: the payload's reasons, its accuracy shortfall and the inherited ones, never a reason of the
    /// discarded live solve), `note` disclosing the refinement's own outcome, and the elapsed time now; its
    /// `cache_provisional` snapshot registered again with the `Final` (the same decision's, so it replaces the
    /// `Provisional`'s), carrying the `Final`'s full coverage reasons. `best_so_far` is the refinement's, for the log's
    /// street verdict only.
    fn retained_final(&self, core: &EngineCore, req: &LiveRequest, r: &Retained, note: String, best_so_far: bool) -> Candidate {
        let mut rec = r.promoted.clone();
        rec.assumptions.notes.push(note);
        rec.assumptions.elapsed_ms = elapsed_ms(req.t0_ms, core.clock.now_ms());
        let snapshot = self.snapshot(req, &r.hit, "cache_provisional", coverage_reasons(&rec.coverage));
        Candidate { rec, record: Record::Snapshot(snapshot), best_so_far }
    }
}

/// Plan 4 Task 10 (plan-3 carry (c), rulings 28-I1/28-I2/28-N1): a cache `Provisional`'s delivery. It is accepted
/// under the identity lock, where its decision must be active and its `Final` not yet delivered (the watchdog claims
/// the `Final` under the same lock); in that same accepted delivery its snapshot is registered by the one registration
/// rule (`replay_bridge::register_accepted`: a `Provisional` never replaces a `Final`) and `promoted` is retained for
/// the watchdog (`set_retained`). The `Provisional` is then handed to the sink with no engine lock held, unless the
/// watchdog delivered the `Final` meanwhile (which is then `promoted` itself). Returns whether it was accepted (its
/// snapshot registered and its payload retained); a decision no longer active, or whose `Final` was delivered, gets
/// nothing, and nothing is registered.
fn deliver_provisional(core: &EngineCore, req: &LiveRequest, rec: Recommendation, snapshot: StreetSnapshot, promoted: Recommendation) -> bool {
    let watch = req.watch.as_ref().unwrap_or_else(|| panic!("decision {:?}: a Provisional for a request at no decision point", req.identity));
    let snapshots = &core.snapshots;
    let mut pending = Some((snapshot, promoted));
    let mut accepted = false;
    let verdict = accept(&core.identity, &req.identity, None, |active| {
        if watch.delivered.load(Ordering::SeqCst) {
            return;
        }
        let (snapshot, promoted) = pending.take().expect("an acceptance runs once");
        register_accepted(&mut lock(snapshots), active, snapshot);
        set_retained(req, promoted);
        accepted = true;
    });
    if verdict == Delivery::Accepted && accepted {
        let mut sink = lock(&req.sink);
        if !watch.delivered.load(Ordering::SeqCst) {
            sink.emit(RecommendationEvent::Provisional(rec));
        }
    }
    accepted
}

/// §5 step 7 / §10.4: stores the live terminal of a flop or turn solve (`flop::cacheable`: never a river's, an
/// experimental surrogate's or a locked baseline's), built by `cache_bridge::entry_from_solution` from the input
/// actually solved (the answering attempt's tree, the `_min` retry's when it answered), the request's inherited reasons,
/// the shared suit permutation `perm` and the request's target, through the non-blocking `Cache::store`. An entry the
/// cache refuses is logged (`EngineCore::log_cache_reject`) and stored nowhere.
#[allow(clippy::too_many_arguments)]
fn store_live_terminal(core: &mut EngineCore, req: &LiveRequest, root: &StreetRootSnapshot, ranges: &RootRanges, out: &SolveOutcome, pot: u32,
    inherited: &[ApproxReason], perm: &SuitPerm, target_bp: u16) {
    let (Terminal::Ok | Terminal::BestSoFar, Some(sol)) = (&out.terminal, &out.solution) else { return };
    // Phase 1 solves the baseline model only (§10.4 `model = baseline`); experimental surrogates never reach here.
    if !cacheable(root.street, false, sol.locks_applied, true) {
        return;
    }
    let solved = SolveInput { root: root.clone(), ranges: [ranges.oop.clone(), ranges.ip.clone()], tree: out.tree.clone(), target_bp };
    let signature = tree_signature(&out.tree, pot);
    match entry_from_solution(&solved, sol, inherited, req.state.config.bb_chips, &req.state.config.rake, &signature, perm, out.elapsed_ms, target_bp) {
        Ok(entry) => core.cache.store(&entry),
        Err(reason) => core.log_cache_reject(&req.identity, &reason),
    }
}

/// The `fast-path` thread (§3.4): §4.4's two equity populations against the one opponent, within the equity phase's
/// own budget on the engine's clock (§7), delivered as `Equity` through the same identity check as every event; the
/// UI merges it into whatever it displays, a `Final` included (§4.4). Hero's cards enter only the hero-combo
/// population's private hero-conditioned copy (`equity`), never a public range.
///
/// Cancellable (spec 7; ruling 28-I4): the routine polls `cancel`, the request's token, between its units of work. It
/// is set on supersession: by a newer request, by `serve_request` when it finds the decision no longer active, and here
/// when the decision is already stale as the thread starts. The request's own `Final` never sets it.
///
/// `opponents` are the seats hero's equity is shown against, each with its public range: the one opponent of a
/// heads-up street, every other seat still in the hand on the preflop path (plan 3 Task 17, spec 6's preflop rows).
pub(crate) fn spawn_equity(core: &EngineCore, req: &LiveRequest, hero_public: Range1326, opponents: Vec<(Seat, Range1326)>, board: Vec<Card>,
    routine: EquityRoutine, cancel: Arc<AtomicBool>) {
    let (ids, identity, sink, clock, hero) = (core.identity.clone(), req.identity.clone(), req.sink.clone(), core.clock.clone(), req.state.hero_cards);
    // Owned by the core (ruling 29-I2): joined at the engine's teardown, never detached.
    core.tasks.spawn("fast-path", move || {
        let stale = !lock(&ids).is_active(&identity);
        if stale {
            cancel.store(true, Ordering::SeqCst);
            return;
        }
        let equity = routine(clock.as_ref(), hero, &hero_public, &opponents, &board, Duration::from_millis(EQUITY_BUDGET_MS), &cancel);
        deliver(&ids, &identity, &sink, None, RecommendationEvent::Equity { identity: identity.clone(), equity });
    });
}

/// §6's analytic fallback facing an all-in, for hero's actual combo against the opponent's public range. `C` is what
/// calling costs hero (`Derived.legal`); the wager hero faces is what hero owes, `facing - committed_this_street[hero]`
/// (chips hero already put in this street are in `Derived.pot`, not faced); the pot is `Derived.pot`. `None` when
/// there is no call to price or no equity came back. Its equity polls `cancel`, the request's own token (final review
/// M9): a supersession stops it as it stops the request's `fast-path` equity (ruling 28-I4).
fn analytic_allin(req: &LiveRequest, d: &Derived, hero_public: &Range1326, opp_public: &Range1326, cancel: &AtomicBool) -> Option<AllInAnswer> {
    let hero = req.state.hero_cards?;
    let call_cost = d.legal.iter().find_map(|l| match l { LegalAction::Call { cost } => Some(*cost), _ => None })?;
    let committed = d.committed_this_street[seat_index(&req.state, req.state.hero)];
    let owed = d.facing.checked_sub(committed).unwrap_or_else(|| panic!("hero committed {committed} chips this street, above the {} chips faced", d.facing));
    let input = AllInInput { hero, board: req.state.board.clone(), opp_public: opp_public.clone(), hero_public: Some(hero_public.clone()), call_cost,
        pot: d.pot, facing: owed, rake: req.state.config.rake, bb_chips: req.state.config.bb_chips };
    facing_allin(&input, Duration::from_millis(EQUITY_BUDGET_MS), cancel).ok()
}

/// A request's side of its one `Final` (§7): what admission armed (its claim, shared with its watchdog, and the record
/// of what the watchdog delivered when it won it) and the request's equity cancellation token. Crate-visible for the
/// preflop path, which answers through it (`settle`).
pub(crate) struct Claim<'a> {
    pub(crate) watch: &'a Watched,
    equity_cancel: &'a AtomicBool,
}

/// A `Final` the engine prepared, what its accepted delivery records in the snapshot store (§9.2) and whether it is a
/// `best_so_far` (§12's violation rule).
struct Candidate {
    rec: Recommendation,
    record: Record,
    best_so_far: bool,
}

impl Candidate {
    /// A `Final` with no solution behind it, and no street root to record a miss at (a row the classifier settles).
    fn unsolved(rec: Recommendation) -> Self { Self { rec, record: Record::Nothing, best_so_far: false } }

    /// A heads-up postflop decision's `Final` with no solution behind it, and the miss its accepted delivery records.
    fn missed(rec: Recommendation, miss: SnapshotMiss) -> Self { Self { rec, record: Record::Miss(miss), best_so_far: false } }
}

/// What a candidate's accepted delivery records in the snapshot store, under the identity lock, as part of the claim
/// (§9.2; plan 3 Task 18).
enum Record {
    /// A validated solution: registered as a snapshot, by the one registration rule (`replay_bridge::register_accepted`).
    Snapshot(StreetSnapshot),
    /// A heads-up postflop decision answered with no solution: its miss, with the engine's cause (spec 9.3), which names
    /// the street's `UnconditionedPriorStreet` for a later replay when the street has no snapshot (Task 15 Q2).
    Miss(SnapshotMiss),
    /// Nothing: a row the classifier settles alone, the preflop path.
    Nothing,
}

impl Record {
    /// The miss this record's decision leaves when the watchdog's `Final` is delivered in the engine's place, with the
    /// cause `cause`: the decision registers no solution then. `None` for a row with no street root.
    fn missed_with(self, cause: String) -> Option<SnapshotMiss> {
        match self {
            Record::Snapshot(s) => Some(SnapshotMiss { identity: s.provenance.identity_at_solve, street: s.key.street, root_board: s.key.root_board,
                prefix: s.provenance.solved_prefix, cause }),
            Record::Miss(m) => Some(SnapshotMiss { cause, ..m }),
            Record::Nothing => None,
        }
    }
}

/// The cause of the `Final` `watch`'s watchdog delivered, if it delivered one (plan 3 Task 18). Read once the generation
/// is retired (`Watchdog::disarm`), so a fire in progress has recorded what it delivered.
fn watchdog_cause(watch: &Watched) -> Option<String> {
    lock(&watch.fired).as_ref().map(|fired| miss_cause_of_final(&fired.rec))
}

/// Records `miss`, the miss of a decision whose `Final` was delivered in the engine's place (its watchdog's, or a panic
/// containment's internal-error `Final`) (plan 3 Task 18). The fire delivers only to a decision still active, checked
/// under the identity lock (`watchdog`'s "Identity at the fire"), so the miss is that decision's delivered outcome and
/// is recorded even when a mutation has superseded the decision since, as its delivered `Final` is logged (ruling
/// W3-I1): the UI may act on that `Final` before `engine-main` gets here. A replay validates every miss against its
/// own state on read, and the next mutation's invalidation keeps or drops it like any other. The store's lock is taken
/// alone.
fn record_delivered_miss(core: &EngineCore, miss: SnapshotMiss) {
    let owner = miss.identity.clone();
    lock(&core.snapshots).record_miss(&owner, miss);
}

/// The street verdict of a request's log record (§7).
enum StreetVerdict {
    /// The solve client's: the first attempt's terminal arrived after the street deadline, or none arrived by it (ruling
    /// 20-I1).
    Judged(bool),
    /// No solve was attempted. A `Final` delivered before the watchdog's fire carries no verdict; one delivered at or
    /// after it (the request expired) carries the shared street deadline's (`StreetDeadline::violated`, ruling 28-O3),
    /// or, when the watchdog thread never recorded the street deadline (a suspend-style jump: the engine's own `Final`
    /// won the claim), the clock's, as the solve client judges a no-terminal outcome: the `Final` came at or after the
    /// street deadline (ruling 28-N3). `None` for the classifier's rows, which solve no street.
    Unattempted(Option<Arc<StreetDeadline>>),
}

/// What the decision log records of how a request ended besides the `Final` delivered (§5 step 10).
struct Logged {
    street: Street,
    verdict: StreetVerdict,
    /// The scaled-range hashes of the public ranges solved, OOP then IP; empty when none were read.
    range_hashes: Vec<String>,
}

impl Logged {
    /// A request answered before any solve, judged by `street_deadline` when given.
    fn unsolved(street: Street, street_deadline: Option<Arc<StreetDeadline>>, range_hashes: Vec<String>) -> Self {
        Self { street, verdict: StreetVerdict::Unattempted(street_deadline), range_hashes }
    }
}

/// The `Final` that reached the sink, when and from whom: the one the decision log records.
struct Delivered<'a> {
    at_ms: u64,
    rec: &'a Recommendation,
    by_watchdog: bool,
    best_so_far: bool,
}

/// The `Final` the watchdog delivered, as its fire recorded it. Read only once the claim was lost to the watchdog and
/// the watchdog retired (`disarm` returns only after a fire in progress, which records before it emits).
fn watchdog_final(fired: &Mutex<Option<Fired>>) -> Fired {
    lock(fired).clone().expect("the watchdog claimed the Final, and its fire records what it delivered before emitting it")
}

/// A `Final` the classifier settles alone, or the preflop path's (plan 3 Task 17): through the request's claim, shared
/// with its watchdog since admission. It solves no street, so it logs no street verdict.
pub(crate) fn settle(core: &mut EngineCore, req: &LiveRequest, hooks: &Hooks, claim: &Claim<'_>, street: Street, rec: Recommendation) {
    let _ = finish(core, req, hooks, claim, Candidate::unsolved(rec), Logged::unsolved(street, None, vec![]));
}

/// The one `Final` path (§5 steps 7, 9, 10; ruling 28-I2). The candidate's delivery is claimed under the identity lock:
/// a decision no longer active gets no `Final` of the engine's and its candidate registers nothing (its equity is
/// cancelled), and the log records only a `Final` its watchdog delivered while it was still active (`retire_stale`,
/// ruling W3-I1; a heads-up postflop decision whose `Final` its watchdog delivered records the watchdog's cause as its
/// miss, plan 3 Task 18); a claim the watchdog already won discards the candidate, which registers nothing (a heads-up
/// postflop decision records the watchdog's cause as its miss instead), and logs the watchdog's `Final` with its
/// delivery time; a claim won records the candidate's `Record` under the same lock, as part of that accepted delivery (a
/// solved candidate's snapshot by the one registration rule, an unsolved heads-up postflop decision's miss), hands the
/// `Final` to the sink with no engine lock held, and logs it. The claim and the handover are recorded on the request
/// (`Watched::claimed_by_engine`, `Watched::handed`) for a panic's containment (ruling F2-I3). The request's watchdog
/// generation is retired in every case. Returns how the claim ended (plan 4 Task 10: a stale candidate's solution is
/// not stored in the cache).
fn finish(core: &mut EngineCore, req: &LiveRequest, hooks: &Hooks, claim: &Claim<'_>, candidate: Candidate, logged: Logged) -> Delivery {
    if let Some(before_claim) = &hooks.before_claim {
        before_claim();
    }
    let at_ms = core.clock.now_ms();
    let Candidate { rec, record, best_so_far } = candidate;
    let snapshots = &core.snapshots;
    // Taken by the accepted claim; still here when the watchdog won it.
    let mut record = Some(record);
    let verdict = accept(&core.identity, &req.identity, Some(&claim.watch.delivered), |active| {
        claim.watch.claimed_by_engine.store(true, Ordering::SeqCst);
        match record.take() {
            Some(Record::Snapshot(snapshot)) => {
                if let Some(at_registration) = &hooks.at_registration {
                    at_registration();
                }
                register_accepted(&mut lock(snapshots), active, snapshot);
            }
            Some(Record::Miss(miss)) => {
                lock(snapshots).record_miss(active, miss);
            }
            Some(Record::Nothing) | None => {}
        }
    });
    match verdict {
        Delivery::Stale => {
            retire_stale(core, req, claim, &logged);
            // A `Final` the watchdog delivered while the decision was still active (ruling W3-I1) is its outcome.
            if let Some(miss) = watchdog_cause(claim.watch).and_then(|cause| record.take().and_then(|r| r.missed_with(cause))) {
                record_delivered_miss(core, miss);
            }
        }
        Delivery::AlreadyDelivered => {
            core.watchdog.disarm(claim.watch.generation);
            let fired = watchdog_final(&claim.watch.fired);
            if let Some(miss) = record.take().and_then(|r| r.missed_with(miss_cause_of_final(&fired.rec))) {
                record_delivered_miss(core, miss);
            }
            log_final(core, req, claim.watch, Delivered { at_ms: fired.at_ms, rec: &fired.rec, by_watchdog: true, best_so_far: false }, &logged);
        }
        Delivery::Accepted => {
            *lock(&claim.watch.handed) = Some((at_ms, rec.clone(), best_so_far));
            lock(&req.sink).emit(RecommendationEvent::Final(rec.clone()));
            core.watchdog.disarm(claim.watch.generation);
            log_final(core, req, claim.watch, Delivered { at_ms, rec: &rec, by_watchdog: false, best_so_far }, &logged);
        }
    }
    verdict
}

/// A stale exit (§4.4, §12; ruling W3-I1): the request's decision is no longer active. Its equity is cancelled (ruling
/// 28-I4) and its watchdog generation retired first, so a fire in progress has completed its emission and has recorded
/// what it delivered (`disarm` returns only after such a fire, which records before it emits). A `Final` the watchdog
/// claimed while the decision was still active was delivered, and it is the request's `Final`: it is logged, exactly
/// once, as the watchdog's, with its recorded delivery time and the request's deadline verdicts (spec 5 step 10, every
/// request and its `Final`; spec 7 and 12, the deadline outcomes). A decision superseded before any claim leaves the
/// slot empty (the fire of a decision no longer active records nothing, `watchdog`'s "Identity at the fire"): no
/// `Final`, no record. Nothing of a candidate is accepted or registered here.
fn retire_stale(core: &mut EngineCore, req: &LiveRequest, claim: &Claim<'_>, logged: &Logged) {
    claim.equity_cancel.store(true, Ordering::SeqCst);
    log_watchdog_final(core, req, claim.watch, logged);
}

/// Retires `watch`'s generation, then logs the `Final` its watchdog delivered, if it delivered one and it is not logged
/// yet (ruling W3-I1): what every exit that delivers nothing of its own ends with.
fn log_watchdog_final(core: &mut EngineCore, req: &LiveRequest, watch: &Watched, logged: &Logged) {
    core.watchdog.disarm(watch.generation);
    let delivered_by_watchdog = lock(&watch.fired).clone();
    if let (Some(fired), false) = (delivered_by_watchdog, watch.logged.load(Ordering::SeqCst)) {
        assert!(watch.delivered.load(Ordering::SeqCst), "the watchdog recorded a Final of decision {:?} without its claim", req.identity);
        log_final(core, req, watch, Delivered { at_ms: fired.at_ms, rec: &fired.rec, by_watchdog: true, best_so_far: false }, logged);
    }
}

/// A request `engine-main` never serves (final review I1: admission arms before the request is served): one dropped
/// from the depth-1 queue by a newer request, or still queued at shutdown. Its decision was superseded; its watchdog
/// generation is retired, and a `Final` its watchdog delivered while the decision was still active (the request waited
/// past its fire) is logged, exactly once (ruling W3-I1; spec 5 step 10), and recorded as its street's miss with the
/// watchdog's cause (plan 3 Task 18, fix round 1, ruling 18-I2). It solved nothing, so the street deadline's verdict is
/// the watchdog's.
pub(crate) fn retire_unserved(core: &mut EngineCore, req: &LiveRequest) {
    if let Some(watch) = &req.watch {
        log_watchdog_final(core, req, watch, &Logged::unsolved(watch.street, Some(watch.street_deadline.clone()), vec![]));
        // Plan 3 Task 18 (fix round 1, ruling 18-I2): a `Final` its watchdog delivered is the request's outcome, recorded
        // as its street's miss with the delivered cause. The request was never served, so its street root is recovered
        // from its state here (the classifier's own heads-up root; none at a preflop, multiway or unreproducible root).
        if let Some(cause) = watchdog_cause(watch) {
            if let Classification::HuStreet { root, .. } = classify(&req.state) {
                record_delivered_miss(core, miss_for(&req.identity, &root, cause));
            }
        }
    }
}

/// Final review I3 (ruling F-I3): `engine-main` caught a panic while it served `req` (an always-on assert of an internal
/// invariant, `message`). The worker may be running the request's job, so it is killed; the next solve relaunches it
/// once (`run_solve`). The request's watchdog generation is retired. Its `Final`, unless one was already delivered, is
/// the non-retryable `Unsupported{EngineError("internal: ..")}` built from the request's fallback (its legal intervals,
/// equity and inherited reasons), delivered through the request's claim to a decision still active, or, when
/// `engine-main` had already taken that claim for its own `Final` and panicked before handing it over (final fix round
/// 2, ruling F2-I3: `Watched::claimed_by_engine` with nothing `handed`), delivered at once under that claim, as the
/// engine's own `Final` would have been. The `Final` delivered (the engine's handed over before the panic, this one, or
/// the watchdog's) is logged once. Every lock here survives the panic (`lock`).
///
/// Plan 3 Task 18 (fix round 1, ruling 18-I2; spec 9.3): the `Final` this containment or the watchdog delivered is the
/// request's outcome, recorded as its street's miss with the delivered cause (`engine error: internal: ..`, or the
/// watchdog's deadline) at the street root `serve_request` kept on the request (`Watched::root`); nothing is recorded
/// when no `Final` was delivered (a decision superseded before any claim), when the engine's own `Final` went out before
/// the panic (its accepted claim recorded it), or when the panic came before the root was derived.
pub(crate) fn contain_panic(core: &mut EngineCore, req: &LiveRequest, message: &str) {
    crate::solve::diagnose(core, core.clock.now_ms(), "panic", Some(&req.identity), format!("engine-main panicked serving this decision: {message}"), None);
    crate::solve::kill_worker(core, &format!("engine-main panicked serving decision {:?}", req.identity));
    let Some(watch) = &req.watch else { return };
    let logged = Logged::unsolved(watch.street, Some(watch.street_deadline.clone()), vec![]);
    core.watchdog.disarm(watch.generation);
    // The engine's own `Final` went out before the panic: it is the request's `Final`, logged once.
    let handed = lock(&watch.handed).clone();
    if let Some((at_ms, rec, best_so_far)) = handed {
        if !watch.logged.load(Ordering::SeqCst) {
            log_final(core, req, watch, Delivered { at_ms, rec: &rec, by_watchdog: false, best_so_far }, &logged);
        }
        return;
    }
    let mut rec = lock(&watch.fallback).clone();
    rec.phase = Phase::Final;
    let partial = match &rec.coverage { Coverage::Unsupported { partial, .. } => partial.clone(), _ => vec![] };
    rec.coverage = Coverage::Unsupported { reason: UnsupportedReason::EngineError { message: format!("internal: {message}"), retryable: false }, partial };
    let at_ms = core.clock.now_ms();
    // A claim `engine-main` took is its own: the watchdog lost it, so no other `Final` went out.
    let verdict = if watch.claimed_by_engine.load(Ordering::SeqCst) { Delivery::Accepted } else { accept(&core.identity, &req.identity, Some(&watch.delivered), |_| {}) };
    let delivered_cause = match verdict {
        Delivery::Accepted => {
            *lock(&watch.handed) = Some((at_ms, rec.clone(), false));
            lock(&req.sink).emit(RecommendationEvent::Final(rec.clone()));
            log_final(core, req, watch, Delivered { at_ms, rec: &rec, by_watchdog: false, best_so_far: false }, &logged);
            Some(miss_cause_of_final(&rec))
        }
        Delivery::AlreadyDelivered | Delivery::Stale => {
            log_watchdog_final(core, req, watch, &logged);
            watchdog_cause(watch)
        }
    };
    let kept_root = lock(&watch.root).clone();
    if let (Some(cause), Some(root)) = (delivered_cause, kept_root) {
        record_delivered_miss(core, miss_for(&req.identity, &root, cause));
    }
}

/// Logs the `Final` delivered (§5 step 10): its coverage, reasons, cache result (its `assumptions.cache`), template and
/// reached exploitability, the time it was delivered at, and the deadline verdicts. A `Final` the watchdog delivered is a
/// final-delivery violation by construction (the engine's own did not come in time). The street verdict is
/// `flop::is_street_violation`'s: a late first terminal always, and a `best_so_far` except on a single-raised-pot flop
/// cache miss (fewer than three preflop wagers, `cache == "miss"`), the designed outcome (§7, §10.6). Marks the
/// request's `Final` logged.
fn log_final(core: &mut EngineCore, req: &LiveRequest, watch: &Watched, delivered: Delivered<'_>, logged: &Logged) {
    let rec = delivered.rec;
    let reasons = coverage_reasons(&rec.coverage);
    let final_violation = delivered.by_watchdog || delivered.at_ms >= watch.deadlines.watchdog_fire_ms();
    let arrival_violation = match &logged.verdict {
        StreetVerdict::Judged(violated) => *violated,
        StreetVerdict::Unattempted(street_deadline) => final_violation && street_deadline.as_ref().is_some_and(|d| d.violated() || delivered.at_ms >= d.deadline_ms()),
    };
    // §12 "Street deadline reached": a `best_so_far` is logged as a violation on every street but a single-raised-pot
    // flop cache miss, the designed outcome (plan 4 Task 10; ruling 10-B2: the brief's `< 3` preflop wagers).
    let best_so_far = delivered.best_so_far;
    let srp_miss = logged.street == Street::Flop && preflop_wagers(&req.state) < 3 && rec.assumptions.cache == "miss";
    let street_violation = is_street_violation(logged.street, srp_miss, best_so_far, arrival_violation);
    watch.logged.store(true, Ordering::SeqCst);
    core.log.append(&DecisionRecord { identity: req.identity.clone(), street: logged.street, coverage: rec.coverage.clone(), reasons,
        elapsed_ms: elapsed_ms(req.t0_ms, delivered.at_ms), cache: rec.assumptions.cache.clone(), presolver_scenario: None, tier: None, reached_bp: rec.assumptions.reached_bp,
        street_violation, final_violation, template_id: rec.assumptions.template_id.clone(), input: InputRecord::from_state(&req.state, logged.range_hashes.clone()) });
}

/// §7 after a `Final` at the watchdog's fire: kill the busy worker, never relaunch it here (ruling F2-Q1; see "After the
/// `Final`" above). Recorded in the diagnostics log (final review M2).
fn kill_the_busy_worker(core: &mut EngineCore) {
    crate::solve::kill_worker(core, "a job was left running at the watchdog's fire");
}

/// §4.4's "exploitability <= x" in basis points of `pot` (ruling 28-I5): the least whole number of basis points not
/// below the raw measurement, computed in f64 from the validated `f32` measurement, never from the rounded or
/// saturated display value `reached_bp`. `exploitability_chips * 10_000` is exact in f64 (a 24-bit significand times
/// 10^4) and the division is correctly rounded: a whole-number ratio comes out exact, and any other lies further from a
/// whole number than the division's rounding error, so its ceiling is the bound. `+ 0.0` turns a validated `-0.0`
/// measurement's `-0` into `0` (ruling 28-N2); the value stays an f64, never narrowed, so no bound saturates.
fn accuracy_bound_bp(exploitability_chips: f32, pot: u32) -> f64 {
    assert!(pot > 0 && exploitability_chips.is_finite() && exploitability_chips >= 0.0, "accuracy bound of {exploitability_chips} chips of a {pot}-chip pot");
    (f64::from(exploitability_chips) * 10_000.0 / f64::from(pot)).ceil() + 0.0
}

/// Milliseconds from the request's admission to `now_ms` on the engine clock. A request ends by its final delivery (at
/// most 35 s after `t0`), so the span fits a `u32`; a reading before `t0` or a span that does not fit is an engine
/// bug, asserted rather than wrapped.
pub(crate) fn elapsed_ms(t0_ms: u64, now_ms: u64) -> u32 {
    let span = now_ms.checked_sub(t0_ms).unwrap_or_else(|| panic!("engine clock reading {now_ms} ms precedes the request's admission at t0 {t0_ms} ms"));
    u32::try_from(span).unwrap_or_else(|_| panic!("a request span of {span} ms does not fit u32"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::DecisionLog;
    use crate::ranges::ExplicitRanges;
    use crate::snapshots::{SnapshotKey, SnapshotProvenance};
    use crate::testing::{board, hand, play, uniform_solution, FakeClock, FakeReply, FakeWorker, IdRef, RecordingSink};
    use proto::worker::{AckStatus, ResultStatus};
    use proto::{resolve_chip_path, Action, OrdinalPath};

    /// Plan 3 Task 14: the snapshot an accepted river `Final` registers is a `core_replay::StreetSnapshot` built at the
    /// register site: keyed by the request's identity, the street root and the public root ranges solved (hero's cards
    /// are in neither: the hashes are the public ranges', never a hero-conditioned copy's) and the solved tree's
    /// signature; its provenance is the request's identity, the street's observed history at the root and `live`; its
    /// tree, nodes, covered ordinal paths, exploitability and reasons are the validated solve's.
    #[test]
    fn an_accepted_final_registers_the_street_snapshot_of_its_solve() {
        let aa = [Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()];
        // Hero holds the button (IP); the BB checks the river to hero.
        let s = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(0), Some(aa));
        let s = play(&s, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call]);
        let s = board(&play(&board(&play(&board(&s, "Kh 7d 2c"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d 9s");
        let s = play(&s, &[Action::Check]);
        let root = core_model::street_root(&s).unwrap();
        assert_eq!(root.history, vec![(Seat(2), Action::Check)]);
        let built = build_tree_full(&root, &TemplateSelection::from_history("river_std_v1", &root.history)).unwrap();
        let sol = uniform_solution(&built.tree, &built.history, 0.2);

        let clock = FakeClock::new();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let script = vec![FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None },
            FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(sol.clone()), error: None, elapsed_ms: 3 }];
        let (worker, _) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
        let mut core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&std::env::temp_dir().join("pokerai_serve_snapshot_log")));
        let mut public = Range1326([1.0; 1326]);
        core_ranges::block_public(&mut public, &s.board);
        // IP's public range differs from OOP's, so the key's OOP-then-IP order is observable.
        let mut ip = public.clone();
        ip.0.iter_mut().take(200).for_each(|w| *w *= 0.5);
        *core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: Some(public.clone()), ip: Some(ip.clone()) });
        let (sink, events) = RecordingSink::new(clock.clone(), None);
        let id = { let mut ids = identity.lock().unwrap(); ids.set_config(); ids.begin_hand(); ids.next_decision().unwrap() };
        let req = LiveRequest::admitted(&core, id.clone(), s.clone(), clock.now_ms(), Arc::new(Mutex::new(Box::new(sink))));
        serve_request(&mut core, req);
        core.shutdown();

        let finals: Vec<Recommendation> = events.lock().unwrap().iter().filter_map(|r| match &r.event { RecommendationEvent::Final(x) => Some(x.clone()), _ => None }).collect();
        assert_eq!(finals.len(), 1);
        assert!(matches!(finals[0].coverage, Coverage::Exact), "{:?}", finals[0].coverage);
        let stored = core.snapshots.lock().unwrap().for_identity(&id);
        assert_eq!(stored.len(), 1);
        let snap = &stored[0];
        let hashes = [hash_scaled(&public), hash_scaled(&ip)];
        assert_ne!(hashes[0], hashes[1]);
        assert_eq!(snap.key, SnapshotKey { hand_id: id.hand_id, config_revision: id.config_revision, model_revision: id.model_revision, street: Street::River,
            root_board: s.board.clone(), root_range_hashes: hashes, tree_signature: finals[0].assumptions.tree_signature.clone() });
        assert_eq!(snap.key.tree_signature, tree_signature(&built.tree, built.pot));
        assert_ne!(hash_scaled(&core_ranges::hero_conditioned(&public, aa)), hashes[0], "hero's cards would change the hash, so they are in neither range");
        assert_eq!(snap.provenance, SnapshotProvenance { identity_at_solve: id.clone(), solved_prefix: vec![(Seat(2), Action::Check)], origin: "live".into() });
        let ordinal: Vec<OrdinalPath> = sol.covered_paths.iter().map(|p| resolve_chip_path(&built.tree.materialized, p).unwrap()).collect();
        assert_eq!((&snap.tree, &snap.nodes, &snap.covered_paths), (&built.tree, &sol.nodes, &ordinal));
        assert_eq!((snap.exploitability_chips, snap.reasons.clone()), (0.2, vec![]));
        assert_eq!(core.snapshots.lock().unwrap().for_hand(id.hand_id).len(), 1, "for_hand (identity_race_golden's view) reads the same store");
    }

    /// Final fix round 2, ruling F2-N3 (spec 7: at the fire the watchdog delivers the retained payload, a `Provisional`
    /// or an earlier `best_so_far`, when there is one): a request armed at admission exposes its watchdog's retained
    /// slot through `set_retained`, and a payload set there is what the watchdog delivers as the request's `Final` at its
    /// fire, in place of the `DeadlineExceeded` fallback, recorded as delivered and with the claim taken.
    #[test]
    fn a_payload_retained_through_set_retained_is_what_the_watchdog_delivers_at_the_fire() {
        let aa = [Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()];
        let s = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), Some(aa));
        let s = play(&s, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call]);
        let s = board(&play(&board(&play(&board(&s, "Kh 7d 2c"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d 9s");
        let clock = FakeClock::new();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let (worker, _) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![]);
        let mut core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&std::env::temp_dir().join("pokerai_serve_retained_log")));
        let ended = core.watchdog.ended_threads();
        let (sink, events) = RecordingSink::new(clock.clone(), None);
        let id = { let mut ids = identity.lock().unwrap(); ids.set_config(); ids.begin_hand(); ids.next_decision().unwrap() };
        let req = LiveRequest::admitted(&core, id.clone(), s, clock.now_ms(), Arc::new(Mutex::new(Box::new(sink))));
        let watch = req.watch.as_ref().expect("a river decision is armed at admission");
        // A validated payload of the request (plan 4's promoted `Provisional` or `best_so_far`), marked so it cannot be
        // mistaken for the fallback.
        let mut retained = lock(&watch.fallback).clone();
        retained.phase = Phase::Provisional;
        retained.coverage = Coverage::Approximate { reasons: vec![ApproxReason::UnconditionedCurrentStreet] };
        retained.assumptions.notes.push("the retained payload".into());
        set_retained(&req, retained.clone());
        clock.set_ms(watch.deadlines.watchdog_fire_ms());
        ended.wait_for(1);
        let finals: Vec<(u64, Recommendation)> = events.lock().unwrap().iter()
            .filter_map(|r| match &r.event { RecommendationEvent::Final(x) => Some((r.at_ms, x.clone())), _ => None }).collect();
        let fired = lock(&watch.fired).clone().map(|f| (f.at_ms, f.rec));
        let claimed = watch.delivered.load(Ordering::SeqCst);
        core.shutdown();
        let mut expected = retained;
        expected.phase = Phase::Final;
        let fire_ms = watch.deadlines.watchdog_fire_ms();
        assert_eq!(finals, vec![(fire_ms, expected.clone())], "the watchdog delivers the retained payload as the Final at the fire");
        assert_eq!((fired, claimed), (Some((fire_ms, expected)), true), "recorded as delivered, the claim taken");
    }
}
