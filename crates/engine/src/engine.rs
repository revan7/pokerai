//! §3.5 public surface: commands never block; `engine-main` serves the request slot of depth 1 (newest wins).
//!
//! Construction. `Engine::new` builds the `EngineCore` (Task 27's defaults: an empty snapshot store, the default
//! session config, `ExplicitRanges` with no ranges; Task 20's watchdog) around the Task 18 process worker link, whose
//! `ready` validation refuses a worker built without AVX2 (§3.6, §3.7), then hands it to `engine-main`. The caller
//! locates the worker binary (`Paths::worker_exe`): the app stages it, the tests and `bench` find it through
//! `POKERAI_WORKER`; the engine never searches for it. `with_core` takes a core built elsewhere (the tests' fake worker
//! and clock).
//!
//! Degraded engine (spec 12 line 658, ruling 29-I4). A worker whose `ready` is refused (protocol version, solver commit,
//! adapter version, `threads`, or `build_features` without AVX2: `WorkerLinkError::ReadyRefused`, follow-up P2.W2) does
//! not fail construction: the core gets a `RefusedWorker` link, which never launches anything, `startup_report` says
//! `worker_ready: false` with the typed refusal, and every decision is answered with the non-retryable
//! `EngineError("worker/proto version mismatch: ...")` until the worker is rebuilt. An invalid config, and a worker
//! that could not be launched at all (`Spawn`: the binary missing, a launch fault, no `ready` in time), still fail
//! construction.
//!
//! Identity (§4.4, §5 step 3). Every call that invalidates the active decision (`set_config`, `begin_hand`,
//! `set_hero_cards`, `apply_action`, `set_board`, `undo`, `recommend`, `cancel` of the active decision, `finish_hand`,
//! `abandon_hand`, `shutdown`) also cancels the `fast-path` equity of the request served last (ruling 28-I4, Task 28
//! fix-Q1, ruling 29-I1), in the same hold of the identity lock (`supersede`), so no superseded request keeps its
//! equity running.
//!
//! Ownership of the core and the threads (rulings 29-I2, 29-I3). `engine-main` owns the `EngineCore` outright: it is
//! moved into that thread, never behind a lock, so no engine lock is held while `serve_request` runs a sink callback
//! (ruling 28-I1), and no command can block behind a running solve (§3.4). Every field a command touches is owned by
//! `Engine` or shared through its own `Arc<Mutex<_>>`, each locked for one step. Every thread the engine causes is
//! joined before `shutdown` returns: `engine-main` itself, and, through the core's teardown (`EngineCore::shutdown`,
//! run by `engine-main` on its way out, a panic included), the watchdog's generation threads and the requests'
//! `fast-path` threads; then the worker is told to shut down and killed. `shutdown` holds no lock while it joins, and
//! `engine-main` finishes the request in hand first: invalidated, it ends at its next identity check. Lock order: the
//! identity lock, then the equity token slot (`serve_request` takes the slot alone); the snapshot store, the config
//! and the range source are each locked alone.
use crate::clock::{Clock, SystemClock};
use crate::core::EngineCore;
use crate::identity::IdentityState;
use crate::log::DecisionLog;
use crate::ranges::{ExplicitRanges, RangeSource};
use crate::serve::{serve_request, LiveRequest};
use crate::snapshots::SnapshotStore;
use crate::startup::StartupReport;
use crate::worker::link::{RefusedWorker, WorkerLink, WorkerLinkError};
use crate::worker::process::ProcessWorker;
use crate::{EngineError, EventSink};
use core_model::{apply_action, begin_hand, set_board, set_hero_cards, BeginHand as CoreBeginHand};
use proto::{Action, Card, DecisionIdentity, GameConfig, HandConfig, HandState, Range1326};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// All four directories are declared here so downstream plans have nothing to add:
/// `log_dir` and `worker_exe` are used by this plan, `preflop` by plan 3, `cache` by plan 4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths { pub log_dir: PathBuf, pub worker_exe: PathBuf, pub preflop: PathBuf, pub cache: PathBuf }

/// §7 spec range for the per-session flop budget.
pub const FLOP_BUDGET_RANGE: std::ops::RangeInclusive<u8> = 1..=30;

/// §4.2 / §13.3: what `set_config` (and so `Engine::new`) accepts.
fn validate_config(cfg: &GameConfig) -> Result<(), EngineError> {
    if !FLOP_BUDGET_RANGE.contains(&cfg.solver.flop_budget_s) {
        return Err(EngineError::Message(format!("flop_budget_s must be between 1 and 30 seconds, got {}", cfg.solver.flop_budget_s)));
    }
    if cfg.sb_chips == 0 || cfg.bb_chips < cfg.sb_chips { return Err(EngineError::Message("blinds must satisfy 0 < sb <= bb".into())); }
    if cfg.solver.threads == 0 { return Err(EngineError::Message("threads must be at least 1".into())); }
    Ok(())
}

struct Slot { pending: Option<LiveRequest>, stop: bool }

/// `engine-main`'s core, torn down (`EngineCore::shutdown`) when that thread ends, by `Engine::shutdown` or a panic.
struct TornDown(EngineCore);

impl Drop for TornDown {
    fn drop(&mut self) { self.0.shutdown(); }
}

/// How `engine-main` serves a request: `serve_request`, or with test seams.
enum Server {
    Production,
    #[cfg(any(test, feature = "testing"))]
    Seams(crate::serve::ServeSeams),
}

impl Server {
    fn serve(&self, core: &mut EngineCore, req: LiveRequest) {
        match self {
            Server::Production => serve_request(core, req),
            #[cfg(any(test, feature = "testing"))]
            Server::Seams(seams) => crate::serve::serve_request_with(core, req, seams.clone()),
        }
    }
}

pub struct Engine {
    identity: Arc<Mutex<IdentityState>>, clock: Arc<dyn Clock>, snapshots: Arc<Mutex<SnapshotStore>>,
    shared_config: Arc<Mutex<GameConfig>>, range_source: Arc<Mutex<Box<dyn RangeSource>>>, startup: StartupReport,
    /// `EngineCore::equity_cancel`: the equity cancellation token of the request served last (ruling 28-I4).
    equity_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    slot: Arc<(Mutex<Slot>, Condvar)>,
    state: Option<HandState>, undo: Vec<HandState>, config: GameConfig, queued_config: Option<GameConfig>,
    main: Option<std::thread::JoinHandle<()>>, stopped: bool,
    /// Unit tests only: runs immediately before the equity token is set (ruling 29-M1).
    #[cfg(test)]
    before_equity_store: Option<Box<dyn Fn() + Send>>,
}

impl Engine {
    /// Validates `cfg`, launches the worker at `paths.worker_exe` (its `ready` validated, §4.5: a worker built without
    /// AVX2 is refused, §3.7) and starts `engine-main`; `cfg` becomes the session's first config revision. A refused
    /// `ready` leaves a degraded engine (see the module doc); an invalid config or a failed launch is an error.
    pub fn new(cfg: GameConfig, paths: Paths) -> Result<Engine, EngineError> {
        validate_config(&cfg)?;
        let worker: Box<dyn WorkerLink> = match ProcessWorker::spawn(&paths.worker_exe, cfg.solver.threads) {
            Ok(w) => Box::new(w),
            // Typed (follow-up P2.W2): no string is parsed to tell a permanent refusal from a failed launch.
            Err(WorkerLinkError::ReadyRefused { exe, refusal }) => Box::new(RefusedWorker::new(exe, refusal)),
            Err(e) => return Err(EngineError::Message(e.to_string())),
        };
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let core = EngineCore::new(worker, clock, identity, DecisionLog::open(&paths.log_dir));
        let mut e = Engine::with_core(core);
        e.set_config(cfg)?;
        Ok(e)
    }

    pub fn with_core(core: EngineCore) -> Engine { Self::start(core, Server::Production) }

    /// `with_core` whose `engine-main` serves every request through `serve::serve_request_with` and these seams (tests
    /// only: a test equity routine, say).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_core_and_seams(core: EngineCore, seams: crate::serve::ServeSeams) -> Engine { Self::start(core, Server::Seams(seams)) }

    fn start(core: EngineCore, server: Server) -> Engine {
        let identity = core.identity.clone();
        let clock = core.clock.clone();
        let snapshots = core.snapshots.clone();
        let shared_config = core.config.clone();
        let range_source = core.range_source.clone();
        let equity_cancel = core.equity_cancel.clone();
        let config = core.config();
        // Captured once, before the core is handed to `engine-main`, which owns it from then on.
        let startup = StartupReport::from_worker(core.worker.as_ref());
        let slot = Arc::new((Mutex::new(Slot { pending: None, stop: false }), Condvar::new()));
        let s2 = slot.clone();
        let main = std::thread::Builder::new().name("engine-main".into()).spawn(move || {
            // Owned here alone (ruling 29-I3), and torn down on every way out of this thread (ruling 29-I2).
            let mut owned = TornDown(core);
            loop {
                let req = {
                    let (m, cv) = &*s2;
                    let mut g = lock(m);
                    while g.pending.is_none() && !g.stop { g = cv.wait(g).unwrap_or_else(|poisoned| poisoned.into_inner()); }
                    if g.stop { return; }
                    g.pending.take().expect("a pending request was just seen")
                };
                server.serve(&mut owned.0, req);
            }
        }).expect("spawn engine-main");
        Engine { identity, clock, snapshots, shared_config, range_source, startup, equity_cancel, slot, state: None, undo: vec![], config, queued_config: None,
            main: Some(main), stopped: false, #[cfg(test)] before_equity_store: None }
    }

    /// §12 startup diagnostics for the UI; never blocks (the worker's `ready` is captured at construction,
    /// and plans 3 and 4 fill `quarantined_bundles` / `cache_state` at the same point).
    pub fn startup_report(&self) -> StartupReport { self.startup.clone() }

    /// §4.2 / §13.3: validates the config, allocates a revision and applies it — immediately when no hand is in
    /// progress, otherwise from the next `begin_hand` (the active hand keeps its frozen `HandConfig`). A rejected config
    /// consumes no revision.
    pub fn set_config(&mut self, cfg: GameConfig) -> Result<u32, EngineError> {
        validate_config(&cfg)?;
        let rev = self.supersede(|ids| ids.set_config());
        let stamped = GameConfig { config_revision: rev, ..cfg };
        if self.state.is_some() { self.queued_config = Some(stamped); } else { self.apply_config(stamped); }
        Ok(rev)
    }

    fn apply_config(&mut self, cfg: GameConfig) { self.config = cfg.clone(); *self.shared_config.lock().unwrap() = cfg; }

    fn stamp(&self, mut s: HandState, hand_id: u64, rev: u32) -> HandState { s.hand_id = hand_id; s.hand_revision = rev; s }

    /// Runs `step`, which invalidates the active decision, under the identity lock and, in that same hold, cancels the
    /// equity of the request served last (ruling 28-I4; Task 28 fix-Q1 and the fix-D1 carry). One hold closes the race
    /// with a request starting on `engine-main`: a token installed before this step is cancelled here, and a request
    /// whose token is installed after it finds its decision already stale when its `fast-path` thread checks the
    /// identity (under this same lock), and cancels its own equity. Lock order: identity, then the token slot.
    fn supersede<R>(&self, step: impl FnOnce(&mut IdentityState) -> R) -> R {
        let mut ids = lock(&self.identity);
        let r = step(&mut ids);
        self.cancel_equity();
        r
    }

    /// Sets the equity token of the request served last; the caller holds the identity lock (see `supersede`).
    fn cancel_equity(&self) {
        if let Some(token) = lock(&self.equity_cancel).as_ref() {
            // Test-only observation immediately before the store (ruling 29-M1): runs on the calling thread.
            #[cfg(test)]
            if let Some(observe) = &self.before_equity_store {
                observe();
            }
            token.store(true, Ordering::SeqCst);
        }
    }

    /// Takes the ID-free admission DTO of §5 step 2; the engine allocates `hand_id` and converts to `core_model`'s input.
    /// The hand is admitted against the config it will freeze (a queued `set_config` included) before anything changes:
    /// a rejected admission leaves the state unchanged (§12), ending no hand and allocating no id. An admitted hand ends
    /// the one in progress, as `finish_hand` would.
    pub fn begin_hand(&mut self, req: proto::BeginHand) -> Result<HandState, EngineError> {
        let cfg = self.queued_config.as_ref().unwrap_or(&self.config);
        // `hand_id` is stamped below, once the admission has passed; `core_model` only stores it.
        let core_req = CoreBeginHand { hand_id: 0, button: req.button, hero: req.hero, dealt: req.dealt, stacks_start: req.stacks, hero_cards: req.hero_cards };
        let s = begin_hand(&HandConfig::from_game(cfg), core_req).map_err(|e| EngineError::Rules(e.to_string()))?;
        if self.state.is_some() { self.end_hand(); }
        if let Some(c) = self.queued_config.take() { self.apply_config(c); }
        let (hand_id, rev) = self.supersede(|ids| ids.begin_hand());
        self.undo.clear();
        self.state = Some(self.stamp(s, hand_id, rev));
        Ok(self.state.clone().unwrap())
    }

    fn mutate(&mut self, next: HandState) -> HandState {
        let rev = self.supersede(|ids| ids.mutate());
        let hand_id = self.state.as_ref().map(|p| p.hand_id).unwrap_or(next.hand_id);
        if let Some(prev) = self.state.take() { self.undo.push(prev); }
        let s = self.stamp(next, hand_id, rev);
        self.snapshots.lock().unwrap().invalidate_hand(hand_id);
        self.state = Some(s.clone());
        s
    }

    /// §5 step 3: a mutation like any other (fresh revision, in-flight work invalidated).
    pub fn set_hero_cards(&mut self, cards: [Card; 2]) -> Result<HandState, EngineError> {
        let cur = self.state.as_ref().ok_or(EngineError::Message("no hand".into()))?;
        let next = set_hero_cards(cur, cards).map_err(|e| EngineError::Rules(e.to_string()))?;
        Ok(self.mutate(next))
    }

    pub fn apply_action(&mut self, a: Action) -> Result<HandState, EngineError> {
        let cur = self.state.as_ref().ok_or(EngineError::Message("no hand".into()))?;
        let next = apply_action(cur, a).map_err(|e| EngineError::Rules(e.to_string()))?;
        Ok(self.mutate(next))
    }

    pub fn set_board(&mut self, cards: &[Card]) -> Result<HandState, EngineError> {
        let cur = self.state.as_ref().ok_or(EngineError::Message("no hand".into()))?;
        let next = set_board(cur, cards).map_err(|e| EngineError::Rules(e.to_string()))?;
        Ok(self.mutate(next))
    }

    pub fn undo(&mut self) -> Result<HandState, EngineError> {
        let prev = self.undo.pop().ok_or(EngineError::Message("nothing to undo".into()))?;
        let rev = self.supersede(|ids| ids.mutate());
        let hand_id = self.state.as_ref().map(|s| s.hand_id).unwrap_or(prev.hand_id);
        let s = self.stamp(prev, hand_id, rev);
        self.snapshots.lock().unwrap().invalidate_hand(hand_id);
        self.state = Some(s.clone());
        Ok(s)
    }

    /// Plan 2 only: the public ranges at the street root (plan 3 installs a replay-backed `RangeSource` instead).
    pub fn set_explicit_ranges(&mut self, oop: Range1326, ip: Range1326) { *self.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: Some(oop), ip: Some(ip) }); }

    /// Allocates the decision's identity, stamps `t0` and queues the request for `engine-main` (depth 1: an older
    /// pending request is dropped); its events go to `sink`. Refused with no hand in progress, and once shut down.
    pub fn recommend(&mut self, sink: Box<dyn EventSink>) -> Result<DecisionIdentity, EngineError> {
        if self.stopped { return Err(EngineError::Message("the engine is shut down".into())); }
        let state = self.state.clone().ok_or(EngineError::Message("no hand".into()))?;
        // The new decision supersedes the active one: allocated in the same identity-lock hold that cancels the equity
        // of the request served last (ruling 29-I1, 28-I4), not when `engine-main` gets to the new request.
        let identity = self.supersede(|ids| ids.next_decision()).ok_or(EngineError::Message("no hand in progress".into()))?;
        let t0_ms = self.clock.now_ms();
        let (m, cv) = &*self.slot;
        let dropped = lock(m).pending.replace(LiveRequest { identity: identity.clone(), state, t0_ms, sink: Arc::new(Mutex::new(sink)) });
        cv.notify_one();
        // Depth 1, newest wins: an older request still pending is dropped (its sink with it), outside the slot's lock.
        drop(dropped);
        Ok(identity)
    }

    /// Cancels `decision_id` if it is the active decision (its equity with it, in the same hold: see `supersede`);
    /// any other id is stale already, and nothing changes.
    pub fn cancel(&mut self, decision_id: u64) {
        let mut ids = lock(&self.identity);
        if ids.active().is_some_and(|a| a.decision_id == decision_id) {
            ids.cancel_active();
            self.cancel_equity();
        }
    }

    pub fn finish_hand(&mut self) { self.end_hand(); }

    pub fn abandon_hand(&mut self) { self.end_hand(); }

    fn end_hand(&mut self) {
        let hand_id = self.state.as_ref().map(|s| s.hand_id);
        self.supersede(|ids| ids.invalidate_hand());
        if let Some(h) = hand_id { self.snapshots.lock().unwrap().invalidate_hand(h); }
        self.state = None;
        self.undo.clear();
        if let Some(c) = self.queued_config.take() { self.apply_config(c); }
    }

    pub fn state(&self) -> Option<HandState> { self.state.clone() }

    /// `&mut self` and idempotent: Tauri managed state cannot move out of the handle (spec §3.5); a second call does
    /// nothing. In order (ruling 29-I2): the hand is invalidated and the last request's equity cancelled (one identity
    /// hold), so a request still running stops at its next identity check rather than at its deadline; scheduling stops
    /// (a pending request is dropped); then `engine-main` is joined, holding no lock, and on its way out it tears the core
    /// down (`EngineCore::shutdown`: the watchdog's and the requests' threads woken and joined, then the worker told to
    /// shut down and killed). When this returns, no thread the engine started is running and the worker is gone.
    pub fn shutdown(&mut self) {
        if self.stopped { return; }
        self.stopped = true;
        self.supersede(|ids| ids.invalidate_hand());
        let dropped = { let (m, cv) = &*self.slot; let mut g = lock(m); g.stop = true; cv.notify_all(); g.pending.take() };
        drop(dropped);
        if let Some(h) = self.main.take() { let _ = h.join(); }
    }
}

impl Drop for Engine { fn drop(&mut self) { self.shutdown(); } }

/// A lock for the paths `shutdown` takes, which also runs from `Drop`: a panic that poisoned one of these locks (an
/// engine bug on `engine-main`, already reported) must not turn the shutdown into a second panic, or into an abort
/// during unwinding. Every value behind them stays consistent at any point a panic could interrupt it (a counter step,
/// an `Option` set or taken, a flag).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cfg_1_2, FakeClock, FakeReply, FakeWorker, ACK_LIVENESS};
    use proto::{RecommendationEvent, Seat};
    use std::sync::mpsc;
    use std::sync::TryLockError;

    fn core() -> EngineCore {
        let clock = FakeClock::new();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let (worker, _state) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![FakeReply::Hang]);
        EngineCore::new(worker, clock, identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_engine_unit_log")))
    }
    fn begin() -> proto::BeginHand {
        let aa = Some([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]);
        proto::BeginHand { button: Seat(0), hero: Seat(2), dealt: (0..6).map(Seat).collect(), stacks: vec![1000; 6], hero_cards: aa }
    }
    fn cards(s: &str) -> Vec<Card> { s.split(' ').map(|c| Card::parse(c).unwrap()).collect() }
    /// From hero's preflop decision (the BB facing the button's raise): hero calls and both check down to the river,
    /// hero to act.
    fn to_the_river(e: &mut Engine) {
        e.apply_action(Action::Call).unwrap();
        for board in ["Kh 7d 2c", "Kh 7d 2c 4d", "Kh 7d 2c 4d 9s"] {
            e.set_board(&cards(board)).unwrap();
            if board.len() < 14 { e.apply_action(Action::Check).unwrap(); e.apply_action(Action::Check).unwrap(); }
        }
    }

    /// 29-M1: every invalidating call sets the equity token while it still holds the identity lock. The observation runs
    /// on the calling thread immediately before the store, where `try_lock` of the identity must find it held (by this
    /// same thread: `try_lock` never blocks). No other thread uses the identity at those points: the one request served
    /// here has delivered its `Final` before the calls that follow it.
    #[test]
    fn the_equity_token_is_set_inside_the_identity_lock_hold() {
        let (cfg, _) = cfg_1_2();
        let mut e = Engine::with_core(core());
        let observed: Arc<Mutex<Vec<bool>>> = Arc::default();
        {
            let (ids, observed) = (e.identity.clone(), observed.clone());
            e.before_equity_store = Some(Box::new(move || observed.lock().unwrap().push(matches!(ids.try_lock(), Err(TryLockError::WouldBlock)))));
        }
        let check = |e: &mut Engine, what: &str, stores: usize, op: &dyn Fn(&mut Engine)| {
            let token = Arc::new(AtomicBool::new(false));
            *e.equity_cancel.lock().unwrap() = Some(token.clone());
            let before = observed.lock().unwrap().len();
            op(e);
            let seen = observed.lock().unwrap()[before..].to_vec();
            assert_eq!(seen, vec![true; stores], "{what}: the token is set with the identity lock held");
            assert_eq!(token.load(Ordering::SeqCst), stores > 0, "{what}: the token is set exactly when the decision is superseded");
        };
        check(&mut e, "set_config", 1, &|e| { e.set_config(cfg.clone()).unwrap(); });
        check(&mut e, "begin_hand", 1, &|e| { e.begin_hand(begin()).unwrap(); });
        check(&mut e, "begin_hand over a hand in progress", 2, &|e| { e.begin_hand(begin()).unwrap(); });
        check(&mut e, "set_hero_cards", 1, &|e| { e.set_hero_cards([Card::parse("Ks").unwrap(), Card::parse("Kd").unwrap()]).unwrap(); });
        check(&mut e, "apply_action", 1, &|e| { e.apply_action(Action::Fold).unwrap(); });
        check(&mut e, "undo", 1, &|e| { e.undo().unwrap(); });
        let (tx, rx) = mpsc::channel();
        struct Acked(mpsc::Sender<()>);
        impl EventSink for Acked { fn emit(&mut self, _ev: RecommendationEvent) { let _ = self.0.send(()); } }
        check(&mut e, "recommend", 1, &|e| { e.recommend(Box::new(Acked(tx.clone()))).unwrap(); });
        rx.recv_timeout(ACK_LIVENESS).expect("the request was served");
        let active = e.identity.lock().unwrap().active().cloned().expect("the served decision is still active");
        check(&mut e, "cancel of a decision that is not the active one", 0, &|e| e.cancel(active.decision_id + 1));
        check(&mut e, "cancel", 1, &|e| e.cancel(active.decision_id));
        check(&mut e, "finish_hand", 1, &|e| e.finish_hand());
        e.begin_hand(begin()).unwrap();
        for a in [Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call] { e.apply_action(a).unwrap(); }
        check(&mut e, "set_board", 1, &|e| { e.set_board(&cards("Kh 7d 2c")).unwrap(); });
        check(&mut e, "abandon_hand", 1, &|e| e.abandon_hand());
        check(&mut e, "shutdown", 1, &|e| e.shutdown());
    }

    /// Named `try_lock` probes of every engine lock (ruling 28-I1: identity, snapshot store, config, range source, and
    /// the equity token slot and the stage the core shares), run inside a sink callback. The `EngineCore` itself has no
    /// lock to probe: `engine-main` owns it outright (ruling 29-I3); before that change this list also probed the
    /// `Arc<Mutex<EngineCore>>` guard `engine-main` held across `serve_request`, and found it held.
    type Probes = Vec<(&'static str, Box<dyn Fn() -> bool + Send>)>;
    fn probes(e: &Engine, stage: &Arc<Mutex<String>>) -> Probes {
        fn free<T: ?Sized + Send + 'static>(m: &Arc<Mutex<T>>) -> Box<dyn Fn() -> bool + Send> { let m = m.clone(); Box::new(move || m.try_lock().is_ok()) }
        vec![("identity", free(&e.identity)), ("snapshots", free(&e.snapshots)), ("config", free(&e.shared_config)), ("range source", free(&e.range_source)),
            ("equity token slot", free(&e.equity_cancel)), ("stage", free(stage))]
    }
    /// Runs its probes in its first callback and reports that event and which locks were free.
    type Report = (RecommendationEvent, Vec<(&'static str, bool)>);
    struct Probe { probes: Probes, report: Option<mpsc::Sender<Report>> }
    impl EventSink for Probe {
        fn emit(&mut self, ev: RecommendationEvent) {
            if let Some(report) = self.report.take() {
                let free = self.probes.iter().map(|(name, free)| (*name, free())).collect();
                let _ = report.send((ev, free));
            }
        }
    }

    /// 29-I3 (ruling 28-I1): no engine lock is held while a sink callback runs on `engine-main`, neither while a request's
    /// `Final` is handed over (`serve::finish`, here hero's preflop decision's) nor while an event goes through
    /// `serve::deliver` (a river request's `Fast`, probed before its `fast-path` thread exists). Only `engine-main` and
    /// the test thread run then.
    #[test]
    fn no_engine_lock_is_held_during_a_sink_callback() {
        let (cfg, _) = cfg_1_2();
        let core = core();
        let stage = core.stage.clone();
        let mut e = Engine::with_core(core);
        e.set_config(cfg).unwrap();
        e.begin_hand(begin()).unwrap();
        for a in [Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold] { e.apply_action(a).unwrap(); }
        let (tx, rx) = mpsc::channel();
        e.recommend(Box::new(Probe { probes: probes(&e, &stage), report: Some(tx) })).unwrap();
        let preflop_final = rx.recv_timeout(ACK_LIVENESS).expect("hero's preflop decision was answered");
        to_the_river(&mut e);
        let board = cards("Kh 7d 2c 4d 9s");
        let mut full = Range1326([1.0; 1326]);
        for (i, w) in full.0.iter_mut().enumerate() { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { *w = 0.0; } }
        e.set_explicit_ranges(full.clone(), full);
        let (tx, rx) = mpsc::channel();
        e.recommend(Box::new(Probe { probes: probes(&e, &stage), report: Some(tx) })).unwrap();
        let river_fast = rx.recv_timeout(ACK_LIVENESS).expect("the river request's Fast was handed over");
        e.shutdown();
        assert!(matches!(preflop_final.0, RecommendationEvent::Final(_)), "hero's preflop decision was answered by its Final: {:?}", preflop_final.0);
        assert!(matches!(river_fast.0, RecommendationEvent::Fast(_)), "the river request's first event is its Fast: {:?}", river_fast.0);
        for (what, (_, report)) in [("a preflop Final", preflop_final), ("a river Fast", river_fast)] {
            let held: Vec<&str> = report.iter().filter(|(_, free)| !free).map(|(name, _)| *name).collect();
            assert!(held.is_empty(), "{what}: engine locks held during the sink callback: {held:?}");
        }
    }
}
