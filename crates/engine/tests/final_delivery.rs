use engine::core::EngineCore;
use engine::identity::IdentityState;
use engine::log::DecisionLog;
use engine::ranges::ExplicitRanges;
use engine::serve::{serve_request, LiveRequest};
use engine::testing::{board, hand, play, FakeClock, FakeReply, FakeWorker, IdRef, RecordingSink};
use proto::worker::{AckStatus, ResultStatus, Stage, WorkerError};
use proto::{Action, Card, Coverage, Range1326, RecommendationEvent, Seat, UnsupportedReason};
use std::sync::{Arc, Mutex};

fn river_state() -> proto::HandState {
    let aa = Some([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]);
    let s = play(&hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), aa), &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call]);
    board(&play(&board(&play(&board(&s, "Kh 7d 2c"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d 9s")
}
fn full(board: &[Card]) -> Range1326 { let mut r = Range1326([1.0; 1326]); for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { r.0[i] = 0.0; } } r }
fn ack() -> FakeReply { FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None } }

fn run(script: Vec<FakeReply>, expected_stage: &str) {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let state = river_state();
    let (worker, fake) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let mut core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&std::env::temp_dir().join("pokerai_delivery_log")));
    *core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: Some(full(&state.board)), ip: Some(full(&state.board)) });
    let (sink, events) = RecordingSink::new(clock.clone(), Some(fake.clone()));
    let id = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
    let req = LiveRequest::admitted(&core, id.clone(), state, 0, Arc::new(Mutex::new(Box::new(sink))));
    serve_request(&mut core, req);
    let ev = events.lock().unwrap();
    let finals: Vec<_> = ev.iter().filter(|r| matches!(r.event, RecommendationEvent::Final(_))).collect();
    assert_eq!(finals.len(), 1, "exactly one Final");
    let f = finals[0];
    assert_eq!((f.at_ms, f.kills), (14_900, 0), "Final at t0 + 14.9 s before any kill or restart");
    match &f.event { RecommendationEvent::Final(r) => match &r.coverage { Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } => assert_eq!(stage, expected_stage), c => panic!("{c:?}") }, _ => unreachable!() }
    assert!(fake.lock().unwrap().kills >= 1, "the kill/restart proceeds after delivery");
}

#[test]
fn final_delivery_independent_of_worker() {
    // (a) a worker that never replies: both attempts hang, the stage never leaves Building
    run(vec![FakeReply::Hang], "building");
    // (b) no_iteration on the first attempt, then the retry hangs: still Building
    run(vec![ack(), FakeReply::Result { id: IdRef::Last, status: ResultStatus::Error, solution: None, error: Some(WorkerError { code: "no_iteration".into(), message: "".into(), retryable: false, estimate_bytes: None }), elapsed_ms: 1 }, ack(), FakeReply::Hang], "building");
    // (c) hangs in extraction: the reported stage is the FURTHEST reached, so the `_min` retry's Building does not
    //     rewind it to "building" (`EngineCore::set_stage` only advances)
    run(vec![ack(), FakeReply::Progress { id: IdRef::Last, stage: Stage::Extracting, iterations: 40, exploitability_chips: Some(0.3), elapsed_ms: 9 }, FakeReply::Hang], "extracting");
}
