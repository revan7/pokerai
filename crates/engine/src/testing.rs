//! Deterministic test doubles for the engine (plan 2 Task 19): a driven clock, a scripted worker link, an event
//! recorder and a builder of valid solutions. Compiled for this crate's own tests or with the `testing` feature only
//! (`lib.rs`), never into a release consumer.
//!
//! Time. Nothing here reads or waits on wall time. `FakeClock` moves only when a test (or `FakeWorker`, whose waiting
//! is what moves it) moves it, only forward, and every move wakes every thread blocked in `Clock::wait_until` (the
//! watchdog, say) through one condition variable.
//!
//! The scripted worker. `FakeWorker` answers `WorkerLink` from a script of `FakeReply` items taken in order. The
//! script is the engine's timeline: worker output (`Ack`, `Progress`, `Result`), faulty lines (`Malformed`,
//! `Oversized`), the ways a worker process ends (`Eof`, `Exit`, `Hang`), a failed relaunch (`SpawnFails`), and two
//! events that are not worker output, `Delay` (fake time passing) and `InvalidateIdentity` (a mutation arriving).
//! It keeps the real link's contract (`worker::link`, `worker::process`), so a script can only produce what the
//! engine can meet in production, and every outcome of that contract can be scripted:
//! - `recv(timeout)` has one deadline, `now + timeout` on the fake clock, for the whole call. Waiting is moving the
//!   fake clock: a `Delay` runs from the moment a call first reaches it to its end, and a call whose deadline comes
//!   first stops there (the rest of the delay carries over to the next call). With nothing due before the deadline
//!   (a `Hang`, the end of the script) the call moves the clock to its deadline and returns `Ok(None)`. A reply due
//!   exactly at the deadline is left for the next call; one already due is returned even with a zero timeout.
//! - Each reply is written and read as the real wire line: serialized with the checked codecs, bounded by the 16 MiB
//!   result-line limit, decoded as the real link decodes it. A scripted reply that no worker could write panics.
//! - Drain before end: the end of the worker's output is reported after every reply scripted before it, and nothing
//!   scripted after it reaches the engine before a restart.
//! - `Eof` is an end of stdout whose exit is not confirmed: that call and every later `recv` spend their whole budget
//!   (the real link waits that long for the exit) and answer `Eof`, and `send` answers `Eof`, until the script
//!   confirms the exit with `Exit` or the worker is killed or restarted. An unconfirmed end is never taken for an exit.
//! - `Exit { code }` is a confirmed exit, answered at once and final: `recv` and `send` keep answering it at once.
//! - A `Hang` produces nothing and never ends on its own: only a kill or a restart ends it.
//! - `Malformed` and `Oversized` are one faulty line each; the link stays live (the caller restarts it, §12).
//! - `kill` is idempotent and leaves no live worker: `send` and `recv` answer `Eof` at once and `ready` is `None`
//!   until `restart`. A kill (and a restart, which kills first) ends the scripted process: when the script's next
//!   worker event is how that process would have ended (a `Hang`; while its output is live, also an `Eof` or `Exit`;
//!   once its output has ended, an `Exit`), that event is discarded with the `Delay`s leading to it. Everything else
//!   stays in the script for the restarted worker.
//! - `restart` is counted, then fails with `Spawn` (no live worker) when a `SpawnFails` is next in the script, and
//!   otherwise revives the link with `default_ready`.
//! - `send` refuses what the real link refuses (a request that does not serialize within the 1 MiB request-line
//!   limit) and records only what a live worker was given.
use crate::clock::Clock;
use crate::identity::IdentityState;
use crate::worker::link::{WorkerLink, WorkerLinkError};
use crate::worker::process::encode_request;
use crate::EventSink;
use proto::worker::{
    validate_solution, AckStatus, EngineMessage, NodeStrategy, Ready, ResultStatus, Stage, StreetSolution, WorkerError, WorkerMessage,
    ADAPTER_VERSION, PROTO_VERSION, RESULT_LINE_MAX, SOLVER_COMMIT,
};
use proto::{index_materialized, Action, EffectiveTree, RecommendationEvent, COMBOS};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// A lock that survives a panic elsewhere: the panicking test has already failed, and a poisoned double must not turn
/// every other thread's use of it (a watchdog emitting, a sink recording) into a second, misleading panic.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) }

/// The engine's `Clock` under a test's control, in milliseconds from 0. Monotonic: it never moves back.
pub struct FakeClock { now: Mutex<u64>, cv: Condvar }

impl FakeClock {
    pub fn new() -> Arc<FakeClock> { Arc::new(FakeClock { now: Mutex::new(0), cv: Condvar::new() }) }

    /// Moves the clock forward by `ms` and wakes every waiter.
    pub fn advance_ms(&self, ms: u64) {
        let mut now = lock(&self.now);
        let next = now.checked_add(ms).unwrap_or_else(|| panic!("FakeClock::advance_ms({ms}) overflows u64 from {}", *now));
        *now = next;
        self.cv.notify_all();
    }

    /// Sets the clock to `t_ms` and wakes every waiter. A `t_ms` before the current time panics: the clock is monotonic.
    pub fn set_ms(&self, t_ms: u64) {
        let mut now = lock(&self.now);
        assert!(t_ms >= *now, "FakeClock::set_ms({t_ms}) would move the clock back from {}: the clock is monotonic", *now);
        *now = t_ms;
        self.cv.notify_all();
    }

    /// Moves the clock to `t_ms` unless it is already there or past it (a test thread may drive it too).
    fn advance_to(&self, t_ms: u64) {
        let mut now = lock(&self.now);
        if t_ms > *now {
            *now = t_ms;
            self.cv.notify_all();
        }
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 { *lock(&self.now) }
    /// Blocks on the condition variable, never on wall time, until some other thread moves the clock to `t_ms`.
    fn wait_until(&self, t_ms: u64) {
        let mut now = lock(&self.now);
        while *now < t_ms {
            now = self.cv.wait(now).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

/// The id a scripted reply carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdRef {
    /// Resolved when the reply is delivered. In an `Ack`, the id of the last request sent (the worker acks every
    /// request, §4.5); in a `Progress` or `Result`, the id of the last `solve` sent (only a solve has them). Resolving
    /// it before any such request was sent is a broken script, and panics.
    Last,
    /// This id, e.g. a superseded request's.
    Fixed(String),
}

/// One item of a `FakeWorker` script (see the module doc for how `recv`, `kill` and `restart` read the script).
#[derive(Debug, Clone)]
pub enum FakeReply {
    /// An `ack` line (`replaced` is present, as `false`, exactly when the status is `staged`, §4.5).
    Ack { id: IdRef, status: AckStatus, reason: Option<String> },
    /// A `progress` line (`memory_bytes` 1).
    Progress { id: IdRef, stage: Stage, iterations: u32, exploitability_chips: Option<f32>, elapsed_ms: u32 },
    /// A `result` line.
    Result { id: IdRef, status: ResultStatus, solution: Option<StreetSolution>, error: Option<WorkerError>, elapsed_ms: u32 },
    /// `ms` of fake time pass with nothing from the worker. The delay starts when a `recv` first reaches it; a `recv`
    /// whose deadline comes first returns `Ok(None)` there and the rest carries over to the next call.
    Delay { ms: u64 },
    /// The worker's stdout ends without a confirmed exit: `Err(Eof)`, after the call's whole budget (see the module doc).
    Eof,
    /// One line that is not a message: `Err(Protocol(..))` with this text.
    Malformed(String),
    /// One line of this many bytes, over the limit: `Err(LineTooLong(n))`.
    Oversized(usize),
    /// Nothing, ever, until a kill or a restart: every `recv` spends its whole budget and returns `Ok(None)`.
    Hang,
    /// Simulates a mutation arriving while the solve is live: the active identity is cancelled.
    /// `recv` consumes it and continues to the next scripted item in the SAME call, so a script that wants the
    /// client to observe the invalidation before the next reply must write `InvalidateIdentity, Delay { ms: 1 }, ...`;
    /// the `Delay` makes `recv` return `Ok(None)` and the receive loop re-check the identity (see Task 28).
    /// Precisely: the first `Delay` a call reaches after an `InvalidateIdentity` ends that call with `Ok(None)` once it
    /// has run (or at the call's deadline, whichever comes first).
    InvalidateIdentity,
    /// The worker process has exited and the exit is confirmed: `Err(Exit { code })` at once, and again for every
    /// later `recv` and `send` until the worker is killed or restarted.
    Exit { code: i32 },
    /// The next `restart` fails to launch a worker: `Err(Spawn(reason))`, leaving no live worker. Only `restart`
    /// consumes it, and only as the next item of the script; a live worker has nothing to say in its place.
    SpawnFails(String),
}

/// What the fake worker was asked to do, shared with the test.
#[derive(Debug, Default)]
pub struct FakeState {
    /// Every request a live worker was given, in order; a `send` the link refused is not recorded.
    pub sent: Vec<EngineMessage>,
    /// `kill` calls. A `restart` kills too, but is counted in `restarts` only.
    pub kills: u32,
    /// `restart` calls, failed ones included.
    pub restarts: u32,
    /// The id of the last `solve` sent.
    pub last_solve_id: Option<String>,
    /// The `target` of every `cancel` sent, in order.
    pub cancels: Vec<String>,
}

/// Where the scripted worker process stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proc {
    /// Running: its output is read from the script.
    Live,
    /// Its stdout has ended (`Eof`) and its exit is not confirmed.
    Ended,
    /// Its exit is confirmed with this code; final until a kill or a restart.
    Exited(i32),
    /// No live worker: killed, or its relaunch failed.
    Gone,
}

/// A scripted `WorkerLink` (see the module doc).
pub struct FakeWorker {
    script: VecDeque<FakeReply>,
    /// When the `Delay` at the front of the script ends, once a `recv` has reached it.
    delay_until: Option<u64>,
    proc: Proc,
    ready: Option<Ready>,
    state: Arc<Mutex<FakeState>>,
    clock: Arc<FakeClock>,
    identity: Arc<Mutex<IdentityState>>,
}

impl FakeWorker {
    /// A `ready` that `validate_ready` accepts for a 16-thread worker.
    pub fn default_ready() -> Ready {
        Ready { proto_version: PROTO_VERSION, solver_commit: SOLVER_COMMIT.into(), adapter_version: ADAPTER_VERSION, threads: 16,
            build_features: vec!["avx2".into()], cpu_features: vec!["avx2".into()],
            capabilities: vec!["solve".into(), "lock".into(), "cancel".into(), "street_export".into(), "i16".into()] }
    }

    /// A live worker that has written `default_ready`, answering from `script`; the state is shared with the test.
    pub fn scripted(clock: Arc<FakeClock>, identity: Arc<Mutex<IdentityState>>, script: Vec<FakeReply>) -> (Box<FakeWorker>, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let w = FakeWorker { script: script.into(), delay_until: None, proc: Proc::Live, ready: Some(Self::default_ready()), state: state.clone(), clock, identity };
        (Box::new(w), state)
    }

    /// `IdRef::Last` resolved against what has been sent so far (see `IdRef`).
    fn resolve(&self, id: IdRef, reply: &str, of_solve: bool) -> String {
        match id {
            IdRef::Fixed(id) => id,
            IdRef::Last => {
                let s = lock(&self.state);
                let last = if of_solve { s.last_solve_id.clone() } else { s.sent.last().map(|m| request_id(m).to_string()) };
                last.unwrap_or_else(|| panic!("FakeReply::{reply} with IdRef::Last: no {} has been sent, so there is no id to answer",
                    if of_solve { "solve" } else { "request" }))
            }
        }
    }

    /// Spends the rest of the call's budget: the fake clock moves to the deadline. A call whose deadline the fake clock
    /// can never reach would wait forever, which a test can only mean by mistake.
    fn wait_out(&self, deadline: Option<u64>, why: &str) {
        let deadline = deadline.unwrap_or_else(|| panic!("FakeWorker::recv: {why}, and the timeout is too large for any fake-clock deadline: the call would never return"));
        self.clock.advance_to(deadline);
    }

    /// A call that ends with nothing delivered: `Ok(None)` while the worker's output is live; once it has ended, the
    /// unconfirmed `Eof` (the real link then waits for the exit, not for a line, and reports that the wait failed).
    fn nothing_by_the_deadline(&self) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        if self.proc == Proc::Ended { Err(WorkerLinkError::Eof) } else { Ok(None) }
    }

    /// The next worker event while its output is live.
    fn live_output(&mut self, deadline: Option<u64>) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        if matches!(self.script.front(), None | Some(FakeReply::Hang | FakeReply::SpawnFails(_))) {
            self.wait_out(deadline, "the worker has nothing more to say (a hang, or the end of the script)");
            return Ok(None);
        }
        let item = self.script.pop_front().expect("the script has a front item");
        let msg = match item {
            FakeReply::Ack { id, status, reason } => {
                let replaced = (status == AckStatus::Staged).then_some(false);
                WorkerMessage::Ack { id: self.resolve(id, "Ack", false), status, reason, replaced }
            }
            FakeReply::Progress { id, stage, iterations, exploitability_chips, elapsed_ms } =>
                WorkerMessage::Progress { id: self.resolve(id, "Progress", true), stage, iterations, exploitability_chips, elapsed_ms, memory_bytes: 1 },
            FakeReply::Result { id, status, solution, error, elapsed_ms } =>
                WorkerMessage::Result { id: self.resolve(id, "Result", true), status, elapsed_ms, solution, error },
            FakeReply::Malformed(text) => return Err(WorkerLinkError::Protocol(text)),
            FakeReply::Oversized(len) => return Err(WorkerLinkError::LineTooLong(len)),
            FakeReply::Eof => {
                self.proc = Proc::Ended;
                self.wait_out(deadline, "the worker's stdout ended and its exit is not confirmed");
                return Err(WorkerLinkError::Eof);
            }
            FakeReply::Exit { code } => {
                self.proc = Proc::Exited(code);
                return Err(WorkerLinkError::Exit { code });
            }
            FakeReply::Delay { .. } | FakeReply::InvalidateIdentity | FakeReply::Hang | FakeReply::SpawnFails(_) => unreachable!("handled before the output"),
        };
        Ok(Some(on_the_wire(msg)))
    }

    /// The next event once the worker's stdout has ended: the scripted confirmation of its exit, or else `Eof` after
    /// the whole budget. Nothing else in the script is this process's; it waits for the restarted worker.
    fn ended_output(&mut self, deadline: Option<u64>) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        if let Some(FakeReply::Exit { code }) = self.script.front() {
            let code = *code;
            self.script.pop_front();
            self.proc = Proc::Exited(code);
            return Err(WorkerLinkError::Exit { code });
        }
        self.wait_out(deadline, "the worker's stdout has ended and its exit is not confirmed");
        Err(WorkerLinkError::Eof)
    }

    /// The kill of the scripted process (see the module doc): its pending end, and the `Delay`s leading to it, go with
    /// it. Leaves no live worker.
    fn end_process(&mut self) {
        let ends: fn(&FakeReply) -> bool = match self.proc {
            Proc::Live => |r| matches!(r, FakeReply::Hang | FakeReply::Eof | FakeReply::Exit { .. }),
            Proc::Ended => |r| matches!(r, FakeReply::Hang | FakeReply::Exit { .. }),
            Proc::Exited(_) | Proc::Gone => |_| false,
        };
        let lead = self.script.iter().take_while(|r| matches!(r, FakeReply::Delay { .. })).count();
        if self.script.get(lead).is_some_and(ends) {
            self.script.drain(..=lead);
            self.delay_until = None;
        }
        self.proc = Proc::Gone;
        self.ready = None;
    }
}

/// The id every request carries.
fn request_id(m: &EngineMessage) -> &str {
    match m {
        EngineMessage::Solve(r) => &r.id,
        EngineMessage::Lock { id, .. } | EngineMessage::Cancel { id, .. } | EngineMessage::Shutdown { id } => id,
    }
}

/// `msg` as the real link would hand it over: written as one wire line with the checked codecs, within the result-line
/// limit counted with its LF, and decoded back. A reply that fails any of that is one no worker writes (the codecs
/// refuse its values, or the worker truncates the export, §4.5), so scripting it is a broken test.
fn on_the_wire(msg: WorkerMessage) -> WorkerMessage {
    let line = serde_json::to_string(&msg).unwrap_or_else(|e| panic!("a scripted reply cannot be written as a worker line: {e}"));
    let len = line.len() + 1;
    assert!(len <= RESULT_LINE_MAX, "a scripted reply is a {len}-byte line, over the {RESULT_LINE_MAX}-byte result-line limit: no worker writes one (script `Oversized` for that)");
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("a scripted reply does not decode as the line it was written as: {e}"))
}

impl WorkerLink for FakeWorker {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> {
        encode_request(msg)?; // the real link's check, before anything else: a request it cannot write is refused
        match self.proc {
            Proc::Live => {}
            Proc::Exited(code) => return Err(WorkerLinkError::Exit { code }),
            // A worker whose output has ended takes no more requests; with no live worker there is nobody to take them.
            Proc::Ended | Proc::Gone => return Err(WorkerLinkError::Eof),
        }
        let mut s = lock(&self.state);
        match msg {
            EngineMessage::Solve(r) => s.last_solve_id = Some(r.id.clone()),
            EngineMessage::Cancel { target, .. } => s.cancels.push(target.clone()),
            EngineMessage::Lock { .. } | EngineMessage::Shutdown { .. } => {}
        }
        s.sent.push(msg.clone());
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        match self.proc {
            Proc::Gone => return Err(WorkerLinkError::Eof),
            Proc::Exited(code) => return Err(WorkerLinkError::Exit { code }),
            Proc::Live | Proc::Ended => {}
        }
        // One deadline for the whole call, in whole milliseconds; `None` when no fake-clock reading can reach it.
        let deadline = u64::try_from(timeout.as_millis()).ok().and_then(|ms| self.clock.now_ms().checked_add(ms));
        let mut invalidated = false;
        // The timeline events ahead of the worker's next output: time passing, mutations arriving.
        loop {
            match self.script.front() {
                Some(FakeReply::Delay { ms }) => {
                    let now = self.clock.now_ms();
                    let ms = *ms;
                    let until = *self.delay_until.get_or_insert_with(|| {
                        now.checked_add(ms).unwrap_or_else(|| panic!("FakeReply::Delay {{ ms: {ms} }} from {now} overflows the fake clock"))
                    });
                    if let Some(d) = deadline.filter(|d| *d < until) {
                        self.clock.advance_to(d); // the rest of the delay carries over to the next call
                        return self.nothing_by_the_deadline();
                    }
                    self.clock.advance_to(until);
                    self.script.pop_front();
                    self.delay_until = None;
                    // A live worker's receive loop re-checks the identity here (see `InvalidateIdentity`; an ended one
                    // is answered `Eof` after the whole budget regardless); a reply due exactly at the deadline is the
                    // next call's.
                    let woken = invalidated && self.proc == Proc::Live;
                    if woken || (until > now && Some(until) == deadline) { return self.nothing_by_the_deadline(); }
                }
                Some(FakeReply::InvalidateIdentity) => {
                    lock(&self.identity).cancel_active();
                    self.script.pop_front();
                    invalidated = true;
                }
                _ => break,
            }
        }
        match self.proc {
            Proc::Live => self.live_output(deadline),
            Proc::Ended => self.ended_output(deadline),
            Proc::Exited(_) | Proc::Gone => unreachable!("answered before the timeline"),
        }
    }

    fn restart(&mut self) -> Result<(), WorkerLinkError> {
        self.end_process();
        lock(&self.state).restarts += 1;
        if let Some(FakeReply::SpawnFails(_)) = self.script.front() {
            let Some(FakeReply::SpawnFails(reason)) = self.script.pop_front() else { unreachable!("the front was a SpawnFails") };
            return Err(WorkerLinkError::Spawn(reason));
        }
        self.proc = Proc::Live;
        self.ready = Some(Self::default_ready());
        Ok(())
    }

    fn kill(&mut self) {
        lock(&self.state).kills += 1;
        self.end_process();
    }

    fn ready(&self) -> Option<&Ready> { self.ready.as_ref() }
}

/// One event as a `RecordingSink` saw it: when (fake clock) and how many kills the fake worker had had by then.
#[derive(Debug, Clone)]
pub struct Recorded { pub at_ms: u64, pub kills: u32, pub event: RecommendationEvent }

/// An `EventSink` that records every event with the fake time and the fake worker's kill count at emission.
pub struct RecordingSink { clock: Arc<dyn Clock>, state: Option<Arc<Mutex<FakeState>>>, pub events: Arc<Mutex<Vec<Recorded>>> }

impl RecordingSink {
    /// The sink and a handle on what it records; `state` is the fake worker's, when kill counts matter.
    pub fn new(clock: Arc<dyn Clock>, state: Option<Arc<Mutex<FakeState>>>) -> (RecordingSink, Arc<Mutex<Vec<Recorded>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (RecordingSink { clock, state, events: events.clone() }, events)
    }
}

impl EventSink for RecordingSink {
    fn emit(&mut self, ev: RecommendationEvent) {
        let kills = self.state.as_ref().map_or(0, |s| lock(s).kills);
        let at_ms = self.clock.now_ms();
        lock(&self.events).push(Recorded { at_ms, kills, event: ev });
    }
}

/// A valid `StreetSolution` for every decision node of the tree's root street, in materialized order: all combos
/// available, uniform probabilities, EV = the action's index in chips (a fold's is exactly 0, §2), `requested` the node
/// whose chip path is `requested`. Panics (always) when `requested` is not such a node, and when the solution is not
/// one `validate_solution` accepts against `tree` (a bad tree, or a negative or non-finite `exploitability_chips`).
pub fn uniform_solution(tree: &EffectiveTree, requested: &[Action], exploitability_chips: f32) -> StreetSolution {
    let index = index_materialized(&tree.materialized);
    let mut nodes = Vec::new();
    for n in tree.materialized.iter().filter(|n| n.street == tree.root_street) {
        let path: Vec<Action> = (0..n.path.len()).map(|k| {
            let parent = index.get(&n.path[..k]).unwrap_or_else(|| panic!("uniform_solution: node {:?} has no materialized parent at {:?}", n.path, &n.path[..k]));
            *parent.actions.get(usize::from(n.path[k])).unwrap_or_else(|| panic!("uniform_solution: node {:?} names action {} of a {}-action parent", n.path, n.path[k], parent.actions.len()))
        }).collect();
        let width = n.actions.len();
        let ev: Vec<f32> = n.actions.iter().enumerate().map(|(i, a)| if *a == Action::Fold { 0.0 } else { i as f32 }).collect();
        nodes.push(NodeStrategy { path, actor: n.actor.clone(), actions: n.actions.clone(), probs: vec![vec![1.0 / width as f32; width]; COMBOS],
            ev_chips: vec![ev; COMBOS], available: vec![true; COMBOS] });
    }
    let requested_index = nodes.iter().position(|n| n.path == requested)
        .unwrap_or_else(|| panic!("uniform_solution: {requested:?} is not the chip path of a {:?} decision node of the tree", tree.root_street));
    let covered_paths = nodes.iter().map(|n| n.path.clone()).collect();
    let solution = StreetSolution { nodes, requested: u32::try_from(requested_index).expect("a node index fits in u32"), exploitability_chips, iterations: 50,
        memory_bytes: 1 << 20, mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths };
    if let Err(e) = validate_solution(&solution, &tree.materialized) { panic!("uniform_solution built a solution the engine refuses: {e}"); }
    solution
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::worker::{AckStatus, EngineMessage};
    use crate::tree::{build_tree_full, TemplateSelection};
    use proto::worker::{NodeLock, SolveRequest, REQUEST_LINE_MAX};
    use proto::{Card, DecisionIdentity, MaterializedNode, Range1326, Seat, Street, StreetRootSnapshot};
    #[test]
    fn fake_worker_advances_the_fake_clock_and_resolves_ids() {
        let clock = FakeClock::new();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let (mut w, state) = FakeWorker::scripted(clock.clone(), identity, vec![
            FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None }, FakeReply::Delay { ms: 700 }, FakeReply::Hang]);
        w.send(&EngineMessage::Cancel { id: "9".into(), target: "x".into() }).unwrap();
        w.send(&EngineMessage::Shutdown { id: "7".into() }).unwrap();
        assert_eq!(state.lock().unwrap().sent.len(), 2);
        assert!(matches!(w.recv(Duration::from_millis(100)).unwrap(), Some(WorkerMessage::Ack { .. })));
        assert!(w.recv(Duration::from_millis(500)).unwrap().is_none());
        assert_eq!(clock.now_ms(), 500);
        assert!(w.recv(Duration::from_millis(500)).unwrap().is_none());     // 200 ms of delay, then the hang eats the rest
        assert_eq!(clock.now_ms(), 1000);
        w.kill();
        assert_eq!(state.lock().unwrap().kills, 1);
    }

    type Rig = (Arc<FakeClock>, Arc<Mutex<IdentityState>>, Box<FakeWorker>, Arc<Mutex<FakeState>>);
    fn rig(script: Vec<FakeReply>) -> Rig {
        let clock = FakeClock::new();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let (w, state) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
        (clock, identity, w, state)
    }
    fn ms(n: u64) -> Duration { Duration::from_millis(n) }
    /// One lock for both counts (two `lock()`s in one expression would deadlock on the first's temporary guard).
    fn kills_and_restarts(state: &Arc<Mutex<FakeState>>) -> (u32, u32) { let s = state.lock().unwrap(); (s.kills, s.restarts) }
    fn ack(id: &str) -> FakeReply { FakeReply::Ack { id: IdRef::Fixed(id.into()), status: AckStatus::Accepted, reason: None } }
    /// The id of a delivered `ack`, or a panic naming what was delivered instead.
    fn acked(got: Result<Option<WorkerMessage>, WorkerLinkError>) -> String {
        match got { Ok(Some(WorkerMessage::Ack { id, .. })) => id, other => panic!("expected an ack, got {other:?}") }
    }
    fn root(street: Street) -> StreetRootSnapshot {
        let board = if street == Street::River { "Qs Jd 7h 3c 2d" } else { "Qs Jd 7h 3c" };
        StreetRootSnapshot { street, board: board.split(' ').map(|c| Card::parse(c).unwrap()).collect(), oop: Seat(2), ip: Seat(0), pot_root: 100,
            stack_oop_root: 400, stack_ip_root: 400, dead_this_street: 0, projected_from: 2, history: vec![], bb_chips: 2 }
    }
    fn tree(street: Street, template: &str) -> EffectiveTree { build_tree_full(&root(street), &TemplateSelection::from_history(template, &[])).unwrap().tree }
    /// A request the real link accepts: it serializes within the request-line limit.
    fn solve(id: &str) -> EngineMessage {
        let r = root(Street::River);
        EngineMessage::Solve(SolveRequest { id: id.into(), spot: "s".repeat(64), board: r.board, oop_range: Range1326([0.0; COMBOS]), ip_range: Range1326([0.0; COMBOS]),
            pot: 100, stack_oop: 400, stack_ip: 400, rake_rate: 0.0, rake_cap_mchips: 0, tree: tree(Street::River, "river_std_v1"), history: vec![], target_bp: 50,
            deadline_ms: 1_850, extraction_margin_ms: 200, memory_limit_bytes: 10 << 30, background: false })
    }

    /// The clock only moves forward, and a thread blocked in `wait_until` is woken by a move, never by wall time.
    #[test]
    fn the_fake_clock_is_monotonic_and_wakes_its_waiters() {
        let clock = FakeClock::new();
        clock.advance_ms(5);
        clock.set_ms(5);
        assert_eq!(clock.now_ms(), 5);
        let waiter = { let c = clock.clone(); std::thread::spawn(move || { c.wait_until(100); c.now_ms() }) };
        clock.set_ms(40);
        clock.advance_ms(60);
        assert_eq!(waiter.join().unwrap(), 100);
        clock.wait_until(50); // already past: returns at once
    }

    #[test]
    #[should_panic(expected = "the clock is monotonic")]
    fn the_fake_clock_never_moves_back() {
        let clock = FakeClock::new();
        clock.set_ms(10);
        clock.set_ms(9);
    }

    /// A thread waiting on the fake clock (a watchdog) wakes when the fake worker's waiting moves the clock.
    #[test]
    fn a_thread_waiting_on_the_clock_wakes_when_the_fake_worker_spends_time() {
        let (clock, _identity, mut w, _state) = rig(vec![FakeReply::Hang]);
        let waiter = { let c = clock.clone(); std::thread::spawn(move || { c.wait_until(2_500); c.now_ms() }) };
        assert!(w.recv(ms(2_500)).unwrap().is_none());
        assert_eq!(waiter.join().unwrap(), 2_500);
    }

    /// One deadline per call on the fake clock: a delay runs from when a call first reaches it and carries over past a
    /// deadline; a reply after it arrives within the same call; a reply due exactly at the deadline is the next call's;
    /// one already due is returned even with a zero timeout; time the test moves counts toward a delay.
    #[test]
    fn recv_measures_every_wait_against_the_fake_clock() {
        let (clock, _identity, mut w, _state) = rig(vec![FakeReply::Delay { ms: 300 }, ack("a"), FakeReply::Delay { ms: 200 }, ack("b"), ack("c"),
            FakeReply::Delay { ms: 100 }, ack("d")]);
        assert!(w.recv(ms(100)).unwrap().is_none());
        assert_eq!(clock.now_ms(), 100);
        assert_eq!(acked(w.recv(ms(1_000))), "a");
        assert_eq!(clock.now_ms(), 300, "the reply arrives when the delay ends, within the call");
        assert!(w.recv(ms(200)).unwrap().is_none(), "due exactly at the deadline: left for the next call");
        assert_eq!(clock.now_ms(), 500);
        assert_eq!(acked(w.recv(ms(0))), "b");
        assert_eq!(acked(w.recv(ms(0))), "c");
        assert!(w.recv(ms(10)).unwrap().is_none(), "the 100 ms delay starts at 500");
        clock.set_ms(900);
        assert_eq!(acked(w.recv(ms(0))), "d", "the delay ended at 600, while the test moved the clock");
        assert_eq!(clock.now_ms(), 900);
        assert!(w.recv(ms(250)).unwrap().is_none(), "past the end of the script the worker is silent");
        assert_eq!(clock.now_ms(), 1_150);
    }

    #[test]
    #[should_panic(expected = "would never return")]
    fn an_unbounded_receive_from_a_hung_worker_panics_instead_of_never_returning() {
        let (_clock, _identity, mut w, _state) = rig(vec![FakeReply::Hang]);
        let _ = w.recv(Duration::MAX);
    }

    /// `InvalidateIdentity` cancels the active decision (`IdentityState::cancel_active`) and the call goes on to the
    /// next reply; followed by a `Delay`, the call ends with `Ok(None)` when the delay has run, before the next reply.
    #[test]
    fn invalidate_identity_cancels_the_active_decision_and_a_delay_after_it_ends_the_call() {
        let (clock, identity, mut w, _state) = rig(vec![FakeReply::InvalidateIdentity, ack("r1"), FakeReply::InvalidateIdentity, FakeReply::Delay { ms: 1 }, ack("r2")]);
        let first = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
        let revision = identity.lock().unwrap().current_revision();
        assert_eq!(acked(w.recv(ms(1_000))), "r1", "consumed within the call that reached it");
        assert!(!identity.lock().unwrap().is_active(&first));
        assert_eq!(identity.lock().unwrap().current_revision(), revision, "a cancel, not a mutation");
        let second = identity.lock().unwrap().next_decision().unwrap();
        assert!(w.recv(ms(1_000)).unwrap().is_none(), "the delay after the invalidation ends the call");
        assert_eq!(clock.now_ms(), 1);
        assert!(!identity.lock().unwrap().is_active(&second));
        assert_eq!(acked(w.recv(ms(1_000))), "r2");
    }

    /// `IdRef::Last` is the last request in an `ack` (every request is acked) and the last solve in a `progress` or a
    /// `result`; `Fixed` passes through. A `staged` ack carries `replaced`, any other does not (§4.5).
    #[test]
    fn ids_resolve_against_what_was_sent() {
        let error = WorkerError { code: "internal".into(), message: "boom".into(), retryable: true, estimate_bytes: None };
        let (_clock, _identity, mut w, state) = rig(vec![
            FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None },
            FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 3, exploitability_chips: Some(0.5), elapsed_ms: 4 },
            FakeReply::Result { id: IdRef::Last, status: ResultStatus::Error, solution: None, error: Some(error.clone()), elapsed_ms: 5 },
            FakeReply::Ack { id: IdRef::Fixed("old".into()), status: AckStatus::Staged, reason: None }]);
        w.send(&solve("s1")).unwrap();
        w.send(&EngineMessage::Cancel { id: "c2".into(), target: "s1".into() }).unwrap();
        match w.recv(ms(0)).unwrap() { Some(WorkerMessage::Ack { id, replaced: None, .. }) => assert_eq!(id, "c2"), other => panic!("{other:?}") }
        match w.recv(ms(0)).unwrap() { Some(WorkerMessage::Progress { id, iterations: 3, memory_bytes: 1, .. }) => assert_eq!(id, "s1"), other => panic!("{other:?}") }
        match w.recv(ms(0)).unwrap() { Some(WorkerMessage::Result { id, status: ResultStatus::Error, error: Some(e), .. }) => assert_eq!((id.as_str(), e), ("s1", error)), other => panic!("{other:?}") }
        match w.recv(ms(0)).unwrap() { Some(WorkerMessage::Ack { id, replaced: Some(false), .. }) => assert_eq!(id, "old"), other => panic!("{other:?}") }
        let s = state.lock().unwrap();
        assert_eq!((s.last_solve_id.as_deref(), s.cancels.clone(), s.sent.len()), (Some("s1"), vec!["s1".to_string()], 2));
    }

    #[test]
    #[should_panic(expected = "no solve has been sent")]
    fn a_progress_for_the_last_solve_before_any_solve_panics() {
        let (_clock, _identity, mut w, _state) = rig(vec![FakeReply::Progress { id: IdRef::Last, stage: Stage::Building, iterations: 0, exploitability_chips: None, elapsed_ms: 1 }]);
        w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap();
        let _ = w.recv(ms(10));
    }

    #[test]
    #[should_panic(expected = "no request has been sent")]
    fn an_ack_for_the_last_request_before_any_request_panics() {
        let (_clock, _identity, mut w, _state) = rig(vec![FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None }]);
        let _ = w.recv(ms(10));
    }

    /// A reply is what the real link would decode from the worker's line; one no worker can write is a broken script.
    #[test]
    #[should_panic(expected = "cannot be written as a worker line")]
    fn a_reply_no_worker_can_write_panics() {
        let (_clock, _identity, mut w, _state) = rig(vec![FakeReply::Progress { id: IdRef::Fixed("1".into()), stage: Stage::Solving, iterations: 1,
            exploitability_chips: Some(f32::NAN), elapsed_ms: 1 }]);
        let _ = w.recv(ms(10));
    }

    /// A faulty line is one error, and the link stays live: the next reply and the next request go through.
    #[test]
    fn faulty_lines_are_single_errors_and_the_link_stays_live() {
        let (_clock, _identity, mut w, _state) = rig(vec![FakeReply::Malformed("not json".into()), FakeReply::Oversized(RESULT_LINE_MAX + 1), ack("ok")]);
        assert!(matches!(w.recv(ms(10)), Err(WorkerLinkError::Protocol(m)) if m == "not json"));
        assert!(matches!(w.recv(ms(10)), Err(WorkerLinkError::LineTooLong(n)) if n == RESULT_LINE_MAX + 1));
        assert_eq!(acked(w.recv(ms(10))), "ok");
        w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap();
    }

    /// Drain before end, and the two ends: an unconfirmed `Eof` spends each call's whole budget, keeps nothing after it
    /// from the engine, and is not final; the scripted `Exit` confirms it, at once and for good, until a restart.
    #[test]
    fn an_unconfirmed_end_is_not_final_and_a_confirmed_exit_is() {
        let (clock, _identity, mut w, state) = rig(vec![ack("before"), FakeReply::Eof, FakeReply::Delay { ms: 300 }, FakeReply::Exit { code: 3 }, ack("next worker")]);
        assert_eq!(acked(w.recv(ms(50))), "before", "every line before the end comes first");
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)));
        assert_eq!(clock.now_ms(), 100, "the exit was waited for, for the whole budget");
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "1".into() }), Err(WorkerLinkError::Eof)));
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)), "still unconfirmed at 200: the exit comes at 400");
        assert_eq!(clock.now_ms(), 200);
        assert!(matches!(w.recv(ms(500)), Err(WorkerLinkError::Exit { code: 3 })), "confirmed within the budget");
        assert_eq!(clock.now_ms(), 400);
        assert!(matches!(w.recv(ms(500)), Err(WorkerLinkError::Exit { code: 3 })), "final, and at once");
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "2".into() }), Err(WorkerLinkError::Exit { code: 3 })));
        assert_eq!(clock.now_ms(), 400);
        assert!(w.ready().is_some() && state.lock().unwrap().sent.is_empty(), "ready lasts until a kill; nothing was given to the worker");
        w.restart().unwrap();
        assert_eq!(acked(w.recv(ms(10))), "next worker");
    }

    /// `kill` is idempotent and leaves no live worker until `restart`; it ends a hang and the silence leading into it,
    /// but a scripted reply is never discarded (its remaining delay still runs).
    #[test]
    fn a_kill_leaves_no_live_worker_and_ends_a_hang_but_no_reply() {
        let (clock, _identity, mut w, state) = rig(vec![ack("a"), FakeReply::Delay { ms: 500 }, FakeReply::Hang, ack("b"), FakeReply::Delay { ms: 50 }, FakeReply::Eof,
            ack("c"), FakeReply::Delay { ms: 500 }, ack("d")]);
        assert_eq!(acked(w.recv(ms(100))), "a");
        assert!(w.recv(ms(100)).unwrap().is_none());
        w.kill();
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "1".into() }), Err(WorkerLinkError::Eof)));
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)));
        assert!(w.ready().is_none());
        assert_eq!(clock.now_ms(), 100, "with no live worker nothing is waited for");
        w.kill();
        assert_eq!(kills_and_restarts(&state), (2, 0));
        w.restart().unwrap();
        assert!(w.ready().is_some());
        assert_eq!(acked(w.recv(ms(1_000))), "b", "the hang went with the killed worker");
        assert_eq!(clock.now_ms(), 100);
        w.restart().unwrap(); // a restart kills first: the pending delay and end go with that worker too
        assert_eq!(acked(w.recv(ms(1_000))), "c");
        assert!(w.recv(ms(100)).unwrap().is_none());
        w.kill();
        w.restart().unwrap();
        assert_eq!(acked(w.recv(ms(1_000))), "d");
        assert_eq!(clock.now_ms(), 600, "a kill does not cut a delay that leads to a reply");
        assert_eq!(kills_and_restarts(&state), (3, 3));
    }

    /// Plan 2 Task 23's heartbeat shape: progress, then silence into a hang; the restart ends that worker, and the
    /// retry's replies arrive at once.
    #[test]
    fn a_restart_ends_a_hung_worker_so_the_retry_is_answered() {
        let (clock, _identity, mut w, state) = rig(vec![ack("1"), FakeReply::Delay { ms: 1_200 },
            FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 1, exploitability_chips: None, elapsed_ms: 1 },
            FakeReply::Delay { ms: 5_100 }, FakeReply::Hang, FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None }, ack("2")]);
        w.send(&solve("1")).unwrap();
        assert_eq!(acked(w.recv(ms(6_500))), "1");
        assert!(matches!(w.recv(ms(6_500)), Ok(Some(WorkerMessage::Progress { .. }))));
        assert!(w.recv(ms(5_000)).unwrap().is_none());
        assert_eq!(clock.now_ms(), 6_200);
        w.restart().unwrap();
        w.send(&solve("r")).unwrap();
        assert_eq!(acked(w.recv(ms(8_650))), "r");
        assert_eq!(acked(w.recv(ms(8_650))), "2");
        assert_eq!((clock.now_ms(), kills_and_restarts(&state)), (6_200, (0, 1)));
    }

    /// A relaunch that fails leaves no live worker (`Spawn`), and a later restart can succeed; a live worker has nothing
    /// to say in place of a scripted failed relaunch.
    #[test]
    fn a_failed_relaunch_leaves_no_live_worker() {
        let (_clock, _identity, mut w, state) = rig(vec![FakeReply::SpawnFails("no binary".into()), ack("a")]);
        assert!(w.recv(ms(10)).unwrap().is_none());
        assert!(matches!(w.restart(), Err(WorkerLinkError::Spawn(m)) if m == "no binary"));
        assert!(w.ready().is_none());
        assert!(matches!(w.recv(ms(10)), Err(WorkerLinkError::Eof)));
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "1".into() }), Err(WorkerLinkError::Eof)));
        w.restart().unwrap();
        assert_eq!(acked(w.recv(ms(10))), "a");
        assert_eq!(state.lock().unwrap().restarts, 2);
    }

    /// `send` refuses exactly what the real link refuses, and records only what a live worker was given.
    #[test]
    fn send_refuses_what_the_real_link_refuses_and_records_what_the_worker_got() {
        let (_clock, _identity, mut w, state) = rig(vec![]);
        let base = encode_request(&EngineMessage::Shutdown { id: String::new() }).unwrap().len();
        let over = EngineMessage::Shutdown { id: "1".repeat(REQUEST_LINE_MAX - base + 1) };
        assert!(matches!(w.send(&over), Err(WorkerLinkError::LineTooLong(n)) if n == REQUEST_LINE_MAX + 1));
        let nan = EngineMessage::Lock { id: "2".into(), spot: "s".into(), locks: vec![NodeLock { path: vec![], actor: "oop".into(), probs: vec![vec![f32::NAN]] }] };
        assert!(matches!(w.send(&nan), Err(WorkerLinkError::Protocol(_))));
        assert!(state.lock().unwrap().sent.is_empty(), "nothing refused is recorded");
        w.send(&solve("3")).unwrap();
        w.send(&EngineMessage::Cancel { id: "4".into(), target: "3".into() }).unwrap();
        let s = state.lock().unwrap();
        assert_eq!(s.sent.iter().map(request_id).collect::<Vec<_>>(), ["3", "4"]);
        assert_eq!((s.last_solve_id.as_deref(), s.cancels.as_slice()), (Some("3"), ["3".to_string()].as_slice()));
    }

    /// Every event is stamped with the fake time and the fake worker's kill count when it was emitted.
    #[test]
    fn the_recording_sink_stamps_fake_time_and_kills() {
        let (clock, _identity, mut w, state) = rig(vec![]);
        let identity = DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 0, model_revision: 0 };
        let event = |reason: &str| RecommendationEvent::NoDecision { identity: identity.clone(), reason: reason.into() };
        let (mut sink, events) = RecordingSink::new(clock.clone(), Some(state.clone()));
        clock.set_ms(40);
        sink.emit(event("a"));
        w.kill();
        clock.advance_ms(2);
        sink.emit(event("b"));
        let (mut bare, bare_events) = RecordingSink::new(clock.clone(), None);
        bare.emit(event("c"));
        let got: Vec<(u64, u32)> = events.lock().unwrap().iter().map(|r| (r.at_ms, r.kills)).collect();
        assert_eq!(got, [(40, 0), (42, 1)]);
        assert_eq!(events.lock().unwrap()[1].event, event("b"));
        assert_eq!(bare_events.lock().unwrap().iter().map(|r| (r.at_ms, r.kills)).collect::<Vec<_>>(), [(42, 0)]);
        assert!(Arc::ptr_eq(&sink.events, &events));
    }

    /// The builder's solutions pass the engine's own validation, cover exactly the root street's decision nodes, name
    /// the requested node, and keep a fold's EV at exactly 0.
    #[test]
    fn uniform_solution_is_valid_for_the_root_street_and_names_the_requested_node() {
        for (street, template) in [(Street::River, "river_std_v1"), (Street::Turn, "turn_std_v1")] {
            let t = tree(street, template);
            let on_street: Vec<&MaterializedNode> = t.materialized.iter().filter(|n| n.street == street).collect();
            let sol = uniform_solution(&t, &[], 0.3);
            assert!(sol.nodes[sol.requested as usize].path.is_empty(), "{template}: the root was requested");
            assert_eq!((sol.nodes.len(), sol.exploitability_chips), (on_street.len(), 0.3), "{template}");
            let paths = validate_solution(&sol, &t.materialized).unwrap();
            assert_eq!(paths, on_street.iter().map(|n| n.path.clone()).collect::<Vec<_>>(), "{template}: every root-street node, in materialized order");
            let deep = sol.nodes.iter().position(|n| n.actions.contains(&Action::Fold)).expect("a node facing a wager");
            let again = uniform_solution(&t, &sol.nodes[deep].path, 0.0);
            assert_eq!(again.requested as usize, deep);
            let fold = again.nodes[deep].actions.iter().position(|a| *a == Action::Fold).unwrap();
            assert!(again.nodes[deep].ev_chips.iter().all(|row| row[fold].to_bits() == 0.0f32.to_bits()));
        }
        assert!(tree(Street::Turn, "turn_std_v1").materialized.iter().any(|n| n.street == Street::River), "the turn tree reaches the river, and those nodes are left out");
    }

    #[test]
    #[should_panic(expected = "is not the chip path")]
    fn uniform_solution_refuses_a_requested_path_that_is_not_a_decision_node() {
        let _ = uniform_solution(&tree(Street::River, "river_std_v1"), &[Action::Bet { to: 12_345 }], 0.1);
    }

    #[test]
    #[should_panic(expected = "refuses")]
    fn uniform_solution_refuses_a_negative_exploitability() {
        let _ = uniform_solution(&tree(Street::River, "river_std_v1"), &[], -1.0);
    }
}
