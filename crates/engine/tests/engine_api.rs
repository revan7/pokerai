use engine::core::EngineCore;
use engine::identity::IdentityState;
use engine::log::DecisionLog;
use engine::testing::{cfg_1_2, FakeClock, FakeReply, FakeWorker};
use engine::{Engine, EngineError};
use proto::{Card, Seat, SolverPrefs};
use std::sync::{Arc, Mutex};

fn engine() -> Engine {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let (worker, _state) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![FakeReply::Hang]);
    Engine::with_core(EngineCore::new(worker, clock, identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_engine_api_log"))))
}
fn begin() -> proto::BeginHand {
    proto::BeginHand { button: Seat(0), hero: Seat(2), dealt: (0..6).map(Seat).collect(), stacks: vec![1000; 6], hero_cards: None }
}

#[test]
fn set_config_validates_and_queues() {
    let (mut cfg, _) = cfg_1_2();
    let mut e = engine();
    cfg.solver = SolverPrefs { threads: 16, target_bp: 50, flop_budget_s: 10 };
    let rev1 = e.set_config(cfg.clone()).unwrap();
    assert!(rev1 >= 1);
    // §13.3: 31 is rejected by set_config; 0 too. The revision is not consumed by a rejected config.
    cfg.solver.flop_budget_s = 31;
    assert!(matches!(e.set_config(cfg.clone()), Err(EngineError::Message(ref m)) if m.contains("flop_budget_s")));
    cfg.solver.flop_budget_s = 0;
    assert!(e.set_config(cfg.clone()).is_err());
    cfg.solver.flop_budget_s = 30;
    let rev2 = e.set_config(cfg.clone()).unwrap();
    assert_eq!(rev2, rev1 + 1, "neither rejected config consumed a revision");
    // §4.2: a config set during a hand is queued for the NEXT hand; the active hand keeps its frozen HandConfig
    let s = e.begin_hand(begin()).unwrap();
    assert_eq!(s.config.config_revision, rev2);
    cfg.solver.flop_budget_s = 12;
    cfg.bb_chips = 20;
    let rev3 = e.set_config(cfg.clone()).unwrap();
    assert_eq!(rev3, rev2 + 1, "the next accepted config takes the next revision");
    assert_eq!(e.state().unwrap().config.bb_chips, 10, "the active hand's config is frozen");
    let next = e.begin_hand(begin()).unwrap();
    assert_eq!((next.config.bb_chips, next.config.config_revision), (20, rev3));
    e.shutdown();
}

#[test]
fn hero_cards_and_shutdown_are_idempotent() {
    let (cfg, _) = cfg_1_2();
    let mut e = engine();
    e.set_config(cfg).unwrap();
    assert!(matches!(e.set_hero_cards([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]), Err(EngineError::Message(_))));   // no hand
    let s = e.begin_hand(begin()).unwrap();
    let rev = s.hand_revision;
    let s = e.set_hero_cards([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]).unwrap();
    assert_eq!(s.hero_cards, Some([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]));
    assert!(s.hand_revision > rev, "set_hero_cards is a mutation: fresh revision, in-flight work invalidated");
    // the startup report is available without a real worker and carries the fake's advertised features
    let rep = e.startup_report();
    assert!(rep.worker_ready && rep.build_features.iter().any(|f| f == "avx2") && !rep.cpu_lacks_avx2);
    e.shutdown();
    e.shutdown();   // once-only: the second call is a no-op, not a panic
}

// ---- Beyond the brief's two tests (P2.T29 carries): the equity cancellation token on every invalidation (fix-Q1,
// ---- fix-D1 acceptance), the §12 rejected admission, `recommend`'s preconditions, and `Engine::new` itself.

use engine::testing::RecordingSink;
use proto::{Action, DecisionIdentity};
use std::sync::atomic::{AtomicBool, Ordering};

/// `EngineCore::equity_cancel`: the equity cancellation token of the request served last (ruling 28-I4).
type Tokens = Arc<Mutex<Option<Arc<AtomicBool>>>>;

/// `engine()` with a handle on the core's equity token slot, taken before the core is handed to `Engine`.
fn engine_with_tokens() -> (Engine, Tokens) {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let (worker, _state) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![FakeReply::Hang]);
    let core = EngineCore::new(worker, clock, identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_engine_api_log")));
    let tokens = core.equity_cancel.clone();
    (Engine::with_core(core), tokens)
}

/// Serves one request of the current decision on `engine-main` and returns its identity and the equity token
/// `serve_request` installed as it started. The request's one event (a preflop `Final` or a `NoDecision` in this plan)
/// acknowledges that it was served; nothing has superseded it yet, so its token is not set.
fn served(e: &mut Engine, tokens: &Tokens) -> (DecisionIdentity, Arc<AtomicBool>) {
    let before = tokens.lock().unwrap().clone();
    let (sink, recorder) = RecordingSink::notifying(FakeClock::new(), None);
    let id = e.recommend(Box::new(sink)).unwrap();
    recorder.wait_for(1);
    let token = tokens.lock().unwrap().clone().expect("serve_request installs its request's equity token as it starts");
    assert!(before.is_none_or(|b| !Arc::ptr_eq(&b, &token)), "each request has its own token");
    assert!(!token.load(Ordering::SeqCst), "nothing has superseded the request yet");
    (id, token)
}

/// A request is served, then `op` supersedes its decision: its equity must be cancelled by `op` itself (a mutation
/// with no newer request to replace the token, fix-Q1).
fn cancels(e: &mut Engine, tokens: &Tokens, what: &str, op: impl FnOnce(&mut Engine)) {
    let (_, token) = served(e, tokens);
    op(e);
    assert!(token.load(Ordering::SeqCst), "{what}: the superseded request's equity is cancelled");
}

/// Fix-Q1 (Task 28 fix round 1) with the fix-D1 carry: every call that invalidates the active decision sets the
/// equity token of the request served last, inside the identity-lock step that invalidates it. A `cancel` naming a
/// decision that is not the active one invalidates nothing and cancels nothing.
#[test]
fn every_invalidation_cancels_the_equity_of_the_request_it_supersedes() {
    let (cfg, _) = cfg_1_2();
    let c = |s: &str| Card::parse(s).unwrap();
    let (mut e, tokens) = engine_with_tokens();
    e.set_config(cfg.clone()).unwrap();
    e.begin_hand(begin()).unwrap();
    cancels(&mut e, &tokens, "set_hero_cards", |e| { e.set_hero_cards([c("Ah"), c("Ad")]).unwrap(); });
    cancels(&mut e, &tokens, "apply_action", |e| { e.apply_action(Action::Fold).unwrap(); });
    cancels(&mut e, &tokens, "undo", |e| { e.undo().unwrap(); });
    // Ruling F-I4: a settings change mid-hand supersedes nothing, so it cancels nothing.
    let (_, token) = served(&mut e, &tokens);
    e.set_config(cfg.clone()).unwrap();
    assert!(!token.load(Ordering::SeqCst), "set_config: a mid-hand settings change does not cancel the decision's equity");
    let (active, token) = served(&mut e, &tokens);
    e.cancel(active.decision_id + 1);
    assert!(!token.load(Ordering::SeqCst), "cancelling a decision that is not the active one changes nothing");
    e.cancel(active.decision_id);
    assert!(token.load(Ordering::SeqCst), "cancel: the cancelled decision's equity is cancelled");
    for a in [Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call] {
        e.apply_action(a).unwrap();
    }
    cancels(&mut e, &tokens, "set_board", |e| { e.set_board(&[c("Kh"), c("7d"), c("2c")]).unwrap(); });
    cancels(&mut e, &tokens, "finish_hand", |e| e.finish_hand());
    e.begin_hand(begin()).unwrap();
    cancels(&mut e, &tokens, "begin_hand over a hand in progress", |e| { e.begin_hand(begin()).unwrap(); });
    cancels(&mut e, &tokens, "abandon_hand", |e| e.abandon_hand());
    e.shutdown();
}

/// A sink whose first event holds `engine-main` inside the callback: it acknowledges the entry, then waits for the
/// test's release (bounded by the liveness allowance, and released at once if the test's end drops the sender, so a
/// failed assertion never leaves `engine-main` held behind a `shutdown`).
struct HeldSink { entered: std::sync::mpsc::Sender<()>, release: std::sync::mpsc::Receiver<()>, held: bool }
impl HeldSink {
    fn new() -> (HeldSink, std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        (HeldSink { entered: entered_tx, release: release_rx, held: false }, entered_rx, release_tx)
    }
}
impl engine::EventSink for HeldSink {
    fn emit(&mut self, _ev: proto::RecommendationEvent) {
        if self.held { return; }
        self.held = true;
        let _ = self.entered.send(());
        let _ = self.release.recv_timeout(engine::testing::ACK_LIVENESS);
    }
}

/// 29-I1: `recommend` supersedes the decision before it, and cancels the equity of the request served last in that
/// same identity-lock hold (ruling 28-I4), not later when `engine-main` starts the new request. The first request is
/// held inside its sink callback on `engine-main`; a second recommendation is accepted meanwhile, and the first
/// request's token is already set when that `recommend` returns, before the callback is released. Released and joined
/// before anything is asserted.
#[test]
fn recommend_cancels_the_superseded_requests_equity_at_once() {
    let (cfg, _) = cfg_1_2();
    let (mut e, tokens) = engine_with_tokens();
    e.set_config(cfg).unwrap();
    e.begin_hand(begin()).unwrap();
    let (held, entered, release) = HeldSink::new();
    let first = e.recommend(Box::new(held)).unwrap();
    let reached = entered.recv_timeout(engine::testing::ACK_LIVENESS).is_ok();
    let token = tokens.lock().unwrap().clone();
    let before = token.as_ref().map(|t| t.load(Ordering::SeqCst));
    let (sink, recorder) = RecordingSink::notifying(FakeClock::new(), None);
    let second = e.recommend(Box::new(sink)).unwrap();
    let after = token.as_ref().map(|t| t.load(Ordering::SeqCst));
    let _ = release.send(());
    recorder.wait_for(1);
    e.shutdown();
    assert!(reached, "the first request reached its sink callback");
    assert_eq!(before, Some(false), "the first request's equity runs while it is the active decision");
    assert_eq!(after, Some(true), "the second recommendation cancelled the first request's equity before returning");
    assert!(second.decision_id > first.decision_id);
}

/// `begin()`'s hand with hero holding AhAd, to hero's preflop decision: the big blind facing the button's raise.
fn hero_preflop_via(e: &mut Engine) {
    e.set_hero_cards([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]).unwrap();
    for a in [Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold] { e.apply_action(a).unwrap(); }
}
/// `begin()`'s hand with hero holding AhAd, checked down to the river through the engine: hero (the big blind) to act
/// against the button.
fn river_via(e: &mut Engine) -> proto::HandState {
    hero_preflop_via(e);
    river_after_hero_preflop(e)
}
/// From hero's preflop decision (`hero_preflop_via`): hero calls, then both check down to the river.
fn river_after_hero_preflop(e: &mut Engine) -> proto::HandState {
    let cards = |s: &str| s.split(' ').map(|c| Card::parse(c).unwrap()).collect::<Vec<_>>();
    e.apply_action(Action::Call).unwrap();
    e.set_board(&cards("Kh 7d 2c")).unwrap();
    e.apply_action(Action::Check).unwrap(); e.apply_action(Action::Check).unwrap();
    e.set_board(&cards("Kh 7d 2c 4d")).unwrap();
    e.apply_action(Action::Check).unwrap(); e.apply_action(Action::Check).unwrap();
    e.set_board(&cards("Kh 7d 2c 4d 9s")).unwrap()
}
/// The same river, built outside any engine, for the scripted worker's solution.
fn river_state() -> proto::HandState {
    use engine::testing::{board, hand, play};
    let aa = Some([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]);
    let s = play(&hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), aa),
        &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call]);
    board(&play(&board(&play(&board(&s, "Kh 7d 2c"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d 9s")
}
fn full(board: &[Card]) -> proto::Range1326 {
    let mut r = proto::Range1326([1.0; 1326]);
    for (i, w) in r.0.iter_mut().enumerate() { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { *w = 0.0; } }
    r
}
/// A recording sink that reports when it is dropped: the last holder of a request's sink is the last thread of that
/// request (`engine-main`'s request, its watchdog generation, its `fast-path` equity thread).
struct DropFlagged(RecordingSink, Arc<AtomicBool>);
impl engine::EventSink for DropFlagged { fn emit(&mut self, ev: proto::RecommendationEvent) { self.0.emit(ev); } }
impl Drop for DropFlagged { fn drop(&mut self) { self.1.store(true, Ordering::SeqCst); } }

/// 29-I2: `Engine` owns every thread it causes. A river request is answered (its `Final` delivered) and leaves behind
/// its retired watchdog generation, waiting on the frozen fake clock for its street deadline, and its `fast-path` equity
/// runner, blocked until its cancellation (a routine that waits on the engine clock, woken by the teardown). When
/// `shutdown` returns: the runner has returned, the watchdog thread has ended, every thread holding the request's sink
/// has dropped it, and the worker was told to shut down and killed, once. A second `shutdown` does nothing.
#[test]
fn shutdown_joins_every_thread_the_engine_started_then_kills_the_worker() {
    use engine::clock::Clock;
    use engine::equity::pending_summary;
    use engine::serve::{EquityRoutine, ServeSeams};
    use engine::testing::{uniform_solution, IdRef};
    use proto::worker::{AckStatus, EngineMessage, ResultStatus};
    let state = river_state();
    let root = core_model::street_root(&state).unwrap();
    let build = engine::tree::build_tree_full(&root, &engine::tree::TemplateSelection::from_history("river_std_v1", &root.history)).unwrap();
    let solution = uniform_solution(&build.tree, &build.history, 0.2);
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let script = vec![FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None },
        FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(solution), error: None, elapsed_ms: 3 }];
    let (worker, fake) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let core = EngineCore::new(worker, clock.clone(), identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_engine_api_log")));
    let watchdog_ended = core.watchdog.ended_threads();
    let tasks = core.tasks.clone();
    let (started_tx, started) = std::sync::mpsc::channel();
    let finished = Arc::new(AtomicBool::new(false));
    let runner_finished = finished.clone();
    let started_tx = Mutex::new(started_tx);
    let runner: EquityRoutine = Arc::new(move |clock: &dyn Clock, _: Option<[Card; 2]>, _: &proto::Range1326, _: &[(Seat, proto::Range1326)], _: &[Card],
        _: std::time::Duration, cancel: &AtomicBool| {
        let _ = started_tx.lock().unwrap().send(());
        clock.wait_until_or_stopped(u64::MAX, cancel);
        runner_finished.store(true, Ordering::SeqCst);
        pending_summary(&[])
    });
    let mut e = Engine::with_core_and_seams(core, ServeSeams { equity: Some(runner), ..ServeSeams::default() });
    let (cfg, _) = cfg_1_2();
    e.set_config(cfg).unwrap();
    e.begin_hand(begin()).unwrap();
    let s = river_via(&mut e);
    e.set_explicit_ranges(full(&s.board), full(&s.board));
    let (sink, recorder) = RecordingSink::notifying(clock.clone(), None);
    let dropped = Arc::new(AtomicBool::new(false));
    e.recommend(Box::new(DropFlagged(sink, dropped.clone()))).unwrap();
    let delivered = recorder.wait_for(2);
    let runner_started = started.recv_timeout(engine::testing::ACK_LIVENESS).is_ok();
    let before = (finished.load(Ordering::SeqCst), watchdog_ended.count(), tasks.unjoined(), dropped.load(Ordering::SeqCst), fake.lock().unwrap().kills);
    e.shutdown();
    let after = (finished.load(Ordering::SeqCst), watchdog_ended.count(), tasks.unjoined(), dropped.load(Ordering::SeqCst), fake.lock().unwrap().kills);
    let (sent, last_sent) = { let f = fake.lock().unwrap(); (f.sent.len(), f.sent.last().cloned()) };
    e.shutdown();
    let again = { let f = fake.lock().unwrap(); (f.kills, f.sent.len()) };
    assert!(matches!(delivered[1].event, proto::RecommendationEvent::Final(_)), "the request was answered: {delivered:?}");
    assert!(runner_started, "the request's equity runner started");
    assert_eq!(before, (false, 0, 1, false, 0), "before shutdown: the runner waits on its thread, the retired watchdog waits, the sink is held, the worker lives");
    assert_eq!(after, (true, 1, 0, true, 1),
        "when shutdown returns: the runner returned and its thread was joined, the watchdog thread ended, the sink was dropped, the worker was killed");
    assert!(matches!(last_sent, Some(EngineMessage::Shutdown { .. })), "the worker was told to shut down: {last_sent:?}");
    assert_eq!(again, (1, sent), "a second shutdown does nothing");
}

/// A recording sink whose first event holds `engine-main` inside the callback until the test releases it (bounded by
/// the liveness allowance, and released at once if the test drops the sender); every event is recorded and acknowledged
/// to the `Recorder`.
struct HeldRecorder { inner: RecordingSink, entered: std::sync::mpsc::Sender<()>, release: std::sync::mpsc::Receiver<()>, held: bool }
impl engine::EventSink for HeldRecorder {
    fn emit(&mut self, ev: proto::RecommendationEvent) {
        self.inner.emit(ev);
        if !self.held {
            self.held = true;
            let _ = self.entered.send(());
            let _ = self.release.recv_timeout(engine::testing::ACK_LIVENESS);
        }
    }
}

/// The `Final` a river request recorded: its events are its `Fast`, its `Equity` (in either order with the `Final`) and
/// its `Final`, so the wait grows until the `Final` is among them (each wait bounded by the liveness allowance).
fn final_of(recorder: &engine::testing::Recorder) -> Option<proto::Recommendation> {
    (2..=3).find_map(|n| recorder.wait_for(n).into_iter().find_map(|r| match r.event { proto::RecommendationEvent::Final(f) => Some(f), _ => None }))
}

/// Final review I4 (orchestrator ruling F-I4; spec 4.2, spec 12: a config changed mid-hand is not applied to the active
/// hand). A decision carries its hand's `HandConfig.config_revision`. A settings change landing while a river decision
/// is in flight (held in its `Fast` callback) allocates the next session revision without cancelling that decision: it
/// stays active, its equity is not cancelled, and its solve is answered and registered. The hand's next decision keeps
/// the hand's revision, so the snapshot solved earlier in the hand stays compatible (plan 3's `for_identity` matches by
/// the identity's config revision); the next hand takes the new revision and the new stakes.
#[test]
fn a_mid_hand_settings_change_keeps_the_hands_config_revision() {
    use engine::testing::{uniform_solution, IdRef};
    use proto::worker::{AckStatus, ResultStatus};
    let state = river_state();
    let root = core_model::street_root(&state).unwrap();
    let build = engine::tree::build_tree_full(&root, &engine::tree::TemplateSelection::from_history("river_std_v1", &root.history)).unwrap();
    let solution = uniform_solution(&build.tree, &build.history, 0.2);
    let answer = || vec![FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None },
        FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(solution.clone()), error: None, elapsed_ms: 3 }];
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let (worker, _fake) = FakeWorker::scripted(clock.clone(), identity.clone(), [answer(), answer()].concat());
    let core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&std::env::temp_dir().join("pokerai_engine_api_log")));
    let (snapshots, tokens) = (core.snapshots.clone(), core.equity_cancel.clone());
    let mut e = Engine::with_core(core);
    let (mut cfg, _) = cfg_1_2();
    let rev1 = e.set_config(cfg.clone()).unwrap();
    e.begin_hand(begin()).unwrap();
    let s = river_via(&mut e);
    assert_eq!(s.config.config_revision, rev1);
    e.set_explicit_ranges(full(&s.board), full(&s.board));
    // decision 1, held in its `Fast` callback while the settings change lands
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let (inner, first_events) = RecordingSink::notifying(clock.clone(), None);
    let first = e.recommend(Box::new(HeldRecorder { inner, entered: entered_tx, release: release_rx, held: false })).unwrap();
    let reached = entered.recv_timeout(engine::testing::ACK_LIVENESS).is_ok();
    cfg.bb_chips = 20;
    let rev2 = e.set_config(cfg).unwrap();
    let still_active = identity.lock().unwrap().is_active(&first);
    let equity_cancelled = tokens.lock().unwrap().as_ref().map(|t| t.load(Ordering::SeqCst));
    let _ = release.send(());
    let first_final = final_of(&first_events);
    // decision 2 of the same hand
    let (sink, second_events) = RecordingSink::notifying(clock.clone(), None);
    let second = e.recommend(Box::new(sink)).unwrap();
    let compatible = snapshots.lock().unwrap().for_identity(&second).iter().map(|x| x.provenance.identity_at_solve.clone()).collect::<Vec<_>>();
    final_of(&second_events);
    // the next hand
    e.finish_hand();
    let next = e.begin_hand(begin()).unwrap();
    let (sink, _) = RecordingSink::new(clock.clone(), None);
    let third = e.recommend(Box::new(sink)).unwrap();
    e.shutdown();
    assert!(reached, "decision 1 reached its Fast callback");
    assert_eq!((rev2, first.config_revision), (rev1 + 1, rev1));
    assert!(still_active, "a mid-hand settings change does not cancel the decision in flight");
    assert_eq!(equity_cancelled, Some(false), "nor its equity");
    assert!(matches!(first_final, Some(ref f) if f.identity == first && f.coverage == proto::Coverage::Exact), "decision 1 is answered: {first_final:?}");
    assert_eq!(second.config_revision, rev1, "the hand's next decision keeps the hand's config revision");
    assert_eq!(compatible, vec![first.clone()], "the snapshot solved earlier in the hand stays compatible");
    assert_eq!((next.config.config_revision, next.config.bb_chips, third.config_revision), (rev2, 20, rev2), "the next hand takes the new revision");
}

/// The scripted worker behind a link that paces `engine-main` for the admission-time watchdog tests: the first receive
/// that returns at or after `pause_at_ms` acknowledges it (`paused`) and waits for the test (`resume`, bounded by the
/// liveness allowance), so the test admits a request at exactly that time; and every restart first spends
/// `restart_ms` of fake time, as a restart whose first launch misses the startup timeout and needs its retry launch
/// does (spec 4.5: `START_ATTEMPTS` launches of up to `STARTUP_TIMEOUT` each).
struct Paced {
    inner: Box<dyn engine::worker::link::WorkerLink>, clock: Arc<FakeClock>, pause_at_ms: Option<u64>, restart_ms: u64,
    paused: std::sync::mpsc::Sender<()>, resume: std::sync::mpsc::Receiver<()>,
}
impl engine::worker::link::WorkerLink for Paced {
    fn send(&mut self, msg: &proto::worker::EngineMessage) -> Result<(), engine::worker::link::WorkerLinkError> { self.inner.send(msg) }
    fn recv(&mut self, timeout: std::time::Duration) -> Result<Option<proto::worker::WorkerMessage>, engine::worker::link::WorkerLinkError> {
        use engine::clock::Clock;
        let got = self.inner.recv(timeout);
        if self.pause_at_ms.is_some_and(|at| self.clock.now_ms() >= at) {
            self.pause_at_ms = None;
            let _ = self.paused.send(());
            let _ = self.resume.recv_timeout(engine::testing::ACK_LIVENESS);
        }
        got
    }
    fn restart(&mut self) -> Result<(), engine::worker::link::WorkerLinkError> { self.clock.advance_ms(self.restart_ms); self.inner.restart() }
    fn kill(&mut self) { self.inner.kill() }
    fn ready(&self) -> Option<&proto::worker::Ready> { self.inner.ready() }
}

/// Every event of a recording's, by kind, with the fake time it was recorded at.
fn kinds_at(events: &[engine::testing::Recorded]) -> Vec<(&'static str, u64)> {
    events.iter().map(|r| (match r.event { proto::RecommendationEvent::Fast(_) => "Fast", proto::RecommendationEvent::Final(_) => "Final",
        proto::RecommendationEvent::Equity { .. } => "Equity", proto::RecommendationEvent::Progress { .. } => "Progress",
        proto::RecommendationEvent::Provisional(_) => "Provisional", proto::RecommendationEvent::NoDecision { .. } => "NoDecision" }, r.at_ms)).collect()
}

/// Final review I1 (orchestrator ruling F-I1; spec 7: `Fast` within 0.3 s, a watchdog independent of the worker client
/// that delivers the `Final` by final delivery - 100 ms; spec 5 step 4: a newer request supersedes the one before it at
/// request time). Request A's river solve hangs in `Building` (no progress, so no heartbeat). B is recommended at t0A +
/// 100 ms, while A's receive is in progress. `engine-main` notices A's supersession within one receive slice, sends A's
/// cancel, and gives way to B at once: B's `Fast` goes out at t0B, not behind A's cancel window. Before anything of B is
/// sent, A's cancel is settled: never confirmed (the worker is hung), the worker is killed at the end of A's 1.5 s
/// window and restarted, and the restart needs its retry launch (10 s of fake time). B's first attempt then has no room
/// (ruling F-M8): its `_min` retry is admitted and hangs too. B's watchdog, armed when B was admitted, delivers B's
/// `Final` at t0B + 14.9 s, `DeadlineExceeded{building}`; A gets no `Final`.
#[test]
fn a_request_admitted_behind_a_hung_solve_gets_its_fast_at_once_and_its_final_by_its_own_watchdog() {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let ack = || FakeReply::Ack { id: engine::testing::IdRef::Last, status: proto::worker::AckStatus::Accepted, reason: None };
    let (worker, fake) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![ack(), FakeReply::Hang, ack(), FakeReply::Hang]);
    let (paused_tx, paused) = std::sync::mpsc::channel();
    let (resume, resume_rx) = std::sync::mpsc::channel();
    let link = Paced { inner: worker, clock: clock.clone(), pause_at_ms: Some(100), restart_ms: 10_000, paused: paused_tx, resume: resume_rx };
    let log_dir = std::env::temp_dir().join(format!("pokerai_engine_api_i1_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&log_dir);
    let mut e = Engine::with_core(EngineCore::new(Box::new(link), clock.clone(), identity, DecisionLog::open(&log_dir)));
    let (cfg, _) = cfg_1_2();
    e.set_config(cfg).unwrap();
    e.begin_hand(begin()).unwrap();
    let s = river_via(&mut e);
    e.set_explicit_ranges(full(&s.board), full(&s.board));
    let (sink, a_events) = RecordingSink::notifying(clock.clone(), None);
    let a = e.recommend(Box::new(sink)).unwrap();
    let paused_in_a = paused.recv_timeout(engine::testing::ACK_LIVENESS).is_ok();
    use engine::clock::Clock;
    let t0b = clock.now_ms();
    let (sink, b_events) = RecordingSink::notifying(clock.clone(), None);
    let b = e.recommend(Box::new(sink)).unwrap();
    let _ = resume.send(());
    let b_final = final_of(&b_events);
    let b_seen = kinds_at(&b_events.recorded());
    e.shutdown();
    let a_seen = kinds_at(&a_events.recorded());
    let (kills, restarts, cancels) = { let f = fake.lock().unwrap(); (f.kills, f.restarts, f.cancels.len()) };
    assert!(paused_in_a, "A's receive returned at 100 ms");
    assert_eq!(t0b, 100, "B was admitted at t0A + 100 ms, while A's receive was in progress");
    assert!(b.decision_id > a.decision_id);
    let b_fast = b_seen.iter().find(|(k, _)| *k == "Fast").map(|(_, at)| *at);
    assert_eq!(b_fast, Some(t0b), "B's Fast is not delayed behind A's cancel window (spec 7: within 0.3 s): {b_seen:?}");
    let b_final = b_final.expect("B was answered");
    let b_final_at = b_seen.iter().find(|(k, _)| *k == "Final").map(|(_, at)| *at);
    assert_eq!((b_final_at, &b_final.coverage), (Some(t0b + 14_900), &proto::Coverage::Unsupported { reason: proto::UnsupportedReason::DeadlineExceeded { stage: "building".into() }, partial: vec![] }),
        "B's Final at t0B + 14.9 s");
    assert!(a_seen.iter().all(|(k, _)| *k != "Final"), "the superseded A gets no Final: {a_seen:?}");
    assert_eq!(cancels, 1, "A's running job was cancelled");
    assert!(kills >= 1 && restarts >= 1, "A's unconfirmed cancel ended in a kill and a restart: {kills} kills, {restarts} restarts");
}

/// Final review I1 (a) (spec 7: a watchdog independent of the worker client and of `engine-main`): the watchdog is
/// armed when a request is admitted, not when `engine-main` gets to it. Request A (hero's preflop decision) holds
/// `engine-main` inside its sink callback; request B, a river decision, is admitted meanwhile and waits behind it. With
/// `engine-main` still held, B's watchdog delivers B's `Final` at t0B + 14.9 s. Released, `engine-main` serves B: nothing
/// but an `Equity` follows B's delivered `Final` (ruling 28-N1), and the decision log records that `Final`, the
/// watchdog's, as B's.
#[test]
fn a_request_waiting_behind_engine_main_is_watched_from_its_admission() {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let (worker, _fake) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![FakeReply::Hang]);
    let log_dir = std::env::temp_dir().join(format!("pokerai_engine_api_i1a_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&log_dir);
    let mut e = Engine::with_core(EngineCore::new(worker, clock.clone(), identity, DecisionLog::open(&log_dir)));
    let (cfg, _) = cfg_1_2();
    e.set_config(cfg).unwrap();
    e.begin_hand(begin()).unwrap();
    hero_preflop_via(&mut e);
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let (inner, _a_events) = RecordingSink::notifying(clock.clone(), None);
    e.recommend(Box::new(HeldRecorder { inner, entered: entered_tx, release: release_rx, held: false })).unwrap();
    let held = entered.recv_timeout(engine::testing::ACK_LIVENESS).is_ok();
    let river = river_after_hero_preflop(&mut e);
    e.set_explicit_ranges(full(&river.board), full(&river.board));
    use engine::clock::Clock;
    let t0b = clock.now_ms();
    let (sink, b_events) = RecordingSink::notifying(clock.clone(), None);
    let b = e.recommend(Box::new(sink)).unwrap();
    clock.set_ms(t0b + 14_900);
    let first = b_events.wait_for(1).into_iter().next().map(|r| (r.at_ms, r.event));
    let _ = release.send(());
    e.shutdown();
    let b_seen = kinds_at(&b_events.recorded());
    let records: Vec<engine::log::DecisionRecord> = std::fs::read_to_string(log_dir.join("decisions.jsonl")).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default();
    assert!(held, "A held engine-main in its sink callback");
    let deadline_exceeded = |r: &proto::Recommendation| matches!(r.coverage, proto::Coverage::Unsupported { reason: proto::UnsupportedReason::DeadlineExceeded { .. }, .. });
    assert!(matches!(&first, Some((at, proto::RecommendationEvent::Final(r))) if *at == t0b + 14_900 && r.identity == b && deadline_exceeded(r)),
        "B's watchdog delivered B's Final at t0B + 14.9 s while engine-main was held: {first:?}");
    assert!(b_seen.iter().skip(1).all(|(k, _)| *k == "Equity"), "nothing but an Equity follows B's delivered Final: {b_seen:?}");
    let b_record = records.iter().find(|r| r.identity == b).expect("B's Final is logged");
    assert!(b_record.final_violation && matches!(b_record.coverage, proto::Coverage::Unsupported { reason: proto::UnsupportedReason::DeadlineExceeded { .. }, .. }),
        "the watchdog's Final is the one logged: {b_record:?}");
    assert_eq!(records.iter().filter(|r| r.identity == b).count(), 1, "exactly once");
}

/// Final review I3 (orchestrator ruling F-I3; spec 7: one `Final` per request by its final delivery). A panic on
/// `engine-main` while it serves a request (a seam that panics once, right after the request's `Fast`, standing for any
/// always-on assert of an internal invariant) is contained at the request boundary: the request is answered by a
/// non-retryable `Unsupported{EngineError("internal: ..")}` `Final` through its claim, its legal intervals kept; its
/// watchdog generation is retired (no second `Final` at the fire); that `Final` is logged; the worker, which may be
/// running the request's job, is killed; and `engine-main` keeps serving: the next request is answered, its solve
/// relaunching the worker once.
#[test]
fn a_panic_while_serving_is_contained_and_the_engine_keeps_serving() {
    use engine::serve::ServeSeams;
    use engine::testing::{uniform_solution, IdRef};
    use proto::worker::{AckStatus, ResultStatus};
    let state = river_state();
    let root = core_model::street_root(&state).unwrap();
    let build = engine::tree::build_tree_full(&root, &engine::tree::TemplateSelection::from_history("river_std_v1", &root.history)).unwrap();
    let solution = uniform_solution(&build.tree, &build.history, 0.2);
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let script = vec![FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None },
        FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(solution), error: None, elapsed_ms: 3 }];
    let (worker, fake) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let log_dir = std::env::temp_dir().join(format!("pokerai_engine_api_i3_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&log_dir);
    let core = EngineCore::new(worker, clock.clone(), identity, DecisionLog::open(&log_dir));
    let ended = core.watchdog.ended_threads();
    let panicked = Arc::new(AtomicBool::new(false));
    let once = panicked.clone();
    let seams = ServeSeams { after_fast: Some(Arc::new(move || if !once.swap(true, Ordering::SeqCst) { panic!("seam: an internal invariant broke") })), ..ServeSeams::default() };
    let mut e = Engine::with_core_and_seams(core, seams);
    let (cfg, _) = cfg_1_2();
    e.set_config(cfg).unwrap();
    e.begin_hand(begin()).unwrap();
    let s = river_via(&mut e);
    e.set_explicit_ranges(full(&s.board), full(&s.board));
    let (sink, first_events) = RecordingSink::notifying(clock.clone(), None);
    let first = e.recommend(Box::new(sink)).unwrap();
    let first_final = final_of(&first_events);
    let (sink, second_events) = RecordingSink::notifying(clock.clone(), None);
    let second = e.recommend(Box::new(sink)).unwrap();
    let second_final = final_of(&second_events);
    // Both requests' fires: the first's generation was retired by the containment, the second's by its own Final.
    use engine::clock::Clock;
    clock.set_ms(clock.now_ms() + 14_900);
    ended.wait_for(2);
    let (kills, restarts) = { let f = fake.lock().unwrap(); (f.kills, f.restarts) }; // before the teardown's own kill
    e.shutdown();
    let first_seen = kinds_at(&first_events.recorded());
    let records: Vec<engine::log::DecisionRecord> = std::fs::read_to_string(log_dir.join("decisions.jsonl")).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default();
    assert!(panicked.load(Ordering::SeqCst), "the seam panicked");
    let first_final = first_final.expect("the request whose serving panicked was answered");
    match &first_final.coverage {
        proto::Coverage::Unsupported { reason: proto::UnsupportedReason::EngineError { message, retryable: false }, .. } =>
            assert!(message.starts_with("internal: ") && message.contains("an internal invariant broke"), "{message}"),
        other => panic!("expected the internal EngineError, got {other:?}"),
    }
    assert_eq!((first_final.identity.clone(), first_final.legal.len()), (first.clone(), s.derived.legal.len()), "the Final answers the request, with its legal intervals");
    assert_eq!(first_seen.iter().filter(|(k, _)| *k == "Final").count(), 1, "one Final: the retired watchdog delivered none at the fire: {first_seen:?}");
    assert!(matches!(second_final, Some(ref f) if f.identity == second && f.coverage == proto::Coverage::Exact), "engine-main kept serving: {second_final:?}");
    assert_eq!((kills, restarts), (1, 1), "the worker was killed at the panic and relaunched by the next solve");
    let logged: Vec<(DecisionIdentity, bool)> = records.iter().map(|r| (r.identity.clone(), matches!(r.coverage, proto::Coverage::Unsupported { .. }))).collect();
    assert_eq!(logged, vec![(first.clone(), true), (second, false)], "both Finals are logged, once each");
    let diagnostics: Vec<engine::log::DiagnosticRecord> = std::fs::read_to_string(log_dir.join("diagnostics.jsonl"))
        .map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default();
    let events: Vec<&str> = diagnostics.iter().map(|d| d.event.as_str()).collect();
    assert_eq!(events, ["panic", "kill", "restart"], "the panic, its kill and the next solve's relaunch are recorded (final review M2): {diagnostics:?}");
    assert!(diagnostics[0].identity.as_ref() == Some(&first) && diagnostics[0].detail.contains("an internal invariant broke"), "{:?}", diagnostics[0]);
}

/// §12: a rejected entry leaves the state unchanged. A `begin_hand` that `core_model` refuses leaves the hand in
/// progress as it was (its history, its undo stack) and consumes no hand id.
#[test]
fn a_rejected_begin_hand_leaves_the_hand_in_progress_unchanged() {
    let (cfg, _) = cfg_1_2();
    let mut e = engine();
    e.set_config(cfg).unwrap();
    let started = e.begin_hand(begin()).unwrap();
    let folded = e.apply_action(Action::Fold).unwrap();
    let hero_not_dealt = proto::BeginHand { button: Seat(0), hero: Seat(2), dealt: vec![Seat(0), Seat(1), Seat(3), Seat(4), Seat(5)], stacks: vec![1000; 5], hero_cards: None };
    assert!(matches!(e.begin_hand(hero_not_dealt), Err(EngineError::Rules(core_model::RulesError::InvalidConfig { ref reason })) if reason.contains("hero is not a dealt seat")));
    // Final review M6 (spec 12: `FormatUnsupported` for formats): the rules error stays typed, so a caller (plan 5) matches
    // the variant instead of its text. Two dealt seats is an unsupported format.
    let heads_up = proto::BeginHand { button: Seat(0), hero: Seat(0), dealt: vec![Seat(0), Seat(1)], stacks: vec![1000; 2], hero_cards: None };
    assert!(matches!(e.begin_hand(heads_up), Err(EngineError::Rules(core_model::RulesError::FormatUnsupported { ref detail })) if detail == "two dealt seats"));
    assert!(matches!(e.apply_action(Action::Raise { to: 1 }), Err(EngineError::Rules(core_model::RulesError::IllegalAction { .. }))), "an illegal action is typed too");
    assert_eq!(e.state(), Some(folded.clone()), "the hand in progress is unchanged");
    let undone = e.undo().unwrap();
    assert_eq!((undone.hand_id, undone.actions.len()), (started.hand_id, 0), "its undo stack is intact");
    assert!(undone.hand_revision > folded.hand_revision);
    let next = e.begin_hand(begin()).unwrap();
    assert_eq!(next.hand_id, started.hand_id + 1, "the rejected admission consumed no hand id");
    e.shutdown();
}

/// `recommend` needs a hand in progress, answers with the identity of the hand state it was given (the config
/// revision the session is on), and refuses once the engine is shut down (nothing would serve it).
#[test]
fn recommend_needs_a_hand_in_progress_and_a_running_engine() {
    let (cfg, _) = cfg_1_2();
    let (mut e, tokens) = engine_with_tokens();
    let rev = e.set_config(cfg).unwrap();
    let (sink, _) = RecordingSink::new(FakeClock::new(), None);
    assert!(matches!(e.recommend(Box::new(sink)), Err(EngineError::Message(_))), "no hand");
    let s = e.begin_hand(begin()).unwrap();
    let (id, _) = served(&mut e, &tokens);
    assert_eq!((id.hand_id, id.hand_revision, id.config_revision), (s.hand_id, s.hand_revision, rev));
    e.shutdown();
    let (sink, _) = RecordingSink::new(FakeClock::new(), None);
    assert!(matches!(e.recommend(Box::new(sink)), Err(EngineError::Message(ref m)) if m.contains("shut down")));
}

/// Plan 5 keeps the engine in Tauri managed state, which requires `Send`.
#[test]
fn the_engine_can_move_across_threads() {
    fn send<T: Send>() {}
    send::<Engine>();
}

/// `Engine::new` validates the config before it launches anything, and reports a worker that cannot be launched as an
/// error (plan 5 shows it at startup).
#[test]
fn new_validates_the_config_first_and_reports_a_missing_worker() {
    let dir = std::env::temp_dir().join(format!("pokerai-engine-api-{}-absent", std::process::id()));
    let paths = || engine::Paths { log_dir: dir.join("log"), worker_exe: dir.join("absent-solver-worker.exe"), preflop: dir.join("preflop"), cache: dir.join("cache") };
    let (mut cfg, _) = cfg_1_2();
    cfg.solver.flop_budget_s = 31;
    match Engine::new(cfg.clone(), paths()) {
        Err(EngineError::Message(m)) => assert!(m.contains("flop_budget_s") && !m.contains("spawn"), "{m}"),
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("an invalid config was accepted"),
    }
    cfg.solver.flop_budget_s = 10;
    match Engine::new(cfg, paths()) {
        Err(EngineError::Message(m)) => assert!(m.contains("spawn") && m.contains("absent-solver-worker.exe"), "{m}"),
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("an engine started without a worker"),
    }
}

/// The startup report of a degraded engine (spec 12, ruling 29-I4): no worker ready, the typed refusal, and one banner.
fn degraded_report(refusal: &engine::worker::ready::ReadyRefusal) -> engine::StartupReport {
    engine::StartupReport { worker_ready: false, worker_refusal: Some(refusal.clone()), cache_state: "absent".into(),
        banners: vec![format!("the solver worker was refused at startup (worker/proto version mismatch: {refusal}); every recommendation answers \
            this error until the worker is rebuilt")], ..Default::default() }
}
/// Requests a recommendation and returns the message of its answer, which must be a single non-retryable
/// `Unsupported{EngineError}` `Final`.
fn answered_with(e: &mut Engine) -> String {
    let (sink, recorder) = RecordingSink::notifying(FakeClock::new(), None);
    e.recommend(Box::new(sink)).unwrap();
    let events = recorder.wait_for(1);
    match &events[0].event {
        proto::RecommendationEvent::Final(r) => match &r.coverage {
            proto::Coverage::Unsupported { reason: proto::UnsupportedReason::EngineError { message, retryable: false }, .. } => message.clone(),
            other => panic!("expected a non-retryable EngineError, got {other:?}"),
        },
        other => panic!("expected a Final, got {other:?}"),
    }
}

/// Ruling 29-I4 without a process: a core whose link is the refusal (`RefusedWorker`, what `Engine::new` builds when
/// the worker's `ready` is refused) is a degraded engine. Its report says so; every decision is answered with the
/// version mismatch before any solve and without a launch (the link has none to make); a request at a point that is no
/// decision still answers `NoDecision` (§5 step 4, which precedes everything).
#[test]
fn a_degraded_engine_answers_every_decision_with_the_version_mismatch() {
    use engine::worker::ready::ReadyRefusal;
    use engine::worker::RefusedWorker;
    let refusal = ReadyRefusal::SolverCommit { reported: "deadbeef".into() };
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let core = EngineCore::new(Box::new(RefusedWorker::new("solver-worker.exe".into(), refusal.clone())), FakeClock::new(), identity,
        DecisionLog::open(&std::env::temp_dir().join("pokerai_engine_api_log")));
    let mut e = Engine::with_core(core);
    assert_eq!(e.startup_report(), degraded_report(&refusal));
    let (cfg, _) = cfg_1_2();
    e.set_config(cfg).unwrap();
    e.begin_hand(begin()).unwrap();
    let mismatch = format!("worker/proto version mismatch: solver commit \"deadbeef\" != pinned {}", proto::worker::SOLVER_COMMIT);
    hero_preflop_via(&mut e);
    assert_eq!(answered_with(&mut e), mismatch, "preflop");
    let river = river_after_hero_preflop(&mut e);
    e.set_explicit_ranges(full(&river.board), full(&river.board));
    assert_eq!(answered_with(&mut e), mismatch, "river");
    e.apply_action(Action::Check).unwrap();
    e.apply_action(Action::Check).unwrap();
    let (sink, recorder) = RecordingSink::notifying(FakeClock::new(), None);
    e.recommend(Box::new(sink)).unwrap();
    assert!(matches!(recorder.wait_for(1)[0].event, proto::RecommendationEvent::NoDecision { .. }), "the hand is complete: no decision");
    e.shutdown();
}

/// `Engine::new` against stand-in workers: batch files that write a `ready` line and then wait on stdin (`set /p`), as
/// the process link's own stand-in tests do (`worker::process`). The real link validates the `ready` (spec 3.7, 4.5).
#[cfg(windows)]
mod stand_in {
    use super::*;
    use engine::{Paths, StartupReport};
    use proto::worker::{Ready, WorkerMessage};
    use std::path::PathBuf;

    /// Each launch appends a line to `launches.txt` next to the script before anything else, as the process link's own
    /// stand-ins do, so a test counts launches.
    struct StandIn { dir: PathBuf }
    impl StandIn {
        fn new(tag: &str, ready: &Ready) -> StandIn {
            let dir = std::env::temp_dir().join(format!("pokerai-engine-api-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let line = serde_json::to_string(&WorkerMessage::Ready(ready.clone())).unwrap();
            std::fs::write(dir.join("worker.cmd"), format!("@echo off\r\necho x>>\"%~dp0launches.txt\"\r\necho {line}\r\nset /p _=\r\n")).unwrap();
            StandIn { dir }
        }
        fn paths(&self) -> Paths {
            Paths { log_dir: self.dir.join("log"), worker_exe: self.dir.join("worker.cmd"), preflop: self.dir.join("preflop"), cache: self.dir.join("cache") }
        }
        fn launches(&self) -> usize { std::fs::read_to_string(self.dir.join("launches.txt")).map(|s| s.lines().count()).unwrap_or(0) }
    }
    impl Drop for StandIn { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.dir); } }

    fn start(s: &StandIn) -> Engine {
        let (cfg, _) = cfg_1_2();
        match Engine::new(cfg, s.paths()) { Ok(e) => e, Err(e) => panic!("{e}") }
    }

    /// A worker whose `ready` is valid starts; the report carries its `ready` as written, and the config given is the
    /// session's first revision.
    #[test]
    fn new_launches_the_worker_and_reports_its_ready() {
        let ready = FakeWorker::default_ready();
        let s = StandIn::new("valid", &ready);
        let mut e = start(&s);
        assert_eq!(e.startup_report(), StartupReport { worker_ready: true, worker_threads: 16, build_features: ready.build_features.clone(),
            cpu_features: ready.cpu_features.clone(), capabilities: ready.capabilities.clone(), cpu_lacks_avx2: false, worker_refusal: None,
            quarantined_bundles: vec![], cache_state: "absent".into(), banners: vec![] });
        assert_eq!(e.state(), None);
        assert_eq!(e.begin_hand(begin()).unwrap().config.config_revision, 1);
        e.shutdown();
        e.shutdown();
        assert_eq!(s.launches(), 1);
    }

    /// Spec 12 line 658 (ruling 29-I4): a worker whose `ready` is refused (§4.5; built without AVX2, §3.7, or of another
    /// protocol version) leaves a DEGRADED engine, not a failed construction: `Engine::new` succeeds, the startup report
    /// says the worker is not ready and why (the typed refusal), and every decision, preflop and river alike, is
    /// answered with the non-retryable `EngineError("worker/proto version mismatch ...")`, the refused build never being
    /// launched again.
    #[test]
    fn a_refused_worker_leaves_a_degraded_engine() {
        use engine::worker::ready::ReadyRefusal;
        use proto::worker::PROTO_VERSION;
        let mut no_avx2 = FakeWorker::default_ready();
        no_avx2.build_features = vec!["sse2".into()];
        let mut old_proto = FakeWorker::default_ready();
        old_proto.proto_version = PROTO_VERSION - 1;
        for (tag, ready, refusal) in [("no-avx2-build", no_avx2, ReadyRefusal::NoAvx2 { build_features: vec!["sse2".into()] }),
            ("old-proto", old_proto, ReadyRefusal::ProtoVersion { reported: PROTO_VERSION - 1 })] {
            let s = StandIn::new(tag, &ready);
            let mut e = start(&s);
            assert_eq!(e.startup_report(), degraded_report(&refusal), "{tag}");
            let mismatch = format!("worker/proto version mismatch: {refusal}");
            e.begin_hand(begin()).unwrap();
            hero_preflop_via(&mut e);
            assert_eq!(answered_with(&mut e), mismatch, "{tag}: preflop");
            let river = river_after_hero_preflop(&mut e);
            e.set_explicit_ranges(full(&river.board), full(&river.board));
            assert_eq!(answered_with(&mut e), mismatch, "{tag}: river");
            e.shutdown();
            assert_eq!(s.launches(), 1, "{tag}: the refused build was launched once, at startup, and never again");
        }
    }

    /// §3.7: a CPU without AVX2 gets a startup banner, not a refusal.
    #[test]
    fn a_cpu_without_avx2_gets_a_startup_banner_not_a_refusal() {
        let mut ready = FakeWorker::default_ready();
        ready.cpu_features = vec!["sse2".into()];
        let s = StandIn::new("no-avx2-cpu", &ready);
        let mut e = start(&s);
        let rep = e.startup_report();
        assert!(rep.worker_ready && rep.cpu_lacks_avx2);
        assert_eq!(rep.banners, vec!["this CPU does not report AVX2; solves will be much slower".to_string()]);
        e.shutdown();
    }
}
