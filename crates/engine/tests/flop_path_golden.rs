//! Plan 4 Task 9 (spec 4.2, 7, 10.1, 13.3): the per-session flop budget and the evidence-based flop template policy.
//!
//! `flop_budget_setting_golden` is the spec 13.3 golden of that name: "fake clock: `flop_budget_s = 10` gives the
//! worker `deadline_ms = 10000 - 150` minus elapsed and the watchdog fires at `t0 + 14.9 s`; `flop_budget_s = 30`
//! gives the watchdog `t0 + 34.9 s` for a flop decision and still `t0 + 14.9 s` for a turn decision; 31 is rejected by
//! `set_config`". Every number comes from plan 2 Task 20's `Deadlines` (the only deadline arithmetic, cross-plan D5);
//! this plan adds only `deadline::flop_budget_valid`, a wrapper over `engine::engine::FLOP_BUDGET_RANGE`, the range
//! plan 2 Task 29's `Engine::set_config` validates with.
//!
//! The pure numbers are asserted on `Deadlines` directly; the rest of this file proves them over the production
//! wiring on the fake clock: the watchdog that `serve::admit` arms fires at exactly those instants, the wire
//! `deadline_ms` `run_solve` sends at 250 ms of elapsed time is `street budget - 150 - 250`, and a request admitted
//! before a config change keeps the deadlines it was admitted with.

// `pub`, so the shared helpers this binary never calls (plan 4 Task 8's `CacheRig`) are reachable rather than dead
// code, with no lint filter.
pub mod support;

use engine::clock::Clock;
use engine::core::EngineCore;
use engine::deadline::Deadlines;
use engine::identity::IdentityState;
use engine::log::DecisionLog;
use engine::serve::LiveRequest;
use engine::solve::{run_solve, SolvePlan, Terminal};
use engine::testing::{board, hand, play, uniform_solution, FakeClock, FakeReply, FakeWorker, IdRef, Recorder, RecordingSink};
use engine::tree::{build_tree_full, TemplateSelection};
use engine::watchdog::{SharedSink, StreetDeadline};
use proto::worker::{AckStatus, EngineMessage, ResultStatus, SolveRequest};
use proto::{Action, Card, Coverage, GameConfig, HandState, Range1326, Rake, RecommendationEvent, Seat, SolveInput, SolverPrefs, Street, StreetRootSnapshot, UnsupportedReason};
use std::sync::{Arc, Mutex};

#[test]
fn flop_budget_setting_golden() {
    use engine::deadline::{flop_budget_valid, Deadlines};
    use engine::flop::FlopPolicy;
    use proto::Street;
    let ten = Deadlines::for_request(1000, Street::Flop, 10);
    assert_eq!((ten.street_deadline_ms, ten.final_delivery_ms, ten.extraction_margin_ms), (11_000, 16_000, 600));
    assert_eq!(ten.watchdog_fire_ms(), 15_900);
    let thirty = Deadlines::for_request(1000, Street::Flop, 30);
    assert_eq!((thirty.street_deadline_ms, thirty.final_delivery_ms), (31_000, 36_000));
    assert_eq!(thirty.watchdog_fire_ms(), 35_900);
    // a turn decision keeps 6 s / 15 s / 14.9 s even at the maximum flop preference
    let turn = Deadlines::for_request(0, Street::Turn, 30);
    assert_eq!((turn.street_deadline_ms, turn.watchdog_fire_ms(), turn.extraction_margin_ms), (6_000, 14_900, 200));
    assert_eq!(Deadlines::for_request(0, Street::Flop, 10).watchdog_fire_ms(), 14_900);
    assert_eq!(Deadlines::for_request(0, Street::Flop, 30).watchdog_fire_ms(), 34_900);
    // the wire deadline at 250 ms of elapsed time (§7 margins 100 + 50)
    assert_eq!(Deadlines::for_request(0, Street::Flop, 10).worker_deadline_ms(250, 10_000), Some(9_600));
    assert_eq!(Deadlines::for_request(0, Street::Flop, 30).worker_deadline_ms(250, 30_000), Some(29_600));
    assert!(!flop_budget_valid(0));
    assert!(flop_budget_valid(1));
    assert!(flop_budget_valid(30));
    assert!(!flop_budget_valid(31));
    assert_eq!(FlopPolicy { min_admitted: false }.live_template(2), "flop_fast_v1");
    assert_eq!(FlopPolicy { min_admitted: true }.live_template(2), "flop_min_v1");
    assert_eq!(FlopPolicy { min_admitted: true }.live_template(3), "flop_fast_v1");
}

#[test]
fn set_config_rejects_out_of_range_flop_budget() {
    let mut engine = support::engine_with_fake_worker();
    let base = support::game_config();
    let accepted = engine.set_config(proto::GameConfig { solver: proto::SolverPrefs { threads: 16, target_bp: 50, flop_budget_s: 10 }, ..base.clone() });
    assert!(accepted.is_ok());
    for bad in [0_u8, 31, 255] {
        let cfg = proto::GameConfig { solver: proto::SolverPrefs { threads: 16, target_bp: 50, flop_budget_s: bad }, ..base.clone() };
        assert!(engine.set_config(cfg).is_err(), "flop_budget_s {bad} must be rejected");
    }
    let zero_threads = proto::GameConfig { solver: proto::SolverPrefs { threads: 0, target_bp: 50, flop_budget_s: 10 }, ..base.clone() };
    assert!(engine.set_config(zero_threads).is_err());
    // a rejected config leaves the previous revision and value in place
    assert_eq!(engine.config().solver.flop_budget_s, 10);
    let accepted = accepted.unwrap();
    assert_eq!(engine.config().config_revision, accepted, "no rejected config replaced the accepted revision");
    let next = engine.set_config(proto::GameConfig { solver: proto::SolverPrefs { threads: 16, target_bp: 50, flop_budget_s: 30 }, ..base });
    assert_eq!(next.unwrap(), accepted + 1, "the four rejected configs consumed no revision");
    assert_eq!(engine.config().solver.flop_budget_s, 30);
}

/// `Engine::set_config` accepts a flop budget exactly when `flop_budget_valid` does, over every `u8`: the validation
/// and the wrapper read the one range.
#[test]
fn set_config_accepts_exactly_the_valid_flop_budgets() {
    use engine::deadline::flop_budget_valid;
    let mut engine = support::engine_with_fake_worker();
    let base = support::game_config();
    let mut accepted = vec![];
    for b in 0..=u8::MAX {
        let ok = engine.set_config(GameConfig { solver: SolverPrefs { flop_budget_s: b, ..base.solver }, ..base.clone() }).is_ok();
        assert_eq!(ok, flop_budget_valid(b), "flop_budget_s {b}: set_config and flop_budget_valid disagree");
        if ok {
            accepted.push(b);
        }
    }
    assert_eq!(accepted, (1..=30).collect::<Vec<u8>>());
    assert_eq!(engine.config().solver.flop_budget_s, 30, "the last accepted budget is the configuration");
}

/// `Engine::config` is what the settings UI reads: the accepted next-hand configuration. While a hand is in progress a
/// new config is queued and returned at once, and the hand keeps the revision it froze; ending the hand applies it.
#[test]
fn config_reads_the_accepted_next_hand_configuration() {
    let mut engine = support::engine_with_fake_worker();
    let base = support::game_config();
    let with_budget = |flop_budget_s: u8| GameConfig { solver: SolverPrefs { flop_budget_s, ..base.solver }, ..base.clone() };
    let first = engine.set_config(with_budget(10)).unwrap();
    assert_eq!((engine.config().config_revision, engine.config().solver.flop_budget_s), (first, 10));
    let aa = Some([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]);
    engine.begin_hand(proto::BeginHand { button: Seat(0), hero: Seat(2), dealt: (0..6).map(Seat).collect(), stacks: vec![1000; 6], hero_cards: aa }).unwrap();
    let queued = engine.set_config(with_budget(30)).unwrap();
    assert_eq!((engine.config().config_revision, engine.config().solver.flop_budget_s), (queued, 30), "the queued next-hand config is what config() reads");
    assert_eq!(engine.state().unwrap().config.config_revision, first, "the hand in progress keeps the config it froze");
    engine.finish_hand();
    assert_eq!((engine.config().config_revision, engine.config().solver.flop_budget_s), (queued, 30));
}

#[test]
fn config_change_does_not_extend_an_admitted_request() {
    use engine::deadline::Deadlines;
    use proto::Street;
    let admitted = Deadlines::for_request(0, Street::Flop, 10);
    let later = Deadlines::for_request(0, Street::Flop, 30);
    assert_eq!(admitted.watchdog_fire_ms(), 14_900);
    assert_eq!(later.watchdog_fire_ms(), 34_900);
    assert_eq!(admitted.watchdog_fire_ms(), 14_900, "the admitted copy is immutable");
}

// ===================== the production admission on the fake clock =====================

/// A core over the fake clock and a fake worker that never replies, whose session config has `flop_budget_s`, and a
/// decision of a fresh hand active on its identity.
struct Admission {
    core: EngineCore,
    clock: Arc<FakeClock>,
    identity: proto::DecisionIdentity,
}

fn admission_rig(flop_budget_s: u8) -> Admission {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let (worker, _state) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![FakeReply::Hang]);
    let log_dir = std::env::temp_dir().join(format!("pokerai-flop-path-golden-admission-{}-{flop_budget_s}", std::process::id()));
    let core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&log_dir));
    core.set_config(with_flop_budget(flop_budget_s));
    let id = {
        let mut s = identity.lock().unwrap();
        s.set_config();
        s.begin_hand();
        s.next_decision().unwrap()
    };
    Admission { core, clock, identity: id }
}

fn with_flop_budget(flop_budget_s: u8) -> GameConfig {
    let base = support::game_config();
    GameConfig { solver: SolverPrefs { flop_budget_s, ..base.solver }, ..base }
}

/// Preflop of a heads-up pot: the button opens to 45, hero (the SB, seat 1) calls, the BB folds; hero acts first on
/// every postflop street.
fn hu_preflop() -> HandState {
    let hero_cards = Some([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]);
    let s = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(1), hero_cards);
    play(&s, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 45 }, Action::Call, Action::Fold])
}
/// Hero's flop decision at the street root.
fn flop_decision() -> HandState {
    board(&hu_preflop(), "Kh 7d 2c")
}
/// Hero's turn decision after a checked-through flop.
fn turn_decision() -> HandState {
    board(&play(&flop_decision(), &[Action::Check, Action::Check]), "Kh 7d 2c 4d")
}

fn recording(clock: &Arc<FakeClock>) -> (SharedSink, Recorder) {
    let (sink, recorder) = RecordingSink::notifying(clock.clone(), None);
    (Arc::new(Mutex::new(Box::new(sink))), recorder)
}

/// Drives the fake clock through the watchdog that admission armed (street deadline, then the fire) and returns the
/// one event it emitted. The watchdog's thread is acknowledged blocked at each instant before the clock reaches it
/// (ruling 20-I2), so it waits for exactly `street_ms` and then exactly `fire_ms`; one millisecond before the fire no
/// event has been emitted.
fn fire_of(clock: &FakeClock, recorder: &Recorder, street_ms: u64, fire_ms: u64) -> engine::testing::Recorded {
    clock.wait_for_waiter(street_ms);
    clock.set_ms(street_ms);
    clock.wait_for_waiter(fire_ms);
    clock.set_ms(fire_ms - 1);
    assert!(recorder.recorded().is_empty(), "nothing is emitted before the fire at {fire_ms} ms");
    clock.set_ms(fire_ms);
    let events = recorder.wait_for(1);
    assert_eq!(events.len(), 1, "exactly one Final");
    events.into_iter().next().unwrap()
}

/// Spec 13.3 over the production admission (`serve::admit`, through `LiveRequest::admitted`): the request's deadlines
/// come from the session config at admission, and the watchdog it arms fires its `Final` at `t0 + 14.9 s` for a flop
/// decision at `flop_budget_s = 10`, `t0 + 34.9 s` at 30, and `t0 + 14.9 s` for a turn decision at 30.
#[test]
fn the_admitted_watchdog_fires_at_the_flop_budget_deadline() {
    for (flop_budget_s, state, street, street_ms, fire_ms) in [
        (10_u8, flop_decision(), Street::Flop, 10_000, 14_900),
        (30, flop_decision(), Street::Flop, 30_000, 34_900),
        (30, turn_decision(), Street::Turn, 6_000, 14_900),
    ] {
        let mut r = admission_rig(flop_budget_s);
        let (sink, recorder) = recording(&r.clock);
        let req = LiveRequest::admitted(&r.core, r.identity.clone(), state, r.clock.now_ms(), sink);
        let watch = req.watch.as_ref().expect("a decision point is armed at admission");
        assert_eq!((watch.street, watch.deadlines), (street, Deadlines::for_request(0, street, flop_budget_s)), "{street:?} at flop_budget_s {flop_budget_s}");
        assert_eq!((watch.deadlines.street_deadline_ms, watch.deadlines.watchdog_fire_ms()), (street_ms, fire_ms));
        let fired = fire_of(&r.clock, &recorder, street_ms, fire_ms);
        assert_eq!(fired.at_ms, fire_ms, "{street:?} at flop_budget_s {flop_budget_s}");
        match &fired.event {
            RecommendationEvent::Final(rec) => {
                assert_eq!(rec.identity, r.identity);
                assert!(matches!(rec.coverage, Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { .. }, .. }), "{:?}", rec.coverage);
            }
            other => panic!("the watchdog emits a Final, not {other:?}"),
        }
        r.core.shutdown();
    }
}

/// A config change never extends an admitted request (spec 4.2, 12): the request owns the copy of the config and the
/// `Deadlines` it was admitted with. Raising the session's `flop_budget_s` from 10 to 30 after admission leaves the
/// admitted flop request at its 10 s budget, and its watchdog still fires at `t0 + 14.9 s`; only a request admitted after
/// the change is given the 30 s budget.
#[test]
fn an_admitted_request_keeps_its_deadlines_across_a_config_change() {
    let mut r = admission_rig(10);
    let (sink, recorder) = recording(&r.clock);
    let req = LiveRequest::admitted(&r.core, r.identity.clone(), flop_decision(), r.clock.now_ms(), sink);
    r.core.set_config(with_flop_budget(30));
    assert_eq!(r.core.config().solver.flop_budget_s, 30, "the session config moved");
    assert_eq!(req.config.solver.flop_budget_s, 10, "the admitted request keeps the config it was admitted with");
    let watch = req.watch.as_ref().expect("a flop decision point is armed");
    assert_eq!(watch.deadlines, Deadlines::for_request(0, Street::Flop, 10), "the admitted deadlines are not moved");
    let fired = fire_of(&r.clock, &recorder, 10_000, 14_900);
    assert!(matches!(fired.event, RecommendationEvent::Final(_)), "{:?}", fired.event);
    // a request admitted after the change is given the new budget
    let next = { let mut s = r.core.identity.lock().unwrap(); s.mutate(); s.next_decision().unwrap() };
    let (sink, _recorder) = recording(&r.clock);
    let later = LiveRequest::admitted(&r.core, next, flop_decision(), r.clock.now_ms(), sink);
    assert_eq!(later.watch.as_ref().unwrap().deadlines, Deadlines::for_request(14_900, Street::Flop, 30));
    assert_eq!(later.watch.as_ref().unwrap().deadlines.watchdog_fire_ms(), 14_900 + 34_900);
    r.core.shutdown();
}

// ===================== the wire deadline through run_solve =====================

/// A flop street root at pot 100, deep stacks, no observed history.
fn flop_root() -> StreetRootSnapshot {
    StreetRootSnapshot { street: Street::Flop, board: "Qs Jd 7h".split(' ').map(|s| Card::parse(s).unwrap()).collect(), oop: Seat(2), ip: Seat(0), pot_root: 100,
        stack_oop_root: 500, stack_ip_root: 500, dead_this_street: 0, projected_from: 2, history: vec![], bb_chips: 10 }
}
/// The same root one street later.
fn turn_root() -> StreetRootSnapshot {
    StreetRootSnapshot { street: Street::Turn, board: "Qs Jd 7h 3c".split(' ').map(|s| Card::parse(s).unwrap()).collect(), ..flop_root() }
}
fn full_range(board: &[Card]) -> Range1326 {
    let mut r = Range1326([1.0; 1326]);
    for (i, w) in r.0.iter_mut().enumerate() {
        let [a, b] = proto::combo_cards(i as u16);
        if board.contains(&a) || board.contains(&b) {
            *w = 0.0;
        }
    }
    r
}

/// Runs one live solve of `root` on `template` through the real `run_solve` and the fake worker, with deadlines
/// `Deadlines::for_request(0, root.street, flop_budget_s)` and the fake clock at `elapsed_ms` when it starts, and
/// returns the one `solve` request it sent.
fn sent_solve(root: StreetRootSnapshot, template: &str, flop_budget_s: u8, elapsed_ms: u64) -> SolveRequest {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let id = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
    let tree = build_tree_full(&root, &TemplateSelection::from_history(template, &[])).unwrap().tree;
    let script = vec![
        FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None },
        FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(uniform_solution(&tree, &[], 0.3)), error: None, elapsed_ms: 5 },
    ];
    let (worker, state) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let log_dir = std::env::temp_dir().join(format!("pokerai-flop-path-golden-wire-{}", std::process::id()));
    let mut core = EngineCore::new(worker, clock.clone(), identity, DecisionLog::open(&log_dir));
    let input = SolveInput { ranges: [full_range(&root.board), full_range(&root.board)], root: root.clone(), tree, target_bp: 50 };
    let deadlines = Deadlines::for_request(0, root.street, flop_budget_s);
    let plan = SolvePlan { identity: id, deadlines, street_deadline: Arc::new(StreetDeadline::new(deadlines.street_deadline_ms)), template_id: template.into(),
        retry_template_id: engine::tree::Templates::min_variant(template).map(String::from), rake: Rake::TimeCharge, hero_actor: "oop".into(), background: false,
        final_claim: None };
    let (sink, _events) = RecordingSink::new(clock.clone(), Some(state.clone()));
    let sink: SharedSink = Arc::new(Mutex::new(Box::new(sink)));
    clock.set_ms(elapsed_ms);
    let out = run_solve(&mut core, &input, &plan, &sink);
    assert_eq!(out.terminal, Terminal::Ok, "{template} at flop_budget_s {flop_budget_s}");
    let solves: Vec<SolveRequest> = state.lock().unwrap().sent.iter().filter_map(|m| if let EngineMessage::Solve(q) = m { Some(q.clone()) } else { None }).collect();
    core.shutdown();
    assert_eq!(solves.len(), 1, "one solve was sent");
    solves.into_iter().next().unwrap()
}

/// Spec 13.3 over the wire: at 250 ms of elapsed time the worker is given `street budget - 100 - 50 - 250` ms, 9 600 at
/// the default flop budget and 29 600 at the maximum, with the flop's 600 ms extraction margin; a turn decision at the
/// maximum flop preference is still given 6 000 - 400 = 5 600 ms with the turn's 200 ms.
#[test]
fn the_wire_deadline_at_250_ms_of_elapsed_time() {
    for (root, template, flop_budget_s, deadline_ms, extraction_margin_ms) in [
        (flop_root(), "flop_fast_v1", 10_u8, 9_600_u32, 600_u32),
        (flop_root(), "flop_fast_v1", 30, 29_600, 600),
        (turn_root(), "turn_std_v1", 30, 5_600, 200),
    ] {
        let street = root.street;
        let sent = sent_solve(root, template, flop_budget_s, 250);
        assert_eq!((sent.deadline_ms, sent.extraction_margin_ms), (deadline_ms, extraction_margin_ms), "{street:?} at flop_budget_s {flop_budget_s}");
    }
}
