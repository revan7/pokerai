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

// ===================== plan 4 Task 10: flop and turn through the cache =====================
//
// The production `serve_request_with` over plan 2's fake worker and fake clock and a real cache (`support::FlopRig`):
// the cache is seeded through the production entry writer, every lookup is the production `Cache::lookup` answering
// from disk, and every snapshot, log record and stored entry is read back from the engine's own stores.

use cache::entry::CacheEntry;
use engine::flop::FlopPolicy;
use engine::serve::ServeSeams;
use proto::{ApproxReason, Phase, Recommendation};
use support::{FlopRig, Seed, Served, BTN, SB};

fn bsf(reached_bp: u16) -> ApproxReason {
    ApproxReason::DeadlineBestSoFar { reached_bp, target_bp: 50 }
}

/// Every action of `rec` carries its EV.
fn every_ev(rec: &Recommendation) -> bool {
    !rec.actions.is_empty() && rec.actions.iter().all(|a| a.ev_bb.is_some())
}

/// A seam that runs `f` on `engine-main`.
fn seam(f: impl Fn() + Send + Sync + 'static) -> Option<Arc<dyn Fn() + Send + Sync>> {
    Some(Arc::new(f))
}

/// Spec 13.3 `deadline_best_so_far_labelling` (spec line 753, every clause this plan's routing drives), over the
/// production flop path: "a `best_so_far` at raw exploitability 1.9 chips of a 100-chip pot yields
/// `Approximate{DeadlineBestSoFar{reached_bp: 190, target_bp: 50}}` with every per-action EV present, is stored with its
/// raw exploitability, and a repeated request at the same identity is served as `Provisional`; an `ok` at target yields
/// no `DeadlineBestSoFar`; a single-raised-pot flop miss is logged as a miss with its reached exploitability and not as a
/// street violation, a turn `best_so_far` is logged as a violation". Plus the brief's: a malformed payload never enters
/// the cache.
#[test]
fn deadline_best_so_far_labelling() {
    let flop = support::srp_flop(SB);
    assert_eq!(core_model::derive(&flop).pot, 100, "the rig's flop is a 100-chip pot");
    assert_eq!(engine::flop::preflop_wagers(&flop), 2, "a single-raised pot");
    // The first request: a cold single-raised-pot flop (nothing stored), the live solve ends best_so_far at 1.9 chips.
    // The repeat request: the stored entry is served as Provisional, then the live refinement answers ok at 0.4 chips.
    let script = [support::live_script(&flop, "flop_fast_v1", 1.9, "best_so_far"), support::live_script(&flop, "flop_fast_v1", 0.4, "ok")].concat();
    let mut rig = FlopRig::new(script);
    let first = rig.serve(&flop);
    assert_eq!(first.kinds(), ["Fast", "Final"]);
    let f = first.final_rec();
    assert_eq!(f.coverage, Coverage::Approximate { reasons: vec![bsf(190)] }, "raw 1.9 of 100 is 190 bp against the 50 bp target");
    assert!(every_ev(f), "every per-action EV is present: {:?}", f.actions);
    assert_eq!((f.assumptions.reached_bp, f.assumptions.cache.as_str(), f.assumptions.template_id.as_str()), (Some(190), "miss", "flop_fast_v1"));
    let (raw, p, target) = (1.9_f32, 100_u32, 50_u16);
    assert!(f64::from(raw) / f64::from(p) > f64::from(target) / 10_000.0);
    assert_eq!((f64::from(raw) / f64::from(p) * 10_000.0).round() as u16, 190);
    // Stored with its raw exploitability, never rounded, and without the request's own DeadlineBestSoFar (spec 10.4).
    let stored = rig.stored();
    assert_eq!(stored.len(), 1, "the live terminal is stored");
    assert_eq!((stored[0].key.root_street, stored[0].exploitability_over_P, stored[0].target_bp), (Street::Flop, f64::from(1.9_f32) / 100.0, 50));
    assert!(stored[0].reasons.is_empty(), "the solve's DeadlineBestSoFar certifies nothing about a later request: {:?}", stored[0].reasons);
    // Logged as a miss with its reached exploitability, not as a street violation (the SRP flop miss is the designed outcome).
    let records = rig.records();
    assert_eq!(records.len(), 1);
    assert_eq!((records[0].cache.as_str(), records[0].reached_bp, records[0].street_violation, records[0].street), ("miss", Some(190), false, Street::Flop));

    // The repeated request at the same identity (hand, hand revision, config and model; a fresh decision id) is served as
    // Provisional before its live refinement.
    let second = rig.serve(&flop);
    assert_eq!(second.kinds(), ["Fast", "Provisional", "Final"]);
    let (a, b) = (&first.id, &second.id);
    assert_eq!((a.hand_id, a.hand_revision, a.config_revision, a.model_revision), (b.hand_id, b.hand_revision, b.config_revision, b.model_revision));
    assert_ne!(a.decision_id, b.decision_id);
    let provisional = second.provisionals()[0];
    assert_eq!((provisional.phase, &provisional.identity), (Phase::Provisional, &second.id));
    assert_eq!((provisional.assumptions.cache.as_str(), provisional.assumptions.reached_bp), ("provisional", Some(190)));
    assert_eq!(provisional.coverage, Coverage::Approximate { reasons: vec![bsf(190)] }, "an above-target hit discloses its shortfall, never Exact (P4T10-I1)");
    assert!(every_ev(provisional));
    assert_eq!(rig.solves().len(), 2, "the Provisional is refined by a live solve");
    let refined = second.final_rec();
    assert_eq!((&refined.identity, &refined.coverage, refined.assumptions.cache.as_str()), (&second.id, &Coverage::Exact, "provisional"),
        "the live refinement at target replaces the Provisional");
    assert!(rig.origins().iter().any(|(street, origin, d)| (*street, origin.as_str(), *d) == (Street::Flop, "live", b.decision_id)),
        "the refinement's Final replaced the decision's Provisional snapshot: {:?}", rig.origins());
    rig.core.shutdown();

    // An ok at target yields no DeadlineBestSoFar.
    let mut at_target = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 0.4, "ok"));
    let ok = at_target.serve(&flop);
    let ok_final = ok.final_rec();
    assert_eq!((&ok_final.coverage, ok_final.assumptions.reached_bp, ok_final.assumptions.cache.as_str()), (&Coverage::Exact, Some(40), "miss"));
    assert!(every_ev(ok_final));
    assert!(!at_target.records()[0].street_violation);
    at_target.core.shutdown();

    // A turn best_so_far is logged as a street violation.
    let turn = support::srp_turn(SB);
    assert_eq!(core_model::derive(&turn).pot, 100);
    let mut on_turn = FlopRig::new(support::live_script(&turn, "turn_std_v1", 1.9, "best_so_far"));
    let t = on_turn.serve(&turn);
    assert_eq!(t.final_rec().coverage, Coverage::Approximate { reasons: vec![bsf(190)] });
    let turn_records = on_turn.records();
    assert_eq!((turn_records[0].street, turn_records[0].street_violation, turn_records[0].reached_bp), (Street::Turn, true, Some(190)));
    on_turn.core.shutdown();

    // A malformed payload never enters the cache: a best_so_far exporting a node of a later street (spec 2: exports are
    // the current street's decision nodes) passes the solve client's validation and is delivered, but its entry is
    // refused and logged, and nothing is stored.
    let (tree, _, _) = support::live_tree(&flop, "flop_fast_v1");
    let mut malformed = support::varied_solution(&tree, &[], 1.9);
    let turn_node = tree.materialized.iter().find(|m| m.street == Street::Turn).unwrap().clone();
    let mut extra = malformed.nodes[0].clone();
    extra.path = cache::entry::chip_path(&tree.materialized, &turn_node.path).unwrap();
    (extra.actor, extra.actions) = (turn_node.actor.clone(), turn_node.actions.clone());
    let width = extra.actions.len();
    extra.probs = vec![vec![1.0 / width as f32; width]; proto::COMBOS];
    extra.ev_chips = vec![extra.actions.iter().map(|a| if *a == Action::Fold { 0.0 } else { 1.0 }).collect(); proto::COMBOS];
    malformed.covered_paths.push(extra.path.clone());
    malformed.nodes.push(extra);
    let script = vec![FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None },
        FakeReply::Result { id: IdRef::Last, status: ResultStatus::BestSoFar, solution: Some(malformed), error: None, elapsed_ms: 5 }];
    let mut refused = FlopRig::new(script);
    let r = refused.serve(&flop);
    assert_eq!(r.final_rec().coverage, Coverage::Approximate { reasons: vec![bsf(190)] }, "delivery is unaffected");
    assert!(refused.stored().is_empty(), "nothing malformed enters the cache");
    let rejects: Vec<serde_json::Value> = refused.diagnostics().into_iter().filter(|d| d["event"] == "cache_reject").collect();
    assert_eq!(rejects.len(), 1, "the refusal is logged: {rejects:?}");
    assert!(rejects[0]["detail"].as_str().unwrap().contains("cache entry"), "{rejects:?}");
    refused.core.shutdown();
}

/// Plan 4 Task 10 Step 1: an above-target hit is emitted as the `Provisional` before the live refinement, and the one
/// `Final` answers the same decision.
#[test]
fn provisional_hit_is_emitted_then_refined() {
    let events = support::run_flop_script(vec![support::provisional_hit(0.019)], 0.4, "ok");
    let phases = events
        .iter()
        .filter_map(|e| match e {
            RecommendationEvent::Provisional(r) | RecommendationEvent::Final(r) => Some((r.phase, r.identity.clone(), r.assumptions.cache.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0].0, Phase::Provisional);
    assert_eq!(phases[1].0, Phase::Final);
    assert_eq!(phases[0].1, phases[1].1, "identity is retained across the refinement");
    assert_eq!(phases[0].2, "provisional");
    assert_eq!(support::final_count(&events), 1);
}

/// Spec 4.4 `Assumptions.cache` (`miss | exact | approximate | provisional`) on the `Final` of every route.
#[test]
fn cache_labels_are_recorded_for_every_route() {
    for (route, expected) in [
        (support::exact_hit(), "exact"),
        (support::approximate_hit(), "approximate"),
        (support::provisional_route(), "provisional"),
        (vec![], "miss"),
    ] {
        let events = support::run_flop_script(route, 0.4, "ok");
        let last = events.iter().rev().find_map(|e| match e { RecommendationEvent::Final(r) => Some(r), _ => None }).unwrap();
        assert_eq!(last.assumptions.cache, expected);
        assert_eq!(support::final_count(&events), 1);
    }
}

/// Plan 4 Task 8 carry 8-C4 (spec 13.1 T4, 9.2): a flop cache hit registers its snapshot as part of its `Final`'s
/// accepted delivery, and after hero's bet, the button's call and a turn card, the turn root ranges the replay hands the
/// turn solve are conditioned through that snapshot: the snapshot is disclosed with its cache origin, the flop is not
/// unconditioned, and hero's turn range carries the stored node's per-combo betting frequencies.
#[test]
fn a_flop_cache_hit_registers_its_snapshot_and_conditions_the_turn_root_ranges() {
    let flop = support::srp_flop(SB);
    let turn = board(&play(&flop, &[Action::Bet { to: 50 }, Action::Call]), support::TURN);
    let mut rig = FlopRig::new(support::live_script(&turn, "turn_std_v1", 0.4, "ok"));
    rig.core.install_replay_ranges();
    // The replay's own flop root ranges (an empty preflop store: the preflop is unconditioned), for the stored entry.
    let root = core_model::street_root(&flop).unwrap();
    let ranges = {
        rig.identity.lock().unwrap().next_decision().unwrap();
        let r = rig.core.range_source.lock().unwrap().ranges_at_root(&flop, &root).unwrap_or_else(|e| panic!("the replay answers the flop root: {e:?}"));
        [r.oop, r.ip]
    };
    let entry = support::seed_entry_with(&Seed::exact("flop_fast_v1"), ranges);
    assert!(rig.core.cache.store_tracked(&entry).wait(std::time::Duration::from_secs(60)), "seeded");
    let hit = rig.serve(&flop);
    let f = hit.final_rec();
    assert_eq!((f.assumptions.cache.as_str(), f.assumptions.source.starts_with("cache@")), ("approximate", true), "the replay's reasons make the hit approximate");
    assert!(rig.solves().is_empty(), "a hit at target starts no live solve");
    assert_eq!(rig.origins(), vec![(Street::Flop, "cache_approximate".to_string(), hit.id.decision_id)], "the hit is registered with its Final");
    let snapshot = rig.core.snapshots.lock().unwrap().for_hand(1)[0].clone();

    // Hero bets 50, the button calls, the turn comes.
    rig.identity.lock().unwrap().mutate();
    let on_turn = rig.serve(&turn);
    let t = on_turn.final_rec();
    let fast = on_turn.events.iter().find_map(|e| match e { RecommendationEvent::Fast(r) => Some(r), _ => None }).unwrap();
    let note = engine::replay_bridge::snapshot_note(Street::Flop, &snapshot.provenance);
    assert!(note.contains("cache_approximate") && fast.assumptions.notes.contains(&note) && t.assumptions.notes.contains(&note), "{:?}", t.assumptions.notes);
    let reasons = match &t.coverage { Coverage::Approximate { reasons } => reasons.clone(), other => panic!("{other:?}") };
    assert!(!reasons.iter().any(|r| matches!(r, ApproxReason::UnconditionedPriorStreet { street: Street::Flop, .. })), "the flop is conditioned: {reasons:?}");
    let solve = &rig.solves()[0];
    // Hero (the small blind) bet at the flop root: combo c bet with probability (1 - (c % 7 + 1) / 8) / (width - 1), so
    // its turn weight, normalized to its maximum, is (7 - c % 7) / 7 for every combo the board leaves.
    let c = |a: &str, b: &str| usize::from(proto::combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()));
    for combo in [c("Qs", "Js"), c("Qs", "Jh"), c("Qs", "Jc"), c("Ts", "9h"), c("5s", "3h")] {
        let expected = (7 - combo % 7) as f32 / 7.0;
        assert!((solve.oop_range.0[combo] - expected).abs() < 1e-4, "combo {combo}: weight {} for {expected}", solve.oop_range.0[combo]);
    }
    rig.core.shutdown();
}

/// Ruling 28-I2 / plan-3 carry (c): a cache `Provisional` registers its snapshot only inside its own accepted delivery.
/// A decision superseded while its cache was asked gets no `Provisional`, no `Final` and no snapshot, and starts no
/// live solve: its stale hit is discarded (a stale `Final` hit likewise), and it records nothing.
#[test]
fn a_stale_cache_hit_is_discarded_and_registers_nothing() {
    let flop = support::srp_flop(SB);
    for seeds in [support::provisional_route(), support::exact_hit()] {
        let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 0.4, "ok"));
        rig.seed(&seeds);
        let identity = rig.identity.clone();
        let served = rig.serve_with(&flop, ServeSeams { before_lookup: seam(move || { identity.lock().unwrap().mutate(); }), ..ServeSeams::default() });
        assert_eq!(served.kinds(), ["Fast"], "{seeds:?}");
        assert!(rig.origins().is_empty(), "nothing registered: {:?}", rig.origins());
        assert!(rig.solves().is_empty() && rig.records().is_empty(), "no live solve, no Final to log");
        assert!(rig.core.snapshots.lock().unwrap().misses_for_identity(&served.id).is_empty(), "a discarded candidate records nothing");
        rig.core.shutdown();
    }
}

/// Spec 7: exactly one `Final` at or before the watchdog's fire even while the cache holds `engine-main` (a reader that
/// does not answer holds a lookup to its bound). V3 has admitted `flop_min_v1`, so the flop decision probes the
/// pre-solver's `flop_fast_v1` (an above-target entry is stored there) and then `flop_min_v1`; the second lookup is held
/// until the watchdog has delivered its `Final`. That `Final` is the only one; the `Provisional` found by the first
/// probe is neither emitted nor registered after it (its delivery is refused under the identity lock, where the claim
/// is taken), and nothing is sent to the worker.
#[test]
fn exactly_one_final_when_the_cache_holds_engine_main_past_the_fire() {
    let flop = support::srp_flop(SB);
    let mut rig = FlopRig::new(support::live_script(&flop, "flop_min_v1", 0.4, "ok"));
    rig.core.flop_policy = FlopPolicy { min_admitted: true };
    rig.seed(&support::provisional_route());
    let (clock, ended, calls) = (rig.clock.clone(), rig.core.watchdog.ended_threads(), Arc::new(std::sync::atomic::AtomicU32::new(0)));
    let held = seam(move || {
        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            clock.set_ms(14_900);
            ended.wait_for(1);
        }
    });
    let served = rig.serve_with(&flop, ServeSeams { before_lookup: held, ..ServeSeams::default() });
    assert_eq!(served.kinds(), ["Fast", "Final"]);
    assert!(matches!(served.final_rec().coverage, Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { .. }, .. }), "{:?}", served.final_rec().coverage);
    assert!(rig.origins().is_empty(), "the Provisional refused after the watchdog's Final registers nothing: {:?}", rig.origins());
    assert!(rig.solves().is_empty(), "nothing is sent past the fire");
    let records = rig.records();
    assert_eq!((records.len(), records[0].final_violation), (1, true), "the watchdog's Final, logged once");
    rig.core.shutdown();
}

/// Spec 7 / ruling F2-N3: the promoted `Provisional` is the payload the watchdog delivers at its fire. The live
/// refinement comes back worse than the retained hit, and the watchdog fires before the engine claims its own `Final`:
/// the one `Final` is exactly the `Provisional`, promoted, and the decision keeps the cache snapshot it registered.
#[test]
fn the_watchdog_delivers_the_promoted_provisional_at_its_fire() {
    let flop = support::srp_flop(SB);
    let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 5.0, "best_so_far"));
    rig.seed(&support::provisional_route());
    let (clock, ended) = (rig.clock.clone(), rig.core.watchdog.ended_threads());
    let fire = seam(move || {
        clock.set_ms(14_900);
        ended.wait_for(1);
    });
    let served = rig.serve_with(&flop, ServeSeams { before_claim: fire, ..ServeSeams::default() });
    assert_eq!(served.kinds(), ["Fast", "Provisional", "Final"]);
    let mut promoted = served.provisionals()[0].clone();
    promoted.phase = Phase::Final;
    assert_eq!(served.final_rec(), &promoted, "the watchdog's Final is the retained Provisional, promoted");
    assert_eq!(promoted.coverage, Coverage::Approximate { reasons: vec![ApproxReason::ChartRounded, bsf(190)] }, "with the Provisional's shortfall (P4T10-I1)");
    assert_eq!(rig.origins(), vec![(Street::Flop, "cache_provisional".to_string(), served.id.decision_id)]);
    rig.core.shutdown();
}

/// Ruling 7-Q2/7-D6 end to end: a stored live terminal is served back in the query's own suits. On a flop that is not
/// its own canonical form (the heart deuce is the lowest card, so the canonical suits swap hearts and clubs), the
/// repeated request's cache hit gives hero's combo exactly the frequencies and EVs of the live solve it stored (the
/// stored rows differ by combo, so a row moved to the wrong combo is observable).
#[test]
fn a_stored_solution_is_served_back_in_the_querys_own_suits() {
    let flop = board(&support::srp_preflop(SB), "Kc 7d 2h");
    let root = core_model::street_root(&flop).unwrap();
    assert_ne!(engine::cache_bridge::canonical_perm(&root.board, &support::full_range(), &support::full_range()), core_iso::SuitPerm::IDENTITY);
    let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 0.4, "ok"));
    let live = rig.serve(&flop);
    assert_eq!(rig.stored().len(), 1, "the live terminal is stored");
    let hit = rig.serve(&flop);
    assert_eq!((hit.final_rec().assumptions.cache.as_str(), rig.solves().len()), ("exact", 1), "the repeated request is a hit");
    assert_eq!(hit.final_rec().actions, live.final_rec().actions, "hero's own row, in the query's suits");
    rig.core.shutdown();
}

// ===================== fix round 1 (review P4T10-I1..I4) =====================

/// Every reason a result's coverage lists, whatever its label.
fn reasons_of(rec: &Recommendation) -> Vec<ApproxReason> {
    match &rec.coverage {
        Coverage::Exact => vec![],
        Coverage::Approximate { reasons } => reasons.clone(),
        Coverage::Unsupported { partial, .. } => partial.clone(),
    }
}

/// A translation and a mapping no request of the rig incurs (its root ranges are explicit): only a stored entry can
/// bring them.
fn cache_only_reasons() -> (ApproxReason, ApproxReason) {
    let translation = ApproxReason::BetTranslation { street: Street::Preflop, seat: BTN, observed_pct: 0.73, mapped: vec![(0.5, 0.468), (1.0, 0.532)], deviation: 0.23,
        prominent: true };
    let mapping = ApproxReason::RakeProfileMapped { actual: "PotRake 5% capped at 5 chips".into(), used: "undocumented chart rake".into() };
    (translation, mapping)
}

/// Review P4T10-I1 (ruling 10-Q2; spec 2, 7, 10.4): an above-target hit discloses its accuracy shortfall in its
/// coverage, `DeadlineBestSoFar { reached_bp: the stored raw exploitability over P in bp, rounded, target_bp: the
/// request's }`, merged with the lookup's own reasons. The raw comparison, never the rounded value, makes it a
/// Provisional: a stored 0.005049 is 50 bp once rounded and still misses the 50 bp target (spec 10.4's own example). The
/// same coverage is the registered snapshot's and the retained payload's (here the refinement fails, so the retained
/// payload is the Final).
#[test]
fn a_provisional_discloses_its_accuracy_shortfall_in_its_coverage() {
    let pre = engine::flop::PRESOLVER_TEMPLATE;
    for (seed, expected) in [
        (Seed::exact(pre).raw(0.019), vec![bsf(190)]),
        (Seed::exact(pre).raw(0.005049).reasons(vec![ApproxReason::ChartRounded]), vec![ApproxReason::ChartRounded, bsf(50)]),
    ] {
        let flop = support::srp_flop(SB);
        let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 0.0, "no_iteration"));
        rig.seed(&[seed.clone()]);
        let served = rig.serve(&flop);
        assert_eq!(served.kinds(), ["Fast", "Provisional", "Final"], "{seed:?}");
        let provisional = served.provisionals()[0];
        assert_eq!((provisional.coverage.clone(), provisional.assumptions.cache.as_str()), (Coverage::Approximate { reasons: expected.clone() }, "provisional"), "{seed:?}");
        assert_eq!(served.final_rec().coverage, provisional.coverage, "{seed:?}: the retained payload keeps it");
        let snapshots = rig.snapshots_of(served.id.decision_id);
        assert_eq!(snapshots.len(), 1);
        assert_eq!((snapshots[0].provenance.origin.as_str(), &snapshots[0].reasons), ("cache_provisional", &expected), "{seed:?}: the snapshot carries it");
        rig.core.shutdown();
    }
}

/// Review P4T10-I2 (ruling 10-Q4): a retained payload that beats the live refinement is served with its own coverage
/// (its shortfall 190/50 and its stored `ChartRounded`) and nothing of the discarded live solve: neither the `Final` nor
/// its rebuilt snapshot carries the live best_so_far's 300/50. The live attempt stays disclosed in the note, naming both
/// raw accuracies, and in the deadline log.
#[test]
fn a_retained_payload_keeps_its_own_coverage_over_a_worse_live_result() {
    let flop = support::srp_flop(SB);
    let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 3.0, "best_so_far"));
    rig.seed(&support::provisional_route());
    let served = rig.serve(&flop);
    let f = served.final_rec();
    let own = vec![ApproxReason::ChartRounded, bsf(190)];
    assert_eq!(f.coverage, Coverage::Approximate { reasons: own.clone() });
    let snapshots = rig.snapshots_of(served.id.decision_id);
    assert_eq!(snapshots.len(), 1);
    assert_eq!((snapshots[0].provenance.origin.as_str(), &snapshots[0].reasons), ("cache_provisional", &own));
    assert!(!reasons_of(f).contains(&bsf(300)) && !snapshots[0].reasons.contains(&bsf(300)), "the discarded live solve's accuracy is nobody's reason");
    let note = f.assumptions.notes.iter().find(|n| n.starts_with("live refinement")).expect("the live attempt is disclosed");
    assert!(note.contains("0.030000") && note.contains("0.019000"), "{note}");
    let records = rig.records();
    assert_eq!((records.len(), &records[0].coverage, records[0].street_violation), (1, &f.coverage, true), "the deadline log keeps the live best_so_far's verdict");
    rig.core.shutdown();
}

/// Review P4T10-I3 (plan-3 F-I1; spec 4.4, 10.4): a cache result lists every translation and mapping of its complete
/// coverage in `assumptions.translations` / `assumptions.mappings`, the ones its stored entry brings included, on the
/// at-target `Final`, the `Provisional`, the retained `Final` and the watchdog's promotion alike.
#[test]
fn cache_results_list_the_entrys_translations_and_mappings() {
    let pre = engine::flop::PRESOLVER_TEMPLATE;
    let (translation, mapping) = cache_only_reasons();
    let both = vec![translation.clone(), mapping.clone()];
    let lists = |r: &Recommendation| (r.assumptions.translations.clone(), r.assumptions.mappings.clone());
    let expected = (vec![translation.clone()], vec![mapping.clone()]);
    let flop = support::srp_flop(SB);
    // The at-target Final.
    let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 0.4, "ok"));
    rig.seed(&[Seed::exact(pre).reasons(both.clone())]);
    let hit = rig.serve(&flop);
    assert_eq!((hit.final_rec().assumptions.cache.as_str(), lists(hit.final_rec())), ("approximate", expected.clone()), "the at-target cache Final");
    rig.core.shutdown();
    // The Provisional and the retained Final.
    let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 0.0, "no_iteration"));
    rig.seed(&[Seed::exact(pre).raw(0.019).reasons(both.clone())]);
    let retained = rig.serve(&flop);
    assert_eq!(lists(retained.provisionals()[0]), expected, "the Provisional");
    assert_eq!(lists(retained.final_rec()), expected, "the retained Final");
    rig.core.shutdown();
    // The watchdog's promotion.
    let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 5.0, "best_so_far"));
    rig.seed(&[Seed::exact(pre).raw(0.019).reasons(both)]);
    let (clock, ended) = (rig.clock.clone(), rig.core.watchdog.ended_threads());
    let promoted = rig.serve_with(&flop, ServeSeams { before_claim: seam(move || { clock.set_ms(14_900); ended.wait_for(1); }), ..ServeSeams::default() });
    assert_eq!(promoted.final_rec().phase, Phase::Final);
    assert_eq!(lists(promoted.final_rec()), expected, "the watchdog's promotion");
    rig.core.shutdown();
}

/// The budget notes a result carries.
fn budget_notes(rec: &Recommendation) -> Vec<String> {
    rec.assumptions.notes.iter().filter(|n| n.starts_with("cache lookup of")).cloned().collect()
}

/// Review P4T10-I4 (spec 7; ruling 10-pre1): the cache phase's cutoff is absolute, the earlier of 500 ms from its
/// start and the street deadline, and the time left is read again immediately before each lookup. A lookup reached
/// past the phase cutoff serves nothing even with an exact entry stored, and no later probe is started (V3 admitted:
/// the flop_min_v1 probe never reaches its lookup); a lookup reached past the street deadline serves nothing either.
/// Both are disclosed as a spent budget, and the decision is answered without the cache.
#[test]
fn a_lookup_reached_past_the_cache_cutoff_serves_no_hit() {
    let pre = engine::flop::PRESOLVER_TEMPLATE;
    let flop = support::srp_flop(SB);
    // Past the 500 ms phase cutoff (the phase starts at 0 ms).
    let mut rig = FlopRig::new(support::live_script(&flop, "flop_min_v1", 0.4, "ok"));
    rig.core.flop_policy = ADMITTED;
    rig.seed(&[Seed::exact(pre), Seed::exact("flop_min_v1")]);
    let (clock, lookups) = (rig.clock.clone(), Arc::new(std::sync::atomic::AtomicU32::new(0)));
    let counted = lookups.clone();
    let past_phase = seam(move || {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        clock.set_ms(501);
    });
    let served = rig.serve_with(&flop, ServeSeams { before_lookup: past_phase, ..ServeSeams::default() });
    let f = served.final_rec();
    assert_eq!((f.assumptions.cache.as_str(), f.assumptions.source.starts_with("solver-worker@")), ("miss", true), "no hit past the phase cutoff");
    assert_eq!(lookups.load(std::sync::atomic::Ordering::SeqCst), 1, "no probe is started once the phase is spent");
    assert_eq!(budget_notes(f).len(), 2, "both templates' lookups are disclosed as a spent budget: {:?}", f.assumptions.notes);
    assert_eq!(rig.origins(), vec![(Street::Flop, "live".to_string(), served.id.decision_id)]);
    rig.core.shutdown();
    // Past the street deadline (the request was admitted at 0 ms and served at 9 900 ms: the street cutoff, 10 000 ms,
    // comes before the phase's 10 400 ms).
    let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 0.4, "ok"));
    rig.seed(&[Seed::exact(pre)]);
    let clock = rig.clock.clone();
    let past_street = seam(move || clock.set_ms(10_000));
    let served = rig.serve_at(&flop, 9_900, ServeSeams { before_lookup: past_street, ..ServeSeams::default() });
    let f = served.final_rec();
    assert_eq!(f.assumptions.cache, "miss", "no hit past the street deadline: {:?}", f.coverage);
    assert!(!f.assumptions.source.starts_with("cache@") && budget_notes(f).len() == 1, "{:?}", f.assumptions);
    assert!(rig.origins().iter().all(|(_, origin, _)| !origin.starts_with("cache")), "{:?}", rig.origins());
    rig.core.shutdown();
}

// ===================== the flop-path golden =====================

/// A result's canonical projection: phase, identity, coverage and reasons, the evaluated actions with their EVs, the
/// range mix, and the assumptions that name its source (`cache`, template, signature, source, accuracy, reached
/// exploitability, notes, translations, mappings). Elapsed time is left out.
fn project(rec: &Recommendation) -> serde_json::Value {
    let a = &rec.assumptions;
    serde_json::json!({
        "phase": rec.phase,
        "decision_id": rec.identity.decision_id,
        "hand_revision": rec.identity.hand_revision,
        "coverage": rec.coverage,
        "actions": rec.actions,
        "range_mix": rec.range_mix,
        "cache": a.cache,
        "template_id": a.template_id,
        "tree_signature": a.tree_signature,
        "source": a.source,
        "source_accuracy": a.source_accuracy,
        "reached_bp": a.reached_bp,
        "notes": a.notes,
        "translations": a.translations,
        "mappings": a.mappings,
    })
}

fn project_served(s: &Served) -> serde_json::Value {
    serde_json::json!({
        "kinds": s.kinds(),
        "results": s.events.iter().filter_map(|e| match e {
            RecommendationEvent::Provisional(r) | RecommendationEvent::Final(r) => Some(project(r)),
            _ => None,
        }).collect::<Vec<_>>(),
    })
}

fn project_entry(e: &CacheEntry) -> serde_json::Value {
    serde_json::json!({ "street": e.key.root_street, "template": e.tree.template_id, "exploitability_over_P": e.exploitability_over_P, "target_bp": e.target_bp,
        "reasons": e.reasons, "export": e.export, "nodes": e.nodes.len(), "pot": e.source.pot })
}

/// One golden case: its served decisions, the solves sent, the snapshots registered, the entries stored and the log.
fn project_rig(rig: &mut FlopRig, served: &[Served]) -> serde_json::Value {
    let stored = rig.stored();
    serde_json::json!({
        "served": served.iter().map(project_served).collect::<Vec<_>>(),
        "solves": rig.solves().iter().map(|q| q.tree.template_id.clone()).collect::<Vec<_>>(),
        "snapshots": rig.origins(),
        "stored": stored.iter().map(project_entry).collect::<Vec<_>>(),
        "log": rig.records().iter().map(|r| serde_json::json!({ "street": r.street, "coverage": r.coverage, "cache": r.cache, "reached_bp": r.reached_bp,
            "street_violation": r.street_violation, "final_violation": r.final_violation, "template_id": r.template_id })).collect::<Vec<_>>(),
    })
}

/// One flop request of the rig's single-raised pot (hero the small blind) over `seeds`, the live solve answering
/// `live_script(template, raw, status)`, with `policy`.
fn flop_case(seeds: Vec<Seed>, policy: FlopPolicy, template: &str, raw: f32, status: &str) -> serde_json::Value {
    let flop = support::srp_flop(SB);
    let mut rig = FlopRig::new(support::live_script(&flop, template, raw, status));
    rig.core.flop_policy = policy;
    rig.seed(&seeds);
    let served = rig.serve(&flop);
    assert_eq!(support::final_count(&served.events), 1, "exactly one Final: {:?}", served.kinds());
    let out = project_rig(&mut rig, &[served]);
    rig.core.shutdown();
    out
}

const CONSERVATIVE: FlopPolicy = FlopPolicy { min_admitted: false };
const ADMITTED: FlopPolicy = FlopPolicy { min_admitted: true };

/// Plan 4 Task 10 Step 5: every flop and turn route's canonical projection, frozen in `golden/flop_path.json` (recorded
/// once with `POKERAI_RECORD_GOLDENS=1` and inspected, then compared).
#[test]
fn flop_path_golden() {
    let mut cases = serde_json::Map::new();
    let pre = engine::flop::PRESOLVER_TEMPLATE;
    // Cache hits at target: served as the Final, no live solve.
    cases.insert("exact_synthetic_hit".into(), flop_case(vec![Seed::exact(pre)], CONSERVATIVE, "flop_fast_v1", 0.4, "ok"));
    cases.insert("chart_hit".into(), flop_case(vec![Seed::exact(pre).reasons(vec![ApproxReason::ChartRounded])], CONSERVATIVE, "flop_fast_v1", 0.4, "ok"));
    cases.insert("menu_only".into(), flop_case(vec![Seed::exact(pre).at(200, 1910, 10_000)], CONSERVATIVE, "flop_fast_v1", 0.4, "ok"));
    cases.insert("spr_and_menu".into(), flop_case(vec![Seed::exact(pre).at(200, 1900, 10_000)], CONSERVATIVE, "flop_fast_v1", 0.4, "ok"));
    cases.insert("presolver_miss_live_template_hit".into(), flop_case(vec![Seed::exact("flop_min_v1")], ADMITTED, "flop_min_v1", 0.4, "ok"));
    cases.insert("presolver_provisional_live_template_exact".into(),
        flop_case(vec![support::provisional_hit(0.019), Seed::exact("flop_min_v1")], ADMITTED, "flop_min_v1", 0.4, "ok"));
    // Above-target hits: the Provisional, then the live refinement.
    cases.insert("provisional_then_ok".into(), flop_case(support::provisional_route(), CONSERVATIVE, "flop_fast_v1", 0.4, "ok"));
    cases.insert("provisional_then_no_iteration".into(), flop_case(support::provisional_route(), CONSERVATIVE, "flop_fast_v1", 0.0, "no_iteration"));
    cases.insert("provisional_then_worse_best_so_far".into(), flop_case(support::provisional_route(), CONSERVATIVE, "flop_fast_v1", 3.0, "best_so_far"));
    cases.insert("two_provisionals_the_better_retained".into(),
        flop_case(vec![support::provisional_hit(0.03), Seed::exact("flop_min_v1").raw(0.012)], ADMITTED, "flop_min_v1", 0.4, "ok"));
    // Misses: solved live, stored.
    cases.insert("cold_srp_best_so_far".into(), flop_case(vec![], CONSERVATIVE, "flop_fast_v1", 1.9, "best_so_far"));
    cases.insert("cold_srp_raw_target_ok".into(), flop_case(vec![], CONSERVATIVE, "flop_fast_v1", 0.4, "ok"));
    cases.insert("srp_live_flop_min".into(), flop_case(vec![], ADMITTED, "flop_min_v1", 0.4, "ok"));
    {
        // The first attempt fails with no_iteration; the `_min` retry answers on its own tree, which is what is stored.
        let flop = support::srp_flop(SB);
        let first = support::live_script(&flop, "flop_fast_v1", 0.0, "no_iteration")[..2].to_vec();
        let mut rig = FlopRig::new([first, support::live_script(&flop, "flop_min_v1", 0.4, "ok")].concat());
        let served = rig.serve(&flop);
        cases.insert("retry_min_is_stored_on_its_own_tree".into(), project_rig(&mut rig, &[served]));
        rig.core.shutdown();
    }
    {
        // A 3-bet pot uses flop_fast_v1 live even with flop_min_v1 admitted: one probe, one template.
        let flop = support::three_bet_flop(SB);
        let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 0.4, "ok"));
        rig.core.flop_policy = ADMITTED;
        let served = rig.serve(&flop);
        cases.insert("three_bet_fast".into(), project_rig(&mut rig, &[served]));
        rig.core.shutdown();
    }
    {
        // The stored entry covers the root node only: hero on the button, facing a check, is a miss for that path.
        let facing_check = play(&support::srp_flop(BTN), &[Action::Check]);
        let mut rig = FlopRig::new(support::live_script(&facing_check, "flop_fast_v1", 0.4, "ok"));
        rig.seed(&[Seed::exact(pre).truncated()]);
        let served = rig.serve(&facing_check);
        cases.insert("missed_path".into(), project_rig(&mut rig, &[served]));
        rig.core.shutdown();
    }
    {
        // The cache root is blocked (a file where its directory should be): every lookup misses, nothing is stored, the
        // decision is answered live.
        let dir = support::TempDir::new();
        std::fs::write(dir.0.join("blocked"), b"not a directory").unwrap();
        let blocked = cache::Cache::open(dir.0.join("blocked").join("v3"), cache::CACHE_QUOTA_BYTES);
        assert!(blocked.availability_warning().is_some());
        let flop = support::srp_flop(SB);
        let mut rig = FlopRig::with_cache(support::live_script(&flop, "flop_fast_v1", 0.4, "ok"), blocked, dir);
        let served = rig.serve(&flop);
        cases.insert("disk_blocked".into(), project_rig(&mut rig, &[served]));
        rig.core.shutdown();
    }
    {
        // A stale result: the decision is superseded once its solve returned; no Final, nothing registered or stored.
        let flop = support::srp_flop(SB);
        let mut rig = FlopRig::new(support::live_script(&flop, "flop_fast_v1", 0.4, "ok"));
        let identity = rig.identity.clone();
        let served = rig.serve_with(&flop, ServeSeams { after_active_check: seam(move || { identity.lock().unwrap().mutate(); }), ..ServeSeams::default() });
        cases.insert("stale_result".into(), project_rig(&mut rig, &[served]));
        rig.core.shutdown();
    }
    {
        // The turn: solved live and stored; the repeated request is served from the entry, with the live solve's own
        // hero frequencies and EVs.
        let turn = support::srp_turn(SB);
        let mut rig = FlopRig::new(support::live_script(&turn, "turn_std_v1", 0.4, "ok"));
        let live = rig.serve(&turn);
        let _ = rig.stored();
        let hit = rig.serve(&turn);
        assert_eq!(hit.final_rec().actions, live.final_rec().actions, "the stored turn serves back the live strategy for hero's combo");
        cases.insert("turn_store_then_hit".into(), project_rig(&mut rig, &[live, hit]));
        rig.core.shutdown();
    }
    {
        // The river: solved live, never stored.
        let river = support::srp_river(SB);
        let mut rig = FlopRig::new(support::live_script(&river, "river_std_v1", 0.4, "ok"));
        let served = rig.serve(&river);
        cases.insert("river_no_store".into(), project_rig(&mut rig, &[served]));
        rig.core.shutdown();
    }
    // Compared as text parsed back by the same parser on both sides: `serde_json` does not parse every float back to the
    // exact bits it printed (the last digit of an f64 may move), so a value built in memory is never compared with one
    // read from the file directly.
    let frozen: serde_json::Value = serde_json::from_slice(&serde_json::to_vec_pretty(&serde_json::Value::Object(cases)).unwrap()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/flop_path.json");
    if std::env::var_os("POKERAI_RECORD_GOLDENS").is_some() {
        std::fs::write(&path, serde_json::to_vec_pretty(&frozen).unwrap()).unwrap();
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("the committed golden {} is missing ({e}); record it once with POKERAI_RECORD_GOLDENS=1 and inspect it", path.display()));
    let expected: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for (name, value) in frozen.as_object().unwrap() {
        assert_eq!(Some(value), expected.get(name), "golden case {name}");
    }
    assert_eq!(frozen, expected);
}
