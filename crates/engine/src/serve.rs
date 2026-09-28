//! Spec §5 steps 4-10 for one live request on `engine-main` (Task 28): the river and turn decision path. The preflop
//! and flop paths attach at the `Classification::Preflop` arm and at the flop guard below (plans 3 and 4).
//!
//! One request. `serve_request` classifies the decision (§6) and answers at once every row the classifier settles
//! alone; a degraded engine (a worker whose `ready` was refused at startup, spec 12, ruling 29-I4) answers every
//! decision that way too, with the version mismatch. For a heads-up river or turn decision it arms the watchdog (§7),
//! reads the public root ranges from the range
//! source (§9: the only provider, which owns their validation, rulings 27-D3/D4), emits `Fast` (§5 step 5), starts the
//! `fast-path` equity thread (§3.4), solves through the solve client (`solve::run_solve`, Tasks 22-23), assembles the
//! `Final` (§4.4, §5 step 7), registers a validated solution as a snapshot (§9.2) and logs the decision (§5 step 10).
//!
//! Identity (ruling 28-I1). Every event goes through `deliver` (`emit` for the crate): it is accepted under the
//! identity lock, where a decision no longer active is refused and a `Final` claims the request's once-only delivery,
//! and an accepted event is handed to the sink only after that lock is released, so this path holds no engine lock
//! during a sink callback and a sink may re-enter the engine (read the identity, mutate the hand). The watchdog's fire
//! makes the same decision under the same lock (follow-up P2.W3, `watchdog`'s "Identity at the fire"): a decision no
//! longer active gets no watchdog `Final` and its claim stays untaken, with no retirement hook needed on supersession;
//! the fire releases the identity lock before its sink callback and keeps only its own generation lock. A mutation that
//! lands between the acceptance and the callback lets that one event through; the UI refuses it by identity (§5 step
//! 9). A request whose decision is no longer active, at admission or once its solve returns, ends with no `Final` of
//! the engine's and no snapshot, and its watchdog is retired; it logs nothing, unless its watchdog had claimed and
//! delivered its `Final` while the decision was still active and the supersession came after that claim: that
//! delivered `Final` is then the request's, and it is logged exactly once, with the delivery time and the deadline
//! verdicts the fire recorded (`retire_stale`, ruling W3-I1; spec 5 step 10). Once the solve returns, the identity and
//! the watchdog's claim are read in two steps; a supersession and a fire landing between them leave the claim untaken,
//! so the engine's own claim then finds the decision stale (re-review observation O4).
//!
//! One `Final` (§7, ruling 28-I2). The engine's `Final` and the watchdog's race for one claim (`Armed::delivered`); the
//! fire records what it delivered and when (`Armed::fired`). The engine claims its candidate in `finish`: the snapshot
//! of a solved candidate is registered under the identity lock as part of the accepted claim (§9.2), and the `Final`
//! delivered is the one logged. When the watchdog won, during the solve or while the candidate was prepared, the
//! candidate is discarded, registers nothing (and, won during the solve, no candidate or analytic fallback is even
//! built), and the decision log records the watchdog's `Final` with its delivery time. Until the fire the watchdog's
//! fallback is refreshed as the request learns more, so a watchdog `Final` keeps the range source's reasons and the
//! assumptions known by then (ruling 28-I6; spec 6). Nothing but an `Equity` of the request follows its delivered
//! `Final` (ruling 28-N1; spec 7, spec 4.4): the request's `Fast` carries its claim, and so does the solve client's
//! `Progress` (`SolvePlan::final_claim`, re-review observation O5), and `deliver` drops either, under the sink lock,
//! once the watchdog has delivered.
//!
//! Street verdict (ruling 20-I1). A request has one `StreetDeadline`, shared by its watchdog (`Armed::street_deadline`)
//! and the solve client (`SolvePlan::street_deadline`), which publishes the first attempt's terminal arrival to it at
//! receipt. The logged verdict is the client's (`SolveOutcome::street_violation`), judged from that arrival and never
//! from when processing finished; a `best_so_far` (the street deadline reached) is logged as a violation too, except on
//! the flop, whose single-raised-pot miss is the designed outcome (§12; plan 4). A request that ends before any solve
//! logs no verdict when answered before the fire, and the shared street deadline's once it expired (ruling 28-O3).
//!
//! After the `Final` (§7, §12, rulings 23-I1 and 28-I3). At the watchdog's fire the client stops and cleans nothing up,
//! and reports whether it left a sent job that may still be running, or a failed link, behind
//! (`SolveOutcome::outstanding_job`). Only then is the worker killed and restarted, once, after the `Final` is out,
//! whoever delivered it; an idle worker (nothing sent, the job's terminal received, the worker's own `no_iteration`) is
//! left alone. The link's `restart` revalidates `ready`; a restart that fails is reported on stderr, is never a panic and
//! is never retried here: it leaves no live worker, which the next request relaunches once (`run_solve`).
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
//! each locked for one step, released before any emission or solve.

use crate::allin::{facing_allin, AllInAnswer, AllInInput};
use crate::assemble::{self, AssemblyCtx};
use crate::clock::Clock;
use crate::core::EngineCore;
use crate::coverage::{classify, seat_index, Classification};
use crate::deadline::Deadlines;
use crate::equity::{equity_summary_with_clock, pending_summary, EQUITY_BUDGET_MS};
use crate::identity::IdentityState;
use crate::log::{DecisionRecord, InputRecord};
use crate::snapshots::{SnapshotKey, SnapshotProvenance, StreetSnapshot};
use crate::solve::{run_solve, SolvePlan, Terminal};
use crate::tree::{build_tree_full, tree_signature, TemplateSelection, Templates};
use crate::watchdog::{Armed, Fired, SharedSink, StreetDeadline};
use core_model::derive;
use core_ranges::hash_scaled;
use proto::worker::SOLVER_COMMIT;
use proto::{
    combo_index, ApproxReason, Assumptions, Card, Coverage, DecisionIdentity, Derived, EquitySummary, HandState, LegalAction, Range1326, Recommendation, RecommendationEvent, Seat,
    SolveInput, Street, UnsupportedReason,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One live request: the decision it answers, the hand state at that decision, its monotonic admission time `t0` on
/// the engine's clock (§5 step 4, §7) and the sink its events go to.
pub struct LiveRequest { pub identity: DecisionIdentity, pub state: HandState, pub t0_ms: u64, pub sink: SharedSink }

/// The decision an event answers.
fn event_identity(ev: &RecommendationEvent) -> &DecisionIdentity {
    match ev {
        RecommendationEvent::Fast(r) | RecommendationEvent::Provisional(r) | RecommendationEvent::Final(r) => &r.identity,
        RecommendationEvent::Equity { identity, .. } | RecommendationEvent::Progress { identity, .. } | RecommendationEvent::NoDecision { identity, .. } => identity,
    }
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
/// lock once the event is accepted (the snapshot registration of an accepted `Final`, §9.2). The lock is released on
/// return, before anything is handed to a sink.
fn accept(ids: &Mutex<IdentityState>, identity: &DecisionIdentity, final_claim: Option<&AtomicBool>, on_accept: impl FnOnce(&DecisionIdentity)) -> Delivery {
    let ids = ids.lock().unwrap();
    if !ids.is_active(identity) {
        return Delivery::Stale;
    }
    if final_claim.is_some_and(|d| d.swap(true, Ordering::SeqCst)) {
        return Delivery::AlreadyDelivered;
    }
    on_accept(ids.active().expect("the active decision was just checked"));
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
    let mut sink = sink.lock().unwrap();
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

/// The watchdog's `Unsupported{DeadlineExceeded}` fallback for this decision with what the request knows so far: the
/// reasons inherited to here and the assumptions (§7; spec 6, inherited reasons survive an `Unsupported` result). The
/// stage is the watchdog's to fill in at the fire.
fn deadline_fallback(ctx: &AssemblyCtx, inherited: &[ApproxReason], assumptions: &Assumptions) -> Recommendation {
    assemble::unsupported(ctx, UnsupportedReason::DeadlineExceeded { stage: String::new() }, inherited.to_vec(), assumptions.clone())
}

/// The fast-path equity routine (§3.4, §7): `equity::equity_summary_with_clock` in production. It polls `cancel` between
/// its units of work and stops once it is set (ruling 28-I4).
pub type EquityRoutine = Arc<dyn Fn(&dyn Clock, Option<[Card; 2]>, &Range1326, &[(Seat, Range1326)], &[Card], Duration, &AtomicBool) -> EquitySummary + Send + Sync>;

/// What `serve` runs that a test may replace (`serve_request_with`); production is `Hooks::production()`.
struct Hooks {
    equity: EquityRoutine,
    /// Runs on `engine-main` with a candidate `Final` assembled, immediately before its delivery is claimed.
    before_claim: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Runs on `engine-main` right after the `Fast` was handed over (the sink released), before the tree is built.
    after_fast: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Runs on `engine-main` once the solve has returned and its decision was found still active, before the watchdog's
    /// claim is consulted.
    after_active_check: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Hooks {
    fn production() -> Self { Self { equity: Arc::new(equity_summary_with_clock), before_claim: None, after_fast: None, after_active_check: None } }
}

/// Test seams of `serve_request` (plan 2 Task 28 fix round 1), compiled for this crate's tests and with the `testing`
/// feature only: `equity` replaces the fast-path equity routine (ruling 28-I4's acknowledged runner); `before_claim` runs
/// on `engine-main` with the candidate `Final` assembled, immediately before its delivery is claimed (ruling 28-I2: a
/// watchdog fire landing there); `after_fast` runs on `engine-main` right after the `Fast` was handed over, the sink
/// released, before the tree is built (ruling 28-O1b: a fire between the watchdog fallback's two refreshes);
/// `after_active_check` runs on `engine-main` once the solve has returned and its decision was found still active,
/// between that identity read and the read of the watchdog's claim (follow-up P2.W3, re-review observation O4: a
/// supersession and a fire landing between the two). `None` keeps production behaviour.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Default)]
pub struct ServeSeams {
    pub equity: Option<EquityRoutine>,
    pub before_claim: Option<Arc<dyn Fn() + Send + Sync>>,
    pub after_fast: Option<Arc<dyn Fn() + Send + Sync>>,
    pub after_active_check: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// `serve_request` with test seams (see `ServeSeams`).
#[cfg(any(test, feature = "testing"))]
pub fn serve_request_with(core: &mut EngineCore, req: LiveRequest, seams: ServeSeams) {
    let production = Hooks::production();
    serve(core, req, &Hooks { equity: seams.equity.unwrap_or(production.equity), before_claim: seams.before_claim, after_fast: seams.after_fast,
        after_active_check: seams.after_active_check });
}

/// §5 steps 4-10 for one request (see the module doc). `req.state` must replay (`core_model::derive`'s precondition)
/// and `req.t0_ms` must not lie ahead of the engine's clock.
pub fn serve_request(core: &mut EngineCore, req: LiveRequest) {
    serve(core, req, &Hooks::production());
}

fn serve(core: &mut EngineCore, req: LiveRequest, hooks: &Hooks) {
    // Ruling 28-I4: this request's equity cancellation token. A newer request supersedes the one served before it (§5
    // step 4), whose equity is cancelled before anything of this one starts.
    let equity_cancel = Arc::new(AtomicBool::new(false));
    let older = core.equity_cancel.lock().unwrap().replace(equity_cancel.clone());
    if let Some(older) = older {
        older.store(true, Ordering::SeqCst);
    }
    let d = derive(&req.state);
    let class = classify(&req.state);
    // One snapshot of the session config per request: a `set_config` that lands mid-request is ignored here and takes
    // effect from the next hand (§4.2).
    let config = core.config();
    let deadlines = Deadlines::for_request(req.t0_ms, d.street, config.solver.flop_budget_s);
    let hero_combo = req.state.hero_cards.map(|h| combo_index(h[0], h[1]));
    let mut ctx = AssemblyCtx { identity: req.identity.clone(), legal: d.legal.clone(), hero_combo, bb_chips: req.state.config.bb_chips, equity: pending_summary(&[]) };
    let mut assumptions = assemble::empty_assumptions("");
    assumptions.target_bp = config.solver.target_bp;
    // Spec 12 (ruling 29-I4): a worker whose `ready` was refused at startup (a degraded engine, `WorkerLink::refused`)
    // answers every decision with the non-retryable version mismatch, before anything else runs and without launching
    // the refused build. A request at a point that is no decision still answers `NoDecision` (§5 step 4).
    let refused = core.worker.refused().map(|refusal| format!("worker/proto version mismatch: {refusal}"));
    if let (Some(message), false) = (refused, matches!(class, Classification::NoDecision { .. })) {
        settle(core, &req, hooks, &deadlines, &equity_cancel, d.street, assemble::unsupported(&ctx, engine_error(&message), vec![], assumptions));
        return;
    }
    let (root, inherited, facing_allin_flag, opponent) = match class {
        Classification::NoDecision { reason } => {
            emit(core, &req, None, RecommendationEvent::NoDecision { identity: req.identity.clone(), reason });
            return;
        }
        // PLAN 3 HOOK: the preflop store lookup replaces this arm.
        Classification::Preflop => {
            let rec = assemble::unsupported(&ctx, engine_error("no preflop path in this build (plan 3)"), vec![], assumptions);
            settle(core, &req, hooks, &deadlines, &equity_cancel, d.street, rec);
            return;
        }
        Classification::Multiway { pot_eligible } => {
            settle(core, &req, hooks, &deadlines, &equity_cancel, d.street, assemble::unsupported(&ctx, UnsupportedReason::MultiwayEv { pot_eligible }, vec![], assumptions));
            return;
        }
        Classification::Unsupported(reason) => {
            settle(core, &req, hooks, &deadlines, &equity_cancel, d.street, assemble::unsupported(&ctx, reason, vec![], assumptions));
            return;
        }
        Classification::HuStreet { root, reasons, facing_allin, opponent } => (root, reasons, facing_allin, opponent),
    };
    // PLAN 4 HOOK: the flop path (cache lookup, pre-solver templates, flop budget) replaces this guard.
    if root.street == Street::Flop {
        settle(core, &req, hooks, &deadlines, &equity_cancel, root.street, assemble::unsupported(&ctx, engine_error("no flop path in this build (plan 4)"), inherited, assumptions));
        return;
    }
    assert!(matches!(root.street, Street::Turn | Street::River), "a heads-up street root on {:?}", root.street);
    ctx.equity = pending_summary(&[opponent]);

    // §7: the request's one street deadline, shared by its watchdog and the solve client (ruling 20-I1); its once-only
    // `Final` claim and the record of what the watchdog delivered when it won it (ruling 28-I2); the fallback the
    // watchdog delivers, refreshed as the request learns more (ruling 28-I6).
    let street_deadline = Arc::new(StreetDeadline::new(deadlines.street_deadline_ms));
    let (delivered, fired): (Arc<AtomicBool>, Arc<Mutex<Option<Fired>>>) = (Arc::default(), Arc::default());
    core.reset_stage("fast");
    let fallback = Arc::new(Mutex::new(deadline_fallback(&ctx, &inherited, &assumptions)));
    core.watchdog.arm(Armed { identity: req.identity.clone(), street_deadline: street_deadline.clone(), fire_ms: deadlines.watchdog_fire_ms(),
        retained: Arc::new(Mutex::new(None)), fallback: fallback.clone(), stage: core.stage.clone(), sink: req.sink.clone(), delivered: delivered.clone(),
        fired: fired.clone(), identity_state: core.identity.clone() });
    let claim = Claim { delivered: &delivered, fired: Some(&fired), equity_cancel: &equity_cancel };

    // Fast phase (§5 step 5). The range source's lock is released at the end of this statement, before any emission.
    let ranges = core.range_source.lock().unwrap().ranges_at_root(&req.state, &root);
    let ranges = match ranges {
        Ok(r) => r,
        Err(reason) => {
            let logged = Logged::unsolved(root.street, Some(street_deadline.clone()), vec![]);
            finish(core, &req, hooks, &deadlines, &claim, Candidate::unsolved(assemble::unsupported(&ctx, reason, inherited, assumptions)), logged);
            return;
        }
    };
    // The public ranges handed to the solve, OOP then IP as in `ranges_used`; hero's cards are in neither (CLAUDE.md 6).
    let range_hashes = vec![hex::encode(hash_scaled(&ranges.oop)), hex::encode(hash_scaled(&ranges.ip))];
    let inherited: Vec<ApproxReason> = inherited.into_iter().chain(ranges.reasons.iter().cloned()).collect();
    assumptions.ranges_used = ranges.ranges_used.clone();
    // Ruling 28-I6: from here on a watchdog `Final` keeps the range source's reasons and the ranges used (spec 6).
    *fallback.lock().unwrap() = deadline_fallback(&ctx, &inherited, &assumptions);
    emit(core, &req, Some(&*delivered), RecommendationEvent::Fast(assemble::fast(&ctx, assemble::accumulate(Coverage::Exact, inherited.clone()), assumptions.clone())));
    if let Some(after_fast) = &hooks.after_fast {
        after_fast();
    }
    let hero_is_oop = root.oop == req.state.hero;
    let (hero_public, opp_public) = if hero_is_oop { (&ranges.oop, &ranges.ip) } else { (&ranges.ip, &ranges.oop) };
    spawn_equity(core, &req, hero_public.clone(), (opponent, opp_public.clone()), root.board.clone(), hooks.equity.clone(), equity_cancel.clone());

    // Tree and solve (§5 step 7): river and turn are rooted at the street root; the turn cache arrives in plan 4.
    let template = if root.street == Street::River { "river_std_v1" } else { "turn_std_v1" };
    let build = match build_tree_full(&root, &TemplateSelection::from_history(template, &root.history)) {
        Ok(b) => b,
        Err(reason) => {
            let logged = Logged::unsolved(root.street, Some(street_deadline.clone()), range_hashes);
            finish(core, &req, hooks, &deadlines, &claim, Candidate::unsolved(assemble::unsupported(&ctx, reason, inherited, assumptions)), logged);
            return;
        }
    };
    assumptions.template_id = template.into();
    assumptions.tree_signature = tree_signature(&build.tree, build.pot);
    *fallback.lock().unwrap() = deadline_fallback(&ctx, &inherited, &assumptions);
    let hero_actor = if hero_is_oop { "oop" } else { "ip" };
    let input = SolveInput { root: root.clone(), ranges: [ranges.oop.clone(), ranges.ip.clone()], tree: build.tree.clone(), target_bp: config.solver.target_bp };
    // `background: false` is this plan's value for a live decision; plan 4's pre-solver builds the same plan with `true`.
    let plan = SolvePlan { identity: req.identity.clone(), deadlines, street_deadline: street_deadline.clone(), template_id: template.into(),
        retry_template_id: Templates::min_variant(template).map(String::from), rake: req.state.config.rake, hero_actor: hero_actor.into(), background: false,
        final_claim: Some(delivered.clone()) };
    let out = run_solve(core, &input, &plan, &req.sink);
    let returned_ms = core.clock.now_ms();
    // The client published what it returned: the first attempt's terminal arrival, which both verdicts read.
    assert!(street_deadline.terminal_arrival_ms() == out.first_terminal_ms, "the shared street deadline holds the first terminal at {:?} ms, the solve returned {:?} ms",
        street_deadline.terminal_arrival_ms(), out.first_terminal_ms);
    // Ruling 28-I3: the client's liveness provenance, not the clock, says whether a job was left running at the fire.
    let restart_after_final = out.outstanding_job;

    let logged = Logged { street: root.street, verdict: StreetVerdict::Judged(out.street_violation), range_hashes };
    let active = core.identity_active(&req.identity);
    if let (true, Some(after_active_check)) = (active, &hooks.after_active_check) {
        after_active_check();
    }
    if !active {
        // Superseded while it ran (§4.4): no candidate `Final` and no snapshot for a decision that is no longer active,
        // and its equity is cancelled (ruling 28-I4); a `Final` its watchdog delivered while it was still active is
        // logged (ruling W3-I1), and nothing else is.
        retire_stale(core, &req, &deadlines, &claim, &logged);
    } else if delivered.load(Ordering::SeqCst) {
        // The watchdog won the delivery during the solve (ruling 28-I2): nothing of a candidate is built, no analytic
        // fallback runs, and the `Final` it delivered is the one logged.
        core.watchdog.disarm();
        let fired = watchdog_final(&fired);
        log_final(core, &req, &deadlines, Delivered { at_ms: fired.at_ms, rec: &fired.rec, by_watchdog: true, best_so_far: false }, &logged);
    } else {
        assumptions.elapsed_ms = elapsed_ms(req.t0_ms, returned_ms);
        assumptions.reached_bp = out.reached_bp;
        assumptions.source = format!("solver-worker@{SOLVER_COMMIT}");
        // §4.4 vocabulary `"unverified" | "exploitability <= x"`, in bp (review m14) and a true bound (ruling 28-I5).
        assumptions.source_accuracy = out.solution.as_ref()
            .map_or_else(|| "unverified".to_string(), |sol| format!("exploitability <= {} bp", accuracy_bound_bp(sol.exploitability_chips, build.pot)));
        let best_so_far = out.terminal == Terminal::BestSoFar;
        let candidate = match &out.terminal {
            Terminal::Ok | Terminal::BestSoFar => {
                let sol = out.solution.as_ref().expect("run_solve: an Ok or BestSoFar outcome carries its validated solution");
                // The template actually solved (the `_min` retry's when it answered) and its tree.
                assumptions.template_id = out.template_used.clone();
                assumptions.tree_signature = tree_signature(&out.tree, build.pot);
                let coverage = assemble::coverage_for_solve(sol.exploitability_chips, build.pot, config.solver.target_bp, best_so_far, inherited.clone());
                let requested = sol.requested as usize;
                let reach = assemble::hero_reach(&sol.nodes, &out.ordinal_paths, requested, hero_public, hero_actor);
                // Registered only as part of this `Final`'s accepted delivery (§9.2, ruling 28-I2). Keyed by the public
                // root ranges solved (hero's cards are in neither) and the solved tree's signature; the solved prefix is
                // the street's observed history at the root; the covered paths are the ordinal paths the solve client
                // resolved from the wire chip paths (§2). Built before `assumptions` moves into the `Final`.
                let snapshot = StreetSnapshot {
                    key: SnapshotKey { hand_id: req.identity.hand_id, config_revision: req.identity.config_revision, model_revision: req.identity.model_revision,
                        street: root.street, root_board: root.board.clone(), root_range_hashes: [hash_scaled(&ranges.oop), hash_scaled(&ranges.ip)],
                        tree_signature: assumptions.tree_signature.clone() },
                    provenance: SnapshotProvenance { identity_at_solve: req.identity.clone(), solved_prefix: root.history.clone(), origin: "live".into() },
                    tree: out.tree.clone(), nodes: sol.nodes.clone(), covered_paths: out.ordinal_paths.clone(), exploitability_chips: sol.exploitability_chips,
                    reasons: inherited.clone() };
                let rec = assemble::final_from_solution(&ctx, &sol.nodes[requested], &reach, coverage, assumptions);
                Candidate { rec, snapshot: Some(snapshot), best_so_far }
            }
            Terminal::Failed(reason) => {
                // §5 step 7 / §6: facing an all-in with the worker failing, the analytic fallback answers.
                let worker_failed = matches!(reason, UnsupportedReason::EngineError { .. } | UnsupportedReason::DeadlineExceeded { .. });
                let analytic = if facing_allin_flag && worker_failed { analytic_allin(&req, &d, hero_public, opp_public) } else { None };
                Candidate::unsolved(match analytic {
                    Some(a) => {
                        let mut rec = assemble::unsupported(&ctx, reason.clone(), vec![], assumptions);
                        rec.coverage = assemble::accumulate(Coverage::Approximate { reasons: vec![ApproxReason::UnconditionedCurrentStreet] }, inherited.clone());
                        rec.actions = a.actions;
                        rec.assumptions.notes.push(format!("analytic all-in fallback: equity {:.4}, W {}, R {:.2}, EV(call) {:.2} chips; headline: highest EV",
                            a.equity, a.w, a.r, a.ev_call_chips));
                        rec
                    }
                    None => assemble::unsupported(&ctx, reason.clone(), inherited.clone(), assumptions),
                })
            }
        };
        finish(core, &req, hooks, &deadlines, &claim, candidate, logged);
    }
    if restart_after_final {
        restart_the_worker(core);
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
fn spawn_equity(core: &EngineCore, req: &LiveRequest, hero_public: Range1326, opponent: (Seat, Range1326), board: Vec<Card>, routine: EquityRoutine,
    cancel: Arc<AtomicBool>) {
    let (ids, identity, sink, clock, hero) = (core.identity.clone(), req.identity.clone(), req.sink.clone(), core.clock.clone(), req.state.hero_cards);
    // Owned by the core (ruling 29-I2): joined at the engine's teardown, never detached.
    core.tasks.spawn("fast-path", move || {
        let stale = !ids.lock().unwrap().is_active(&identity);
        if stale {
            cancel.store(true, Ordering::SeqCst);
            return;
        }
        let equity = routine(clock.as_ref(), hero, &hero_public, &[opponent], &board, Duration::from_millis(EQUITY_BUDGET_MS), &cancel);
        deliver(&ids, &identity, &sink, None, RecommendationEvent::Equity { identity: identity.clone(), equity });
    });
}

/// §6's analytic fallback facing an all-in, for hero's actual combo against the opponent's public range. `C` is what
/// calling costs hero (`Derived.legal`); the wager hero faces is what hero owes, `facing - committed_this_street[hero]`
/// (chips hero already put in this street are in `Derived.pot`, not faced); the pot is `Derived.pot`. `None` when
/// there is no call to price or no equity came back.
fn analytic_allin(req: &LiveRequest, d: &Derived, hero_public: &Range1326, opp_public: &Range1326) -> Option<AllInAnswer> {
    let hero = req.state.hero_cards?;
    let call_cost = d.legal.iter().find_map(|l| match l { LegalAction::Call { cost } => Some(*cost), _ => None })?;
    let committed = d.committed_this_street[seat_index(&req.state, req.state.hero)];
    let owed = d.facing.checked_sub(committed).unwrap_or_else(|| panic!("hero committed {committed} chips this street, above the {} chips faced", d.facing));
    let input = AllInInput { hero, board: req.state.board.clone(), opp_public: opp_public.clone(), hero_public: Some(hero_public.clone()), call_cost,
        pot: d.pot, facing: owed, rake: req.state.config.rake, bb_chips: req.state.config.bb_chips };
    facing_allin(&input, Duration::from_millis(EQUITY_BUDGET_MS), &AtomicBool::new(false)).ok()
}

/// A request's side of its one `Final` (§7): the claim it shares with its watchdog, what the watchdog delivered when it
/// won the claim (`None` for a request no watchdog watches), and the request's equity cancellation token.
struct Claim<'a> {
    delivered: &'a AtomicBool,
    fired: Option<&'a Mutex<Option<Fired>>>,
    equity_cancel: &'a AtomicBool,
}

/// A `Final` the engine prepared, the snapshot its accepted delivery registers (§9.2) and whether it is a
/// `best_so_far` (§12's violation rule).
struct Candidate {
    rec: Recommendation,
    snapshot: Option<StreetSnapshot>,
    best_so_far: bool,
}

impl Candidate {
    /// A `Final` with no solution behind it.
    fn unsolved(rec: Recommendation) -> Self { Self { rec, snapshot: None, best_so_far: false } }
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
    /// street deadline (ruling 28-N3). `None` when no watchdog watches the request (the classifier's rows).
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
    /// A request answered before any solve, watched by the watchdog of `street_deadline` when given.
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
    fired.lock().unwrap().clone().expect("the watchdog claimed the Final, and its fire records what it delivered before emitting it")
}

/// A `Final` the classifier settles alone, before anything is armed: its claim is its own.
fn settle(core: &mut EngineCore, req: &LiveRequest, hooks: &Hooks, deadlines: &Deadlines, equity_cancel: &AtomicBool, street: Street, rec: Recommendation) {
    let claim = Claim { delivered: &AtomicBool::new(false), fired: None, equity_cancel };
    finish(core, req, hooks, deadlines, &claim, Candidate::unsolved(rec), Logged::unsolved(street, None, vec![]));
}

/// The one `Final` path (§5 steps 7, 9, 10; ruling 28-I2). The candidate's delivery is claimed under the identity lock:
/// a decision no longer active gets no `Final` of the engine's and its candidate registers nothing (its equity is
/// cancelled), and the log records only a `Final` its watchdog delivered while it was still active (`retire_stale`,
/// ruling W3-I1); a claim the watchdog already won discards the candidate, which registers nothing, and logs the
/// watchdog's `Final` with its delivery time; a claim won registers the candidate's snapshot under the same lock, as
/// part of that accepted delivery, hands the `Final` to the sink with no engine lock held, and logs it. The watchdog is
/// retired in every case.
fn finish(core: &mut EngineCore, req: &LiveRequest, hooks: &Hooks, deadlines: &Deadlines, claim: &Claim<'_>, candidate: Candidate, logged: Logged) {
    if let Some(before_claim) = &hooks.before_claim {
        before_claim();
    }
    let at_ms = core.clock.now_ms();
    let Candidate { rec, snapshot, best_so_far } = candidate;
    let snapshots = &core.snapshots;
    let verdict = accept(&core.identity, &req.identity, Some(claim.delivered), |active| {
        if let Some(snapshot) = snapshot {
            snapshots.lock().unwrap().register(active, snapshot);
        }
    });
    match verdict {
        Delivery::Stale => retire_stale(core, req, deadlines, claim, &logged),
        Delivery::AlreadyDelivered => {
            core.watchdog.disarm();
            let fired = watchdog_final(claim.fired.expect("only a watchdog shares a request's Final claim"));
            log_final(core, req, deadlines, Delivered { at_ms: fired.at_ms, rec: &fired.rec, by_watchdog: true, best_so_far: false }, &logged);
        }
        Delivery::Accepted => {
            req.sink.lock().unwrap().emit(RecommendationEvent::Final(rec.clone()));
            core.watchdog.disarm();
            log_final(core, req, deadlines, Delivered { at_ms, rec: &rec, by_watchdog: false, best_so_far }, &logged);
        }
    }
}

/// A stale exit (§4.4, §12; ruling W3-I1): the request's decision is no longer active. Its equity is cancelled (ruling
/// 28-I4) and its watchdog retired first, so a fire in progress has completed its emission and has recorded what it
/// delivered (`disarm` returns only after such a fire, which records before it emits). A `Final` the watchdog claimed
/// while the decision was still active was delivered, and it is the request's `Final`: it is logged, exactly once, as
/// the watchdog's, with its recorded delivery time and the request's deadline verdicts (spec 5 step 10, every request
/// and its `Final`; spec 7 and 12, the deadline outcomes). A decision superseded before any claim leaves the slot empty
/// (the fire of a decision no longer active records nothing, `watchdog`'s "Identity at the fire"): no `Final`, no
/// record. Nothing of a candidate is accepted or registered here.
fn retire_stale(core: &mut EngineCore, req: &LiveRequest, deadlines: &Deadlines, claim: &Claim<'_>, logged: &Logged) {
    claim.equity_cancel.store(true, Ordering::SeqCst);
    core.watchdog.disarm();
    let delivered_by_watchdog = claim.fired.and_then(|fired| fired.lock().unwrap().clone());
    if let Some(fired) = delivered_by_watchdog {
        assert!(claim.delivered.load(Ordering::SeqCst), "the watchdog recorded a Final of decision {:?} without its claim", req.identity);
        log_final(core, req, deadlines, Delivered { at_ms: fired.at_ms, rec: &fired.rec, by_watchdog: true, best_so_far: false }, logged);
    }
}

/// Logs the `Final` delivered (§5 step 10): its coverage, reasons, template and reached exploitability, the time it was
/// delivered at, and the deadline verdicts. A `Final` the watchdog delivered is a final-delivery violation by
/// construction (the engine's own did not come in time).
fn log_final(core: &mut EngineCore, req: &LiveRequest, deadlines: &Deadlines, delivered: Delivered<'_>, logged: &Logged) {
    let rec = delivered.rec;
    let reasons = match &rec.coverage {
        Coverage::Exact => vec![],
        Coverage::Approximate { reasons } => reasons.clone(),
        Coverage::Unsupported { partial, .. } => partial.clone(),
    };
    let final_violation = delivered.by_watchdog || delivered.at_ms >= deadlines.watchdog_fire_ms();
    let arrival_violation = match &logged.verdict {
        StreetVerdict::Judged(violated) => *violated,
        StreetVerdict::Unattempted(street_deadline) => final_violation && street_deadline.as_ref().is_some_and(|d| d.violated() || delivered.at_ms >= d.deadline_ms()),
    };
    // §12 "Street deadline reached": a `best_so_far` is logged as a violation on every street but the flop, whose
    // single-raised-pot miss is the designed outcome (plan 4 refines the flop by pot type).
    let best_so_far = delivered.best_so_far;
    let street_violation = if logged.street == Street::Flop { arrival_violation && !best_so_far } else { arrival_violation || best_so_far };
    core.log.append(&DecisionRecord { identity: req.identity.clone(), street: logged.street, coverage: rec.coverage.clone(), reasons,
        elapsed_ms: elapsed_ms(req.t0_ms, delivered.at_ms), cache: "miss".into(), presolver_scenario: None, tier: None, reached_bp: rec.assumptions.reached_bp,
        street_violation, final_violation, template_id: rec.assumptions.template_id.clone(), input: InputRecord::from_state(&req.state, logged.range_hashes.clone()) });
}

/// §7 after a `Final` at the watchdog's fire: kill and restart the worker (see "After the `Final`" above).
fn restart_the_worker(core: &mut EngineCore) {
    core.worker.kill();
    if let Err(e) = core.worker.restart() {
        eprintln!("engine: restarting the worker after a DeadlineExceeded final failed: {e}; no live worker is left, and the next request relaunches it once");
    }
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
fn elapsed_ms(t0_ms: u64, now_ms: u64) -> u32 {
    let span = now_ms.checked_sub(t0_ms).unwrap_or_else(|| panic!("engine clock reading {now_ms} ms precedes the request's admission at t0 {t0_ms} ms"));
    u32::try_from(span).unwrap_or_else(|_| panic!("a request span of {span} ms does not fit u32"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::DecisionLog;
    use crate::ranges::ExplicitRanges;
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
        serve_request(&mut core, LiveRequest { identity: id.clone(), state: s.clone(), t0_ms: clock.now_ms(), sink: Arc::new(Mutex::new(Box::new(sink))) });
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
}
