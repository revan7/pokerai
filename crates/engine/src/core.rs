//! The state `engine-main` owns (spec 3.4): the worker link, the clock every deadline is measured on, the decision
//! identity, the watchdog, the decision log, the request-id counter, the memory limit and the furthest stage the live
//! request has reached, and (Task 27) the snapshot store, the game config and the range source, and (Task 28) the
//! equity cancellation token of the request served last, and (ruling 29-I2) the threads its requests start, and (plan
//! 3 Task 17) the preflop store, loaded once before the core is handed to `engine-main`.
//!
//! Ownership (rulings 29-I2, 29-I3). `Engine` hands the core to `engine-main`, which owns it alone (no lock around it)
//! and tears it down on its way out (`shutdown`): every thread the core started is stopped and joined, then the worker
//! is told to shut down and killed.

use crate::clock::Clock;
use crate::identity::IdentityState;
use crate::log::DecisionLog;
use crate::ranges::{ExplicitRanges, RangeSource};
use crate::snapshots::SnapshotStore;
use crate::watchdog::Watchdog;
use crate::worker::link::WorkerLink;
use core_preflop::PreflopStore;
use proto::worker::EngineMessage;
use proto::{DecisionIdentity, GameConfig, Rake, SolverPrefs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

/// §10.3: the engine's default `memory_limit_bytes` on every `solve` (10 GiB).
pub const DEFAULT_MEMORY_LIMIT_BYTES: u64 = 10 << 30;

/// The worker stages in the order a request passes them (§4.5); 0 for anything else (`"fast"`, before the request is
/// sent to the worker).
fn stage_rank(s: &str) -> u8 {
    match s {
        "building" => 1,
        "solving" => 2,
        "extracting" => 3,
        _ => 0,
    }
}

/// A lock that survives a panic elsewhere: the teardown runs while `engine-main` unwinds too, and must neither panic
/// again nor stop short of killing the worker; `engine-main` goes on serving after a contained panic (final fix round
/// 2, ruling N2). Every value behind these locks stays consistent at every point a panic could interrupt it (an
/// `Option` set or taken, a list of handles, the identity state, a stage or config replaced whole).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The threads a request starts beside `engine-main` (the `fast-path` equity runner, §3.4), owned by the core (ruling
/// 29-I2): each is registered as it starts, those already ended are joined at the next start, and `join_all` joins the
/// rest. Nothing it runs is detached.
#[derive(Default)]
pub struct Tasks {
    handles: Mutex<Vec<JoinHandle<()>>>,
}

impl Tasks {
    /// Starts `f` on a thread named `name` and keeps its handle.
    pub fn spawn(&self, name: &str, f: impl FnOnce() + Send + 'static) {
        let mut handles = lock(&self.handles);
        let (ended, running): (Vec<_>, Vec<_>) = handles.drain(..).partition(|h| h.is_finished());
        for h in ended {
            let _ = h.join();
        }
        *handles = running;
        handles.push(std::thread::Builder::new().name(name.into()).spawn(f).unwrap_or_else(|e| panic!("spawn the {name} thread: {e}")));
    }

    /// Joins every thread started so far; returns once each has ended. A thread that panicked has reported it already.
    pub fn join_all(&self) {
        let handles = std::mem::take(&mut *lock(&self.handles));
        for h in handles {
            let _ = h.join();
        }
    }

    /// How many started threads have not been joined yet.
    pub fn unjoined(&self) -> usize {
        lock(&self.handles).len()
    }
}

/// State owned by `engine-main` (§3.4). The snapshot store, the game config and the range source (Task 27) are each
/// behind an `Arc<Mutex<_>>` shared with `Engine`, so a settings command or a hand mutation never waits for a running
/// request to release the `EngineCore` lock (§3.4: commands never block).
pub struct EngineCore {
    pub worker: Box<dyn WorkerLink>,
    pub clock: Arc<dyn Clock>,
    pub identity: Arc<Mutex<IdentityState>>,
    /// Shared with `Engine`, which arms each request's generation at admission (final review I1), before `engine-main`
    /// gets to it; `engine-main` retires it (`Watchdog::disarm`) and stops the watchdog at the teardown.
    pub watchdog: Arc<Watchdog>,
    pub log: DecisionLog,
    /// The id of the next request sent to the worker (`solve`, `cancel`, `lock`, `shutdown` all take one), a decimal
    /// string on the wire (§4.5). Never reused within this core.
    pub next_request_id: u64,
    pub memory_limit_bytes: u64,
    /// The furthest stage the request being served has reached, shared with its watchdog (`watchdog::Armed::stage`),
    /// which reports it in `Unsupported{DeadlineExceeded{stage}}`. Each request has a slot of its own, made at its
    /// admission (`serve::admit`, `"queued"` until `engine-main` starts it): `serve_request` installs it here as it
    /// starts the request, so a request waiting behind another never reports the other's stage (final review I1).
    pub stage: Arc<Mutex<String>>,
    /// `core_replay::SnapshotStore` (re-exported by `crate::snapshots`, plan 3 Task 14). Shared with `Engine` so a
    /// mutation can invalidate snapshots without waiting for a running request.
    pub snapshots: Arc<Mutex<SnapshotStore>>,
    /// Read once, as a snapshot, at the start of each request.
    pub config: Arc<Mutex<GameConfig>>,
    /// The only root-range provider (`RangeSource::ranges_at_root`); plan 3 installs its replay-backed source here.
    pub range_source: Arc<Mutex<Box<dyn RangeSource>>>,
    /// The equity cancellation token of the request served last (ruling 28-I4): `serve_request` sets it when the next
    /// request starts (a newer request supersedes it), and `Engine` can clone this handle to set it on a mutation.
    pub equity_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    /// The `fast-path` threads the core's requests started (ruling 29-I2), joined by `shutdown`. Shared so that a test can
    /// see, once the engine is shut down, that none is left unjoined.
    pub tasks: Arc<Tasks>,
    /// A cancel sent for a superseded job whose confirmation is still awaited (spec 7, 12): the wait gave way to a newer
    /// decision's request (final review I1), and is settled before anything more is sent to the worker (`run_solve`)
    /// or while `engine-main` is idle. Cleared by the job's `result{cancelled}` and by any kill or restart.
    pub(crate) pending_cancel: Option<crate::solve::PendingCancel>,
    /// The preflop store (spec 8.1), loaded once, before the core is handed to `engine-main` (`Engine::new` loads it from
    /// `Paths::preflop`, `preflop::load_store`; a core built for a test takes the store it is given), and shared with
    /// `Engine`, which hands it out (`Engine::preflop_store`). Read-only from then on: no recommendation reads the disk.
    /// An empty store until one is installed.
    pub preflop: Arc<PreflopStore>,
    /// `shutdown` has run.
    shut_down: bool,
}

impl EngineCore {
    /// Takes the decision log from the start (Task 21), so this constructor never changes arity (cross-plan section 4).
    pub fn new(worker: Box<dyn WorkerLink>, clock: Arc<dyn Clock>, identity: Arc<Mutex<IdentityState>>, log: DecisionLog) -> Self {
        Self {
            watchdog: Arc::new(Watchdog::new(clock.clone())),
            worker,
            clock,
            identity,
            log,
            next_request_id: 1,
            memory_limit_bytes: DEFAULT_MEMORY_LIMIT_BYTES,
            stage: Arc::new(Mutex::new("fast".into())),
            snapshots: Arc::new(Mutex::new(SnapshotStore::new())),
            config: Arc::new(Mutex::new(GameConfig { config_revision: 0, chip_label: "$1".into(), sb_chips: 5, bb_chips: 10, straddle: None, rake: Rake::TimeCharge, seats: vec![], solver: SolverPrefs { threads: 16, target_bp: 50, flop_budget_s: 10 } })),
            range_source: Arc::new(Mutex::new(Box::new(ExplicitRanges { oop: None, ip: None }))),
            equity_cancel: Arc::new(Mutex::new(None)),
            tasks: Arc::default(),
            pending_cancel: None,
            preflop: Arc::new(PreflopStore::from_sources(vec![])),
            shut_down: false,
        }
    }

    /// The teardown (ruling 29-I2), run once, by `engine-main` on its way out (`Engine::shutdown`, or a panic): the
    /// equity of the request served last is cancelled (earlier ones were cancelled by the requests after them) and every
    /// thread waiting on the engine clock is woken to see it; the watchdog is stopped, which wakes and joins its
    /// generation threads; the `fast-path` threads are joined; then the worker is told to shut down and killed (the link
    /// reaps the process). It returns only once all of that is done. A second call does nothing. Nothing here waits on
    /// an engine lock held elsewhere: `Engine` holds none while it joins `engine-main`, and the threads joined take
    /// the identity and sink locks only briefly.
    pub fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;
        if let Some(token) = lock(&self.equity_cancel).as_ref() {
            token.store(true, Ordering::SeqCst);
        }
        self.clock.wake_waiters();
        self.watchdog.stop();
        self.tasks.join_all();
        let id = self.next_id();
        let _ = self.worker.send(&EngineMessage::Shutdown { id });
        self.worker.kill();
    }

    /// The next request id, `"1"`, `"2"`, ... The counter is checked in its own width: an id is never reused, even in a
    /// release build (identity-state rule).
    pub fn next_id(&mut self) -> String {
        let id = self.next_request_id;
        self.next_request_id = id.checked_add(1).expect("request id counter overflowed u64");
        id.to_string()
    }

    /// Whether `id` is still the active decision (§4.4): every reply is checked against it before it is acted on.
    pub fn identity_active(&self, id: &DecisionIdentity) -> bool {
        lock(&self.identity).is_active(id)
    }

    /// Advances the reported stage to the worker stage `s`; never rewinds it. The watchdog reports the FURTHEST stage a
    /// request reached, so a `_min` retry that starts Building again must not rewind a stage the first attempt already
    /// reached (§7). `s` must be a worker stage (`building`, `solving`, `extracting`): anything else is a caller bug.
    pub fn set_stage(&self, s: &str) {
        assert!(stage_rank(s) > 0, "set_stage({s:?}): not a worker stage (building, solving, extracting)");
        let mut g = lock(&self.stage);
        if stage_rank(s) > stage_rank(&g) {
            *g = s.to_string();
        }
    }

    /// Starts the request being served at `s`, in its own stage slot; only `serve_request` calls this, as it starts a request.
    pub fn reset_stage(&self, s: &str) {
        *lock(&self.stage) = s.to_string();
    }

    pub fn stage(&self) -> String {
        lock(&self.stage).clone()
    }

    /// A snapshot of the session config; `serve_request` takes one per request so a mid-request change is ignored.
    pub fn config(&self) -> GameConfig {
        lock(&self.config).clone()
    }

    pub fn set_config(&self, cfg: GameConfig) {
        *lock(&self.config) = cfg;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeClock, FakeWorker};

    fn core() -> EngineCore {
        let clock = FakeClock::new();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let (worker, _state) = FakeWorker::scripted(clock.clone(), identity.clone(), vec![]);
        EngineCore::new(worker, clock, identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_core_unit_log")))
    }

    /// The reported stage only moves forward through the worker stages; `reset_stage` starts a new request.
    #[test]
    fn the_stage_only_advances_until_a_reset() {
        let c = core();
        assert_eq!(c.stage(), "fast");
        c.set_stage("solving");
        c.set_stage("building");
        assert_eq!(c.stage(), "solving", "a retry that builds again does not rewind the stage");
        c.set_stage("extracting");
        assert_eq!(c.stage(), "extracting");
        c.reset_stage("fast");
        c.set_stage("building");
        assert_eq!(c.stage(), "building");
    }

    #[test]
    #[should_panic(expected = "not a worker stage")]
    fn setting_a_stage_that_is_not_a_worker_stage_is_a_bug() {
        core().set_stage("fast");
    }

    /// Ids are decimal strings from 1, never reused; the active identity is read through the shared state.
    #[test]
    fn ids_count_up_and_identity_is_read_from_the_shared_state() {
        let mut c = core();
        assert_eq!((c.next_id(), c.next_id(), c.next_request_id), ("1".to_string(), "2".to_string(), 3));
        let id = { let mut s = c.identity.lock().unwrap(); s.begin_hand(); s.next_decision().unwrap() };
        assert!(c.identity_active(&id));
        c.identity.lock().unwrap().mutate();
        assert!(!c.identity_active(&id));
    }

    /// Task 27: `config()` is a snapshot (a later `set_config` never reaches a copy already taken) and the config handle
    /// is shared (a clone held by `Engine` sees what the core stores); the snapshot store starts empty.
    #[test]
    fn config_is_a_snapshot_and_the_config_handle_is_shared() {
        let c = core();
        let before = c.config();
        assert_eq!((before.config_revision, before.bb_chips, before.solver.target_bp, before.solver.flop_budget_s), (0, 10, 50, 10));
        let shared = c.config.clone();
        let mut next = before.clone();
        next.config_revision = 4;
        next.bb_chips = 20;
        c.set_config(next.clone());
        assert_eq!(before.config_revision, 0, "an earlier snapshot is never changed by set_config");
        assert_eq!(c.config(), next);
        assert_eq!(*shared.lock().unwrap(), next, "a clone of the handle sees the new config");
        assert!(c.snapshots.lock().unwrap().for_hand(1).is_empty());
    }

    #[test]
    #[should_panic(expected = "request id counter overflowed u64")]
    fn the_request_id_counter_never_wraps() {
        let mut c = core();
        c.next_request_id = u64::MAX;
        let _ = c.next_id();
    }
}
