//! Spec §5 steps 4-10 for one live request on `engine-main` (Task 28): the river and turn decision path. The preflop
//! and flop paths attach at the `Classification::Preflop` arm and at the flop guard below (plans 3 and 4).
//!
//! One request. `serve_request` classifies the decision (§6) and answers at once every row the classifier settles
//! alone. For a heads-up river or turn decision it arms the watchdog (§7), reads the public root ranges from the range
//! source (§9: the only provider, which owns their validation, rulings 27-D3/D4), emits `Fast` (§5 step 5), starts the
//! `fast-path` equity thread (§3.4), solves through the solve client (`solve::run_solve`, Tasks 22-23), assembles the
//! `Final` (§4.4, §5 step 7), registers a validated solution as a snapshot (§9.2) and logs the decision (§5 step 10).
//!
//! Identity (ruling 28-I1). Every event goes through `deliver` (`emit` for the crate): it is accepted under the
//! identity lock, where a decision no longer active is refused and a `Final` claims the request's once-only delivery,
//! and an accepted event is handed to the sink only after that lock is released, so this path holds no engine lock
//! during a sink callback and a sink may re-enter the engine (read the identity, mutate the hand); the watchdog's fire
//! still emits under its generation lock (follow-up P2.W3). A mutation that lands between
//! the acceptance and the callback lets that one event through; the UI refuses it by identity (§5 step 9). A request
//! whose decision is no longer active, at admission or once its solve returns, ends with no `Final`, no snapshot and no
//! log record, and its watchdog is retired.
//!
//! One `Final` (§7, ruling 28-I2). The engine's `Final` and the watchdog's race for one claim (`Armed::delivered`); the
//! fire records what it delivered and when (`Armed::fired`). The engine claims its candidate in `finish`: the snapshot
//! of a solved candidate is registered under the identity lock as part of the accepted claim (§9.2), and the `Final`
//! delivered is the one logged. When the watchdog won, during the solve or while the candidate was prepared, the
//! candidate is discarded, registers nothing (and, won during the solve, no candidate or analytic fallback is even
//! built), and the decision log records the watchdog's `Final` with its delivery time. Until the fire the watchdog's
//! fallback is refreshed as the request learns more, so a watchdog `Final` keeps the range source's reasons and the
//! assumptions known by then (ruling 28-I6; spec 6). Nothing but an `Equity` of the request follows its delivered
//! `Final` (ruling 28-N1; spec 7, spec 4.4): the request's `Fast` carries its claim, and `deliver` drops it, under the
//! sink lock, once the watchdog has delivered.
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
//! (`EngineCore::equity_cancel` holds the last request's token), when this request finds its decision no longer active,
//! and by the fast path itself when the decision is already stale as it starts; never by the request's own `Final`,
//! since a late `Equity` of the active decision still enriches it.
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
use crate::snapshots::SolvedStreet;
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
enum Delivery {
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
/// dropped. `Equity` and `NoDecision` never take it.
fn deliver(ids: &Mutex<IdentityState>, identity: &DecisionIdentity, sink: &SharedSink, delivered: Option<&AtomicBool>, ev: RecommendationEvent) -> Delivery {
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
}

impl Hooks {
    fn production() -> Self { Self { equity: Arc::new(equity_summary_with_clock), before_claim: None } }
}

/// Test seams of `serve_request` (plan 2 Task 28 fix round 1), compiled for this crate's tests and with the `testing`
/// feature only: `equity` replaces the fast-path equity routine (ruling 28-I4's acknowledged runner); `before_claim` runs
/// on `engine-main` with the candidate `Final` assembled, immediately before its delivery is claimed (ruling 28-I2: a
/// watchdog fire landing there). `None` keeps production behaviour.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Default)]
pub struct ServeSeams {
    pub equity: Option<EquityRoutine>,
    pub before_claim: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// `serve_request` with test seams (see `ServeSeams`).
#[cfg(any(test, feature = "testing"))]
pub fn serve_request_with(core: &mut EngineCore, req: LiveRequest, seams: ServeSeams) {
    let production = Hooks::production();
    serve(core, req, &Hooks { equity: seams.equity.unwrap_or(production.equity), before_claim: seams.before_claim });
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
        fired: fired.clone() });
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
        retry_template_id: Templates::min_variant(template).map(String::from), rake: req.state.config.rake, hero_actor: hero_actor.into(), background: false };
    let out = run_solve(core, &input, &plan, &req.sink);
    let returned_ms = core.clock.now_ms();
    // The client published what it returned: the first attempt's terminal arrival, which both verdicts read.
    assert!(street_deadline.terminal_arrival_ms() == out.first_terminal_ms, "the shared street deadline holds the first terminal at {:?} ms, the solve returned {:?} ms",
        street_deadline.terminal_arrival_ms(), out.first_terminal_ms);
    // Ruling 28-I3: the client's liveness provenance, not the clock, says whether a job was left running at the fire.
    let restart_after_final = out.outstanding_job;

    let logged = Logged { street: root.street, verdict: StreetVerdict::Judged(out.street_violation), range_hashes };
    if !core.identity_active(&req.identity) {
        // Superseded while it ran (§4.4): no `Final`, snapshot or log record for a decision that is no longer active,
        // and its equity is cancelled (ruling 28-I4).
        equity_cancel.store(true, Ordering::SeqCst);
        core.watchdog.disarm();
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
                let rec = assemble::final_from_solution(&ctx, &sol.nodes[requested], &reach, coverage, assumptions);
                // Registered only as part of this `Final`'s accepted delivery (§9.2, ruling 28-I2).
                let snapshot = SolvedStreet { identity_at_solve: req.identity.clone(), street: root.street, board: root.board.clone(), tree: out.tree.clone(),
                    nodes: sol.nodes.clone(), ordinal_paths: out.ordinal_paths.clone(), exploitability_chips: sol.exploitability_chips, reasons: inherited.clone(),
                    solved_prefix: root.history.clone() };
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
    std::thread::Builder::new()
        .name("fast-path".into())
        .spawn(move || {
            let stale = !ids.lock().unwrap().is_active(&identity);
            if stale {
                cancel.store(true, Ordering::SeqCst);
                return;
            }
            let equity = routine(clock.as_ref(), hero, &hero_public, &[opponent], &board, Duration::from_millis(EQUITY_BUDGET_MS), &cancel);
            deliver(&ids, &identity, &sink, None, RecommendationEvent::Equity { identity: identity.clone(), equity });
        })
        .expect("spawn the fast-path thread");
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
    snapshot: Option<SolvedStreet>,
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
    /// after it (the request expired) carries the shared street deadline's (`StreetDeadline::violated`, ruling 28-O3).
    /// `None` when no watchdog watches the request (the classifier's rows).
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
/// a decision no longer active has no `Final` and no log record (and its equity is cancelled); a claim the watchdog
/// already won discards the candidate, which registers nothing, and logs the watchdog's `Final` with its delivery
/// time; a claim won registers the candidate's snapshot under the same lock, as part of that accepted delivery, hands
/// the `Final` to the sink with no engine lock held, and logs it. The watchdog is retired in every case.
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
        Delivery::Stale => {
            claim.equity_cancel.store(true, Ordering::SeqCst);
            core.watchdog.disarm();
        }
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
        StreetVerdict::Unattempted(street_deadline) => final_violation && street_deadline.as_ref().is_some_and(|d| d.violated()),
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
