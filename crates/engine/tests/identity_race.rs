use engine::clock::Clock;
use engine::core::EngineCore;
use engine::identity::IdentityState;
use engine::log::DecisionLog;
use engine::ranges::ExplicitRanges;
use engine::serve::{serve_request, LiveRequest};
use engine::testing::{board, hand, play, uniform_solution, FakeClock, FakeReply, FakeWorker, IdRef, RecordingSink};
use engine::tree::{build_tree_full, TemplateSelection};
use proto::worker::{AckStatus, ResultStatus, Stage};
use proto::{Action, Card, DecisionIdentity, Range1326, RecommendationEvent, Seat, Street};
use std::sync::{Arc, Mutex};

fn river_state() -> proto::HandState {
    let aa = Some([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]);
    let s = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), aa);
    let s = play(&s, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call]);
    board(&play(&board(&play(&board(&s, "Kh 7d 2c"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d 9s")
    // river, BB (hero) to act, pot 65, stacks 970
}
fn full(board: &[Card]) -> Range1326 { let mut r = Range1326([1.0; 1326]); for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { r.0[i] = 0.0; } } r }
fn ident(events: &[engine::testing::Recorded]) -> Vec<DecisionIdentity> {
    events.iter().map(|r| match &r.event { RecommendationEvent::Fast(x) | RecommendationEvent::Provisional(x) | RecommendationEvent::Final(x) => x.identity.clone(), RecommendationEvent::Equity { identity, .. } | RecommendationEvent::Progress { identity, .. } | RecommendationEvent::NoDecision { identity, .. } => identity.clone() }).collect()
}
fn solution_for(state: &proto::HandState) -> proto::worker::StreetSolution {
    let root = core_model::street_root(state).unwrap();
    let b = build_tree_full(&root, &TemplateSelection::from_history("river_std_v1", &root.history)).unwrap();
    uniform_solution(&b.tree, &b.history, 0.2)
}

#[test]
fn identity_race_golden() {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let state_a = river_state();
    let sol = solution_for(&state_a);
    // Hand A: the solve is live when the undo arrives. `InvalidateIdentity` is followed by `Delay { ms: 1 }` so that
    // `FakeWorker::recv` returns `Ok(None)` and the receive loop re-checks the identity BEFORE A's late `ok` result
    // is offered; without the delay `recv` would hand back the result in the same call and no cancel would happen.
    // A's late result then arrives during the 1.5 s cancel window, is not a `result{cancelled}`, and the worker is killed.
    let script = vec![
        FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None },
        FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 10, exploitability_chips: Some(0.5), elapsed_ms: 2 },
        FakeReply::InvalidateIdentity, FakeReply::Delay { ms: 1 },
        FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(sol.clone()), error: None, elapsed_ms: 3 },
        FakeReply::Delay { ms: 1500 },                       // no result{cancelled} within 1.5 s: kill and restart
        // hand B, request 2 (the re-request): request 1's stale result arrives first, then request 2's
        FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None },
        FakeReply::Result { id: IdRef::Fixed("B1".into()), status: ResultStatus::Ok, solution: Some(sol.clone()), error: None, elapsed_ms: 3 },
        FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(sol.clone()), error: None, elapsed_ms: 4 }];
    let (worker, state) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let mut core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&std::env::temp_dir().join("pokerai_race_log")));
    *core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: Some(full(&state_a.board)), ip: Some(full(&state_a.board)) });
    let (sink, events) = RecordingSink::new(clock.clone(), Some(state.clone()));
    let sink: engine::watchdog::SharedSink = Arc::new(Mutex::new(Box::new(sink)));
    let id_a = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); for _ in 0..6 { s.mutate(); } s.next_decision().unwrap() };
    assert_eq!(id_a.hand_revision, 7);
    serve_request(&mut core, LiveRequest { identity: id_a.clone(), state: state_a.clone(), t0_ms: clock.now_ms(), sink: sink.clone() });
    // undo to a non-decision, then hand B; request B once, then re-request (a new decision_id)
    let (id_b1, id_b2) = { let mut s = identity.lock().unwrap(); s.mutate(); s.begin_hand(); let b1 = s.next_decision().unwrap(); let b2 = s.next_decision().unwrap(); (b1, b2) };
    // §4.3's counter is monotonic and never reused, so "hand B with the same displayed revision" is vacuous:
    // B's revision is 9, not A's 7. What the scenario is about is that A's identity is refused and B's accepted.
    assert_eq!((id_b1.hand_revision, id_b2.hand_revision), (9, 9));
    assert!(id_b1.hand_id != id_a.hand_id && id_b2.decision_id > id_b1.decision_id);
    assert!(!identity.lock().unwrap().is_active(&id_a) && !identity.lock().unwrap().is_active(&id_b1));
    serve_request(&mut core, LiveRequest { identity: id_b2.clone(), state: state_a.clone(), t0_ms: clock.now_ms(), sink: sink.clone() });
    let ev = events.lock().unwrap();
    let ids = ident(&ev);
    // A produced Fast and Progress only (its Final was never emitted); nothing carries B1; exactly one Final and it is B2's
    assert!(ids.iter().all(|i| *i == id_a || *i == id_b2), "{ids:?}");
    assert!(!ev.iter().any(|r| matches!(&r.event, RecommendationEvent::Final(x) if x.identity == id_a)));
    assert_eq!(ev.iter().filter(|r| matches!(r.event, RecommendationEvent::Final(_))).count(), 1);
    assert!(ev.iter().any(|r| matches!(&r.event, RecommendationEvent::Final(x) if x.identity == id_b2 && matches!(x.coverage, proto::Coverage::Exact))));
    // no stale snapshot: the store holds B2's solution only; A's request cancelled and then killed the worker
    assert_eq!(core.snapshots.lock().unwrap().for_hand(id_b2.hand_id).len(), 1);
    assert_eq!(core.snapshots.lock().unwrap().for_hand(id_a.hand_id).len(), 0);
    let st = state.lock().unwrap();
    assert_eq!((st.cancels.len(), st.kills), (1, 1));
    // §4.4 accuracy vocabulary: basis points, not raw chips
    let final_rec = ev.iter().find_map(|r| if let RecommendationEvent::Final(x) = &r.event { Some(x) } else { None }).unwrap();
    assert_eq!(final_rec.assumptions.source_accuracy, "exploitability <= 31 bp");   // 0.2 chips of a 65-chip pot
    let _ = Street::River;
}

// --- Beyond the brief's golden: the other paths of `serve_request`, one behaviour per test. Every wait is on the fake
// clock and every reply comes from the scripted worker; nothing here sleeps or spins. Each test logs to a directory of
// its own, read back as the `DecisionRecord`s `serve_request` appended (spec 5 step 10). ---

use engine::log::DecisionRecord;
use engine::testing::{FakeState, Recorded};
use engine::watchdog::SharedSink;
use proto::worker::{EngineMessage, SolveRequest, StreetSolution, WorkerError};
use proto::{ApproxReason, Coverage, HandState, Recommendation, UnsupportedReason};
use std::path::PathBuf;

/// An engine core on a scripted worker and a fake clock, with its own decision log and a recording sink. Both public
/// ranges are the full range as given (the range source blocks the board, never hero's cards).
struct Rig { core: EngineCore, clock: Arc<FakeClock>, identity: Arc<Mutex<IdentityState>>, state: Arc<Mutex<FakeState>>, sink: SharedSink, events: Arc<Mutex<Vec<Recorded>>>, log_dir: PathBuf }

fn rig(name: &str, script: Vec<FakeReply>) -> Rig {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    identity.lock().unwrap().set_config();
    let (worker, state) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let log_dir = std::env::temp_dir().join(format!("pokerai_serve_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&log_dir);
    let core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&log_dir));
    *core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: Some(Range1326([1.0; 1326])), ip: Some(Range1326([1.0; 1326])) });
    let (sink, events) = RecordingSink::new(clock.clone(), Some(state.clone()));
    Rig { core, clock, identity, state, sink: Arc::new(Mutex::new(Box::new(sink))), events, log_dir }
}
/// A new decision (of a new hand, as far as the identity goes), admitted and served now.
fn serve(r: &mut Rig, state: &HandState) -> DecisionIdentity {
    let id = { let mut s = r.identity.lock().unwrap(); s.begin_hand(); s.next_decision().unwrap() };
    serve_request(&mut r.core, LiveRequest { identity: id.clone(), state: state.clone(), t0_ms: r.clock.now_ms(), sink: r.sink.clone() });
    id
}
/// Every event of decision `id` but its `Equity` (the fast-path thread delivers that whenever it finishes).
fn events_of(r: &Rig, id: &DecisionIdentity) -> Vec<Recorded> {
    r.events.lock().unwrap().iter().filter(|e| ident(std::slice::from_ref(*e))[0] == *id && !matches!(e.event, RecommendationEvent::Equity { .. })).cloned().collect()
}
/// The `Final`s of decision `id`, with the fake time and the kill count at their emission.
fn finals(r: &Rig, id: &DecisionIdentity) -> Vec<(u64, u32, Recommendation)> {
    events_of(r, id).into_iter().filter_map(|e| match e.event { RecommendationEvent::Final(x) => Some((e.at_ms, e.kills, x)), _ => None }).collect()
}
fn records(r: &Rig) -> Vec<DecisionRecord> {
    std::fs::read_to_string(r.log_dir.join("decisions.jsonl")).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default()
}
fn kills_and_restarts(r: &Rig) -> (u32, u32) { let s = r.state.lock().unwrap(); (s.kills, s.restarts) }
fn solves(r: &Rig) -> Vec<SolveRequest> { r.state.lock().unwrap().sent.iter().filter_map(|m| if let EngineMessage::Solve(q) = m { Some(q.clone()) } else { None }).collect() }
fn ack() -> FakeReply { FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None } }
fn delay(ms: u64) -> FakeReply { FakeReply::Delay { ms } }
/// A valid solution of `state`'s street root on `template`, at `expl` chips.
fn solution_on(state: &HandState, template: &str, expl: f32) -> StreetSolution {
    let root = core_model::street_root(state).unwrap();
    let b = build_tree_full(&root, &TemplateSelection::from_history(template, &root.history)).unwrap();
    uniform_solution(&b.tree, &b.history, expl)
}
fn result(status: ResultStatus, sol: StreetSolution) -> FakeReply { FakeReply::Result { id: IdRef::Last, status, solution: Some(sol), error: None, elapsed_ms: 5 } }
fn error(code: &str, retryable: bool, estimate_bytes: Option<u64>) -> FakeReply {
    FakeReply::Result { id: IdRef::Last, status: ResultStatus::Error, solution: None, error: Some(WorkerError { code: code.into(), message: code.into(), retryable, estimate_bytes }), elapsed_ms: 1 }
}
fn card(s: &str) -> Card { Card::parse(s).unwrap() }

/// Ruling 20-I1 in the decision log: the street verdict is the first attempt's terminal ARRIVAL against the street
/// deadline (t0 + 2 s on the river), never the time processing finished: on time at the deadline, late 1 ms after it
/// (and still accepted, before final delivery), and not late when the first attempt's terminal arrived on time even
/// though the `_min` retry that answered finished after the deadline. The `Final` and the log name the template that
/// was solved. Hero's cards stay in both public ranges of the solve (CLAUDE.md 6), and the log's range hashes are those
/// of the ranges solved, OOP then IP.
#[test]
fn the_logged_street_verdict_follows_the_first_terminal_arrival() {
    let s = river_state();
    let std_ok = || result(ResultStatus::Ok, solution_on(&s, "river_std_v1", 0.2));
    let cases = [
        ("on_time", vec![ack(), delay(2_000), std_ok()], 2_000u64, false, "river_std_v1"),
        ("late", vec![ack(), delay(2_001), std_ok()], 2_001, true, "river_std_v1"),
        ("retry_after", vec![ack(), delay(1_999), error("tree_too_large", true, Some(1 << 30)), ack(), delay(1_001), result(ResultStatus::Ok, solution_on(&s, "river_min_v1", 0.2))], 3_000, false, "river_min_v1"),
    ];
    for (name, script, at_ms, violation, template) in cases {
        let mut r = rig(name, script);
        let id = serve(&mut r, &s);
        let f = finals(&r, &id);
        assert_eq!(f.len(), 1, "{name}: one Final");
        let (at, _, rec) = &f[0];
        assert!(matches!(rec.coverage, Coverage::Exact) && *at == at_ms, "{name}: {:?} at {at} ms", rec.coverage);
        let signature = { let root = core_model::street_root(&s).unwrap(); let b = build_tree_full(&root, &TemplateSelection::from_history(template, &root.history)).unwrap(); engine::tree::tree_signature(&b.tree, b.pot) };
        assert_eq!((rec.assumptions.template_id.as_str(), &rec.assumptions.tree_signature, rec.assumptions.elapsed_ms), (template, &signature, at_ms as u32), "{name}");
        let recs = records(&r);
        assert_eq!(recs.len(), 1, "{name}: one log record");
        assert_eq!((recs[0].street_violation, recs[0].final_violation, recs[0].template_id.as_str(), recs[0].elapsed_ms, recs[0].reached_bp), (violation, false, template, at_ms as u32, Some(31)), "{name}");
        let req = &solves(&r)[0];
        let aa = proto::combo_index(card("Ah"), card("Ad")) as usize;
        assert_eq!((req.oop_range.0[aa], req.ip_range.0[aa]), (1.0, 1.0), "{name}: hero's combo stays in both public ranges");
        let hash = |x: &Range1326| hex::encode(core_ranges::hash_scaled(x));
        assert_eq!(recs[0].input.range_hashes, vec![hash(&req.oop_range), hash(&req.ip_range)], "{name}");
    }
}

/// Spec 12 ("Street deadline reached"): a river `best_so_far` short of the target is `Approximate{DeadlineBestSoFar}`
/// and is logged as a street violation even though it arrived before the street deadline (only the single-raised-pot
/// flop miss is not, plan 4); its accuracy is shown in basis points (§4.4).
#[test]
fn a_river_best_so_far_is_logged_as_a_street_violation() {
    let s = river_state();
    let mut r = rig("best_so_far", vec![ack(), delay(1_500), result(ResultStatus::BestSoFar, solution_on(&s, "river_std_v1", 1.9))]);
    let id = serve(&mut r, &s);
    let f = finals(&r, &id);
    assert_eq!(f.len(), 1);
    // 1.9 chips of the 65-chip pot is 292.3 bp
    assert_eq!(f[0].2.coverage, Coverage::Approximate { reasons: vec![ApproxReason::DeadlineBestSoFar { reached_bp: 292, target_bp: 50 }] });
    assert_eq!((f[0].2.assumptions.reached_bp, f[0].2.assumptions.source_accuracy.as_str()), (Some(292), "exploitability <= 292 bp"));
    let recs = records(&r);
    assert_eq!((recs.len(), recs[0].street_violation, recs[0].final_violation, recs[0].reached_bp), (1, true, false, Some(292)));
}

/// §7 and rulings 23-I1 / decision 3 of the dispatch: at the watchdog's fire the client stops and cleans nothing up, the
/// watchdog's `Final` goes out (at t0 + 14.9 s, before any kill), and after it the still-busy worker is killed and
/// restarted. Here that restart fails: it is not a panic and not retried; no live worker is left, and the next request
/// relaunches it once (the solve client's own rule) and is answered.
#[test]
fn after_a_deadline_exceeded_final_the_busy_worker_is_killed_and_restarted() {
    let s = river_state();
    let script = vec![
        ack(), FakeReply::Hang,                               // attempt 0: no terminal by its hang bound (2.5 s): restart, `_min` retry
        ack(), FakeReply::Hang,                               // the retry: still running at the watchdog's fire (14.9 s)
        FakeReply::SpawnFails("no worker binary".into()),     // the restart after the Final fails
        ack(), result(ResultStatus::Ok, solution_on(&s, "river_std_v1", 0.2))]; // the next request relaunches the worker
    let mut r = rig("post_final", script);
    let first = serve(&mut r, &s);
    let f = finals(&r, &first);
    assert_eq!(f.len(), 1, "exactly one Final");
    let (at, kills_at_emission, rec) = &f[0];
    assert_eq!(rec.coverage, Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: "building".into() }, partial: vec![] });
    assert_eq!((*at, *kills_at_emission), (14_900, 0), "delivered at the fire, before any kill");
    assert_eq!(kills_and_restarts(&r), (1, 2), "the retry's restart, then the kill and the failed restart after the Final");
    assert!(r.core.worker.ready().is_none(), "a failed restart leaves no live worker");
    let recs = records(&r);
    assert_eq!((recs.len(), recs[0].street_violation, recs[0].final_violation), (1, true, true));
    // the next request relaunches the worker once and is answered
    let second = serve(&mut r, &s);
    let f = finals(&r, &second);
    assert!(f.len() == 1 && matches!(f[0].2.coverage, Coverage::Exact), "{f:?}");
    assert_eq!(kills_and_restarts(&r), (1, 3));
}

/// A `DeadlineExceeded` that is not the watchdog's fire (here the worker's `no_iteration`, on the first attempt and on
/// its `_min` retry) leaves an idle worker: it is answered by the engine's own `Final` and nothing is killed or
/// restarted.
#[test]
fn a_deadline_exceeded_before_the_fire_restarts_nothing() {
    let s = river_state();
    let mut r = rig("no_iteration", vec![ack(), error("no_iteration", false, None), ack(), error("no_iteration", false, None)]);
    let id = serve(&mut r, &s);
    let f = finals(&r, &id);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].2.coverage, Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: "solving".into() }, partial: vec![] });
    assert_eq!((f[0].0, solves(&r).len(), kills_and_restarts(&r)), (0, 2, (0, 0)));
}

/// §4.4 / §5 step 9 / §12: a request whose decision is no longer active when it is served (a mutation landed after the
/// decision was issued) emits nothing, not even `Fast` or `NoDecision`, on the classifier's rows and on the solve path;
/// its solve starts no work, and nothing is registered or logged.
#[test]
fn a_request_for_a_decision_no_longer_active_emits_and_writes_nothing() {
    let mut r = rig("stale", vec![ack(), result(ResultStatus::Ok, solution_on(&river_state(), "river_std_v1", 0.2))]);
    let start = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), Some([card("Ah"), card("Ad")]));
    let preflop = play(&start, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold]);
    let mut stale = Vec::new();
    for state in [start, preflop, river_state()] {
        let id = { let mut s = r.identity.lock().unwrap(); s.begin_hand(); let id = s.next_decision().unwrap(); s.mutate(); id };
        serve_request(&mut r.core, LiveRequest { identity: id.clone(), state, t0_ms: r.clock.now_ms(), sink: r.sink.clone() });
        stale.push(id);
    }
    let events = r.events.lock().unwrap().clone();
    assert!(events.is_empty(), "{:?}", events.iter().map(|e| &e.event).collect::<Vec<_>>());
    assert!(r.state.lock().unwrap().sent.is_empty() && kills_and_restarts(&r) == (0, 0), "a stale decision's solve starts no work");
    assert!(stale.iter().all(|id| r.core.snapshots.lock().unwrap().for_hand(id.hand_id).is_empty()));
    assert!(records(&r).is_empty(), "no Final, so no log record");
}

/// The rows the classifier settles alone (§6) and a range source that refuses the root ranges (§12 `InvalidRanges`,
/// owned by the range source, ruling 27-D3/D4): each answered at once with its one `Final` (or `NoDecision`), no `Fast`,
/// nothing sent to the worker; every `Final` is logged, a `NoDecision` is not (it has none).
#[test]
fn classifier_rows_and_refused_ranges_are_answered_without_the_worker() {
    let mut r = rig("early", vec![]);
    let start = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), Some([card("Ah"), card("Ad")]));
    let unsupported = |message: &str| Coverage::Unsupported { reason: UnsupportedReason::EngineError { message: message.into(), retryable: false }, partial: vec![] };
    // UTG to act: no decision
    let no_decision = serve(&mut r, &start);
    let e = events_of(&r, &no_decision);
    assert!(e.len() == 1 && matches!(&e[0].event, RecommendationEvent::NoDecision { reason, .. } if reason == "another seat is to act"), "{e:?}");
    // BB facing a raise preflop: the plan-3 hook
    let preflop = play(&start, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold]);
    let id = serve(&mut r, &preflop);
    assert_eq!(finals(&r, &id).iter().map(|f| f.2.coverage.clone()).collect::<Vec<_>>(), [unsupported("no preflop path in this build (plan 3)")]);
    // three players see the flop: multiway, no numeric EV
    let three = board(&play(&start, &[Action::Fold, Action::Fold, Action::Call, Action::Call, Action::Fold, Action::Check]), "Kh 7d 2c");
    let id = serve(&mut r, &three);
    assert_eq!(finals(&r, &id).iter().map(|f| f.2.coverage.clone()).collect::<Vec<_>>(), [Coverage::Unsupported { reason: UnsupportedReason::MultiwayEv { pot_eligible: 3 }, partial: vec![] }]);
    // a heads-up flop: the plan-4 hook
    let flop = board(&play(&preflop, &[Action::Call]), "Kh 7d 2c");
    let id = serve(&mut r, &flop);
    assert_eq!(finals(&r, &id).iter().map(|f| f.2.coverage.clone()).collect::<Vec<_>>(), [unsupported("no flop path in this build (plan 4)")]);
    // a river whose root ranges the range source refuses
    *r.core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: None, ip: None });
    let id = serve(&mut r, &river_state());
    assert_eq!(finals(&r, &id).iter().map(|f| f.2.coverage.clone()).collect::<Vec<_>>(), [Coverage::Unsupported { reason: UnsupportedReason::InvalidRanges, partial: vec![] }]);
    for f in r.events.lock().unwrap().iter() { assert!(!matches!(f.event, RecommendationEvent::Fast(_) | RecommendationEvent::Equity { .. })); }
    assert!(r.state.lock().unwrap().sent.is_empty() && kills_and_restarts(&r) == (0, 0), "nothing reaches the worker");
    let logged: Vec<(Street, Coverage)> = records(&r).into_iter().map(|x| (x.street, x.coverage)).collect();
    assert_eq!(logged, [(Street::Preflop, unsupported("no preflop path in this build (plan 3)")),
        (Street::Flop, Coverage::Unsupported { reason: UnsupportedReason::MultiwayEv { pot_eligible: 3 }, partial: vec![] }),
        (Street::Flop, unsupported("no flop path in this build (plan 4)")),
        (Street::River, Coverage::Unsupported { reason: UnsupportedReason::InvalidRanges, partial: vec![] })]);
}

/// §6 facing an all-in, the worker failing (a non-retryable `tree_mismatch`): the analytic fallback answers with fold/call
/// for hero's actual combo, labelled `Approximate{UnconditionedCurrentStreet}`. Hero bet 20 into 65 and was jammed on
/// to 970: hero owes 950, not the 970 wager, so the matched pot is W = 1055 + 2 * 950 - 950 = 2005 (65 + 970 + 970), not
/// 1985, and the 5% rake is capped at 5 chips.
#[test]
fn facing_an_allin_with_a_failed_solve_falls_back_to_the_analytic_answer_on_what_hero_owes() {
    let s = play(&river_state(), &[Action::Bet { to: 20 }, Action::AllIn { to: 970 }]);
    let d = core_model::derive(&s);
    assert_eq!((d.pot, d.facing, d.committed_this_street[2], d.legal.clone()), (1055, 970, 20, vec![proto::LegalAction::Fold, proto::LegalAction::Call { cost: 950 }]));
    let mut r = rig("facing_allin", vec![ack(), error("tree_mismatch", false, None)]);
    let id = serve(&mut r, &s);
    let f = finals(&r, &id);
    assert_eq!(f.len(), 1);
    let rec = &f[0].2;
    assert_eq!(rec.coverage, Coverage::Approximate { reasons: vec![ApproxReason::UnconditionedCurrentStreet] });
    assert_eq!(rec.actions.iter().map(|a| a.action).collect::<Vec<_>>(), [Action::Fold, Action::Call]);
    assert!(rec.actions.iter().all(|a| a.frequency.is_some() && a.ev_bb.is_some()) && rec.actions.iter().filter(|a| a.headline).count() == 1, "{:?}", rec.actions);
    let note = rec.assumptions.notes.iter().find(|n| n.starts_with("analytic all-in fallback")).unwrap_or_else(|| panic!("{:?}", rec.assumptions.notes));
    assert!(note.contains(", W 2005, R 5.00, "), "{note}");
    assert_eq!((solves(&r).len(), kills_and_restarts(&r)), (1, (0, 0)));
}
