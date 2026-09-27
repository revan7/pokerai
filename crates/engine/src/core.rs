//! The state `engine-main` owns (spec 3.4): the worker link, the clock every deadline is measured on, the decision
//! identity, the watchdog, the decision log, the request-id counter, the memory limit and the furthest stage the live
//! request has reached. Task 27 adds the snapshot store, the game config and the range source.

use crate::clock::Clock;
use crate::identity::IdentityState;
use crate::log::DecisionLog;
use crate::watchdog::Watchdog;
use crate::worker::link::WorkerLink;
use proto::DecisionIdentity;
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

/// State owned by `engine-main` (§3.4). Task 27 adds the snapshot store, the game config and the range source.
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

    #[test]
    #[should_panic(expected = "request id counter overflowed u64")]
    fn the_request_id_counter_never_wraps() {
        let mut c = core();
        c.next_request_id = u64::MAX;
        let _ = c.next_id();
    }
}
