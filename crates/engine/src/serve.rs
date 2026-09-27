//! Spec §5 steps 4-10 for one live request on `engine-main` (Task 28): the river and turn decision path. The preflop
//! and flop paths attach at the `Classification::Preflop` arm and at the flop guard below (plans 3 and 4).
//!
//! One request. `serve_request` classifies the decision (§6) and answers at once every row the classifier settles
//! alone. For a heads-up river or turn decision it arms the watchdog (§7), reads the public root ranges from the range
//! source (§9: the only provider, which owns their validation, rulings 27-D3/D4), emits `Fast` (§5 step 5), starts the
//! `fast-path` equity thread (§3.4), solves through the solve client (`solve::run_solve`, Tasks 22-23), assembles the
//! `Final` (§4.4, §5 step 7), registers a validated solution as a snapshot (§9.2) and logs the decision (§5 step 10).
//!
//! Identity. Every event goes through `emit`, which delivers it only while the request's decision is the active one
//! and holds the identity lock through the delivery, so an invalidation is linearized with it: once `mutate`,
//! `begin_hand` or any other invalidation of `IdentityState` returns, this path emits nothing more for an earlier
//! decision (§4.4, §5 step 9, §12). A snapshot is registered under the identity lock, against the identity active
//! there, so a result of a decision already invalidated is never written (§9.2, §12). A request whose decision is no
//! longer active, at admission or once its solve returns, ends with no `Final`, no snapshot and no log record, and its
//! watchdog is retired.
//!
//! One `Final`. Every `Final` goes through `finish`: the engine's and the watchdog's share `delivered`, and whichever
//! swaps it first delivers (§7); then the watchdog is retired and the decision logged, whichever side delivered.
//!
//! Street verdict (ruling 20-I1). A request has one `StreetDeadline`, shared by its watchdog (`Armed::street_deadline`)
//! and the solve client (`SolvePlan::street_deadline`), which publishes the first attempt's terminal arrival to it at
//! receipt. The logged verdict is the client's (`SolveOutcome::street_violation`), judged from that arrival and never
//! from when processing finished; a `best_so_far` (the street deadline reached) is logged as a violation too, except on
//! the flop, whose single-raised-pot miss is the designed outcome (§12; plan 4).
//!
//! After the `Final` (§7, ruling 23-I1). At the watchdog's fire the client stops and cleans nothing up: the worker may
//! still be running the job, or its link may have failed. So when the solve ended `DeadlineExceeded` at or after the
//! fire, the worker is killed and restarted once the `Final` is out, whoever delivered it. A `DeadlineExceeded` before
//! the fire (the worker's own `no_iteration`, or no room left to send a request) leaves an idle worker, which nothing
//! restarts. The link's `restart` revalidates `ready`; a restart that fails is reported on stderr, is never a panic and
//! is never retried here: it leaves no live worker, which the next request relaunches once (`run_solve`).
//!
//! Locks. The identity lock is taken before the sink or the snapshot store and released after them, never the other
//! way round. The snapshot store, the config and the range source are each locked for one call, released before any
//! emission or solve.

use crate::allin::{facing_allin, AllInAnswer, AllInInput};
use crate::assemble::{self, AssemblyCtx};
use crate::core::EngineCore;
use crate::coverage::{classify, seat_index, Classification};
use crate::deadline::Deadlines;
use crate::equity::{equity_summary_with_clock, pending_summary, EQUITY_BUDGET_MS};
use crate::identity::IdentityState;
use crate::log::{DecisionRecord, InputRecord};
use crate::snapshots::SolvedStreet;
use crate::solve::{run_solve, SolvePlan, Terminal};
use crate::tree::{build_tree_full, tree_signature, TemplateSelection, Templates};
use crate::watchdog::{Armed, SharedSink, StreetDeadline};
use core_model::derive;
use core_ranges::hash_scaled;
use proto::worker::SOLVER_COMMIT;
use proto::{
    combo_index, ApproxReason, Card, Coverage, DecisionIdentity, Derived, HandState, LegalAction, Range1326, Recommendation, RecommendationEvent, Seat,
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

/// How a delivery ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    Emitted,
    /// The decision is no longer active: nothing was emitted.
    Stale,
    /// A `Final` whose request's `Final` was already delivered (by the watchdog): nothing was emitted.
    AlreadyDelivered,
}

/// Delivers `ev`, an event of decision `identity`, to `sink` only while that decision is active, holding the identity
/// lock through the delivery (see "Identity" above); a `Final` only if `delivered`, when given, was not yet set, and
/// setting it.
fn deliver(ids: &Mutex<IdentityState>, identity: &DecisionIdentity, sink: &SharedSink, delivered: Option<&AtomicBool>, ev: RecommendationEvent) -> Delivery {
    assert!(event_identity(&ev) == identity, "an event of decision {:?} emitted for decision {identity:?}", event_identity(&ev));
    let ids = ids.lock().unwrap();
    if !ids.is_active(identity) {
        return Delivery::Stale;
    }
    if let (Some(d), RecommendationEvent::Final(_)) = (delivered, &ev) {
        if d.swap(true, Ordering::SeqCst) {
            return Delivery::AlreadyDelivered;
        }
    }
    sink.lock().unwrap().emit(ev);
    drop(ids);
    Delivery::Emitted
}

/// Crate-visible: plan 3 Task 17's `preflop.rs` imports this same helper (`use crate::serve::emit;`) rather than
/// creating a second event-emission path, so the identity check stays in one place. `delivered`, when given, is the
/// request's once-only `Final` flag, shared with its watchdog.
pub(crate) fn emit(core: &EngineCore, req: &LiveRequest, delivered: Option<&AtomicBool>, ev: RecommendationEvent) {
    deliver(&core.identity, &req.identity, &req.sink, delivered, ev);
}

fn engine_error(message: &str) -> UnsupportedReason { UnsupportedReason::EngineError { message: message.into(), retryable: false } }

/// §5 steps 4-10 for one request (see the module doc). `req.state` must replay (`core_model::derive`'s precondition)
/// and `req.t0_ms` must not lie ahead of the engine's clock.
pub fn serve_request(core: &mut EngineCore, req: LiveRequest) {
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
            settle(core, &req, &deadlines, d.street, rec);
            return;
        }
        Classification::Multiway { pot_eligible } => {
            settle(core, &req, &deadlines, d.street, assemble::unsupported(&ctx, UnsupportedReason::MultiwayEv { pot_eligible }, vec![], assumptions));
            return;
        }
        Classification::Unsupported(reason) => {
            settle(core, &req, &deadlines, d.street, assemble::unsupported(&ctx, reason, vec![], assumptions));
            return;
        }
        Classification::HuStreet { root, reasons, facing_allin, opponent } => (root, reasons, facing_allin, opponent),
    };
    // PLAN 4 HOOK: the flop path (cache lookup, pre-solver templates, flop budget) replaces this guard.
    if root.street == Street::Flop {
        settle(core, &req, &deadlines, root.street, assemble::unsupported(&ctx, engine_error("no flop path in this build (plan 4)"), inherited, assumptions));
        return;
    }
    assert!(matches!(root.street, Street::Turn | Street::River), "a heads-up street root on {:?}", root.street);
    ctx.equity = pending_summary(&[opponent]);

    // §7: the request's one street deadline, shared by its watchdog and the solve client (ruling 20-I1), and its
    // once-only `Final` flag.
    let street_deadline = Arc::new(StreetDeadline::new(deadlines.street_deadline_ms));
    let delivered = Arc::new(AtomicBool::new(false));
    core.reset_stage("fast");
    let fallback = assemble::unsupported(&ctx, UnsupportedReason::DeadlineExceeded { stage: String::new() }, inherited.clone(), assumptions.clone());
    core.watchdog.arm(Armed { identity: req.identity.clone(), street_deadline: street_deadline.clone(), fire_ms: deadlines.watchdog_fire_ms(),
        retained: Arc::new(Mutex::new(None)), fallback, stage: core.stage.clone(), sink: req.sink.clone(), delivered: delivered.clone() });

    // Fast phase (§5 step 5). The range source's lock is released at the end of this statement, before any emission.
    let ranges = core.range_source.lock().unwrap().ranges_at_root(&req.state, &root);
    let ranges = match ranges {
        Ok(r) => r,
        Err(reason) => {
            finish(core, &req, &deadlines, &delivered, assemble::unsupported(&ctx, reason, inherited, assumptions), Logged::unsolved(root.street, vec![]));
            return;
        }
    };
    // The public ranges handed to the solve, OOP then IP as in `ranges_used`; hero's cards are in neither (CLAUDE.md 6).
    let range_hashes = vec![hex::encode(hash_scaled(&ranges.oop)), hex::encode(hash_scaled(&ranges.ip))];
    let inherited: Vec<ApproxReason> = inherited.into_iter().chain(ranges.reasons.iter().cloned()).collect();
    assumptions.ranges_used = ranges.ranges_used.clone();
    emit(core, &req, None, RecommendationEvent::Fast(assemble::fast(&ctx, assemble::accumulate(Coverage::Exact, inherited.clone()), assumptions.clone())));
    let hero_is_oop = root.oop == req.state.hero;
    let (hero_public, opp_public) = if hero_is_oop { (&ranges.oop, &ranges.ip) } else { (&ranges.ip, &ranges.oop) };
    spawn_equity(core, &req, hero_public.clone(), (opponent, opp_public.clone()), root.board.clone());

    // Tree and solve (§5 step 7): river and turn are rooted at the street root; the turn cache arrives in plan 4.
    let template = if root.street == Street::River { "river_std_v1" } else { "turn_std_v1" };
    let build = match build_tree_full(&root, &TemplateSelection::from_history(template, &root.history)) {
        Ok(b) => b,
        Err(reason) => {
            finish(core, &req, &deadlines, &delivered, assemble::unsupported(&ctx, reason, inherited, assumptions), Logged::unsolved(root.street, range_hashes));
            return;
        }
    };
    assumptions.template_id = template.into();
    assumptions.tree_signature = tree_signature(&build.tree, build.pot);
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
    let restart_after_final = matches!(out.terminal, Terminal::Failed(UnsupportedReason::DeadlineExceeded { .. })) && returned_ms >= deadlines.watchdog_fire_ms();

    if !core.identity_active(&req.identity) {
        // Superseded while it ran (§4.4): no `Final`, snapshot or log record for a decision that is no longer active.
        core.watchdog.disarm();
    } else {
        assumptions.elapsed_ms = elapsed_ms(req.t0_ms, returned_ms);
        assumptions.reached_bp = out.reached_bp;
        assumptions.source = format!("solver-worker@{SOLVER_COMMIT}");
        // §4.4 vocabulary `"unverified" | "exploitability <= x"`, displayed in bp (§7 compares raw chips; review m14).
        assumptions.source_accuracy = out.reached_bp.map_or_else(|| "unverified".to_string(), |bp| format!("exploitability <= {bp} bp"));
        let best_so_far = out.terminal == Terminal::BestSoFar;
        let rec = match &out.terminal {
            Terminal::Ok | Terminal::BestSoFar => {
                let sol = out.solution.as_ref().expect("run_solve: an Ok or BestSoFar outcome carries its validated solution");
                // The template actually solved (the `_min` retry's when it answered) and its tree.
                assumptions.template_id = out.template_used.clone();
                assumptions.tree_signature = tree_signature(&out.tree, build.pot);
                let coverage = assemble::coverage_for_solve(sol.exploitability_chips, build.pot, config.solver.target_bp, best_so_far, inherited.clone());
                let requested = sol.requested as usize;
                let reach = assemble::hero_reach(&sol.nodes, &out.ordinal_paths, requested, hero_public, hero_actor);
                let rec = assemble::final_from_solution(&ctx, &sol.nodes[requested], &reach, coverage, assumptions);
                register(core, SolvedStreet { identity_at_solve: req.identity.clone(), street: root.street, board: root.board.clone(), tree: out.tree.clone(),
                    nodes: sol.nodes.clone(), ordinal_paths: out.ordinal_paths.clone(), exploitability_chips: sol.exploitability_chips, reasons: inherited.clone(),
                    solved_prefix: root.history.clone() });
                rec
            }
            Terminal::Failed(reason) => {
                // §5 step 7 / §6: facing an all-in with the worker failing, the analytic fallback answers.
                let worker_failed = matches!(reason, UnsupportedReason::EngineError { .. } | UnsupportedReason::DeadlineExceeded { .. });
                let analytic = if facing_allin_flag && worker_failed { analytic_allin(&req, &d, hero_public, opp_public) } else { None };
                match analytic {
                    Some(a) => {
                        let mut rec = assemble::unsupported(&ctx, reason.clone(), vec![], assumptions);
                        rec.coverage = assemble::accumulate(Coverage::Approximate { reasons: vec![ApproxReason::UnconditionedCurrentStreet] }, inherited.clone());
                        rec.actions = a.actions;
                        rec.assumptions.notes.push(format!("analytic all-in fallback: equity {:.4}, W {}, R {:.2}, EV(call) {:.2} chips; headline: highest EV",
                            a.equity, a.w, a.r, a.ev_call_chips));
                        rec
                    }
                    None => assemble::unsupported(&ctx, reason.clone(), inherited.clone(), assumptions),
                }
            }
        };
        let logged = Logged { street: root.street, reached_bp: out.reached_bp, street_violation: out.street_violation, best_so_far, range_hashes };
        finish(core, &req, &deadlines, &delivered, rec, logged);
    }
    if restart_after_final {
        restart_the_worker(core);
    }
}

/// The `fast-path` thread (§3.4): §4.4's two equity populations against the one opponent, within the equity phase's
/// own budget on the engine's clock (§7), delivered as `Equity` through the same identity check as every event; the
/// UI merges it into whatever it displays, a `Final` included (§4.4). Hero's cards enter only the hero-combo
/// population's private hero-conditioned copy (`equity`), never a public range.
fn spawn_equity(core: &EngineCore, req: &LiveRequest, hero_public: Range1326, opponent: (Seat, Range1326), board: Vec<Card>) {
    let (ids, identity, sink, clock, hero) = (core.identity.clone(), req.identity.clone(), req.sink.clone(), core.clock.clone(), req.state.hero_cards);
    std::thread::Builder::new()
        .name("fast-path".into())
        .spawn(move || {
            let cancel = AtomicBool::new(false);
            let equity = equity_summary_with_clock(clock.as_ref(), hero, &hero_public, &[opponent], &board, Duration::from_millis(EQUITY_BUDGET_MS), &cancel);
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

/// The single registration path of §9.2, under the identity lock: `register` refuses a result whose decision is not
/// the one active there, and no invalidation can land between that check and the write.
fn register(core: &EngineCore, snap: SolvedStreet) -> bool {
    let ids = core.identity.lock().unwrap();
    let Some(active) = ids.active() else { return false };
    core.snapshots.lock().unwrap().register(active, snap)
}

/// What the decision log records of how a request ended, besides its `Final` (§5 step 10).
struct Logged {
    street: Street,
    reached_bp: Option<u16>,
    /// The first attempt's terminal arrived after the street deadline, or none arrived by it (ruling 20-I1).
    street_violation: bool,
    best_so_far: bool,
    /// The scaled-range hashes of the public ranges solved, OOP then IP; empty when none were read.
    range_hashes: Vec<String>,
}

impl Logged {
    /// A request answered before any solve: nothing measured, no attempt to judge.
    fn unsolved(street: Street, range_hashes: Vec<String>) -> Self { Self { street, reached_bp: None, street_violation: false, best_so_far: false, range_hashes } }
}

/// A `Final` the classifier settles alone, before anything is armed: its delivery flag is its own.
fn settle(core: &mut EngineCore, req: &LiveRequest, deadlines: &Deadlines, street: Street, rec: Recommendation) {
    finish(core, req, deadlines, &AtomicBool::new(false), rec, Logged::unsolved(street, vec![]));
}

/// The one `Final` path (§5 steps 9-10): emitted unless the watchdog delivered first (`delivered`), then the watchdog
/// retired and the decision logged. A decision no longer active has no `Final` and no log record, as a request
/// superseded while it ran has none.
fn finish(core: &mut EngineCore, req: &LiveRequest, deadlines: &Deadlines, delivered: &AtomicBool, rec: Recommendation, logged: Logged) {
    let now = core.clock.now_ms();
    // §7: from the watchdog's fire on, the watchdog delivers the `Final`; the engine's own came too late.
    let final_violation = now >= deadlines.watchdog_fire_ms();
    let (coverage, template_id) = (rec.coverage.clone(), rec.assumptions.template_id.clone());
    let delivery = deliver(&core.identity, &req.identity, &req.sink, Some(delivered), RecommendationEvent::Final(rec));
    core.watchdog.disarm();
    if delivery == Delivery::Stale {
        return;
    }
    let reasons = match &coverage {
        Coverage::Exact => vec![],
        Coverage::Approximate { reasons } => reasons.clone(),
        Coverage::Unsupported { partial, .. } => partial.clone(),
    };
    // §12 "Street deadline reached": a `best_so_far` is logged as a violation on every street but the flop, whose
    // single-raised-pot miss is the designed outcome (plan 4 refines the flop by pot type).
    let street_violation = if logged.street == Street::Flop { logged.street_violation && !logged.best_so_far } else { logged.street_violation || logged.best_so_far };
    core.log.append(&DecisionRecord { identity: req.identity.clone(), street: logged.street, coverage, reasons, elapsed_ms: elapsed_ms(req.t0_ms, now), cache: "miss".into(),
        presolver_scenario: None, tier: None, reached_bp: logged.reached_bp, street_violation, final_violation, template_id,
        input: InputRecord::from_state(&req.state, logged.range_hashes) });
}

/// §7 after a `Final` at the watchdog's fire: kill and restart the worker (see "After the `Final`" above).
fn restart_the_worker(core: &mut EngineCore) {
    core.worker.kill();
    if let Err(e) = core.worker.restart() {
        eprintln!("engine: restarting the worker after a DeadlineExceeded final failed: {e}; no live worker is left, and the next request relaunches it once");
    }
}

/// Milliseconds from the request's admission to `now_ms` on the engine clock. A request ends by its final delivery (at
/// most 35 s after `t0`), so the span fits a `u32`; a reading before `t0` or a span that does not fit is an engine
/// bug, asserted rather than wrapped.
fn elapsed_ms(t0_ms: u64, now_ms: u64) -> u32 {
    let span = now_ms.checked_sub(t0_ms).unwrap_or_else(|| panic!("engine clock reading {now_ms} ms precedes the request's admission at t0 {t0_ms} ms"));
    u32::try_from(span).unwrap_or_else(|_| panic!("a request span of {span} ms does not fit u32"))
}
