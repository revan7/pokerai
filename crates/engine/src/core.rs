//! The state `engine-main` owns (spec 3.4): the worker link, the clock every deadline is measured on, the decision
//! identity, the watchdog, the decision log, the request-id counter, the memory limit and the furthest stage the live
//! request has reached, and (Task 27) the snapshot store, the game config and the range source.

use crate::clock::Clock;
use crate::identity::IdentityState;
use crate::log::DecisionLog;
use crate::ranges::{ExplicitRanges, RangeSource};
use crate::snapshots::SnapshotStore;
use crate::watchdog::Watchdog;
use crate::worker::link::WorkerLink;
use proto::{DecisionIdentity, GameConfig, Rake, SolverPrefs};
use std::sync::{Arc, Mutex};

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

/// State owned by `engine-main` (§3.4). The snapshot store, the game config and the range source (Task 27) are each
/// behind an `Arc<Mutex<_>>` shared with `Engine`, so a settings command or a hand mutation never waits for a running
/// request to release the `EngineCore` lock (§3.4: commands never block).
pub struct EngineCore {
    pub worker: Box<dyn WorkerLink>,
    pub clock: Arc<dyn Clock>,
    pub identity: Arc<Mutex<IdentityState>>,
    pub watchdog: Watchdog,
    pub log: DecisionLog,
    /// The id of the next request sent to the worker (`solve`, `cancel`, `lock`, `shutdown` all take one), a decimal
    /// string on the wire (§4.5). Never reused within this core.
    pub next_request_id: u64,
    pub memory_limit_bytes: u64,
    /// The furthest stage the live request has reached, shared with the watchdog (`watchdog::Armed::stage`), which
    /// reports it in `Unsupported{DeadlineExceeded{stage}}`.
    pub stage: Arc<Mutex<String>>,
    /// Shared with `Engine` so a mutation can invalidate snapshots without waiting for a running request.
    pub snapshots: Arc<Mutex<SnapshotStore>>,
    /// Read once, as a snapshot, at the start of each request.
    pub config: Arc<Mutex<GameConfig>>,
    /// The only root-range provider (`RangeSource::ranges_at_root`); plan 3 installs its replay-backed source here.
    pub range_source: Arc<Mutex<Box<dyn RangeSource>>>,
}

impl EngineCore {
    /// Takes the decision log from the start (Task 21), so this constructor never changes arity (cross-plan section 4).
    pub fn new(worker: Box<dyn WorkerLink>, clock: Arc<dyn Clock>, identity: Arc<Mutex<IdentityState>>, log: DecisionLog) -> Self {
        Self {
            watchdog: Watchdog::new(clock.clone()),
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
        }
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
        self.identity.lock().unwrap().is_active(id)
    }

    /// Advances the reported stage to the worker stage `s`; never rewinds it. The watchdog reports the FURTHEST stage a
    /// request reached, so a `_min` retry that starts Building again must not rewind a stage the first attempt already
    /// reached (§7). `s` must be a worker stage (`building`, `solving`, `extracting`): anything else is a caller bug.
    pub fn set_stage(&self, s: &str) {
        assert!(stage_rank(s) > 0, "set_stage({s:?}): not a worker stage (building, solving, extracting)");
        let mut g = self.stage.lock().unwrap();
        if stage_rank(s) > stage_rank(&g) {
            *g = s.to_string();
        }
    }

    /// Starts a new request at `s`; only `serve_request` calls this, at admission.
    pub fn reset_stage(&self, s: &str) {
        *self.stage.lock().unwrap() = s.to_string();
    }

    pub fn stage(&self) -> String {
        self.stage.lock().unwrap().clone()
    }

    /// A snapshot of the session config; `serve_request` takes one per request so a mid-request change is ignored.
    pub fn config(&self) -> GameConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn set_config(&self, cfg: GameConfig) {
        *self.config.lock().unwrap() = cfg;
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
