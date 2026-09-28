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
    // 1.9 chips of the 65-chip pot is 292.3 bp: displayed rounded (292), bounded from above (293, ruling 28-I5)
    assert_eq!(f[0].2.coverage, Coverage::Approximate { reasons: vec![ApproxReason::DeadlineBestSoFar { reached_bp: 292, target_bp: 50 }] });
    assert_eq!((f[0].2.assumptions.reached_bp, f[0].2.assumptions.source_accuracy.as_str()), (Some(292), "exploitability <= 293 bp"));
    let recs = records(&r);
    assert_eq!((recs.len(), recs[0].street_violation, recs[0].final_violation, recs[0].reached_bp), (1, true, false, Some(292)));
}

/// Ruling 28-I5 (§4.4 "exploitability <= x"): the accuracy string is a true upper bound, the ceiling of the raw
/// measurement in basis points of the solved pot computed in f64, never the rounded or saturated display value
/// `reached_bp`: an exact whole number stays itself (0.8125 chips of 65 is exactly 125 bp), a value rounded down for
/// display is bounded by the next whole number (1.9 chips: 292.3 bp shows 292, bounded by 293), a measurement just
/// under the target rounds and bounds alike (0.325f32 chips: 49.99999816 bp), and one past the `u16` display domain
/// (1 000 chips: 153 846.2 bp, displayed as 65 535) is bounded by its own value.
#[test]
fn the_accuracy_string_is_a_true_upper_bound_on_the_raw_measurement() {
    let s = river_state();
    for (name, status, chips, reached_bp, bound) in [
        ("exact", ResultStatus::BestSoFar, 0.8125f32, 125u16, "125"),
        ("rounded_down", ResultStatus::BestSoFar, 1.9, 292, "293"),
        ("under_target", ResultStatus::Ok, 0.325, 50, "50"),
        ("over_u16", ResultStatus::BestSoFar, 1_000.0, u16::MAX, "153847"),
    ] {
        let mut r = rig(&format!("accuracy_{name}"), vec![ack(), result(status, solution_on(&s, "river_std_v1", chips))]);
        let id = serve(&mut r, &s);
        let f = finals(&r, &id);
        assert_eq!(f.len(), 1, "{name}");
        let a = &f[0].2.assumptions;
        assert_eq!((a.reached_bp, a.source_accuracy.clone()), (Some(reached_bp), format!("exploitability <= {bound} bp")), "{name}: {chips} chips of 65");
    }
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

/// A sink that re-enters the engine from its callback: it takes the shared identity lock (`try_lock`, so a callback run
/// under that lock is recorded instead of deadlocking the test), reads the active decision, and on the first `Fast`
/// mutates the hand, as a UI command handler on the delivery thread could.
struct ReentrantSink { identity: Arc<Mutex<IdentityState>>, seen: Arc<Mutex<Vec<String>>>, mutated: bool }
impl engine::EventSink for ReentrantSink {
    fn emit(&mut self, ev: RecommendationEvent) {
        let kind = match &ev { RecommendationEvent::Fast(_) => "Fast", RecommendationEvent::Final(_) => "Final", RecommendationEvent::NoDecision { .. } => "NoDecision",
            RecommendationEvent::Equity { .. } => "Equity", RecommendationEvent::Progress { .. } => "Progress", RecommendationEvent::Provisional(_) => "Provisional" };
        match r#try(&self.identity) {
            Some(mut ids) => {
                let active = ids.active().is_some();
                if kind == "Fast" && !self.mutated {
                    ids.mutate();
                    self.mutated = true;
                }
                self.seen.lock().unwrap().push(format!("{kind}: active {active}"));
            }
            None => self.seen.lock().unwrap().push(format!("{kind}: the identity lock was held during the callback")),
        }
    }
}
/// The identity lock if it is free, `None` if some thread holds it.
fn r#try(m: &Mutex<IdentityState>) -> Option<std::sync::MutexGuard<'_, IdentityState>> {
    match m.try_lock() { Ok(g) => Some(g), Err(std::sync::TryLockError::WouldBlock) => None, Err(std::sync::TryLockError::Poisoned(p)) => panic!("identity lock poisoned: {p}") }
}

/// Ruling 28-I1: an event is accepted under the identity lock (the stale check and the `Final` claim) and handed to the
/// sink only after that lock is released, so a sink may read the identity and even mutate the hand from its callback.
/// A `NoDecision`, a classifier `Final`, a refused-ranges `Final` and a `Fast` whose callback supersedes its own
/// decision all complete: no callback ran under the identity lock, and the superseded request sends nothing.
#[test]
fn a_sink_that_reads_the_identity_and_mutates_from_its_callback_completes() {
    let s = river_state();
    let mut r = rig("reentrant", vec![ack(), result(ResultStatus::Ok, solution_on(&s, "river_std_v1", 0.2))]);
    let seen = Arc::new(Mutex::new(Vec::new()));
    r.sink = Arc::new(Mutex::new(Box::new(ReentrantSink { identity: r.identity.clone(), seen: seen.clone(), mutated: false })));
    let start = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), Some([card("Ah"), card("Ad")]));
    let preflop = play(&start, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold]);
    serve(&mut r, &start);
    serve(&mut r, &preflop);
    *r.core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: None, ip: None });
    serve(&mut r, &s);
    *r.core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: Some(Range1326([1.0; 1326])), ip: Some(Range1326([1.0; 1326])) });
    let superseded = serve(&mut r, &s);
    assert_eq!(*seen.lock().unwrap(), ["NoDecision: active true", "Final: active true", "Final: active true", "Fast: active true"]);
    assert!(!r.identity.lock().unwrap().is_active(&superseded) && solves(&r).is_empty(), "the Fast's callback superseded its decision before the solve");
}

// --- Handshakes for the delivery races (fix round 1). A `FinalGate` sink acknowledges each `Final` it records; the
// `WatchdogFirst` clock makes `engine-main`'s first reading at or after the watchdog's fire wait for that
// acknowledgement, so the watchdog deterministically wins the delivery. Every wait is bounded by `ACK_LIVENESS` of wall
// time (ruling 20-A), a liveness allowance, never a condition on the engine's time. ---

use engine::ranges::{RangeSource, RootRanges};
use engine::testing::ACK_LIVENESS;
use std::sync::Condvar;

/// How many `Final`s a `FinalGate` recorded, and the condition variable each one notifies.
type Finals = Arc<(Mutex<u32>, Condvar)>;
struct FinalGate { inner: RecordingSink, finals: Finals }
impl engine::EventSink for FinalGate {
    fn emit(&mut self, ev: RecommendationEvent) {
        let is_final = matches!(ev, RecommendationEvent::Final(_));
        self.inner.emit(ev);
        if is_final {
            let (count, recorded) = &*self.finals;
            *count.lock().unwrap() += 1;
            recorded.notify_all();
        }
    }
}
/// Blocks until `n` `Final`s are recorded; fails naming the acknowledgement after `ACK_LIVENESS`.
fn wait_finals(finals: &Finals, n: u32) {
    let (count, recorded) = &**finals;
    let deadline = std::time::Instant::now() + ACK_LIVENESS;
    let mut c = count.lock().unwrap();
    while *c < n {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!left.is_zero(), "FinalGate: {n} Final(s) awaited, {} recorded, no acknowledgement within the {ACK_LIVENESS:?} liveness bound", *c);
        c = recorded.wait_timeout(c, left).unwrap().0;
    }
}
/// The fake clock as a slow `engine-main` sees it: the engine thread's first reading at or after `fire_ms` waits until
/// the watchdog's `Final` is recorded. Other threads (the watchdog, the fast path) read the fake clock as it is.
struct WatchdogFirst { fake: Arc<FakeClock>, engine: std::thread::ThreadId, fire_ms: u64, finals: Finals, waited: std::sync::atomic::AtomicBool }
impl Clock for WatchdogFirst {
    fn now_ms(&self) -> u64 {
        let t = self.fake.now_ms();
        if t >= self.fire_ms && std::thread::current().id() == self.engine && !self.waited.swap(true, std::sync::atomic::Ordering::SeqCst) {
            wait_finals(&self.finals, 1);
        }
        t
    }
    fn wait_until(&self, t_ms: u64) { self.fake.wait_until(t_ms) }
}
/// A rig whose sink is a `FinalGate` and whose engine clock is `WatchdogFirst` for a river request admitted at 0.
fn watchdog_first_rig(name: &str, script: Vec<FakeReply>) -> (Rig, Finals) {
    let mut r = rig(name, vec![]);
    let finals: Finals = Arc::default();
    let clock: Arc<dyn Clock> = Arc::new(WatchdogFirst { fake: r.clock.clone(), engine: std::thread::current().id(), fire_ms: 14_900, finals: finals.clone(), waited: Default::default() });
    let (worker, state) = FakeWorker::scripted(r.clock.clone(), r.identity.clone(), script);
    let range_source = std::mem::replace(&mut *r.core.range_source.lock().unwrap(), Box::new(ExplicitRanges { oop: None, ip: None }));
    r.core = EngineCore::new(worker, clock, r.identity.clone(), DecisionLog::open(&r.log_dir));
    *r.core.range_source.lock().unwrap() = range_source;
    let (inner, events) = RecordingSink::new(r.clock.clone(), Some(state.clone()));
    (Rig { sink: Arc::new(Mutex::new(Box::new(FinalGate { inner, finals: finals.clone() }))), events, state, ..r }, finals)
}
/// The full public ranges, plus one inherited reason the range source reports (a replay-backed source's).
struct WithReason(ApproxReason);
impl RangeSource for WithReason {
    fn ranges_at_root(&self, state: &HandState, root: &proto::StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason> {
        let mut r = ExplicitRanges { oop: Some(Range1326([1.0; 1326])), ip: Some(Range1326([1.0; 1326])) }.ranges_at_root(state, root)?;
        r.reasons.push(self.0.clone());
        Ok(r)
    }
}

/// Ruling 28-I6 (spec 6: inherited reasons survive into an `Unsupported` result): the watchdog's fallback is read at the
/// fire from a slot the request refreshes once the range source has answered, so a request whose worker hangs to the
/// fire gets a watchdog `Final` that keeps the range source's reason and the ranges used, and the decision log records
/// that same `Final`.
#[test]
fn a_watchdog_final_keeps_the_range_sources_reasons() {
    let sentinel = ApproxReason::UnconditionedPriorStreet { street: Street::Turn, seat: Seat(0), cause: "sentinel".into() };
    let (mut r, _finals) = watchdog_first_rig("watchdog_reasons", vec![ack(), FakeReply::Hang, ack(), FakeReply::Hang]);
    *r.core.range_source.lock().unwrap() = Box::new(WithReason(sentinel.clone()));
    let id = serve(&mut r, &river_state());
    let f = finals(&r, &id);
    assert_eq!(f.len(), 1);
    let (at, _, rec) = &f[0];
    assert_eq!((*at, &rec.coverage), (14_900, &Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: "building".into() }, partial: vec![sentinel.clone()] }));
    assert_eq!(rec.assumptions.ranges_used.len(), 2, "the ranges used survive into the watchdog Final");
    let recs = records(&r);
    assert_eq!((recs.len(), &recs[0].coverage, &recs[0].reasons), (1, &rec.coverage, &vec![sentinel]));
}

/// A range source slow enough that the watchdog fires while it answers: it moves the fake clock to the fire and returns
/// the full ranges once the watchdog's `Final` is recorded.
struct SlowRanges { clock: Arc<FakeClock>, fire_ms: u64, finals: Finals }
impl RangeSource for SlowRanges {
    fn ranges_at_root(&self, state: &HandState, root: &proto::StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason> {
        self.clock.set_ms(self.fire_ms);
        wait_finals(&self.finals, 1);
        ExplicitRanges { oop: Some(Range1326([1.0; 1326])), ip: Some(Range1326([1.0; 1326])) }.ranges_at_root(state, root)
    }
}

/// Ruling 28-I3 (spec 12: restart the worker "if a job was running"): the cleanup after a `DeadlineExceeded` `Final` is
/// conditioned on the solve client's liveness provenance, not on the clock. A request whose range retrieval ran past the
/// watchdog's fire reaches the solve with no room to send anything: its `DeadlineExceeded` leaves an idle worker, which
/// is neither killed nor restarted.
#[test]
fn a_deadline_exceeded_with_no_job_outstanding_restarts_nothing() {
    let (mut r, gate) = watchdog_first_rig("slow_ranges", vec![]);
    *r.core.range_source.lock().unwrap() = Box::new(SlowRanges { clock: r.clock.clone(), fire_ms: 14_900, finals: gate });
    let id = serve(&mut r, &river_state());
    let f = finals(&r, &id);
    assert!(f.len() == 1 && matches!(&f[0].2.coverage, Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { .. }, .. }), "{f:?}");
    assert_eq!((solves(&r).len(), kills_and_restarts(&r)), (0, (0, 0)), "nothing was sent, so nothing is cleaned up");
}

// --- The delivery race after the solve (ruling 28-I2) and the equity's cancellation (ruling 28-I4), through the test
// seams of `serve_request_with`: `before_claim` runs on `engine-main` with the candidate `Final` assembled, right before
// its delivery is claimed; `equity` replaces the fast-path equity routine. ---

use engine::equity::pending_summary;
use engine::serve::{serve_request_with, EquityRoutine, ServeSeams};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

fn serve_with(r: &mut Rig, state: &HandState, seams: ServeSeams) -> DecisionIdentity {
    let id = { let mut s = r.identity.lock().unwrap(); s.begin_hand(); s.next_decision().unwrap() };
    serve_request_with(&mut r.core, LiveRequest { identity: id.clone(), state: state.clone(), t0_ms: r.clock.now_ms(), sink: r.sink.clone() }, seams);
    id
}
/// A rig whose sink is a `FinalGate`, on the plain fake clock.
fn gated_rig(name: &str, script: Vec<FakeReply>) -> (Rig, Finals) {
    let mut r = rig(name, script);
    let finals: Finals = Arc::default();
    let (inner, events) = RecordingSink::new(r.clock.clone(), Some(r.state.clone()));
    r.sink = Arc::new(Mutex::new(Box::new(FinalGate { inner, finals: finals.clone() })));
    (Rig { events, ..r }, finals)
}
/// A `before_claim` seam: the watchdog's fire lands while the candidate is prepared, and it has delivered its `Final`
/// before the engine claims the delivery.
fn fire_before_claim(r: &Rig, finals: &Finals) -> ServeSeams {
    let (clock, finals) = (r.clock.clone(), finals.clone());
    ServeSeams { before_claim: Some(Arc::new(move || { clock.set_ms(14_900); wait_finals(&finals, 1); })), ..ServeSeams::default() }
}
/// The one `Final` of `id`, which must be the watchdog's at 14 900 ms, and the one log record, which must be that same
/// `Final`: its coverage, its reasons and its delivery time (ruling 28-I2).
fn assert_the_watchdog_final_is_delivered_and_logged(r: &Rig, id: &DecisionIdentity, what: &str) {
    let f = finals(r, id);
    assert_eq!(f.len(), 1, "{what}: one Final");
    let (at, kills, rec) = &f[0];
    assert_eq!((*at, *kills, &rec.coverage), (14_900, 0, &Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: "building".into() }, partial: vec![] }), "{what}");
    let recs = records(r);
    assert_eq!(recs.len(), 1, "{what}: one log record");
    assert_eq!((&recs[0].coverage, &recs[0].reasons, recs[0].elapsed_ms, recs[0].final_violation, recs[0].reached_bp, recs[0].template_id.as_str()),
        (&rec.coverage, &vec![], 14_900, true, None, rec.assumptions.template_id.as_str()), "{what}: the log records the Final delivered");
    assert!(r.core.snapshots.lock().unwrap().for_hand(id.hand_id).is_empty(), "{what}: no losing solution is registered");
}

/// Ruling 28-I2: a solve that answered before the fire, whose `Final` is still being assembled when the watchdog fires,
/// loses the delivery. The watchdog's `Final` is the one delivered, the decision log records it (its coverage, reasons
/// and delivery time) and not the discarded candidate, and the losing solution is not registered as a snapshot. The
/// job had ended, so nothing is restarted.
#[test]
fn a_watchdog_final_during_the_assembly_is_the_one_delivered_and_logged() {
    let s = river_state();
    let (mut r, gate) = gated_rig("watchdog_wins_assembly", vec![ack(), result(ResultStatus::Ok, solution_on(&s, "river_std_v1", 0.2))]);
    let seams = fire_before_claim(&r, &gate);
    let id = serve_with(&mut r, &s, seams);
    assert_the_watchdog_final_is_delivered_and_logged(&r, &id, "solved candidate");
    assert_eq!(kills_and_restarts(&r), (0, 0));
}

/// Ruling 28-I2 on the all-in path: facing an all-in with the worker failing, the analytic fallback's candidate loses to
/// a watchdog fire landing while it is prepared; the watchdog's `Final` is delivered and logged, never the analytic one.
#[test]
fn a_watchdog_final_during_the_allin_fallback_is_the_one_delivered_and_logged() {
    let s = play(&river_state(), &[Action::Bet { to: 20 }, Action::AllIn { to: 970 }]);
    let (mut r, gate) = gated_rig("watchdog_wins_allin", vec![ack(), error("tree_mismatch", false, None)]);
    let seams = fire_before_claim(&r, &gate);
    let id = serve_with(&mut r, &s, seams);
    assert_the_watchdog_final_is_delivered_and_logged(&r, &id, "analytic candidate");
}

/// One command to an acknowledged equity runner: run the next unit of work (replying whether it stopped instead,
/// having found its cancellation set), or finish.
enum Step { Unit(mpsc::Sender<bool>), Finish }
/// An equity routine that acknowledges each call twice, handing the test a command channel and telling the worker link
/// it started, then runs units of work on command, polling its cancellation before each one (as
/// `equity::equity_summary_with_clock` does between estimates). A call the test no longer listens for returns at once.
fn acknowledged_equity(to_test: mpsc::Sender<mpsc::Sender<Step>>, to_link: mpsc::Sender<()>) -> EquityRoutine {
    let senders = Mutex::new((to_test, to_link));
    Arc::new(move |_: &dyn Clock, _: Option<[Card; 2]>, _: &Range1326, _: &[(Seat, Range1326)], _: &[Card], _: Duration, cancel: &AtomicBool| {
        let (tx, rx) = mpsc::channel();
        {
            let s = senders.lock().unwrap();
            if s.0.send(tx).is_err() || s.1.send(()).is_err() { return pending_summary(&[]); }
        }
        while let Ok(Step::Unit(reply)) = rx.recv_timeout(ACK_LIVENESS) {
            let stopped = cancel.load(std::sync::atomic::Ordering::SeqCst);
            let _ = reply.send(stopped);
            if stopped { break; }
        }
        pending_summary(&[])
    })
}
/// The scripted worker behind a link that sends a solve only once the request's equity runner has started, so the
/// runner is running (past the fast path's own stale check) before anything the script does to the request.
struct AfterEquityStarts { inner: Box<dyn engine::worker::link::WorkerLink>, runner_started: mpsc::Receiver<()> }
impl engine::worker::link::WorkerLink for AfterEquityStarts {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), engine::worker::link::WorkerLinkError> {
        if matches!(msg, EngineMessage::Solve(_)) { received(&self.runner_started, "the request's equity runner started before its solve"); }
        self.inner.send(msg)
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<proto::worker::WorkerMessage>, engine::worker::link::WorkerLinkError> { self.inner.recv(timeout) }
    fn restart(&mut self) -> Result<(), engine::worker::link::WorkerLinkError> { self.inner.restart() }
    fn kill(&mut self) { self.inner.kill() }
    fn ready(&self) -> Option<&proto::worker::Ready> { self.inner.ready() }
}
fn received<T>(rx: &mpsc::Receiver<T>, what: &str) -> T {
    rx.recv_timeout(ACK_LIVENESS).unwrap_or_else(|e| panic!("{what}: no acknowledgement within the {ACK_LIVENESS:?} liveness bound ({e})"))
}
/// Whether the runner stopped at its next unit of work (its cancellation set) rather than running it.
fn stopped_at_next_unit(runner: &mpsc::Sender<Step>) -> bool {
    let (tx, rx) = mpsc::channel();
    runner.send(Step::Unit(tx)).unwrap();
    received(&rx, "the equity runner answered its unit of work")
}

/// Ruling 28-I4 (spec 7: the equity phase is cancellable; spec 5 step 3: a mutation cancels in-flight work): each
/// request's equity runs with its own cancellation token. Its own `Final` does not set it (a late `Equity` of the
/// active decision is still wanted); a newer request sets the older request's token before its own work starts; a
/// request superseded while it solves sets its own. The runner, polling its token between units, stops.
#[test]
fn superseding_a_request_cancels_its_equity_and_its_own_final_does_not() {
    let s = river_state();
    let script = vec![ack(), result(ResultStatus::Ok, solution_on(&s, "river_std_v1", 0.2)),   // request 1: answered
        ack(), FakeReply::InvalidateIdentity, delay(1), FakeReply::Hang];                         // request 2: superseded while solving
    let mut r = rig("equity_cancel", vec![]);
    let (to_test, started) = mpsc::channel();
    let (to_link, runner_started) = mpsc::channel();
    let (worker, state) = FakeWorker::scripted(r.clock.clone(), r.identity.clone(), script);
    r.core = EngineCore::new(Box::new(AfterEquityStarts { inner: worker, runner_started }), r.clock.clone(), r.identity.clone(), DecisionLog::open(&r.log_dir));
    *r.core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: Some(Range1326([1.0; 1326])), ip: Some(Range1326([1.0; 1326])) });
    r.state = state;
    let seams = ServeSeams { equity: Some(acknowledged_equity(to_test, to_link)), ..ServeSeams::default() };
    let first = serve_with(&mut r, &s, seams.clone());
    let runner1 = received(&started, "request 1's equity runner started");
    assert_eq!(finals(&r, &first).len(), 1);
    assert!(!stopped_at_next_unit(&runner1), "request 1's own Final does not cancel its equity");
    let second = serve_with(&mut r, &s, seams);
    let runner2 = received(&started, "request 2's equity runner started");
    assert!(!r.identity.lock().unwrap().is_active(&second) && finals(&r, &second).is_empty());
    assert!(stopped_at_next_unit(&runner1), "the newer request cancelled request 1's equity");
    assert!(stopped_at_next_unit(&runner2), "request 2, superseded while it solved, cancelled its own equity");
    let _ = (runner1.send(Step::Finish), runner2.send(Step::Finish));
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
