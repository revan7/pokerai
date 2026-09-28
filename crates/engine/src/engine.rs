//! §3.5 public surface: commands never block; `engine-main` serves the request slot of depth 1 (newest wins).
//!
//! Ownership. `Engine::new` builds the `EngineCore` (Task 27's defaults: an empty snapshot store, the default session
//! config, `ExplicitRanges` with no ranges; Task 20's watchdog) around the Task 18 process worker link, whose `ready`
//! validation refuses a worker built without AVX2 (§3.6, §3.7), then hands it to `engine-main`. The caller locates the
//! worker binary (`Paths::worker_exe`): the app stages it, the tests and `bench` find it through `POKERAI_WORKER`; the
//! engine never searches for it. `with_core` takes a core built elsewhere (the tests' fake worker and clock).
//!
//! Identity (§4.4, §5 step 3). Every call that invalidates the active decision (`set_config`, `begin_hand`,
//! `set_hero_cards`, `apply_action`, `set_board`, `undo`, `cancel` of the active decision, `finish_hand`,
//! `abandon_hand`) also cancels the `fast-path` equity of the request served last (ruling 28-I4, Task 28 fix-Q1), in
//! the same hold of the identity lock (`supersede`), so no superseded request keeps its equity running.
//!
//! Locks. Every field a command touches is either owned by `Engine` or shared through its own `Arc<Mutex<_>>`. Only
//! `shutdown` locks the `EngineCore` itself, and it joins `engine-main` first, so no command can block behind a running
//! solve (§3.4). Lock order: the identity lock, then the equity token slot (`serve_request` takes the slot alone); the
//! snapshot store, the config and the range source are each locked alone.
use crate::clock::{Clock, SystemClock};
use crate::core::EngineCore;
use crate::identity::IdentityState;
use crate::log::DecisionLog;
use crate::ranges::{ExplicitRanges, RangeSource};
use crate::serve::{serve_request, LiveRequest};
use crate::snapshots::SnapshotStore;
use crate::startup::StartupReport;
use crate::worker::process::ProcessWorker;
use crate::{EngineError, EventSink};
use core_model::{apply_action, begin_hand, set_board, set_hero_cards, BeginHand as CoreBeginHand};
use proto::worker::EngineMessage;
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

pub struct Engine {
    identity: Arc<Mutex<IdentityState>>, clock: Arc<dyn Clock>, snapshots: Arc<Mutex<SnapshotStore>>,
    shared_config: Arc<Mutex<GameConfig>>, range_source: Arc<Mutex<Box<dyn RangeSource>>>, startup: StartupReport,
    /// `EngineCore::equity_cancel`: the equity cancellation token of the request served last (ruling 28-I4).
    equity_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    core: Arc<Mutex<EngineCore>>, slot: Arc<(Mutex<Slot>, Condvar)>,
    state: Option<HandState>, undo: Vec<HandState>, config: GameConfig, queued_config: Option<GameConfig>,
    main: Option<std::thread::JoinHandle<()>>, stopped: bool,
}

impl Engine {
    /// Validates `cfg`, launches the worker at `paths.worker_exe` (its `ready` validated, §4.5: a worker built without
    /// AVX2 is refused, §3.7) and starts `engine-main`; `cfg` becomes the session's first config revision.
    pub fn new(cfg: GameConfig, paths: Paths) -> Result<Engine, EngineError> {
        validate_config(&cfg)?;
        let worker = ProcessWorker::spawn(&paths.worker_exe, cfg.solver.threads).map_err(|e| EngineError::Message(e.to_string()))?;
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let core = EngineCore::new(Box::new(worker), clock, identity, DecisionLog::open(&paths.log_dir));
        let mut e = Engine::with_core(core);
        e.set_config(cfg)?;
        Ok(e)
    }

    pub fn with_core(core: EngineCore) -> Engine {
        let identity = core.identity.clone();
        let clock = core.clock.clone();
        let snapshots = core.snapshots.clone();
        let shared_config = core.config.clone();
        let range_source = core.range_source.clone();
        let equity_cancel = core.equity_cancel.clone();
        let config = core.config();
        // Captured once, before the core is handed to `engine-main`, so `startup_report` never takes that lock.
        let startup = StartupReport::from_ready(core.worker.ready());
        let core = Arc::new(Mutex::new(core));
        let slot = Arc::new((Mutex::new(Slot { pending: None, stop: false }), Condvar::new()));
        let (c2, s2) = (core.clone(), slot.clone());
        let main = std::thread::Builder::new().name("engine-main".into()).spawn(move || loop {
            let req = { let (m, cv) = &*s2; let mut g = m.lock().unwrap(); while g.pending.is_none() && !g.stop { g = cv.wait(g).unwrap(); } if g.stop { return; } g.pending.take().unwrap() };
            serve_request(&mut c2.lock().unwrap(), req);
        }).expect("engine-main");
        Engine { identity, clock, snapshots, shared_config, range_source, startup, equity_cancel, core, slot, state: None, undo: vec![], config, queued_config: None,
            main: Some(main), stopped: false }
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
        let identity = self.identity.lock().unwrap().next_decision().ok_or(EngineError::Message("no hand in progress".into()))?;
        let t0_ms = self.clock.now_ms();
        let (m, cv) = &*self.slot;
        m.lock().unwrap().pending = Some(LiveRequest { identity: identity.clone(), state, t0_ms, sink: Arc::new(Mutex::new(sink)) });
        cv.notify_one();
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

    /// `&mut self` and idempotent: Tauri managed state cannot move out of the handle (spec §3.5). The hand is
    /// invalidated first, so a request still running stops at its next identity check rather than at its deadline; then
    /// `engine-main` is joined, and the worker is asked to shut down and killed. Never panics, even when a panic on
    /// `engine-main` poisoned the core: the worker is killed either way.
    pub fn shutdown(&mut self) {
        if self.stopped { return; }
        self.stopped = true;
        self.supersede(|ids| ids.invalidate_hand());
        { let (m, cv) = &*self.slot; lock(m).stop = true; cv.notify_all(); }
        if let Some(h) = self.main.take() { let _ = h.join(); }
        let mut core = lock(&self.core);
        let id = core.next_id();
        let _ = core.worker.send(&EngineMessage::Shutdown { id });
        core.worker.kill();
    }
}

impl Drop for Engine { fn drop(&mut self) { self.shutdown(); } }

/// A lock for the paths `shutdown` takes, which also runs from `Drop`: a panic that poisoned one of these locks (an
/// engine bug on `engine-main`, already reported) must not turn the shutdown into a second panic, or into an abort
/// during unwinding, and must not leave the worker process running. Every value behind them stays consistent at any
/// point a panic could interrupt it (a counter step, an `Option` set or taken, a flag).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) }
