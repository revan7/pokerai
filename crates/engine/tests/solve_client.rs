use engine::core::EngineCore;
use engine::deadline::Deadlines;
use engine::identity::IdentityState;
use engine::log::DecisionLog;
use engine::solve::{run_solve, SolvePlan, Terminal};
use engine::testing::{uniform_solution, FakeClock, FakeReply, FakeWorker, IdRef, RecordingSink};
use engine::tree::{build_tree_full, TemplateSelection};
use engine::watchdog::{SharedSink, StreetDeadline};
use proto::worker::{AckStatus, EngineMessage, ResultStatus, Stage, WorkerError};
use proto::{Action, Card, Range1326, Rake, RecommendationEvent, Seat, SolveInput, Street, StreetRootSnapshot, UnsupportedReason};
use std::sync::{Arc, Mutex};

fn snap(street: Street) -> StreetRootSnapshot {
    let board = match street { Street::River => "Qs Jd 7h 3c 2d", _ => "Qs Jd 7h 3c" };
    StreetRootSnapshot { street, board: board.split(' ').map(|s| Card::parse(s).unwrap()).collect(), oop: Seat(2), ip: Seat(0), pot_root: 100,
        stack_oop_root: 100, stack_ip_root: 100, dead_this_street: 0, projected_from: 2, history: vec![], bb_chips: 2 }
}
fn full_range(board: &[Card]) -> Range1326 { let mut r = Range1326([1.0; 1326]); for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { r.0[i] = 0.0; } } r }
/// `published_at_restart`: the street deadline's published arrival at each restart of a `stalled_rig`'s link.
struct Rig { core: EngineCore, input: SolveInput, plan: SolvePlan, sink: SharedSink, events: Arc<Mutex<Vec<engine::testing::Recorded>>>, state: Arc<Mutex<engine::testing::FakeState>>, clock: Arc<FakeClock>, published_at_restart: Arc<Mutex<Vec<Option<u64>>>> }
/// Sets the plan's deadlines and the shared street deadline of the request with them (ruling 22-I4).
fn set_deadlines(plan: &mut SolvePlan, deadlines: Deadlines) {
    plan.deadlines = deadlines;
    plan.street_deadline = Arc::new(StreetDeadline::new(deadlines.street_deadline_ms));
}
fn rig(street: Street, script: Vec<FakeReply>) -> Rig {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let id = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
    let (worker, state) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let core = EngineCore::new(worker, clock.clone(), identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
    let root = snap(street);
    let template = if street == Street::River { "river_std_v1" } else { "turn_std_v1" };
    let tree = build_tree_full(&root, &TemplateSelection::from_history(template, &root.history)).unwrap().tree;
    let input = SolveInput { root: root.clone(), ranges: [full_range(&root.board), full_range(&root.board)], tree, target_bp: 50 };
    let deadlines = Deadlines::for_request(0, street, 10);
    let plan = SolvePlan { identity: id, deadlines, street_deadline: Arc::new(StreetDeadline::new(deadlines.street_deadline_ms)), template_id: template.into(), retry_template_id: engine::tree::Templates::min_variant(template).map(String::from), rake: Rake::TimeCharge, hero_actor: "oop".into(), background: false };
    let (sink, events) = RecordingSink::new(clock.clone(), Some(state.clone()));
    Rig { core, input, plan, sink: Arc::new(Mutex::new(Box::new(sink))), events, state, clock, published_at_restart: Arc::default() }
}
fn ok_for(street: Street, template: &str, expl: f32) -> FakeReply {
    let root = snap(street);
    let tree = build_tree_full(&root, &TemplateSelection::from_history(template, &[])).unwrap().tree;
    FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(uniform_solution(&tree, &[], expl)), error: None, elapsed_ms: 5 }
}
fn ack() -> FakeReply { FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None } }
fn err(code: &str, retryable: bool, est: Option<u64>) -> FakeReply { FakeReply::Result { id: IdRef::Last, status: ResultStatus::Error, solution: None, error: Some(WorkerError { code: code.into(), message: code.into(), retryable, estimate_bytes: est }), elapsed_ms: 1 } }
fn solves(state: &Arc<Mutex<engine::testing::FakeState>>) -> Vec<proto::worker::SolveRequest> { state.lock().unwrap().sent.iter().filter_map(|m| if let EngineMessage::Solve(r) = m { Some(r.clone()) } else { None }).collect() }

#[test]
fn deadline_arithmetic_and_request_fields() {
    let mut r = rig(Street::River, vec![ack(), FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 10, exploitability_chips: Some(0.8), elapsed_ms: 3 }, ok_for(Street::River, "river_std_v1", 0.3)]);
    r.clock.set_ms(500);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!(out.terminal, Terminal::Ok);
    let req = &solves(&r.state)[0];
    assert_eq!((req.deadline_ms, req.extraction_margin_ms, req.memory_limit_bytes, req.background), (2000 - 500 - 150, 200, 10 << 30, false));
    assert_eq!((req.pot, req.stack_oop, req.stack_ip, req.rake_rate, req.rake_cap_mchips), (100, 100, 100, 0.0, 0));
    assert_eq!(req.spot.len(), 64);
    assert_eq!((out.reached_bp, out.template_used.as_str(), out.street_violation, out.restarts), (Some(30), "river_std_v1", false, 0));
    assert_eq!(out.ordinal_paths[out.solution.as_ref().unwrap().requested as usize], out.decision_path);
    let ev = r.events.lock().unwrap();
    assert!(matches!(&ev[0].event, RecommendationEvent::Progress { stage, iterations: 10, exploitability_pct: Some(p), .. } if stage == "solving" && (*p - 0.8).abs() < 1e-6));
    // the watchdog fires at t0 + 14.9 s for a river decision
    assert_eq!(r.plan.deadlines.watchdog_fire_ms(), 14_900);
    assert_eq!(Deadlines::for_request(0, Street::Turn, 10).street_deadline_ms, 6_000);
    drop(ev);
    // `background` is a request parameter, not an engine invariant: plan 4's pre-solver sends `true`
    let mut bg = rig(Street::River, vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    bg.plan.background = true;
    assert_eq!(run_solve(&mut bg.core, &bg.input, &bg.plan, &bg.sink).terminal, Terminal::Ok);
    assert!(solves(&bg.state)[0].background);
}

#[test]
fn stale_ids_discarded() {
    let root = snap(Street::River);
    let tree = build_tree_full(&root, &TemplateSelection::from_history("river_std_v1", &[])).unwrap().tree;
    let stale = FakeReply::Result { id: IdRef::Fixed("old".into()), status: ResultStatus::Ok, solution: Some(uniform_solution(&tree, &[], 0.1)), error: None, elapsed_ms: 1 };
    let mut r = rig(Street::River, vec![ack(), stale, FakeReply::Progress { id: IdRef::Fixed("old".into()), stage: Stage::Solving, iterations: 3, exploitability_chips: None, elapsed_ms: 1 }, ok_for(Street::River, "river_std_v1", 0.3)]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.reached_bp), (Terminal::Ok, Some(30)));
    assert!(r.events.lock().unwrap().is_empty(), "a stale progress is never forwarded");
    let _ = (Action::Check, UnsupportedReason::InvalidRanges, err("x", false, None));
}

// --- Beyond the brief's two tests: the standing rulings of the solve client, one behaviour per test. Every wait is on
// the fake clock and every reply comes from the scripted worker (ruling 19: its timeline starts at the engine's first
// call); nothing here sleeps, spins or arms a watchdog thread. ---

use engine::clock::Clock;
use engine::worker::link::{WorkerLink, WorkerLinkError};
use proto::worker::{Ready, WorkerMessage};
use std::time::Duration;

/// A `result` line for `tree` with `status`, requested node `requested`, at `expl` chips.
fn result_for(tree: &proto::EffectiveTree, status: ResultStatus, requested: &[Action], expl: f32) -> FakeReply {
    FakeReply::Result { id: IdRef::Last, status, solution: Some(uniform_solution(tree, requested, expl)), error: None, elapsed_ms: 5 }
}
fn river_tree() -> proto::EffectiveTree { build_tree_full(&snap(Street::River), &TemplateSelection::from_history("river_std_v1", &[])).unwrap().tree }
fn failed_engine_error(t: &Terminal) -> (String, bool) {
    match t { Terminal::Failed(UnsupportedReason::EngineError { message, retryable }) => (message.clone(), *retryable), other => panic!("expected an EngineError, got {other:?}") }
}
fn kills_and_restarts(state: &Arc<Mutex<engine::testing::FakeState>>) -> (u32, u32) { let s = state.lock().unwrap(); (s.kills, s.restarts) }

/// Spec 4.5 `solve.spot` is the structural identity of the game (ruling 22-S): the one `bench_support::spot_identity`
/// computes, stacks and rake included; the solve parameters (id, deadline, target) never enter it.
#[test]
fn the_spot_is_the_structural_identity_of_the_game() {
    let spot_of = |r: &mut Rig| { let _ = run_solve(&mut r.core, &r.input, &r.plan, &r.sink); solves(&r.state)[0].clone() };
    let mut a = rig(Street::River, vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    let req = spot_of(&mut a);
    assert_eq!(req.spot, engine::bench_support::spot_identity(&req));
    assert_eq!(req.spot, engine::solve::spot_hash(&req));
    // the same game at another time and target: the same identity
    let mut b = rig(Street::River, vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    b.clock.set_ms(300);
    b.input.target_bp = 30;
    assert_eq!(spot_of(&mut b).spot, req.spot);
    // deeper stacks (another realized tree) or a rake: another game
    let mut deep = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::Hang]);
    deep.input.root.stack_oop_root = 200;
    deep.input.root.stack_ip_root = 200;
    assert_ne!(spot_of(&mut deep).spot, req.spot);
    let mut raked = rig(Street::River, vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    raked.plan.rake = Rake::PotRake { rate: 0.05, cap_mchips: 3_000, no_flop_no_drop: true };
    let raked_req = spot_of(&mut raked);
    assert_eq!((raked_req.rake_rate, raked_req.rake_cap_mchips), (0.05, 3_000));
    assert_ne!(raked_req.spot, req.spot);
}

/// §7 per street: the worker's `deadline_ms` is what is left until the street deadline at send time, less the delivery
/// and pipe margins, and the extraction margin is the street's (flop 0.6 s, river/turn 0.2 s); the flop budget moves
/// only the flop (§13.3 `flop_budget_setting_golden`'s wire numbers).
#[test]
fn request_deadline_and_margin_per_street() {
    // turn at t0 + 500 ms: 6 000 - 500 - 150
    let mut t = rig(Street::Turn, vec![ack(), ok_for(Street::Turn, "turn_std_v1", 0.3)]);
    t.clock.set_ms(500);
    assert_eq!(run_solve(&mut t.core, &t.input, &t.plan, &t.sink).terminal, Terminal::Ok);
    let req = &solves(&t.state)[0];
    assert_eq!((req.deadline_ms, req.extraction_margin_ms, req.target_bp, req.tree.template_id.as_str()), (5_350, 200, 50, "turn_std_v1"));
    // flop at t0 + 500 ms with a 10 s and a 30 s budget
    let flop = StreetRootSnapshot { street: Street::Flop, board: "Qs Jd 7h".split(' ').map(|s| Card::parse(s).unwrap()).collect(), ..snap(Street::Turn) };
    let flop_tree = build_tree_full(&flop, &TemplateSelection::from_history("flop_fast_v1", &[])).unwrap().tree;
    for (budget, expected) in [(10u8, 9_350u32), (30, 29_350)] {
        let mut f = rig(Street::Turn, vec![ack(), result_for(&flop_tree, ResultStatus::Ok, &[], 0.3)]);
        f.input = SolveInput { root: flop.clone(), ranges: [full_range(&flop.board), full_range(&flop.board)], tree: flop_tree.clone(), target_bp: 50 };
        set_deadlines(&mut f.plan, Deadlines::for_request(0, Street::Flop, budget));
        f.plan.template_id = "flop_fast_v1".into();
        f.plan.retry_template_id = Some("flop_min_v1".into());
        f.clock.set_ms(500);
        assert_eq!(run_solve(&mut f.core, &f.input, &f.plan, &f.sink).terminal, Terminal::Ok, "flop budget {budget}");
        let req = &solves(&f.state)[0];
        assert_eq!((req.deadline_ms, req.extraction_margin_ms, req.board.len()), (expected, 600, 3), "flop budget {budget}");
    }
}

/// Rulings 26-Q4 and 22-I3 (spec 2: only reasons actually incurred; spec 7: `best_so_far` is the measured solution at
/// a deadline stop). `Terminal::Ok` iff the raw exploitability meets the raw target, compared exactly (`expl * 10_000
/// <= target_bp * pot` in f64, the predicate of `assemble::coverage_for_solve`). `BestSoFar`, which assembly labels
/// `DeadlineBestSoFar`, is kept for a genuine deadline stop, a worker `best_so_far`, that misses it. A worker `ok` that
/// misses the raw target breaks the worker's contract (`ok` means the target was met): the worker stops at an
/// f32-rounded target, at 30 bp of a 100-chip pot 0.3f32, half an ulp above the raw 0.3 chips, so an immediate `ok` at
/// 0.3f32 is a non-retryable worker-contract `EngineError` naming both thresholds, never `Exact` and never a deadline
/// stop it did not have.
#[test]
fn q4_an_ok_short_of_the_raw_target_is_a_worker_contract_error_never_a_deadline_stop() {
    let tree = river_tree();
    // the solve returns at once: no deadline was reached, whatever the terminal
    let run = |status: ResultStatus, expl: f32, target_bp: u16| {
        let mut r = rig(Street::River, vec![ack(), result_for(&tree, status, &[], expl)]);
        r.input.target_bp = target_bp;
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        assert_eq!((r.clock.now_ms(), out.restarts, kills_and_restarts(&r.state)), (0, 0, (0, 0)), "{status:?} at {expl} of a {target_bp} bp target");
        out
    };
    // the worker's own threshold admits 0.3f32 at 30 bp; the raw comparison does not
    assert!((f64::from(100u32) * 30.0 / 10_000.0) as f32 >= 0.3f32 && f64::from(0.3f32) * 10_000.0 > 30.0 * 100.0);
    for (expl, target_bp) in [(0.3f32, 30u16), (1.9, 50)] {
        let out = run(ResultStatus::Ok, expl, target_bp);
        assert_ne!(out.terminal, Terminal::BestSoFar, "an ok at {expl} of a {target_bp} bp target never claims a deadline stop");
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.starts_with("worker contract") && message.contains("raw target") && message.contains("f32-rounded threshold") && !retryable, "{message}");
        assert!(out.solution.is_none() && out.reached_bp.is_none(), "nothing of a contract-breaking ok is exposed");
    }
    // the rounded-threshold case names the measurement, the raw target and the worker's threshold
    let (message, _) = failed_engine_error(&run(ResultStatus::Ok, 0.3, 30).terminal);
    let f32_chips = f64::from(0.3f32).to_string();
    assert!(message.contains(&format!("`ok` at {f32_chips} chips")) && message.contains("raw target 0.3 chips (30 bp of the 100-chip pot)")
        && message.contains(&format!("f32-rounded threshold is {f32_chips} chips")), "{message}");
    // an ok at a representable raw target: Ok, Exact
    let out = run(ResultStatus::Ok, 0.5, 50);
    assert_eq!((out.terminal, out.reached_bp), (Terminal::Ok, Some(50)));
    assert_eq!(engine::assemble::coverage_for_solve(0.5, 100, 50, false, vec![]), proto::Coverage::Exact);
    // a best_so_far is a deadline stop: short of the raw target it is BestSoFar (DeadlineBestSoFar downstream); one that
    // nevertheless meets the raw target is Ok (Exact)
    let out = run(ResultStatus::BestSoFar, 1.9, 50);
    assert_eq!((out.terminal, out.reached_bp), (Terminal::BestSoFar, Some(190)));
    assert_eq!(engine::assemble::coverage_for_solve(1.9, 100, 50, true, vec![]),
        proto::Coverage::Approximate { reasons: vec![proto::ApproxReason::DeadlineBestSoFar { reached_bp: 190, target_bp: 50 }] });
    let out = run(ResultStatus::BestSoFar, 0.3, 50);
    assert_eq!((out.terminal, out.reached_bp), (Terminal::Ok, Some(30)));
}

/// Ruling 20-I1 for the client: the street verdict is judged from the engine-clock time the first attempt's terminal
/// arrived (a terminal at the deadline is on time, 1 ms later is a violation, and the late result is still accepted),
/// and with no terminal it is a violation once the attempt ends at or past the deadline.
#[test]
fn the_street_verdict_follows_the_terminal_arrival_time() {
    let tree = river_tree();
    for (delay, violated) in [(2_000u64, false), (2_001, true)] {
        let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: delay }, result_for(&tree, ResultStatus::Ok, &[], 0.3)]);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        assert_eq!((out.terminal, out.street_violation, r.clock.now_ms()), (Terminal::Ok, violated, delay), "terminal at {delay} ms");
    }
    // no terminal: the worker hangs, the attempt ends at the hang bound (sent 0 + 1 850 + 150 + 500 = 2 500 ms); one
    // attempt only (Task 23: the `_min` retry would run on to the watchdog's fire, as its own test shows)
    let mut r = rig(Street::River, vec![ack(), FakeReply::Hang]);
    r.plan.retry_template_id = None;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert!(out.street_violation && r.clock.now_ms() == 2_500);
}

/// The client never waits past a deadline of its own: `recv` is given only what is left until the attempt's hang bound
/// or the watchdog's fire, whichever is first, and at the fire the attempt ends with the stage reached (the watchdog's
/// `Final` goes out independently, §7).
#[test]
fn receives_never_outlast_the_hang_bound_or_the_watchdog_fire() {
    // a hung worker: the attempt ends exactly at the hang bound and the worker is restarted (one attempt only: Task 23's
    // `_min` retry is `a_retry_cut_by_the_watchdog_fire_leaves_the_cleanup_to_the_delivery`)
    let mut r = rig(Street::River, vec![ack(), FakeReply::Hang]);
    r.plan.retry_template_id = None;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("no terminal result") && retryable, "{message}");
    assert_eq!((r.clock.now_ms(), out.restarts, kills_and_restarts(&r.state)), (2_500, 1, (0, 1)));
    // a watchdog fire before the hang bound: t0 0, street 2 000, final delivery 2 300, fire 2 200
    let mut r = rig(Street::River, vec![ack(), FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 1, exploitability_chips: None, elapsed_ms: 1 }, FakeReply::Hang]);
    set_deadlines(&mut r.plan, Deadlines { t0_ms: 0, street_deadline_ms: 2_000, final_delivery_ms: 2_300, extraction_margin_ms: 200 });
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!(out.terminal, Terminal::Failed(UnsupportedReason::DeadlineExceeded { stage: "solving".into() }));
    assert_eq!((r.clock.now_ms(), out.restarts, r.core.stage()), (2_200, 0, "solving".to_string()));
}

/// Identity is checked on every reply: a progress that arrives after a mutation (in the same receive) is not forwarded,
/// and a result that races one is discarded unvalidated; nothing is emitted for a superseded identity.
#[test]
fn identity_is_checked_on_every_reply() {
    let progress = FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 4, exploitability_chips: Some(0.9), elapsed_ms: 1 };
    for script in [vec![ack(), FakeReply::InvalidateIdentity, progress], vec![ack(), FakeReply::InvalidateIdentity, ok_for(Street::River, "river_std_v1", 0.3)]] {
        let mut r = rig(Street::River, script);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.contains("superseded") && !retryable, "{message}");
        assert!(out.solution.is_none() && r.events.lock().unwrap().is_empty(), "nothing is emitted or accepted for a superseded identity");
    }
}

/// A `rejected` ack for this solve ends the attempt at once (no wait for a terminal that will never come, no restart);
/// an ack for another id, rejected or accepted, is discarded and frees nothing.
#[test]
fn a_rejected_solve_ends_at_once_and_stale_acks_free_nothing() {
    let stale_reject = FakeReply::Ack { id: IdRef::Fixed("old".into()), status: AckStatus::Rejected, reason: Some("busy".into()) };
    let stale_accept = FakeReply::Ack { id: IdRef::Fixed("old".into()), status: AckStatus::Accepted, reason: None };
    let mut r = rig(Street::River, vec![stale_reject.clone(), stale_accept, ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    assert_eq!(run_solve(&mut r.core, &r.input, &r.plan, &r.sink).terminal, Terminal::Ok);
    let mut r = rig(Street::River, vec![stale_reject, FakeReply::Ack { id: IdRef::Last, status: AckStatus::Rejected, reason: Some("busy".into()) }, FakeReply::Hang]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("busy") && retryable, "{message}");
    assert_eq!((r.clock.now_ms(), solves(&r.state).len(), out.restarts, kills_and_restarts(&r.state)), (0, 1, 0, (0, 0)));
}

/// A link whose `ready` the client refuses: any request, receive, kill or restart on it is a test failure. With no
/// `ready` at all (no live worker) the client relaunches it once (Task 23), and that relaunch fails.
/// The relaunch failure is not retryable (P2T23-I3).
struct NoWork(Option<Ready>);
impl WorkerLink for NoWork {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { panic!("a request reached a worker whose ready was refused: {msg:?}") }
    fn recv(&mut self, _timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> { panic!("the client waited on a worker whose ready was refused") }
    fn restart(&mut self) -> Result<(), WorkerLinkError> {
        assert!(self.0.is_none(), "the client restarted a worker whose ready was refused");
        Err(WorkerLinkError::Spawn("no worker could be launched".into()))
    }
    fn kill(&mut self) { panic!("the client killed a worker whose ready was refused") }
    fn ready(&self) -> Option<&Ready> { self.0.as_ref() }
}

/// §4.5/§12: the worker's `ready` is validated before any request, and a refused solve starts no work: no request, no
/// wait, no restart. A version, commit or AVX2 mismatch is a non-retryable `EngineError`; so is no live worker once the
/// one relaunch it gets (Task 23) has failed (P2T23-I3).
#[test]
fn ready_is_validated_before_any_request() {
    let mut no_avx2 = FakeWorker::default_ready();
    no_avx2.build_features = vec!["sse4.2".into()];
    let mut old_proto = FakeWorker::default_ready();
    old_proto.proto_version = 2;
    for (ready, fragment, retryable, restarts) in [(Some(no_avx2), "AVX2", false, 0), (Some(old_proto), "proto_version 2", false, 0), (None, "no live worker", false, 1)] {
        let mut r = rig(Street::River, vec![]);
        let log = DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log"));
        let identity = r.core.identity.clone();
        r.core = EngineCore::new(Box::new(NoWork(ready)), r.clock.clone(), identity, log);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, got_retryable) = failed_engine_error(&out.terminal);
        assert!(message.contains(fragment) && got_retryable == retryable, "{message}");
        assert_eq!((r.clock.now_ms(), out.restarts), (0, restarts));
    }
    // a deadline that leaves no iteration is refused before the request too (river at 1 650 ms: 200 <= 200)
    let mut r = rig(Street::River, vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    r.clock.set_ms(1_650);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.street_violation), (Terminal::Failed(UnsupportedReason::DeadlineExceeded { stage: "fast".into() }), false));
    assert!(solves(&r.state).is_empty());
}

/// §4.5 output validation: a solution whose requested node is not the decision node, whose requested actor is not hero,
/// or that fails the matrix validator is `EngineError("invalid solution")`, non-retryable, and never accepted.
#[test]
fn an_invalid_solution_is_refused() {
    let tree = river_tree();
    let mut bad_row = uniform_solution(&tree, &[], 0.3);
    bad_row.nodes[0].probs[0] = vec![0.1; bad_row.nodes[0].actions.len()];
    let cases = [
        (result_for(&tree, ResultStatus::Ok, &[Action::Check], 0.3), "oop", "not the decision node"),
        (result_for(&tree, ResultStatus::Ok, &[], 0.3), "ip", "is not hero's ip"),
        (FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(bad_row), error: None, elapsed_ms: 5 }, "oop", "sums to"),
    ];
    for (reply, hero, fragment) in cases {
        let mut r = rig(Street::River, vec![ack(), reply]);
        r.plan.hero_actor = hero.into();
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.starts_with("invalid solution") && message.contains(fragment) && !retryable, "{message}");
        assert!(out.solution.is_none() && out.reached_bp.is_none());
    }
}

/// A reply that breaks the protocol (a negative exploitability, a result whose status and payload disagree, a failure
/// code outside §4.5's list, a solve acked as `staged`) is a protocol error: the worker is restarted (§12) and nothing is
/// emitted from it. One attempt only: Task 23's `_min` retry after the restart is its own tests'.
#[test]
fn protocol_violations_restart_the_worker() {
    let negative = FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 1, exploitability_chips: Some(-0.5), elapsed_ms: 1 };
    let ok_without_solution = FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: None, error: None, elapsed_ms: 1 };
    let staged = FakeReply::Ack { id: IdRef::Last, status: AckStatus::Staged, reason: None };
    for (script, fragment) in [(vec![ack(), negative], "negative"), (vec![ack(), ok_without_solution], "without a solution"),
        (vec![ack(), err("boom", true, None)], "boom"), (vec![staged], "staged")] {
        let mut r = rig(Street::River, script);
        r.plan.retry_template_id = None;
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.starts_with("protocol error") && message.contains(fragment) && retryable, "{message}");
        assert_eq!((out.restarts, kills_and_restarts(&r.state)), (1, (0, 1)));
        assert!(r.events.lock().unwrap().is_empty());
    }
}

/// Worker death (ruling 18): a confirmed exit is `WorkerExit{code}`; an end of stdout whose exit is never confirmed is
/// not given a code. Both restart the worker (kill, reap, respawn, validate `ready`). One attempt only: Task 23's `_min`
/// retry after the restart is `error_codes_retry_policy`'s.
#[test]
fn a_worker_exit_and_an_unconfirmed_end_restart_the_worker() {
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::Exit { code: 3 }]);
    r.plan.retry_template_id = None;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("WorkerExit{code: 3}") && retryable, "{message}");
    assert_eq!((out.restarts, kills_and_restarts(&r.state), r.clock.now_ms()), (1, (0, 1), 1));
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::Eof, FakeReply::Hang]);
    r.plan.retry_template_id = None;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("not confirmed") && !message.contains("WorkerExit") && retryable, "{message}");
    assert_eq!((out.restarts, kills_and_restarts(&r.state), r.clock.now_ms()), (1, (0, 1), 2_500));
    assert!(r.core.worker.ready().is_some(), "the restarted worker is live again");
}

// --- Fix round 1 (review P2T22R, rulings 22-I1, 22-I2, 22-I3, 22-I4): expiry after every receive, identity before the
// send, the first terminal's arrival. Time still comes only from the fake clock; the links and the clock below only
// decide when the client observes what the scripted worker wrote. ---

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The engine clock as a busy engine thread sees it: the fake clock, except that once `per_read_ms` is set, every
/// reading costs that much engine time (the reading is returned, then the clock moves on). It models the client's own
/// processing after a receive (validation, a slow or preempted thread). Waiting is the fake clock's.
struct Busy { fake: Arc<FakeClock>, per_read_ms: Arc<AtomicU64> }
impl Clock for Busy {
    fn now_ms(&self) -> u64 {
        let t = self.fake.now_ms();
        let cost = self.per_read_ms.load(Ordering::SeqCst);
        if cost > 0 { self.fake.advance_ms(cost); }
        t
    }
    fn wait_until(&self, t_ms: u64) { self.fake.wait_until(t_ms) }
}

/// The scripted worker behind an engine thread that loses time between the pipe and the client. When `stalls_on` picks
/// a reply that `recv` is about to hand over, the fake clock first jumps to `resume_at_ms` (a suspend/resume or a
/// scheduler stall: the reply was read before it, the client sees it after). From then on every engine-clock reading
/// costs `per_read_ms` (see `Busy`). At each restart it notes the arrival the request's street deadline has published
/// by then (`Rig::published_at_restart`).
struct Stalled {
    inner: Box<dyn WorkerLink>, clock: Arc<FakeClock>, stalls_on: fn(&WorkerMessage) -> bool, resume_at_ms: Option<u64>, per_read_ms: u64, busy: Arc<AtomicU64>,
    street: Arc<StreetDeadline>, published_at_restart: Arc<Mutex<Vec<Option<u64>>>>,
}
impl WorkerLink for Stalled {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { self.inner.send(msg) }
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        let got = self.inner.recv(timeout);
        if let Ok(Some(msg)) = &got {
            if (self.stalls_on)(msg) {
                if let Some(t) = self.resume_at_ms { self.clock.set_ms(t); }
                self.busy.store(self.per_read_ms, Ordering::SeqCst);
            }
        }
        got
    }
    fn restart(&mut self) -> Result<(), WorkerLinkError> {
        self.published_at_restart.lock().unwrap().push(self.street.terminal_arrival_ms());
        self.inner.restart()
    }
    fn kill(&mut self) { self.inner.kill() }
    fn ready(&self) -> Option<&Ready> { self.inner.ready() }
}
fn is_result(m: &WorkerMessage) -> bool { matches!(m, WorkerMessage::Result { .. }) }
fn is_progress(m: &WorkerMessage) -> bool { matches!(m, WorkerMessage::Progress { .. }) }

/// A river rig whose scripted worker is seen through `Stalled` and whose engine clock is `Busy`. Its street deadline
/// (2 000 ms) stays the request's for every deadline set below, which all keep it.
fn stalled_rig(script: Vec<FakeReply>, stalls_on: fn(&WorkerMessage) -> bool, resume_at_ms: Option<u64>, per_read_ms: u64) -> Rig {
    let mut r = rig(Street::River, vec![]);
    let identity = r.core.identity.clone();
    let (worker, state) = FakeWorker::scripted(r.clock.clone(), identity.clone(), script);
    let busy = Arc::new(AtomicU64::new(0));
    let link = Stalled { inner: worker, clock: r.clock.clone(), stalls_on, resume_at_ms, per_read_ms, busy: busy.clone(),
        street: r.plan.street_deadline.clone(), published_at_restart: r.published_at_restart.clone() };
    let clock = Arc::new(Busy { fake: r.clock.clone(), per_read_ms: busy });
    r.core = EngineCore::new(Box::new(link), clock, identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
    r.state = state;
    r
}
/// t0 0, street 2 000 ms (the river rig's), final delivery 2 300 ms: the watchdog fires at 2 200 ms, before the river
/// attempt's hang bound at 2 500 ms.
const SHORT: Deadlines = Deadlines { t0_ms: 0, street_deadline_ms: 2_000, final_delivery_ms: 2_300, extraction_margin_ms: 200 };
fn deadline_exceeded(stage: &str) -> Terminal { Terminal::Failed(UnsupportedReason::DeadlineExceeded { stage: stage.into() }) }

/// Ruling 22-I1 (spec 7: after a suspend/resume the request is expired and late replies are rejected by identity and
/// expiry; spec 12: late replies are discarded). A reply is judged at the engine-clock time the client observes it,
/// not by the bound its receive was given. One observed at or after the watchdog's fire is neither forwarded nor
/// accepted: the attempt ends `DeadlinePassed` (the watchdog delivers the `Final`). One observed at or after the
/// attempt's hang bound ends it as the hang the client would have declared had it resumed a moment later, at the top of
/// its loop.
#[test]
fn a_reply_observed_after_expiry_is_neither_forwarded_nor_accepted() {
    let tree = river_tree();
    let result = || result_for(&tree, ResultStatus::Ok, &[], 0.3);
    // the review's probe: a valid result read before a suspend and seen at 15 001 ms, past the 15 000 ms final delivery
    let mut r = stalled_rig(vec![ack(), result()], is_result, Some(15_001), 0);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.solution.is_none(), out.restarts), (deadline_exceeded("building"), true, 0), "result seen at 15 001 ms");
    // the exact boundary at the watchdog's fire (`SHORT`: fire 2 200 ms, hang bound 2 500 ms)
    for (seen_at, accepted) in [(2_199u64, true), (2_200, false)] {
        let mut r = stalled_rig(vec![ack(), result()], is_result, Some(seen_at), 0);
        r.plan.deadlines = SHORT;
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let expected = if accepted { (Terminal::Ok, false) } else { (deadline_exceeded("building"), true) };
        assert_eq!((out.terminal, out.solution.is_none()), expected, "result seen at {seen_at} ms, fire at 2 200 ms");
        assert_eq!(out.restarts, 0);
    }
    // a progress seen at the fire is not forwarded, and does not advance the stage
    let progress = FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 7, exploitability_chips: Some(0.9), elapsed_ms: 1 };
    let mut r = stalled_rig(vec![ack(), progress, FakeReply::Hang], is_progress, Some(2_200), 0);
    r.plan.deadlines = SHORT;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, r.core.stage()), (deadline_exceeded("building"), "building".to_string()));
    assert!(r.events.lock().unwrap().is_empty(), "a progress seen at the fire is never forwarded");
    // the exact boundary at the hang bound (river: sent at 0, 1 850 + 150 + 500 = 2 500 ms; the fire is at 14 900 ms),
    // for the one attempt (Task 23: after the hang a `_min` retry would run on to the fire)
    for (seen_at, accepted) in [(2_499u64, true), (2_500, false)] {
        let mut r = stalled_rig(vec![ack(), result()], is_result, Some(seen_at), 0);
        r.plan.retry_template_id = None;
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        if accepted {
            assert_eq!((out.terminal, out.street_violation, out.restarts), (Terminal::Ok, true, 0), "result seen at {seen_at} ms");
        } else {
            let (message, retryable) = failed_engine_error(&out.terminal);
            assert!(message.contains("no terminal result") && retryable && out.solution.is_none(), "result seen at {seen_at} ms: {message}");
            assert_eq!((out.restarts, kills_and_restarts(&r.state)), (1, (0, 1)));
        }
    }
}

/// Ruling 22-I1, the second half: expiry is checked again before a validated success is exposed, since validation (or
/// anything else the client does after the receive) can cross the watchdog's fire. A result seen 1 ms before the fire
/// passes the receive's check; with every later engine-clock reading costing 1 ms, the success would be exposed at or
/// after the fire, so the attempt ends `DeadlineExceeded` instead.
#[test]
fn a_success_is_not_exposed_once_processing_crosses_the_watchdog_fire() {
    let tree = river_tree();
    let mut r = stalled_rig(vec![ack(), result_for(&tree, ResultStatus::Ok, &[], 0.3)], is_result, Some(2_199), 1);
    r.plan.deadlines = SHORT;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.solution.is_none(), out.reached_bp, out.restarts), (deadline_exceeded("building"), true, None, 0));
    // the terminal did arrive, at 2 199 ms, and was published at receipt (ruling 22-I4)
    assert_eq!((out.first_terminal_ms, r.plan.street_deadline.terminal_arrival_ms()), (Some(2_199), Some(2_199)));
}

/// A link over the scripted worker through which a mutation arrives while the client prepares its request: the active
/// decision is cancelled when the client reads the worker's `ready`, after the tree is built and before the request is
/// sent.
struct MutatedWhilePreparing { inner: Box<dyn WorkerLink>, identity: Arc<Mutex<IdentityState>> }
impl WorkerLink for MutatedWhilePreparing {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { self.inner.send(msg) }
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> { self.inner.recv(timeout) }
    fn restart(&mut self) -> Result<(), WorkerLinkError> { self.inner.restart() }
    fn kill(&mut self) { self.inner.kill() }
    fn ready(&self) -> Option<&Ready> { self.identity.lock().unwrap().cancel_active(); self.inner.ready() }
}

/// A link the client must not touch at all, not even to read `ready`.
struct Untouched;
impl WorkerLink for Untouched {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { panic!("a superseded decision sent {msg:?}") }
    fn recv(&mut self, _timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> { panic!("a superseded decision waited on the worker") }
    fn restart(&mut self) -> Result<(), WorkerLinkError> { panic!("a superseded decision restarted the worker") }
    fn kill(&mut self) { panic!("a superseded decision killed the worker") }
    fn ready(&self) -> Option<&Ready> { panic!("a superseded decision went on to prepare its request (it read the worker's ready)") }
}

/// Ruling 22-I2 (spec 5/7 admission; spec 12: stale work has no effects): the plan's identity is checked before any
/// construction and again immediately before the send. A decision superseded before `run_solve` starts, or while its
/// request is prepared, starts no work: no request of any kind, no progress, no kill, no restart, no time spent. One
/// superseded before entry is refused before anything is prepared: the worker is not even asked for its `ready`.
#[test]
fn a_superseded_decision_starts_no_work() {
    let mut untouched = rig(Street::River, vec![]);
    let identity = untouched.core.identity.clone();
    untouched.core = EngineCore::new(Box::new(Untouched), untouched.clock.clone(), identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
    untouched.core.identity.lock().unwrap().mutate();
    let out = run_solve(&mut untouched.core, &untouched.input, &untouched.plan, &untouched.sink);
    assert_eq!((failed_engine_error(&out.terminal), out.first_terminal_ms), (("superseded by a newer request".to_string(), false), None));

    let script = || vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)];
    let before = rig(Street::River, script());
    before.core.identity.lock().unwrap().mutate();
    let mut preparing = rig(Street::River, vec![]);
    let identity = preparing.core.identity.clone();
    let (worker, state) = FakeWorker::scripted(preparing.clock.clone(), identity.clone(), script());
    let link = MutatedWhilePreparing { inner: worker, identity: identity.clone() };
    preparing.core = EngineCore::new(Box::new(link), preparing.clock.clone(), identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
    preparing.state = state;
    for (case, mut r) in [("superseded before entry", before), ("superseded while the request is prepared", preparing)] {
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.contains("superseded") && !retryable, "{case}: {message}");
        assert!(r.state.lock().unwrap().sent.is_empty(), "{case}: no request of any kind reaches the worker");
        assert!(r.events.lock().unwrap().is_empty(), "{case}: no progress");
        assert_eq!((kills_and_restarts(&r.state), out.restarts, r.clock.now_ms(), out.solution.is_none()), ((0, 0), 0, 0, true), "{case}");
        assert_eq!((out.first_terminal_ms, r.plan.street_deadline.terminal_arrival_ms()), (None, None), "{case}: nothing published");
    }
}

/// Ruling 22-I4 (ruling 20-I1; spec 7): the engine-clock arrival of the first attempt's terminal `result` is published
/// to the request's shared `StreetDeadline` at receipt, before validation or recovery, and kept in
/// `SolveOutcome::first_terminal_ms` through every outcome (`None` when no terminal arrived), so the independent
/// watchdog, and the request's delivery after it, judge the street deadline from the real arrival. A terminal that
/// arrives on time stays on time however long the client then takes.
#[test]
fn the_first_terminal_arrival_is_published_at_receipt_and_kept_through_every_outcome() {
    let tree = river_tree();
    let ok = || result_for(&tree, ResultStatus::Ok, &[], 0.3);
    // on time at 1 999 ms; every engine-clock reading after it costs 5 ms, so the client finishes after the 2 000 ms
    // street deadline
    let mut r = stalled_rig(vec![ack(), FakeReply::Delay { ms: 1_999 }, ok()], is_result, None, 5);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert!(r.clock.now_ms() > 2_000 && out.elapsed_ms > 2_000, "processing ended after the street deadline, at {} ms", r.clock.now_ms());
    assert_eq!((out.terminal, out.first_terminal_ms, out.street_violation), (Terminal::Ok, Some(1_999), false));
    assert_eq!((r.plan.street_deadline.terminal_arrival_ms(), r.plan.street_deadline.violated()), (Some(1_999), false));
    // published whatever becomes of the terminal, and before any recovery: an invalid solution, a worker error, one at
    // the deadline, a late one, and one that breaks the protocol (the worker is restarted after the publication)
    let invalid = result_for(&tree, ResultStatus::Ok, &[Action::Check], 0.3);
    let malformed = FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: None, error: None, elapsed_ms: 1 };
    let cases = [(invalid, 700u64, false, 0usize), (err("no_iteration", false, None), 1_200, false, 0), (ok(), 2_000, false, 0), (ok(), 2_001, true, 0), (malformed, 1_000, false, 1)];
    for (terminal, at, violated, restarts) in cases {
        let mut r = stalled_rig(vec![ack(), FakeReply::Delay { ms: at }, terminal], is_result, None, 0);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        assert_eq!((out.first_terminal_ms, out.street_violation, usize::from(out.restarts)), (Some(at), violated, restarts), "terminal at {at} ms: {:?}", out.terminal);
        assert_eq!((r.plan.street_deadline.terminal_arrival_ms(), r.plan.street_deadline.violated()), (Some(at), violated), "terminal at {at} ms");
        assert_eq!(*r.published_at_restart.lock().unwrap(), vec![Some(at); restarts], "terminal at {at} ms: published before the restart");
    }
    // no terminal of this decision's attempt: none arrived (a hang), one raced a mutation (stale work publishes
    // nothing), one was seen at the watchdog's fire or past the hang bound (discarded unread)
    let superseded = rig(Street::River, vec![ack(), FakeReply::InvalidateIdentity, ok()]);
    let mut at_fire = stalled_rig(vec![ack(), ok()], is_result, Some(2_200), 0);
    at_fire.plan.deadlines = SHORT;
    let hung = rig(Street::River, vec![ack(), FakeReply::Hang]);
    let past_hang_bound = stalled_rig(vec![ack(), ok()], is_result, Some(2_500), 0);
    // (the verdict without a terminal: violated once the attempt ends at or past the street deadline)
    for (case, mut r, violated) in [("a hang", hung, true), ("superseded at 0 ms", superseded, false), ("seen at the fire", at_fire, true), ("seen at the hang bound", past_hang_bound, true)] {
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        assert_eq!((out.first_terminal_ms, r.plan.street_deadline.terminal_arrival_ms(), out.street_violation), (None, None, violated), "{case}: {:?}", out.terminal);
    }
}

// --- Fix round 2 (re-review P2T22-N1): the pre-send expiry check sends nothing once the watchdog's fire has come. ---

/// A clock that models a suspend landing between `worker_deadline_ms`'s reading (`run_solve`, right after
/// `ready_for_requests`) and the pre-send expiry check `run_attempt` makes immediately before the send
/// (`solve.rs:279`). Once armed (by `ArmsOnReady::ready()`, called as the worker's `ready` is checked, one reading
/// before `worker_deadline_ms`'s), it counts the readings taken through it: the first is the fake clock's own (so the
/// request is still built and reaches the send, as it would without a suspend), and on the second it jumps the fake
/// clock to `fire_ms` before returning it — the pre-send check's reading.
struct ArmedClock { fake: Arc<FakeClock>, armed: Arc<AtomicBool>, fire_ms: u64, readings_since_armed: AtomicU64 }
impl Clock for ArmedClock {
    fn now_ms(&self) -> u64 {
        if self.armed.load(Ordering::SeqCst) && self.readings_since_armed.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
            self.fake.set_ms(self.fire_ms);
        }
        self.fake.now_ms()
    }
    fn wait_until(&self, t_ms: u64) { self.fake.wait_until(t_ms) }
}

/// The scripted worker behind a link whose `ready()` arms `ArmedClock`.
struct ArmsOnReady { inner: Box<dyn WorkerLink>, armed: Arc<AtomicBool> }
impl WorkerLink for ArmsOnReady {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { self.inner.send(msg) }
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> { self.inner.recv(timeout) }
    fn restart(&mut self) -> Result<(), WorkerLinkError> { self.inner.restart() }
    fn kill(&mut self) { self.inner.kill() }
    fn ready(&self) -> Option<&Ready> { self.armed.store(true, Ordering::SeqCst); self.inner.ready() }
}

/// P2T22-N1 (re-review 1, `task-22-rereview-1.md`): the pre-send `ended_at` check (`solve.rs:279`) also ends the
/// attempt `DeadlinePassed`, sending nothing, once the watchdog's fire has come by then — reachable exactly in the
/// suspend scenario spec 7 names, a mutation or a stalled thread landing between the tree being built and the request
/// being sent. Reducing that check to identity only (mutant MC of the re-review) left the committed suite green,
/// since nothing forced the pre-send reading itself to observe the fire: this test does, with a clock that jumps to
/// `watchdog_fire_ms()` exactly on that reading (armed as the worker's `ready` is checked, one reading before it, so
/// `worker_deadline_ms` still sees room for an attempt and the flow reaches the send). Nothing reaches the worker, no
/// progress is forwarded, nothing is killed or restarted, and no arrival is published to the street deadline: the same
/// `DeadlineExceeded{"building"}` the loop's own top would have declared a moment later, had anything been sent.
#[test]
fn a_request_expired_while_prepared_is_never_sent() {
    let mut r = rig(Street::River, vec![]);
    let identity = r.core.identity.clone();
    let armed = Arc::new(AtomicBool::new(false));
    let clock: Arc<dyn Clock> = Arc::new(ArmedClock {
        fake: r.clock.clone(), armed: armed.clone(), fire_ms: r.plan.deadlines.watchdog_fire_ms(), readings_since_armed: AtomicU64::new(0),
    });
    let (worker, state) = FakeWorker::scripted(r.clock.clone(), identity.clone(), vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    let link = ArmsOnReady { inner: worker, armed };
    r.core = EngineCore::new(Box::new(link), clock, identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
    r.state = state;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!(
        (out.terminal, solves(&r.state).is_empty(), kills_and_restarts(&r.state), out.restarts, out.first_terminal_ms, r.plan.street_deadline.terminal_arrival_ms()),
        (deadline_exceeded("building"), true, (0, 0), 0, None, None),
    );
}

#[test]
fn superseded_request_cancels_then_kills_after_1_5s() {
    // the mutation lands 100 ms after the ack; the client cancels, waits 1.5 s for result{cancelled}, then kills
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 100 }, FakeReply::InvalidateIdentity, FakeReply::Hang]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert!(matches!(out.terminal, Terminal::Failed(UnsupportedReason::EngineError { ref message, .. }) if message.contains("superseded")));
    let s = r.state.lock().unwrap();
    assert_eq!((s.cancels.len(), s.kills, s.restarts), (1, 1, 1));
    assert!(r.clock.now_ms() >= 100 + 1500);
    assert!(r.events.lock().unwrap().is_empty());   // nothing is emitted for a superseded identity
}

#[test]
fn heartbeat_failure_restarts_and_retries_min() {
    // Progress at t = 1200 ms, then nothing for 5 s: at t = 6200 the heartbeat fires, the worker is killed and
    // respawned, and one retry with turn_min_v1 is admitted. 6200 is past the 6 s turn budget and the first attempt
    // never produced a terminal, so the street was violated even though the retry succeeds.
    let mut r = rig(Street::Turn, vec![ack(), FakeReply::Delay { ms: 1200 },
        FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 1, exploitability_chips: None, elapsed_ms: 1 },
        FakeReply::Delay { ms: 5100 }, FakeReply::Hang, ack(), ok_for(Street::Turn, "turn_min_v1", 0.4)]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal.clone(), out.template_used.as_str(), out.restarts), (Terminal::Ok, "turn_min_v1", 1));
    let sent = solves(&r.state);
    assert_eq!((sent.len(), sent[1].tree.template_id.as_str()), (2, "turn_min_v1"));
    assert_eq!(r.clock.now_ms(), 6_200);
    // the retry gets only the time remaining to final delivery, minus the delivery and pipe margins
    assert_eq!(sent[1].deadline_ms, 15_000 - 6_200 - 150);
    assert!(out.street_violation, "no first-attempt terminal arrived before t0 + 6 s");
}

#[test]
fn error_codes_retry_policy() {
    // tree_too_large twice: TreeTooLarge with the retry's estimate
    let mut r = rig(Street::Turn, vec![ack(), err("tree_too_large", false, Some(9_000_000_000)), ack(), err("tree_too_large", false, Some(3_000_000_000))]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert!(matches!(out.terminal, Terminal::Failed(UnsupportedReason::TreeTooLarge { estimate_bytes: 3_000_000_000 })));
    assert_eq!(solves(&r.state).len(), 2);
    assert!(!out.street_violation, "the first attempt produced a terminal well inside the 6 s budget");
    // no_iteration: not retried on the same template, the _min template is tried
    let mut r = rig(Street::Turn, vec![ack(), err("no_iteration", false, None), ack(), ok_for(Street::Turn, "turn_min_v1", 0.4)]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.template_used.as_str(), out.restarts), (Terminal::Ok, "turn_min_v1", 0));
    // tree_mismatch: never retried, non-retryable EngineError, exactly one solve
    let mut r = rig(Street::River, vec![ack(), err("tree_mismatch", false, None)]);
    assert!(matches!(run_solve(&mut r.core, &r.input, &r.plan, &r.sink).terminal, Terminal::Failed(UnsupportedReason::EngineError { retryable: false, .. })));
    assert_eq!(solves(&r.state).len(), 1);
    // worker exit: restart and retry with _min; a second failure is a retryable EngineError (P2T23-I1: two confirmed
    // exits, at 1 ms and 2 ms, well before the watchdog's cutoff; an end returned at the cutoff is the watchdog's, see
    // `an_end_of_stdout_returned_at_the_fire_is_left_to_the_delivery`)
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::Exit { code: 3 }, ack(), FakeReply::Delay { ms: 1 }, FakeReply::Exit { code: 4 }]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert!(matches!(out.terminal, Terminal::Failed(UnsupportedReason::EngineError { ref message, retryable: true }) if message.contains("WorkerExit{code: 4}")));
    assert_eq!((out.restarts, r.state.lock().unwrap().restarts, r.clock.now_ms()), (2, 2, 2));
    // a rejected ack (busy) frees nothing by itself: the outcome is a retryable EngineError and no result was accepted
    let mut r = rig(Street::River, vec![FakeReply::Ack { id: IdRef::Last, status: AckStatus::Rejected, reason: Some("busy".into()) }]);
    assert!(matches!(run_solve(&mut r.core, &r.input, &r.plan, &r.sink).terminal, Terminal::Failed(UnsupportedReason::EngineError { retryable: true, .. })));
}

// --- Task 23 beyond the brief's three tests: one behaviour per test for the parts of the resilience policy they do not
// reach (the cancel's bound and its one confirmation, the relaunch of a missing worker and the ready revalidation after
// a restart, retry admission at its boundary, what a retry keeps and what it never sends). Every wait is on the fake
// clock; nothing sleeps or spins. ---

/// §7/§12 cancel-then-kill, exactly. The client observes the mutation at 101 ms (`InvalidateIdentity, Delay { ms: 1 }`
/// ends that receive) and cancels the superseded solve at once, with a request id of its own. Only that solve's
/// `result{cancelled}` confirms the cancel: the worker is then left running, nothing killed. Anything else read in the
/// window is discarded, a late reply of the job included (§4.5: a completion racing a cancel never also yields
/// `cancelled`), and the worker is killed and restarted exactly `CANCEL_KILL_MS` after the cancel. A cancel the link
/// cannot send (the worker has exited) is answered by a restart at once. Nothing is emitted for the superseded decision.
#[test]
fn a_cancel_is_confirmed_only_by_result_cancelled_else_the_worker_is_killed_1_5s_after_it() {
    let superseded_at_101 = |tail: Vec<FakeReply>| [vec![ack(), FakeReply::Delay { ms: 100 }, FakeReply::InvalidateIdentity, FakeReply::Delay { ms: 1 }], tail].concat();
    let cancelled = FakeReply::Result { id: IdRef::Last, status: ResultStatus::Cancelled, solution: None, error: None, elapsed_ms: 200 };
    let cancel_ack = FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None };
    // (case, script after the mutation, whether a cancel was sent, kills, restarts, the clock at return)
    let cases = [
        ("no reply", vec![FakeReply::Hang], true, 1u32, 1u32, 1_601u64),
        ("result{cancelled} at 301 ms", vec![cancel_ack, FakeReply::Delay { ms: 200 }, cancelled, FakeReply::Hang], true, 0, 0, 301),
        ("the job's late ok", vec![ok_for(Street::River, "river_std_v1", 0.3), FakeReply::Hang], true, 1, 1, 1_601),
        ("the worker has exited", vec![FakeReply::Exit { code: 3 }], false, 0, 1, 101),
    ];
    for (case, tail, cancel_sent, kills, restarts, now) in cases {
        let mut r = rig(Street::River, superseded_at_101(tail));
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.contains("superseded") && !retryable && out.solution.is_none(), "{case}: {message}");
        assert!(r.events.lock().unwrap().is_empty(), "{case}: nothing is emitted for a superseded decision");
        let solve_id = solves(&r.state)[0].id.clone();
        let s = r.state.lock().unwrap();
        let cancel = s.sent.iter().find_map(|m| if let EngineMessage::Cancel { id, target } = m { Some((id.clone(), target.clone())) } else { None });
        assert_eq!(cancel.is_some(), cancel_sent, "{case}");
        if let Some((id, target)) = cancel { assert!(target == solve_id && id != solve_id, "{case}: cancel {id} of {target}, solve {solve_id}"); }
        assert_eq!((s.kills, s.restarts, u32::from(out.restarts), r.clock.now_ms()), (kills, restarts, restarts, now), "{case}");
    }
}

/// A link over the scripted worker whose relaunched worker writes `relaunched` as its `ready` (a rebuilt binary without
/// AVX2, say): what the engine sees after any restart.
struct RelaunchedAs { inner: Box<dyn WorkerLink>, relaunched: Ready, restarted: bool }
impl WorkerLink for RelaunchedAs {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { self.inner.send(msg) }
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> { self.inner.recv(timeout) }
    fn restart(&mut self) -> Result<(), WorkerLinkError> { self.restarted = true; self.inner.restart() }
    fn kill(&mut self) { self.inner.kill() }
    fn ready(&self) -> Option<&Ready> { if self.restarted { self.inner.ready().map(|_| &self.relaunched) } else { self.inner.ready() } }
}
fn without_avx2() -> Ready { let mut r = FakeWorker::default_ready(); r.build_features = vec!["sse4.2".into()]; r }

/// A link with no live worker whose restart reports success and still leaves none (a broken link contract): nothing
/// may be sent to it, waited on or killed.
struct RestartsToNothing;
impl WorkerLink for RestartsToNothing {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { panic!("a request reached a link with no live worker: {msg:?}") }
    fn recv(&mut self, _timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> { panic!("the client waited on a link with no live worker") }
    fn restart(&mut self) -> Result<(), WorkerLinkError> { Ok(()) }
    fn kill(&mut self) { panic!("the client killed a link with no live worker") }
    fn ready(&self) -> Option<&Ready> { None }
}

/// Decision 4 (review P2T22R): a request that finds no live worker (killed, or left without one by a restart that
/// failed) relaunches it once before anything is sent and validates the relaunched worker's `ready`. Relaunched, the
/// solve goes ahead. A relaunch that fails ends it with a non-retryable `EngineError` naming the failure (P2T23-I3):
/// both a launch that never became ready and the process link's own refusal of the relaunched worker's `ready`
/// (`worker::process`, `Spawn("<exe>: ready refused: <reason>")`), which the engine never sees as a `ready`. A relaunched
/// worker whose `ready` the link reports and the engine refuses (no AVX2, spec 3.6/3.7) ends it with the non-retryable
/// version mismatch of §12.
#[test]
fn a_missing_worker_is_relaunched_once_before_the_request() {
    let mut r = rig(Street::River, vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    r.core.worker.kill();
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.restarts, kills_and_restarts(&r.state), solves(&r.state).len()), (Terminal::Ok, 1, (1, 1), 1));

    for spawn_failure in [DID_NOT_BECOME_READY, READY_REFUSED] {
        let mut r = rig(Street::River, vec![FakeReply::SpawnFails(spawn_failure.into())]);
        r.core.worker.kill();
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.contains("no live worker") && message.contains(spawn_failure) && !retryable, "{message}");
        assert_eq!((out.restarts, kills_and_restarts(&r.state), solves(&r.state).len(), r.clock.now_ms()), (1, (1, 1), 0, 0));
        assert!(r.core.worker.ready().is_none(), "no live worker is left; the next request tries again");
    }

    // a relaunch that reports success and leaves no live worker is a failed restart too
    let mut r = rig(Street::River, vec![]);
    let log = DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log"));
    let identity = r.core.identity.clone();
    r.core = EngineCore::new(Box::new(RestartsToNothing), r.clock.clone(), identity, log);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("no live worker") && !retryable, "{message}");
    assert_eq!((out.restarts, r.clock.now_ms()), (1, 0));

    let mut r = rig(Street::River, vec![]);
    let identity = r.core.identity.clone();
    let (mut worker, state) = FakeWorker::scripted(r.clock.clone(), identity.clone(), vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    worker.kill();
    let link = RelaunchedAs { inner: worker, relaunched: without_avx2(), restarted: false };
    r.core = EngineCore::new(Box::new(link), r.clock.clone(), identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
    r.state = state;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("worker/proto version mismatch") && message.contains("AVX2") && !retryable, "{message}");
    assert_eq!((out.restarts, kills_and_restarts(&r.state), solves(&r.state).len()), (1, (1, 1), 0));
}

/// The process link's failures of a relaunch (`worker::process::ProcessWorker::start`): no launch became ready, or the
/// relaunched worker's `ready` was refused, which the link reports as `Spawn` after killing that worker (the engine
/// never sees the refused `ready`).
const DID_NOT_BECOME_READY: &str = r"D:\PokerAI\solver-worker.exe did not become ready after 2 attempts (attempt 1: spawn: startup timeout; attempt 2: spawn: startup timeout)";
const READY_REFUSED: &str = r"D:\PokerAI\solver-worker.exe: ready refused: worker built without AVX2";

/// Decision 4 mid-request (P2T23-I3): after a failed attempt, a restart that fails leaves no live worker and ends the
/// solve with a non-retryable `EngineError` naming both causes, whether no launch became ready or the process link
/// refused the relaunched worker's `ready` (no retry is sent; the next request relaunches the worker once). A restarted
/// worker whose `ready` the link reports and the engine refuses ends it with the non-retryable version mismatch, never
/// a retry on it.
#[test]
fn a_failed_restart_or_a_refused_ready_after_a_restart_ends_the_solve() {
    let exits = || vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::Exit { code: 3 }];
    for spawn_failure in [DID_NOT_BECOME_READY, READY_REFUSED] {
        let mut r = rig(Street::River, [exits(), vec![FakeReply::SpawnFails(spawn_failure.into())]].concat());
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.contains("WorkerExit{code: 3}") && message.contains(spawn_failure) && !retryable, "{message}");
        assert_eq!((out.restarts, kills_and_restarts(&r.state), solves(&r.state).len(), r.clock.now_ms()), (1, (0, 1), 1, 1));
        assert!(r.core.worker.ready().is_none());
    }

    let mut r = rig(Street::River, vec![]);
    let identity = r.core.identity.clone();
    let (worker, state) = FakeWorker::scripted(r.clock.clone(), identity.clone(), [exits(), vec![ack(), ok_for(Street::River, "river_min_v1", 0.3)]].concat());
    let link = RelaunchedAs { inner: worker, relaunched: without_avx2(), restarted: false };
    r.core = EngineCore::new(Box::new(link), r.clock.clone(), identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
    r.state = state;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("worker/proto version mismatch") && message.contains("AVX2") && !retryable, "{message}");
    assert_eq!((out.restarts, kills_and_restarts(&r.state), solves(&r.state).len()), (1, (0, 1), 1));
}

/// §7 retry admission (decision 5): the `_min` retry is admitted only if what is left until final delivery covers the
/// template's p95 (the street budget until plan 4's bench matrix exists) plus the delivery, pipe and extraction margins,
/// and it gets only what is left until final delivery less the delivery and pipe margins. River with final delivery at
/// 4 s: 2 000 + 100 + 50 + 200 fits from 1 650 ms, not from 1 651 ms. Without a `_min` template there is no retry.
#[test]
fn the_min_retry_is_admitted_only_when_its_p95_and_the_margins_fit() {
    let short_final = Deadlines { t0_ms: 0, street_deadline_ms: 2_000, final_delivery_ms: 4_000, extraction_margin_ms: 200 };
    for (at, admitted) in [(1_650u64, true), (1_651, false)] {
        let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: at }, err("no_iteration", false, None), ack(), ok_for(Street::River, "river_min_v1", 0.3)]);
        set_deadlines(&mut r.plan, short_final);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let sent = solves(&r.state);
        if admitted {
            assert_eq!((out.terminal, out.template_used.as_str(), sent.len()), (Terminal::Ok, "river_min_v1", 2), "no_iteration at {at} ms");
            assert_eq!((sent[1].deadline_ms, sent[1].tree.template_id.as_str()), (4_000 - 1_650 - 150, "river_min_v1"));
        } else {
            assert_eq!((out.terminal, out.template_used.as_str(), sent.len()), (deadline_exceeded("solving"), "river_std_v1", 1), "no_iteration at {at} ms");
        }
    }
    let mut r = rig(Street::River, vec![ack(), err("no_iteration", false, None), ack(), ok_for(Street::River, "river_min_v1", 0.3)]);
    r.plan.retry_template_id = None;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, solves(&r.state).len()), (deadline_exceeded("solving"), 1));
}

/// Decisions 2 and 5: a retry never rewinds the reported stage (`set_stage` only advances), and never replaces the first
/// attempt's terminal: `first_terminal_ms` and the street deadline's published arrival stay attempt 0's, or stay `None`
/// when attempt 0 had none, however the retry ends; the street verdict is judged from them (ruling 20-I1).
#[test]
fn a_retry_keeps_the_furthest_stage_and_the_first_attempts_terminal() {
    let extracting = FakeReply::Progress { id: IdRef::Last, stage: Stage::Extracting, iterations: 40, exploitability_chips: Some(0.4), elapsed_ms: 1 };
    let mut r = rig(Street::River, vec![ack(), extracting, FakeReply::Delay { ms: 300 }, err("tree_too_large", false, Some(9_000_000_000)),
        FakeReply::Delay { ms: 400 }, ack(), ok_for(Street::River, "river_min_v1", 0.3)]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.template_used.as_str(), r.core.stage(), r.clock.now_ms()), (Terminal::Ok, "river_min_v1", "extracting".to_string(), 700));
    assert_eq!((out.first_terminal_ms, r.plan.street_deadline.terminal_arrival_ms(), out.street_violation), (Some(300), Some(300), false));
    // attempt 0 has no terminal (the worker exits at 1 ms); the retry's terminal, at 2 501 ms, is never published
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::Exit { code: 3 }, ack(), FakeReply::Delay { ms: 2_500 }, ok_for(Street::River, "river_min_v1", 0.3)]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.restarts, r.clock.now_ms()), (Terminal::Ok, 1, 2_501));
    assert_eq!((out.first_terminal_ms, r.plan.street_deadline.terminal_arrival_ms(), out.street_violation), (None, None, true));
}

/// §12 heartbeat: only while the worker reports `Solving`, measured from its last `progress` (each resets it). No
/// progress for 5 s after one at 4 999 ms ends the attempt at 9 999 ms, not at 5 000 ms; silence in `Building` or after
/// `Extracting` is bounded by the attempt's hang bound alone. River with the street deadline at 14 s (hang bound
/// 14 500 ms, fire 14 900 ms), so no retry fits after either failure (the street budget, 14 s, is the p95 proxy).
#[test]
fn the_heartbeat_runs_only_while_solving_from_the_last_progress() {
    let progress = |stage| FakeReply::Progress { id: IdRef::Last, stage, iterations: 1, exploitability_chips: None, elapsed_ms: 1 };
    let long = Deadlines { t0_ms: 0, street_deadline_ms: 14_000, final_delivery_ms: 15_000, extraction_margin_ms: 200 };
    let cases = [
        (vec![ack(), progress(Stage::Solving), FakeReply::Delay { ms: 4_999 }, progress(Stage::Solving), FakeReply::Hang], "heartbeat", 9_999u64),
        (vec![ack(), progress(Stage::Solving), FakeReply::Delay { ms: 4_999 }, progress(Stage::Extracting), FakeReply::Hang], "no terminal result", 14_500),
        (vec![ack(), progress(Stage::Building), FakeReply::Hang], "no terminal result", 14_500),
    ];
    for (script, fragment, ended_ms) in cases {
        let mut r = rig(Street::River, script);
        set_deadlines(&mut r.plan, long);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.contains(fragment) && retryable, "{message}");
        assert_eq!((r.clock.now_ms(), out.restarts, kills_and_restarts(&r.state), solves(&r.state).len()), (ended_ms, 1, (0, 1), 1), "{fragment}");
    }
}

/// A link over the scripted worker through which a mutation arrives while the worker is restarted.
struct MutatesOnRestart { inner: Box<dyn WorkerLink>, identity: Arc<Mutex<IdentityState>> }
impl WorkerLink for MutatesOnRestart {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { self.inner.send(msg) }
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> { self.inner.recv(timeout) }
    fn restart(&mut self) -> Result<(), WorkerLinkError> { let restarted = self.inner.restart(); self.identity.lock().unwrap().cancel_active(); restarted }
    fn kill(&mut self) { self.inner.kill() }
    fn ready(&self) -> Option<&Ready> { self.inner.ready() }
}

/// Decision 3 for the retry (ruling 22-I2): a decision superseded while the worker is restarted after a failed attempt
/// starts no retry. Nothing more is sent, and since nothing was sent there is nothing to cancel or kill.
#[test]
fn a_decision_superseded_before_the_retry_is_sent_starts_no_retry() {
    let mut r = rig(Street::River, vec![]);
    let identity = r.core.identity.clone();
    let (worker, state) = FakeWorker::scripted(r.clock.clone(), identity.clone(), vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::Exit { code: 3 }, ack(), ok_for(Street::River, "river_min_v1", 0.3)]);
    r.core = EngineCore::new(Box::new(MutatesOnRestart { inner: worker, identity: identity.clone() }), r.clock.clone(), identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
    r.state = state;
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("superseded") && !retryable, "{message}");
    let s = r.state.lock().unwrap();
    assert_eq!((s.sent.len(), s.cancels.len(), s.kills, s.restarts, out.restarts, r.clock.now_ms()), (1, 0, 0, 1, 1, 1));
}

/// A worker that never answers (plan 2 Task 29's case (a) at the client): the first attempt hangs to its bound, the
/// worker is restarted and the `_min` retry is sent with what is left until final delivery; it runs on to the watchdog's
/// fire, where it ends `DeadlineExceeded` with the furthest stage. The client neither kills nor restarts anything then:
/// the watchdog delivers the `Final`, and the cleanup after it is `serve_request`'s (Task 28).
#[test]
fn a_retry_cut_by_the_watchdog_fire_leaves_the_cleanup_to_the_delivery() {
    let mut r = rig(Street::River, vec![ack(), FakeReply::Hang]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let sent = solves(&r.state);
    assert_eq!((out.terminal, out.template_used.as_str(), r.clock.now_ms()), (deadline_exceeded("building"), "river_min_v1", 14_900));
    assert_eq!((sent.len(), sent[1].deadline_ms, out.restarts, kills_and_restarts(&r.state)), (2, 15_000 - 2_500 - 150, 1, (0, 1)));
    assert!(out.street_violation && out.first_terminal_ms.is_none());
}

// --- Fix round 1 (review P2T23R): expiry and identity on every receive result (P2T23-I1), the cancel bound judged
// after the receive (P2T23-I2), a failed restart is not retryable (P2T23-I3), the protocol-error retry (P2T23-M1). ---

/// P2T23-I1: identity and the watchdog's cutoff are judged on every receive result, a link failure included, before it
/// is classified or anything is restarted. An unconfirmed end of stdout spends its whole receive, so it is returned at
/// the earlier of the hang bound and the fire. Returned at the fire, the attempt ends `DeadlinePassed`: no restart (the
/// cleanup after the `Final` is `serve_request`'s), no payload, and the first attempt's terminal state as it was.
/// Returned 1 ms before it, the end is classified as before (`Ended`, a restart).
#[test]
fn an_end_of_stdout_returned_at_the_fire_is_left_to_the_delivery() {
    // the brief's two unconfirmed ends: the first at the hang bound (2 500 ms, restart), the retry's at the fire
    let mut r = rig(Street::River, vec![ack(), FakeReply::Eof, ack(), FakeReply::Eof]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.solution.is_none(), out.restarts, kills_and_restarts(&r.state), r.clock.now_ms()), (deadline_exceeded("building"), true, 1, (0, 1), 14_900));
    assert_eq!((out.first_terminal_ms, r.plan.street_deadline.terminal_arrival_ms(), solves(&r.state).len()), (None, None, 2));
    // the first attempt's terminal arrived (no_iteration at 100 ms): the retry's end at the fire leaves it as it was
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 100 }, err("no_iteration", false, None), ack(), FakeReply::Eof]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.solution.is_none(), out.restarts, kills_and_restarts(&r.state), r.clock.now_ms()), (deadline_exceeded("building"), true, 0, (0, 0), 14_900));
    assert_eq!((out.first_terminal_ms, r.plan.street_deadline.terminal_arrival_ms(), out.street_violation), (Some(100), Some(100), false));
    // the boundary on one attempt: the end is returned at its hang bound, 2 500 ms, with the fire at 2 500 or 2 501 ms
    for (final_delivery_ms, at_fire) in [(2_600u64, true), (2_601, false)] {
        let mut r = rig(Street::River, vec![ack(), FakeReply::Eof]);
        set_deadlines(&mut r.plan, Deadlines { t0_ms: 0, street_deadline_ms: 2_000, final_delivery_ms, extraction_margin_ms: 200 });
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        assert_eq!(r.clock.now_ms(), 2_500, "final delivery at {final_delivery_ms} ms");
        if at_fire {
            assert_eq!((out.terminal, out.restarts, kills_and_restarts(&r.state)), (deadline_exceeded("building"), 0, (0, 0)));
        } else {
            let (message, retryable) = failed_engine_error(&out.terminal);
            assert!(message.contains("not confirmed") && retryable, "{message}");
            assert_eq!((out.restarts, kills_and_restarts(&r.state)), (1, (0, 1)));
        }
    }
}

/// The scripted worker behind a link that loses time between a failure on the pipe and the client: when `recv` is about
/// to return an error, the fake clock first jumps to `resume_at_ms` (a suspend or a stalled thread).
struct StallsOnFailure { inner: Box<dyn WorkerLink>, clock: Arc<FakeClock>, resume_at_ms: u64 }
impl WorkerLink for StallsOnFailure {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { self.inner.send(msg) }
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        let got = self.inner.recv(timeout);
        if got.is_err() { self.clock.set_ms(self.resume_at_ms); }
        got
    }
    fn restart(&mut self) -> Result<(), WorkerLinkError> { self.inner.restart() }
    fn kill(&mut self) { self.inner.kill() }
    fn ready(&self) -> Option<&Ready> { self.inner.ready() }
}

/// P2T23-I1 for every kind of failure and for identity. A confirmed exit or a faulty line written at 1 ms but observed
/// at the watchdog's fire ends the attempt `DeadlinePassed` and restarts nothing; observed 1 ms before the fire it is
/// classified as before (a restart, and no retry fits). A failure that arrives with a mutation is the superseded
/// decision's: its job is cancelled (then killed), and no worker failure of the live decision is reported.
#[test]
fn a_link_failure_is_judged_by_identity_and_the_fire_before_it_is_classified() {
    for (failure, fragment) in [(FakeReply::Exit { code: 3 }, "WorkerExit{code: 3}"), (FakeReply::Malformed("{\"type\":".into()), "protocol error")] {
        for (seen_at, at_fire) in [(14_900u64, true), (14_899, false)] {
            let mut r = rig(Street::River, vec![]);
            let identity = r.core.identity.clone();
            let (worker, state) = FakeWorker::scripted(r.clock.clone(), identity.clone(), vec![ack(), FakeReply::Delay { ms: 1 }, failure.clone()]);
            let link = StallsOnFailure { inner: worker, clock: r.clock.clone(), resume_at_ms: seen_at };
            r.core = EngineCore::new(Box::new(link), r.clock.clone(), identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
            r.state = state;
            let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
            if at_fire {
                assert_eq!((out.terminal, out.restarts, kills_and_restarts(&r.state)), (deadline_exceeded("building"), 0, (0, 0)), "{fragment} seen at {seen_at} ms");
            } else {
                let (message, retryable) = failed_engine_error(&out.terminal);
                assert!(message.contains(fragment) && retryable, "seen at {seen_at} ms: {message}");
                assert_eq!((out.restarts, kills_and_restarts(&r.state), solves(&r.state).len()), (1, (0, 1), 1), "{fragment} seen at {seen_at} ms");
            }
        }
    }
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::InvalidateIdentity, FakeReply::Malformed("{".into()), FakeReply::Hang]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("superseded") && !retryable, "{message}");
    let sent = solves(&r.state).len();
    let s = r.state.lock().unwrap();
    assert_eq!((sent, s.cancels.len(), s.kills, s.restarts, r.clock.now_ms()), (1, 1, 1, 1, 1 + 1_500));
}

/// P2T23-M1: a protocol error on the first attempt restarts the worker and admits the `_min` retry (§12), which then
/// succeeds: two requests, the second on `river_min_v1` with what is left until final delivery less the delivery and
/// pipe margins, exactly one restart, and nothing of the faulty reply forwarded (a negative exploitability, a line that
/// is not a message).
#[test]
fn a_protocol_error_restarts_the_worker_and_the_min_retry_succeeds() {
    let negative = FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 1, exploitability_chips: Some(-0.5), elapsed_ms: 1 };
    for (case, faulty) in [("a negative exploitability", negative), ("a line that is not a message", FakeReply::Malformed("{\"type\":".into()))] {
        let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 300 }, faulty, ack(), ok_for(Street::River, "river_min_v1", 0.3)]);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let sent = solves(&r.state);
        assert_eq!((out.terminal, out.template_used.as_str(), out.restarts, kills_and_restarts(&r.state), sent.len()), (Terminal::Ok, "river_min_v1", 1, (0, 1), 2), "{case}");
        assert_eq!((sent[1].tree.template_id.as_str(), sent[1].deadline_ms, r.clock.now_ms()), ("river_min_v1", 15_000 - 300 - 150, 300), "{case}");
        assert!(r.events.lock().unwrap().is_empty(), "{case}: nothing of the faulty reply is forwarded");
    }
}

fn is_cancelled(m: &WorkerMessage) -> bool { matches!(m, WorkerMessage::Result { status: ResultStatus::Cancelled, .. }) }

/// P2T23-I2: the cancel's 1.5 s bound is judged at the engine-clock time the client observes the confirmation, not by
/// the bound its receive was given (a suspend, a stalled thread). The job's `result{cancelled}`, written at 301 ms for
/// a cancel sent at 101 ms, confirms the cancel when observed at 1 600 ms (nothing killed); observed at 1 601 ms, the
/// bound, it is too late and the worker is killed and restarted. Nothing is emitted for the superseded decision.
#[test]
fn a_cancel_confirmation_observed_after_the_bound_is_answered_by_a_kill() {
    let cancelled = FakeReply::Result { id: IdRef::Last, status: ResultStatus::Cancelled, solution: None, error: None, elapsed_ms: 200 };
    for (seen_at, confirmed) in [(1_600u64, true), (1_601, false)] {
        let script = vec![ack(), FakeReply::Delay { ms: 100 }, FakeReply::InvalidateIdentity, FakeReply::Delay { ms: 1 }, FakeReply::Delay { ms: 200 }, cancelled.clone(), FakeReply::Hang];
        let mut r = stalled_rig(script, is_cancelled, Some(seen_at), 0);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.contains("superseded") && !retryable, "seen at {seen_at} ms: {message}");
        assert!(r.events.lock().unwrap().is_empty(), "seen at {seen_at} ms: nothing is emitted for a superseded decision");
        let cancels = r.state.lock().unwrap().cancels.len();
        let expected = if confirmed { (0, (0, 0)) } else { (1, (1, 1)) };
        assert_eq!((cancels, out.restarts, kills_and_restarts(&r.state), r.clock.now_ms()), (1, expected.0, expected.1, seen_at), "confirmation seen at {seen_at} ms");
    }
}
