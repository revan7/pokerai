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
    assert!(rev2 > rev1);
    // §4.2: a config set during a hand is queued for the NEXT hand; the active hand keeps its frozen HandConfig
    let s = e.begin_hand(begin()).unwrap();
    assert_eq!(s.config.config_revision, rev2);
    cfg.solver.flop_budget_s = 12;
    cfg.bb_chips = 20;
    let rev3 = e.set_config(cfg.clone()).unwrap();
    assert!(rev3 > rev2);
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
    cancels(&mut e, &tokens, "set_config", |e| { e.set_config(cfg.clone()).unwrap(); });
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
    assert!(matches!(e.begin_hand(hero_not_dealt), Err(EngineError::Rules(ref m)) if m.contains("hero is not a dealt seat")));
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

/// `Engine::new` against stand-in workers: batch files that write a `ready` line and then wait on stdin (`set /p`), as
/// the process link's own stand-in tests do (`worker::process`). The real link validates the `ready` (spec 3.7, 4.5).
#[cfg(windows)]
mod stand_in {
    use super::*;
    use engine::{Paths, StartupReport};
    use proto::worker::{Ready, WorkerMessage};
    use std::path::PathBuf;

    struct StandIn { dir: PathBuf }
    impl StandIn {
        fn new(tag: &str, ready: &Ready) -> StandIn {
            let dir = std::env::temp_dir().join(format!("pokerai-engine-api-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let line = serde_json::to_string(&WorkerMessage::Ready(ready.clone())).unwrap();
            std::fs::write(dir.join("worker.cmd"), format!("@echo off\r\necho {line}\r\nset /p _=\r\n")).unwrap();
            StandIn { dir }
        }
        fn paths(&self) -> Paths {
            Paths { log_dir: self.dir.join("log"), worker_exe: self.dir.join("worker.cmd"), preflop: self.dir.join("preflop"), cache: self.dir.join("cache") }
        }
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
            cpu_features: ready.cpu_features.clone(), capabilities: ready.capabilities.clone(), cpu_lacks_avx2: false, quarantined_bundles: vec![],
            cache_state: "absent".into(), banners: vec![] });
        assert_eq!(e.state(), None);
        assert_eq!(e.begin_hand(begin()).unwrap().config.config_revision, 1);
        e.shutdown();
        e.shutdown();
    }

    /// §3.7: a worker built without AVX2 is refused (`EngineError("worker built without AVX2")`).
    #[test]
    fn new_refuses_a_worker_built_without_avx2() {
        let mut ready = FakeWorker::default_ready();
        ready.build_features = vec!["sse2".into()];
        let s = StandIn::new("no-avx2-build", &ready);
        let (cfg, _) = cfg_1_2();
        match Engine::new(cfg, s.paths()) {
            Err(EngineError::Message(m)) => assert!(m.contains("worker built without AVX2"), "{m}"),
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("a worker built without AVX2 was accepted"),
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
