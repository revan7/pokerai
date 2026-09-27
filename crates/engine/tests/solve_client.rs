use engine::core::EngineCore;
use engine::deadline::Deadlines;
use engine::identity::IdentityState;
use engine::log::DecisionLog;
use engine::solve::{run_solve, SolvePlan, Terminal};
use engine::testing::{uniform_solution, FakeClock, FakeReply, FakeWorker, IdRef, RecordingSink};
use engine::tree::{build_tree_full, TemplateSelection};
use engine::watchdog::SharedSink;
use proto::worker::{AckStatus, EngineMessage, ResultStatus, Stage, WorkerError};
use proto::{Action, Card, Range1326, Rake, RecommendationEvent, Seat, SolveInput, Street, StreetRootSnapshot, UnsupportedReason};
use std::sync::{Arc, Mutex};

fn snap(street: Street) -> StreetRootSnapshot {
    let board = match street { Street::River => "Qs Jd 7h 3c 2d", _ => "Qs Jd 7h 3c" };
    StreetRootSnapshot { street, board: board.split(' ').map(|s| Card::parse(s).unwrap()).collect(), oop: Seat(2), ip: Seat(0), pot_root: 100,
        stack_oop_root: 100, stack_ip_root: 100, dead_this_street: 0, projected_from: 2, history: vec![], bb_chips: 2 }
}
fn full_range(board: &[Card]) -> Range1326 { let mut r = Range1326([1.0; 1326]); for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { r.0[i] = 0.0; } } r }
struct Rig { core: EngineCore, input: SolveInput, plan: SolvePlan, sink: SharedSink, events: Arc<Mutex<Vec<engine::testing::Recorded>>>, state: Arc<Mutex<engine::testing::FakeState>>, clock: Arc<FakeClock> }
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
    let plan = SolvePlan { identity: id, deadlines: Deadlines::for_request(0, street, 10), template_id: template.into(), retry_template_id: engine::tree::Templates::min_variant(template).map(String::from), rake: Rake::TimeCharge, hero_actor: "oop".into(), background: false };
    let (sink, events) = RecordingSink::new(clock.clone(), Some(state.clone()));
    Rig { core, input, plan, sink: Arc::new(Mutex::new(Box::new(sink))), events, state, clock }
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
        f.plan.deadlines = Deadlines::for_request(0, Street::Flop, budget);
        f.plan.template_id = "flop_fast_v1".into();
        f.plan.retry_template_id = Some("flop_min_v1".into());
        f.clock.set_ms(500);
        assert_eq!(run_solve(&mut f.core, &f.input, &f.plan, &f.sink).terminal, Terminal::Ok, "flop budget {budget}");
        let req = &solves(&f.state)[0];
        assert_eq!((req.deadline_ms, req.extraction_margin_ms, req.board.len()), (expected, 600, 3), "flop budget {budget}");
    }
}

/// Ruling 26-Q4, decided here: `Terminal::Ok` iff the raw exploitability meets the raw target, compared exactly
/// (`expl * 10_000 <= target_bp * pot` in f64, the predicate of `assemble::coverage_for_solve`), whatever the worker's
/// `ok`/`best_so_far` said. The worker stops at an f32-rounded target: at 30 bp of a 100-chip pot that is 0.3f32, one
/// half-ulp above the raw 0.3 chips, so its `ok` at 0.3f32 missed the raw target and is `BestSoFar` here; an `ok` never
/// reaches assembly as the unexplained `Approximate { reasons: [] }`.
#[test]
fn q4_the_raw_target_decides_ok_and_best_so_far() {
    let tree = river_tree();
    let run = |status: ResultStatus, expl: f32, target_bp: u16| {
        let mut r = rig(Street::River, vec![ack(), result_for(&tree, status, &[], expl)]);
        r.input.target_bp = target_bp;
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let coverage = engine::assemble::coverage_for_solve(expl, 100, target_bp, out.terminal == Terminal::BestSoFar, vec![]);
        assert_ne!(coverage, proto::Coverage::Approximate { reasons: vec![] }, "{status:?} at {expl} of a {target_bp} bp target");
        (out.terminal, out.reached_bp, coverage)
    };
    // the worker's own threshold admits 0.3f32 at 30 bp; the raw comparison does not
    assert!((f64::from(100u32) * 30.0 / 10_000.0) as f32 >= 0.3f32 && f64::from(0.3f32) * 10_000.0 > 30.0 * 100.0);
    let (terminal, bp, coverage) = run(ResultStatus::Ok, 0.3, 30);
    assert_eq!((terminal, bp), (Terminal::BestSoFar, Some(30)));
    assert_eq!(coverage, proto::Coverage::Approximate { reasons: vec![proto::ApproxReason::DeadlineBestSoFar { reached_bp: 30, target_bp: 30 }] });
    // exactly at a representable raw target: Ok and Exact
    assert_eq!(run(ResultStatus::Ok, 0.5, 50), (Terminal::Ok, Some(50), proto::Coverage::Exact));
    // a best_so_far that nevertheless meets the raw target is Ok; one above it stays BestSoFar
    assert_eq!(run(ResultStatus::BestSoFar, 0.3, 50).0, Terminal::Ok);
    assert_eq!(run(ResultStatus::BestSoFar, 1.9, 50).0, Terminal::BestSoFar);
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
    // no terminal: the worker hangs, the attempt ends at the hang bound (sent 0 + 1 850 + 150 + 500 = 2 500 ms)
    let mut r = rig(Street::River, vec![ack(), FakeReply::Hang]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert!(out.street_violation && r.clock.now_ms() == 2_500);
}

/// The client never waits past a deadline of its own: `recv` is given only what is left until the attempt's hang bound
/// or the watchdog's fire, whichever is first, and at the fire the attempt ends with the stage reached (the watchdog's
/// `Final` goes out independently, §7).
#[test]
fn receives_never_outlast_the_hang_bound_or_the_watchdog_fire() {
    // a hung worker: the attempt ends exactly at the hang bound and the worker is restarted
    let mut r = rig(Street::River, vec![ack(), FakeReply::Hang]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("no terminal result") && retryable, "{message}");
    assert_eq!((r.clock.now_ms(), out.restarts, kills_and_restarts(&r.state)), (2_500, 1, (0, 1)));
    // a watchdog fire before the hang bound: t0 0, street 2 000, final delivery 2 300, fire 2 200
    let mut r = rig(Street::River, vec![ack(), FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 1, exploitability_chips: None, elapsed_ms: 1 }, FakeReply::Hang]);
    r.plan.deadlines = Deadlines { t0_ms: 0, street_deadline_ms: 2_000, final_delivery_ms: 2_300, extraction_margin_ms: 200 };
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

/// A link whose `ready` the client refuses: any request, receive, kill or restart on it is a test failure.
struct NoWork(Option<Ready>);
impl WorkerLink for NoWork {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> { panic!("a request reached a worker whose ready was refused: {msg:?}") }
    fn recv(&mut self, _timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> { panic!("the client waited on a worker whose ready was refused") }
    fn restart(&mut self) -> Result<(), WorkerLinkError> { panic!("the client restarted a worker whose ready was refused") }
    fn kill(&mut self) { panic!("the client killed a worker whose ready was refused") }
    fn ready(&self) -> Option<&Ready> { self.0.as_ref() }
}

/// §4.5/§12: the worker's `ready` is validated before any request, and a refused solve starts no work: no request, no
/// wait, no restart. A version, commit or AVX2 mismatch is a non-retryable `EngineError`; no live worker is retryable.
#[test]
fn ready_is_validated_before_any_request() {
    let mut no_avx2 = FakeWorker::default_ready();
    no_avx2.build_features = vec!["sse4.2".into()];
    let mut old_proto = FakeWorker::default_ready();
    old_proto.proto_version = 2;
    for (ready, fragment, retryable) in [(Some(no_avx2), "AVX2", false), (Some(old_proto), "proto_version 2", false), (None, "no live worker", true)] {
        let mut r = rig(Street::River, vec![]);
        let log = DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log"));
        let identity = r.core.identity.clone();
        r.core = EngineCore::new(Box::new(NoWork(ready)), r.clock.clone(), identity, log);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, got_retryable) = failed_engine_error(&out.terminal);
        assert!(message.contains(fragment) && got_retryable == retryable, "{message}");
        assert_eq!((r.clock.now_ms(), out.restarts), (0, 0));
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
/// emitted from it.
#[test]
fn protocol_violations_restart_the_worker() {
    let negative = FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 1, exploitability_chips: Some(-0.5), elapsed_ms: 1 };
    let ok_without_solution = FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: None, error: None, elapsed_ms: 1 };
    let staged = FakeReply::Ack { id: IdRef::Last, status: AckStatus::Staged, reason: None };
    for (script, fragment) in [(vec![ack(), negative], "negative"), (vec![ack(), ok_without_solution], "without a solution"),
        (vec![ack(), err("boom", true, None)], "boom"), (vec![staged], "staged")] {
        let mut r = rig(Street::River, script);
        let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
        let (message, retryable) = failed_engine_error(&out.terminal);
        assert!(message.starts_with("protocol error") && message.contains(fragment) && retryable, "{message}");
        assert_eq!((out.restarts, kills_and_restarts(&r.state)), (1, (0, 1)));
        assert!(r.events.lock().unwrap().is_empty());
    }
}

/// Worker death (ruling 18): a confirmed exit is `WorkerExit{code}`; an end of stdout whose exit is never confirmed is
/// not given a code. Both restart the worker (kill, reap, respawn, validate `ready`).
#[test]
fn a_worker_exit_and_an_unconfirmed_end_restart_the_worker() {
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::Exit { code: 3 }]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("WorkerExit{code: 3}") && retryable, "{message}");
    assert_eq!((out.restarts, kills_and_restarts(&r.state), r.clock.now_ms()), (1, (0, 1), 1));
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 1 }, FakeReply::Eof, FakeReply::Hang]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    let (message, retryable) = failed_engine_error(&out.terminal);
    assert!(message.contains("not confirmed") && !message.contains("WorkerExit") && retryable, "{message}");
    assert_eq!((out.restarts, kills_and_restarts(&r.state), r.clock.now_ms()), (1, (0, 1), 2_500));
    assert!(r.core.worker.ready().is_some(), "the restarted worker is live again");
}
