//! §3.5 public surface: commands never block; `engine-main` serves the request slot of depth 1 (newest wins).
//!
//! Construction. `Engine::new` builds the `EngineCore` (Task 27's defaults: an empty snapshot store, the default
//! session config, `ExplicitRanges` with no ranges; Task 20's watchdog) around the Task 18 process worker link, whose
//! `ready` validation refuses a worker built without AVX2 (§3.6, §3.7), then hands it to `engine-main`. The caller
//! locates the worker binary (`Paths::worker_exe`): the app stages it, the tests and `bench` find it through
//! `POKERAI_WORKER`; the engine never searches for it. `with_core` takes a core built elsewhere (the tests' fake worker
//! and clock).
//!
//! The preflop store (plan 3 Task 17, spec 5 step 1, spec 8.2). `Engine::new` loads it once, from `Paths::preflop`
//! (`preflop::load_store`: installed bundle directories with the store's own quarantine, then the packaged chart pairs
//! Plan 5 stages there, read-only), before the core is handed to `engine-main`; every loader banner joins the startup
//! report's, and the sources that failed validation are its `quarantined_bundles`. The core keeps the store behind an
//! `Arc`, cloned into `Engine` for `preflop_store`, so no recommendation reads the disk. A missing or empty store is a
//! banner, never a construction error: every preflop decision then answers `MissingPreflopNode`.
//!
//! Degraded engine (spec 12 line 658, ruling 29-I4). A worker whose `ready` is refused (protocol version, solver commit,
//! adapter version, `threads`, or `build_features` without AVX2: `WorkerLinkError::ReadyRefused`, follow-up P2.W2) does
//! not fail construction: the core gets a `RefusedWorker` link, which never launches anything, `startup_report` says
//! `worker_ready: false` with the typed refusal, and every decision is answered with the non-retryable
//! `EngineError("worker/proto version mismatch: ...")` until the worker is rebuilt. An invalid config, and a worker
//! that could not be launched at all (`Spawn`: the binary missing, a launch fault, no `ready` in time), still fail
//! construction.
//!
//! Identity (§4.4, §5 step 3). Every call that invalidates the active decision (`begin_hand`, `set_hero_cards`,
//! `apply_action`, `set_board`, `undo`, `recommend`, `cancel` of the active decision, `finish_hand`, `abandon_hand`,
//! `shutdown`) also cancels the `fast-path` equity of the request served last (ruling 28-I4, Task 28 fix-Q1, ruling
//! 29-I1), in the same hold of the identity lock (`supersede`), so no superseded request keeps its equity running.
//! `set_config` invalidates nothing: a config change is not applied to the hand in progress (spec 12), whose decisions
//! keep the hand's config revision (final review I4, ruling F-I4).
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
//!
//! Admission (final review I1, orchestrator ruling F-I1; spec 7, spec 5 step 4). `recommend` supersedes the active
//! decision, allocates the new one, stamps `t0` and admits the request (`serve::admit`): a request at a decision point
//! is armed on the shared watchdog right there, before `engine-main` gets to it, so its `Final` comes at its fire
//! whatever `engine-main` is busy with. The request then waits in the depth-1 slot. One it replaces there is never
//! served: `engine-main` retires it (`serve::retire_unserved`), logging a `Final` its watchdog delivered, as it retires
//! whatever is still queued when it stops.
//!
//! The scheduler (`engine-main`). In order: requests to retire, the pending request, and, with nothing else to do, a
//! cancel left pending by a superseded job (`solve::await_pending_cancel`, final review I1), awaited in slices and given
//! up for the next request as soon as one arrives. A superseded job's cancel window thus never delays the newer
//! request's `Fast`.
//!
//! Panics (final review I3, ruling F-I3). `engine-main` serves each request inside `catch_unwind`: a panic is contained
//! at that boundary (`serve::contain_panic`: the request's internal-error `Final` through its claim unless the watchdog
//! delivered, its generation retired, the `Final` logged, the worker killed) and `engine-main` goes on serving. Should
//! the loop end anyway, `recommend` refuses (the loop has finished, or its watchdog is stopped) rather than accept a
//! request nothing would serve.
use crate::clock::{Clock, SystemClock};
use crate::core::EngineCore;
use crate::identity::IdentityState;
use crate::log::DecisionLog;
use crate::ranges::{ExplicitRanges, RangeSource};
use crate::serve::{admit, LiveRequest};
use crate::snapshots::SnapshotStore;
use crate::startup::StartupReport;
use crate::watchdog::Watchdog;
use crate::worker::link::{RefusedWorker, WorkerLink, WorkerLinkError};
use crate::worker::process::ProcessWorker;
use crate::{EngineError, EventSink};
use core_preflop::PreflopStore;
use core_model::{apply_action, begin_hand, set_board, set_hero_cards, BeginHand as CoreBeginHand};
use proto::{Action, Card, DecisionIdentity, GameConfig, HandConfig, HandState, Range1326};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// All four directories are declared here so downstream plans have nothing to add:
/// `log_dir` and `worker_exe` are used by this plan, `preflop` by plan 3 (the directory `preflop::load_store` reads once,
/// at construction: installed bundle directories and the packaged chart pairs), `cache` by plan 4.
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

/// The request slot of depth 1: the newest request, those it replaced (never served, to retire), and the stop flag.
struct Slot { pending: Option<LiveRequest>, dropped: Vec<LiveRequest>, stop: bool }

/// What `engine-main` does next (see "The scheduler" in the module doc).
enum Work {
    /// Requests replaced in the slot before `engine-main` got to them.
    Retire(Vec<LiveRequest>),
    Serve(LiveRequest),
    /// Nothing queued, and a superseded job's cancel still awaits its confirmation.
    AwaitCancel,
    /// `shutdown`: whatever is still queued is retired, then the loop ends.
    Stop(Vec<LiveRequest>),
}

/// The text of a caught panic's payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|m| m.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic with no message".into())
}

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
    fn serve(&self, core: &mut EngineCore, req: &LiveRequest) {
        match self {
            Server::Production => crate::serve::serve_admitted(core, req),
            #[cfg(any(test, feature = "testing"))]
            Server::Seams(seams) => crate::serve::serve_admitted_with(core, req, seams),
        }
    }

    /// Serves `req` inside `catch_unwind` and contains a panic at that boundary (final review I3); the containment is
    /// guarded the same way, so a panicking sink cannot end `engine-main` either.
    fn serve_contained(&self, core: &mut EngineCore, req: &LiveRequest) {
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.serve(core, req))) {
            let message = panic_message(payload.as_ref());
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::serve::contain_panic(core, req, &message)));
        }
    }
}

/// Retires requests `engine-main` never served (`serve::retire_unserved`), each guarded like a served one.
fn retire_all(core: &mut EngineCore, reqs: Vec<LiveRequest>) {
    for req in reqs {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::serve::retire_unserved(core, &req)));
    }
}

pub struct Engine {
    identity: Arc<Mutex<IdentityState>>, clock: Arc<dyn Clock>, snapshots: Arc<Mutex<SnapshotStore>>,
    /// `EngineCore::watchdog`, shared: `recommend` arms each request at admission (final review I1).
    watchdog: Arc<Watchdog>,
    shared_config: Arc<Mutex<GameConfig>>, range_source: Arc<Mutex<Box<dyn RangeSource>>>, startup: StartupReport,
    /// `EngineCore::preflop`, loaded once before the core was handed to `engine-main` (plan 3 Task 17).
    preflop: Arc<PreflopStore>,
    /// `EngineCore::equity_cancel`: the equity cancellation token of the request served last (ruling 28-I4).
    equity_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    slot: Arc<(Mutex<Slot>, Condvar)>,
    state: Option<HandState>, undo: Vec<HandState>, config: GameConfig, queued_config: Option<GameConfig>,
    main: Option<std::thread::JoinHandle<()>>, stopped: bool,
    /// Unit tests only: runs immediately before the equity token is set (ruling 29-M1).
    #[cfg(test)]
    before_equity_store: Option<Box<dyn Fn() + Send>>,
    /// Unit tests only: the stage slot of the request admitted last (its own, final review I1), for the lock probes.
    #[cfg(test)]
    admitted_stage: Arc<Mutex<Option<Arc<Mutex<String>>>>>,
}

impl Engine {
    /// Validates `cfg`, launches the worker at `paths.worker_exe` (its `ready` validated, §4.5: a worker built without
    /// AVX2 is refused, §3.7), loads the preflop store from `paths.preflop` (plan 3 Task 17; its banners and quarantined
    /// bundles join the startup report) and starts `engine-main`; `cfg` becomes the session's first config revision. A
    /// refused `ready` leaves a degraded engine (see the module doc); an invalid config or a failed launch is an error,
    /// and neither touches the preflop directory.
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
        let mut core = EngineCore::new(worker, clock, identity, DecisionLog::open(&paths.log_dir));
        // Loaded once, here, before the core is handed to `engine-main` (plan 3 Task 17).
        let loaded = crate::preflop::load_store(&paths.preflop);
        core.preflop = Arc::new(loaded.store);
        let mut e = Engine::with_core(core);
        e.startup.banners.extend(loaded.banners);
        e.startup.quarantined_bundles = loaded.quarantined;
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
        let watchdog = core.watchdog.clone();
        let preflop = core.preflop.clone();
        let config = core.config();
        // Captured once, before the core is handed to `engine-main`, which owns it from then on.
        let startup = StartupReport::from_worker(core.worker.as_ref());
        let slot = Arc::new((Mutex::new(Slot { pending: None, dropped: vec![], stop: false }), Condvar::new()));
        let s2 = slot.clone();
        let main = std::thread::Builder::new().name("engine-main".into()).spawn(move || {
            // Owned here alone (ruling 29-I3), and torn down on every way out of this thread (ruling 29-I2).
            let mut owned = TornDown(core);
            let queued = |slot: &Mutex<Slot>| { let g = lock(slot); g.pending.is_some() || !g.dropped.is_empty() || g.stop };
            loop {
                let work = {
                    let (m, cv) = &*s2;
                    let mut g = lock(m);
                    loop {
                        if g.stop {
                            let mut rest = std::mem::take(&mut g.dropped);
                            rest.extend(g.pending.take());
                            break Work::Stop(rest);
                        }
                        if !g.dropped.is_empty() { break Work::Retire(std::mem::take(&mut g.dropped)); }
                        if let Some(req) = g.pending.take() { break Work::Serve(req); }
                        if owned.0.pending_cancel.is_some() { break Work::AwaitCancel; }
                        g = cv.wait(g).unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                };
                match work {
                    Work::Retire(reqs) => retire_all(&mut owned.0, reqs),
                    Work::Serve(req) => server.serve_contained(&mut owned.0, &req),
                    // Given up as soon as anything is queued (the slot is checked between receive slices).
                    Work::AwaitCancel => { crate::solve::await_pending_cancel(&mut owned.0, &mut |_| queued(&s2.0)); }
                    Work::Stop(rest) => {
                        retire_all(&mut owned.0, rest);
                        return;
                    }
                }
            }
        }).expect("spawn engine-main");
        Engine { identity, clock, snapshots, watchdog, shared_config, range_source, startup, preflop, equity_cancel, slot, state: None, undo: vec![], config,
            queued_config: None, main: Some(main), stopped: false, #[cfg(test)] before_equity_store: None, #[cfg(test)] admitted_stage: Arc::default() }
    }

    /// §12 startup diagnostics for the UI; never blocks (the worker's `ready` is captured at construction,
    /// and plans 3 and 4 fill `quarantined_bundles` / `cache_state` at the same point).
    pub fn startup_report(&self) -> StartupReport { self.startup.clone() }

    /// The preflop store the engine loaded once at construction (plan 3 Task 17; plan 4's consumption point): the same
    /// store every preflop decision reads, never reloaded.
    pub fn preflop_store(&self) -> &PreflopStore { &self.preflop }

    /// §4.2 / §13.3: validates the config, allocates a revision and applies it — immediately when no hand is in
    /// progress, otherwise from the next `begin_hand` (the active hand keeps its frozen `HandConfig`). A rejected config
    /// consumes no revision. Spec 12, "not applied to the active hand" (final review I4, ruling F-I4): the hand's
    /// decisions keep the hand's config revision, so nothing is superseded, the decision in flight included.
    pub fn set_config(&mut self, cfg: GameConfig) -> Result<u32, EngineError> {
        validate_config(&cfg)?;
        let rev = lock(&self.identity).set_config();
        let stamped = GameConfig { config_revision: rev, ..cfg };
        if self.state.is_some() { self.queued_config = Some(stamped); } else { self.apply_config(stamped); }
        Ok(rev)
    }

    fn apply_config(&mut self, cfg: GameConfig) { self.config = cfg.clone(); *lock(&self.shared_config) = cfg; }

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
        let s = begin_hand(&HandConfig::from_game(cfg), core_req).map_err(EngineError::Rules)?;
        if self.state.is_some() { self.end_hand(); }
        if let Some(c) = self.queued_config.take() { self.apply_config(c); }
        let ((hand_id, rev), hand_config_revision) = self.supersede(|ids| (ids.begin_hand(), ids.hand_config_revision()));
        // The hand's identities carry the revision of the config it froze (ruling F-I4): the session's latest, which
        // every accepted `set_config` stamps and this hand has just applied.
        assert!(hand_config_revision == Some(s.config.config_revision),
            "hand {hand_id} froze config revision {} but its identities carry {hand_config_revision:?}", s.config.config_revision);
        self.undo.clear();
        self.state = Some(self.stamp(s, hand_id, rev));
        Ok(self.state.clone().unwrap())
    }

    fn mutate(&mut self, next: HandState) -> HandState {
        let rev = self.supersede(|ids| ids.mutate());
        let hand_id = self.state.as_ref().map(|p| p.hand_id).unwrap_or(next.hand_id);
        if let Some(prev) = self.state.take() { self.undo.push(prev); }
        let s = self.stamp(next, hand_id, rev);
        // §9.2 prefix-based invalidation against the new state: an append-only mutation keeps earlier roots.
        lock(&self.snapshots).invalidate(&s);
        self.state = Some(s.clone());
        s
    }

    /// §5 step 3: a mutation like any other (fresh revision, in-flight work invalidated).
    pub fn set_hero_cards(&mut self, cards: [Card; 2]) -> Result<HandState, EngineError> {
        let cur = self.state.as_ref().ok_or(EngineError::Message("no hand".into()))?;
        let next = set_hero_cards(cur, cards).map_err(EngineError::Rules)?;
        Ok(self.mutate(next))
    }

    pub fn apply_action(&mut self, a: Action) -> Result<HandState, EngineError> {
        let cur = self.state.as_ref().ok_or(EngineError::Message("no hand".into()))?;
        let next = apply_action(cur, a).map_err(EngineError::Rules)?;
        Ok(self.mutate(next))
    }

    pub fn set_board(&mut self, cards: &[Card]) -> Result<HandState, EngineError> {
        let cur = self.state.as_ref().ok_or(EngineError::Message("no hand".into()))?;
        let next = set_board(cur, cards).map_err(EngineError::Rules)?;
        Ok(self.mutate(next))
    }

    pub fn undo(&mut self) -> Result<HandState, EngineError> {
        let prev = self.undo.pop().ok_or(EngineError::Message("nothing to undo".into()))?;
        let rev = self.supersede(|ids| ids.mutate());
        let hand_id = self.state.as_ref().map(|s| s.hand_id).unwrap_or(prev.hand_id);
        let s = self.stamp(prev, hand_id, rev);
        // §9.2: later streets and same-street snapshots whose solved prefix no longer fits are dropped; retained ones
        // keep their original identity (the new revision never rewrites it).
        lock(&self.snapshots).invalidate(&s);
        self.state = Some(s.clone());
        Ok(s)
    }

    /// Plan 2 only: the public ranges at the street root (plan 3 installs a replay-backed `RangeSource` instead).
    pub fn set_explicit_ranges(&mut self, oop: Range1326, ip: Range1326) { *lock(&self.range_source) = Box::new(ExplicitRanges { oop: Some(oop), ip: Some(ip) }); }

    /// Allocates the decision's identity, stamps `t0`, admits the request (`serve::admit`: a decision point is armed on
    /// the watchdog here, final review I1) and queues it for `engine-main` (depth 1: an older pending request is
    /// replaced, and `engine-main` retires it); its events go to `sink`. Refused with no hand in progress, once shut
    /// down, and once `engine-main` has ended (final review I3): nothing would serve the request.
    pub fn recommend(&mut self, sink: Box<dyn EventSink>) -> Result<DecisionIdentity, EngineError> {
        if self.stopped { return Err(EngineError::Message("the engine is shut down".into())); }
        if self.main.as_ref().is_none_or(|h| h.is_finished()) {
            return Err(EngineError::Message("the engine is not running: engine-main has ended".into()));
        }
        let state = self.state.clone().ok_or(EngineError::Message("no hand".into()))?;
        // The new decision supersedes the active one: allocated in the same identity-lock hold that cancels the equity
        // of the request served last (ruling 29-I1, 28-I4), not when `engine-main` gets to the new request.
        let identity = self.supersede(|ids| ids.next_decision()).ok_or(EngineError::Message("no hand in progress".into()))?;
        let t0_ms = self.clock.now_ms();
        let req = match admit(&self.watchdog, &self.identity, self.config.clone(), identity.clone(), state, t0_ms, Arc::new(Mutex::new(sink))) {
            Ok(req) => req,
            Err(e) => {
                // A stopped watchdog: `engine-main` has ended. The decision allocated above is withdrawn with the request.
                let mut ids = lock(&self.identity);
                if ids.is_active(&identity) { ids.cancel_active(); }
                return Err(e);
            }
        };
        #[cfg(test)]
        { *lock(&self.admitted_stage) = req.watch.as_ref().map(|w| w.stage.clone()); }
        let (m, cv) = &*self.slot;
        {
            let mut g = lock(m);
            // Depth 1, newest wins: an older request still pending is replaced, and retired by `engine-main`.
            if let Some(older) = g.pending.replace(req) { g.dropped.push(older); }
        }
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
        if let Some(h) = hand_id { lock(&self.snapshots).invalidate_hand(h); }
        self.state = None;
        self.undo.clear();
        if let Some(c) = self.queued_config.take() { self.apply_config(c); }
    }

    pub fn state(&self) -> Option<HandState> { self.state.clone() }

    /// `&mut self` and idempotent: Tauri managed state cannot move out of the handle (spec §3.5); a second call does
    /// nothing. In order (ruling 29-I2): the hand is invalidated and the last request's equity cancelled (one identity
    /// hold), so a request still running stops at its next identity check rather than at its deadline; scheduling stops
    /// (`engine-main` retires whatever is still queued, logging a `Final` a queued request's watchdog delivered); then
    /// `engine-main` is joined, holding no lock, and on its way out it tears the core down (`EngineCore::shutdown`: the
    /// watchdog's and the requests' threads woken and joined, then the worker told to shut down and killed). When this
    /// returns, no thread the engine started is running and the worker is gone.
    pub fn shutdown(&mut self) {
        if self.stopped { return; }
        self.stopped = true;
        self.supersede(|ids| ids.invalidate_hand());
        { let (m, cv) = &*self.slot; lock(m).stop = true; cv.notify_all(); }
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
        // Ruling F-I4: a settings change supersedes nothing (the hand keeps its config revision), so it stores nothing.
        check(&mut e, "set_config", 0, &|e| { e.set_config(cfg.clone()).unwrap(); });
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

    /// Final review I3 (ruling F-I3): once `engine-main` has ended other than by `shutdown` (a panic beyond its
    /// containment; here its loop is stopped under the engine's feet, which tears its core down and stops the watchdog),
    /// `recommend` refuses: nothing would serve the request. It refuses too when the watchdog is stopped while the loop
    /// is still running (the teardown under way), and then withdraws the decision it allocated: no decision is left
    /// active for a request nothing serves.
    #[test]
    fn recommend_refuses_once_engine_main_has_ended() {
        struct Nothing;
        impl EventSink for Nothing { fn emit(&mut self, _ev: RecommendationEvent) {} }
        let (cfg, _) = cfg_1_2();
        // The watchdog stopped under a running loop: the decision allocated is withdrawn.
        let mut e = Engine::with_core(core());
        e.set_config(cfg.clone()).unwrap();
        e.begin_hand(begin()).unwrap();
        for a in [Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold] { e.apply_action(a).unwrap(); } // hero to act: a decision point
        e.watchdog.stop();
        let refused = e.recommend(Box::new(Nothing));
        assert!(matches!(refused, Err(EngineError::Message(ref m)) if m.contains("not running")), "{refused:?}");
        assert!(e.identity.lock().unwrap().active().is_none(), "no decision is left active for a request nothing serves");
        e.shutdown();
        // The loop ended.
        let mut e = Engine::with_core(core());
        e.set_config(cfg).unwrap();
        e.begin_hand(begin()).unwrap();
        for a in [Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold] { e.apply_action(a).unwrap(); } // hero to act: a decision point
        { let (m, cv) = &*e.slot; lock(m).stop = true; cv.notify_all(); }
        e.main.take().expect("engine-main runs").join().expect("engine-main ended without a panic");
        let refused = e.recommend(Box::new(Nothing));
        assert!(matches!(refused, Err(EngineError::Message(ref m)) if m.contains("not running")), "{refused:?}");
        assert!(e.identity.lock().unwrap().active().is_none());
        e.shutdown();
    }

    /// A flop snapshot of the engine's current hand, solved for decision `decision_id` at `prefix` (plan 3 Task 14).
    fn flop_snapshot(e: &Engine, decision_id: u64, prefix: Vec<(Seat, Action)>) -> (DecisionIdentity, crate::snapshots::StreetSnapshot) {
        use crate::snapshots::{SnapshotKey, SnapshotProvenance, StreetSnapshot};
        let s = e.state().expect("a hand in progress");
        let identity = DecisionIdentity { hand_id: s.hand_id, hand_revision: s.hand_revision, decision_id, config_revision: s.config.config_revision, model_revision: 0 };
        let tree = proto::EffectiveTree { rules_version: 3, template_id: "t".into(), root_street: proto::Street::Flop, menus: Default::default(), add_allin_threshold: 0.0,
            force_allin_threshold: 0.0, merging_threshold: 0.0, wager_cap: 1, inserted: vec![], materialized: vec![] };
        let snapshot = StreetSnapshot {
            key: SnapshotKey { hand_id: s.hand_id, config_revision: s.config.config_revision, model_revision: 0, street: proto::Street::Flop, root_board: s.board[..3].to_vec(),
                root_range_hashes: [[0; 32]; 2], tree_signature: "t".into() },
            provenance: SnapshotProvenance { identity_at_solve: identity.clone(), solved_prefix: prefix, origin: "live".into() },
            tree, nodes: vec![], covered_paths: vec![], exploitability_chips: 0.1, reasons: vec![] };
        (identity, snapshot)
    }
    fn solved_for(e: &Engine, hand_id: u64) -> Vec<(u64, u32)> {
        e.snapshots.lock().unwrap().for_hand(hand_id).iter().map(|s| (s.provenance.identity_at_solve.decision_id, s.provenance.identity_at_solve.hand_revision)).collect()
    }

    /// Plan 3 Task 14 (spec 9.2): `apply_action`, `set_board`, `set_hero_cards` and `undo` invalidate by prefix against the
    /// new state, so an append-only mutation keeps every snapshot whose solved prefix still fits, with its original
    /// identity, and an undo drops only those it no longer fits; `finish_hand` (as `begin_hand` over a hand and
    /// `abandon_hand`) drops the hand's snapshots by identity.
    #[test]
    fn mutations_invalidate_snapshots_by_prefix_and_ending_the_hand_drops_them() {
        let (cfg, _) = cfg_1_2();
        let mut e = Engine::with_core(core());
        e.set_config(cfg).unwrap();
        e.begin_hand(begin()).unwrap();
        for a in [Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call] { e.apply_action(a).unwrap(); }
        e.set_board(&cards("Kh 7d 2c")).unwrap();
        // Hero (the BB, seat 2) is first to act on the flop: a decision at the street root.
        let (root_id, at_root) = flop_snapshot(&e, 1, vec![]);
        assert!(e.snapshots.lock().unwrap().register(&root_id, at_root));
        // Hero checks and the button bets: hero's next decision, solved at the street root's history there (ruling 14-I1:
        // every snapshot is registered at a hero decision, in its street root's history domain).
        e.apply_action(Action::Check).unwrap();
        e.apply_action(Action::Bet { to: 40 }).unwrap();
        let (bet_id, facing_bet) = flop_snapshot(&e, 2, vec![(Seat(2), Action::Check), (Seat(0), Action::Bet { to: 40 })]);
        assert!(e.snapshots.lock().unwrap().register(&bet_id, facing_bet));
        let hand = root_id.hand_id;
        let solved = vec![(1, root_id.hand_revision), (2, bet_id.hand_revision)];
        // Append-only: hero calls; both solved decisions are still in the flop's history; nothing is rewritten.
        e.apply_action(Action::Call).unwrap();
        assert_eq!(solved_for(&e, hand), solved, "apply_action keeps prefix-valid snapshots with their identity");
        e.set_hero_cards([Card::parse("Ks").unwrap(), Card::parse("Kd").unwrap()]).unwrap();
        assert_eq!(solved_for(&e, hand), solved, "set_hero_cards changes nothing public");
        // Undo the hero cards, then the call: back at hero's decision facing the bet; both still fit.
        e.undo().unwrap();
        e.undo().unwrap();
        assert_eq!(solved_for(&e, hand), solved, "undo keeps what still fits");
        // Undo the bet: hero's decision facing it is no longer in the history; the root snapshot keeps its old revision.
        e.undo().unwrap();
        assert!(e.state().unwrap().hand_revision > root_id.hand_revision);
        assert_eq!(solved_for(&e, hand), vec![(1, root_id.hand_revision)], "undo drops a same-street snapshot whose decision is gone");
        e.finish_hand();
        assert!(solved_for(&e, hand).is_empty(), "finish_hand drops the hand's snapshots");
        e.shutdown();
    }

    /// Named `try_lock` probes of every engine lock (ruling 28-I1: identity, snapshot store, config, range source, and
    /// the equity token slot and the stage slot the request shares with its watchdog: its own since admission, final
    /// review I1), run inside a sink callback. The `EngineCore` itself has no lock to probe: `engine-main` owns it
    /// outright (ruling 29-I3); before that change this list also probed the `Arc<Mutex<EngineCore>>` guard `engine-main`
    /// held across `serve_request`, and found it held.
    type Probes = Vec<(&'static str, Box<dyn Fn() -> bool + Send>)>;
    fn probes(e: &Engine) -> Probes {
        fn free<T: ?Sized + Send + 'static>(m: &Arc<Mutex<T>>) -> Box<dyn Fn() -> bool + Send> { let m = m.clone(); Box::new(move || m.try_lock().is_ok()) }
        let admitted = e.admitted_stage.clone();
        let stage: Box<dyn Fn() -> bool + Send> = Box::new(move || admitted.lock().unwrap().as_ref().expect("the request was admitted with a stage slot").try_lock().is_ok());
        vec![("identity", free(&e.identity)), ("snapshots", free(&e.snapshots)), ("config", free(&e.shared_config)), ("range source", free(&e.range_source)),
            ("equity token slot", free(&e.equity_cancel)), ("stage", stage)]
    }
    /// Runs its probes in the callback of the first event `at` selects and reports that event and which locks were free;
    /// acknowledges an `Equity` event on `equity`, when given.
    type Report = (RecommendationEvent, Vec<(&'static str, bool)>);
    struct Probe { probes: Probes, at: fn(&RecommendationEvent) -> bool, report: Option<mpsc::Sender<Report>>, equity: Option<mpsc::Sender<()>> }
    impl EventSink for Probe {
        fn emit(&mut self, ev: RecommendationEvent) {
            if let (RecommendationEvent::Equity { .. }, Some(equity)) = (&ev, &self.equity) {
                let _ = equity.send(());
            }
            if (self.at)(&ev) {
                if let Some(report) = self.report.take() {
                    let free = self.probes.iter().map(|(name, free)| (*name, free())).collect();
                    let _ = report.send((ev, free));
                }
            }
        }
    }

    /// 29-I3 (ruling 28-I1): no engine lock is held while a sink callback runs on `engine-main`, neither while a request's
    /// `Final` is handed over (`serve::finish`, here hero's preflop decision's, through `serve::settle`: plan 3 Task 17;
    /// probed before its `fast-path` thread exists, which starts after the `Final`) nor while an event goes through
    /// `serve::deliver` (a river request's `Fast`, probed before its `fast-path` thread exists). The preflop request's
    /// equity (an instant routine) is acknowledged before the hand moves on, so its thread no longer takes any engine lock
    /// when the river request is probed. Only `engine-main` and the test thread run then.
    #[test]
    fn no_engine_lock_is_held_during_a_sink_callback() {
        use crate::serve::{EquityRoutine, ServeSeams};
        let (cfg, _) = cfg_1_2();
        let core = core();
        let instant: EquityRoutine = Arc::new(|_: &dyn Clock, _: Option<[Card; 2]>, _: &Range1326, _: &[(Seat, Range1326)], _: &[Card], _: std::time::Duration,
            _: &AtomicBool| crate::equity::pending_summary(&[]));
        let mut e = Engine::with_core_and_seams(core, ServeSeams { equity: Some(instant), ..ServeSeams::default() });
        e.set_config(cfg).unwrap();
        e.begin_hand(begin()).unwrap();
        for a in [Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold] { e.apply_action(a).unwrap(); }
        let (tx, rx) = mpsc::channel();
        let (equity_tx, equity_rx) = mpsc::channel();
        let is_final: fn(&RecommendationEvent) -> bool = |ev| matches!(ev, RecommendationEvent::Final(_));
        e.recommend(Box::new(Probe { probes: probes(&e), at: is_final, report: Some(tx), equity: Some(equity_tx) })).unwrap();
        let preflop_final = rx.recv_timeout(ACK_LIVENESS).expect("hero's preflop decision was answered");
        equity_rx.recv_timeout(ACK_LIVENESS).expect("the preflop request's equity was delivered");
        to_the_river(&mut e);
        let board = cards("Kh 7d 2c 4d 9s");
        let mut full = Range1326([1.0; 1326]);
        for (i, w) in full.0.iter_mut().enumerate() { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { *w = 0.0; } }
        e.set_explicit_ranges(full.clone(), full);
        let (tx, rx) = mpsc::channel();
        let is_fast: fn(&RecommendationEvent) -> bool = |ev| matches!(ev, RecommendationEvent::Fast(_));
        e.recommend(Box::new(Probe { probes: probes(&e), at: is_fast, report: Some(tx), equity: None })).unwrap();
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
