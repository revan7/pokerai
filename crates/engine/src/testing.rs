//! Deterministic test doubles for the engine (plan 2 Task 19): a driven clock, a scripted worker link, an event
//! recorder and a builder of valid solutions. Compiled for this crate's own tests or with the `testing` feature only
//! (`lib.rs`), never into a release consumer.
//!
//! Time. Nothing here reads or waits on wall time. `FakeClock` moves only when a test (or `FakeWorker`, whose waiting
//! is what moves it) moves it, only forward, and every move wakes every thread blocked in `Clock::wait_until` (the
//! watchdog, say) through one condition variable.
//!
//! The scripted worker. `FakeWorker` answers `WorkerLink` from a script of `FakeReply` items taken in order. The
//! script is the worker's timeline: its output (`Ack`, `Progress`, `Result`), faulty lines (`Malformed`,
//! `Oversized`), what happens to the process (`Eof`: its stdout ends; `StdinClosed`: it stops taking requests;
//! `Exit`: it exits; `Hang`: nothing more, ever), a failed relaunch (`SpawnFails`), and two events that are not the
//! worker's, `Delay` (fake time passing) and `InvalidateIdentity` (a mutation arriving). It keeps the real link's
//! contract (`worker::link`, `worker::process`), so a script can only produce what the engine can meet in production,
//! and every outcome of that contract can be scripted.
//!
//! The worker's timeline. Every item is due at a time on the fake clock: the `Delay`s before it add up from the start
//! of the worker's timeline, which is the engine's first call on the worker after its launch (its creation by
//! `scripted`, or a `restart`): a `send`, a `recv`, or the kill that ends it. So a test may set the clock before the
//! engine's first request without spending the worker's delays. Due times do not depend on what the engine has read:
//! the process writes, closes its stdin and exits when those are due, however far behind the engine's reading is. Nor
//! do they wait for requests: an item with no `Delay` before it is due at the engine's first call, and a `Delay` before
//! the replies to a later request runs from the item before it, not from that request (an engine that idles past it
//! finds those replies already written when it sends). So an end meant to follow a request (`Exit`, `Eof`,
//! `StdinClosed`) is scripted after that request's replies with a `Delay` of at least 1 ms before it (`ack,
//! Delay { ms: 1 }, Exit { .. }`); without one it is due at the engine's first call, and the `send` of that very
//! request reports it before the request is taken (a reply for `IdRef::Last` then has no request to answer, and its
//! panic says why). What the engine sees of them:
//! - `recv(timeout)` has one deadline, `now + timeout` on the fake clock, for the whole call. It reads the worker's
//!   stdout in script order; waiting for the next item is moving the fake clock to its due time. With nothing due by
//!   the deadline (the next item is due later, a `Hang`, the end of the script) the call moves the clock to its
//!   deadline and returns `Ok(None)`. An item due exactly at the deadline is left for the next call; one already due
//!   is returned even with a zero timeout.
//! - Each reply is written and read as the real wire line: serialized with the checked codecs, bounded by the 16 MiB
//!   result-line limit, decoded as the real link decodes it. A scripted reply that no worker could write panics.
//!   `Malformed` and `Oversized` are one faulty line each; the link stays live (the caller restarts it, §12).
//! - Drain before end: the end of stdout (`Eof`, or the `Exit` that ends it) reaches `recv` after every reply
//!   scripted before it. Once stdout has ended, `recv` waits for the exit within what is left of the call's budget:
//!   the exit confirmed by then is `Exit { code }`, final (both calls keep answering it at once until a kill or a
//!   restart); otherwise the call spends its whole budget and answers `Eof`, the unconfirmed end, which is never
//!   taken for an exit.
//! - `send` never waits: it makes one poll of the process at the current fake time, as the real `send` does. An exit
//!   due by then is `Exit { code }` at once, even while replies written before it are still queued for `recv` (they
//!   stay there); a worker that has closed its stdin is `Eof` at once. Neither is recorded as a confirmed exit. The end
//!   of stdout alone does not stop the worker taking requests. `send` also refuses what the real link refuses (a
//!   request that does not serialize within the 1 MiB request-line limit), and records only what a worker took.
//!
//! Process generations. A worker process owns its end: the `Delay`s and `StdinClosed`s leading to its terminator
//! (`Hang`, `Exit`, or `Eof`), and after an `Eof` the confirmation of its exit (`Delay`s and `StdinClosed`s ending at
//! an `Exit`, or at a `Hang`: never confirmed). An `InvalidateIdentity` among them is not the process's (a mutation
//! reaches the engine, not the worker) and interrupts none of it. Anything else after an `Eof` is the next process's:
//! none of it reaches the engine, nor does its time pass, before a restart. `kill` is idempotent and leaves no live
//! worker: `send` and `recv` answer `Eof` at once and `ready` is `None` until `restart`. A kill (and a restart, which
//! kills first) takes with the killed process what the real kill drops with its channels:
//! - once its exit is due on the fake clock (whether or not `send` or `recv` has reported it yet), everything scripted
//!   up to and including that exit, the lines it wrote that the engine has not read included: what an exited process
//!   wrote goes with it;
//! - else, once the closure of its stdin is due (a `StdinClosed`, its writer state), likewise everything up to and
//!   including that `StdinClosed`, and then whatever is left of its end: the next worker's stdin is open;
//! - before either, whatever is left of its end, delays included, and nothing else: a scripted reply is never
//!   discarded unless its process's exit or stdin closure is already due at the kill (a reply after a due closure
//!   is the next worker's, as is one after any kill point).
//!
//! An `InvalidateIdentity` a kill passes stays, in order, at the front of the script: the mutation still arrives, at
//! the next worker's first `recv`. `restart` is counted, then fails with `Spawn` (no live worker) when a `SpawnFails`
//! is next in the script, and otherwise launches a new worker process with `default_ready`, its stdin open and a
//! timeline of its own: what is left of the script is that worker's, measured from its own start.
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
    /// it before any such request was sent is a broken script, and panics; when the engine did send one and `send`
    /// refused it (an end already due at the engine's first call, say), the panic says why.
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
    /// `ms` of fake time pass on the worker's timeline before its next item: the delay runs from the item before it
    /// (from the start of the timeline for a first item), whether or not the engine has read that item, and whenever
    /// the engine sends the request that the next item answers: the timeline does not wait for requests. A `recv`
    /// whose deadline comes first returns there, and the rest of the delay is still ahead of the next call.
    Delay { ms: u64 },
    /// The worker's stdout ends; the process lives on, and keeps taking requests, until its exit, which the items
    /// after it may confirm (`Delay`s and `StdinClosed`s ending at an `Exit`; see the module doc). `recv` answers
    /// `Err(Eof)` after spending its whole budget for as long as the exit is not confirmed. With no `Delay` before it,
    /// it is due at the engine's first call (see the module doc): `Delay { ms: 1 }` before it places it after the
    /// replies to that call's request.
    Eof,
    /// One line that is not a message: `Err(Protocol(..))` with this text.
    Malformed(String),
    /// One line of this many bytes, over the limit: `Err(LineTooLong(n))`.
    Oversized(usize),
    /// Nothing, ever, until a kill or a restart: every `recv` spends its whole budget and returns `Ok(None)` (after an
    /// `Eof`, `Err(Eof)`: the exit is never confirmed).
    Hang,
    /// The worker closes its stdin and lives on: from then on `send` answers `Err(Eof)` at once (the exit unconfirmed)
    /// until its exit is due. Its stdout is not affected: `recv` goes on reading the script. With no `Delay` before it,
    /// it is due at the engine's first call (see the module doc). The real link's first `send` after its worker closes
    /// its stdin is usually queued and lost (`Ok`, and no reply ever comes), only a later `send` answering `Eof`; a
    /// script says so by placing `StdinClosed` just after that send on the timeline (a `Delay` before it), so that the
    /// send is `Ok` and recorded (the engine did send it) and no reply to it is scripted. Once due, a kill discards it
    /// with the process, with the lines written before it (see the module doc): the next worker's stdin is open.
    StdinClosed,
    /// Simulates a mutation arriving while the solve is live: the active identity is cancelled.
    /// `recv` consumes it and continues to the next scripted item in the SAME call, so a script that wants the
    /// client to observe the invalidation before the next reply must write `InvalidateIdentity, Delay { ms: 1 }, ...`;
    /// the `Delay` makes `recv` return `Ok(None)` and the receive loop re-check the identity (see Task 28).
    /// Precisely: the first `Delay` a call reaches after an `InvalidateIdentity` ends that call with `Ok(None)` once it
    /// has run (or at the call's deadline, whichever comes first). It is not the worker's: a kill passes over it (the
    /// killed process's end behind it goes with that process) and leaves it at the front of the script, for the next
    /// worker's first `recv`; after an `Eof` it interrupts no confirmation, and the call that reaches it on the timeline
    /// (confirming the exit, or waiting for it) consumes it.
    InvalidateIdentity,
    /// The worker process exits with `code`, and its stdout ends with it (after an `Eof`: the confirmation of that
    /// end). `recv` answers `Err(Exit { code })` once it has returned every reply scripted before it, and keeps
    /// answering it at once, as `send` does, until the worker is killed or restarted; `send` answers it as soon as the
    /// exit is due, even while those replies are still queued. With no `Delay` before it, it is due at the engine's
    /// first call, whose `send` then reports it before the request is taken (see the module doc): `Delay { ms: 1 }`
    /// before it places it after the replies to that call's request.
    Exit { code: i32 },
    /// The next `restart` fails to launch a worker: `Err(Spawn(reason))`, leaving no live worker. Only `restart`
    /// consumes it, and only as the next item of the script (past any `InvalidateIdentity` a kill left at the front); a
    /// live worker has nothing to say in its place.
    SpawnFails(String),
}

/// What the fake worker was asked to do, shared with the test.
#[derive(Debug, Default)]
pub struct FakeState {
    /// Every request the worker took, in order; a `send` the link refused (no live worker, an exited worker, a closed
    /// stdin, a request it cannot write) is not recorded.
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
    /// Running with its stdout open: `recv` reads its output from the script.
    Live,
    /// Its stdout has ended (`Eof`) and its exit is not confirmed; it may still take requests.
    Ended,
    /// Its exit is confirmed with this code; final until a kill or a restart.
    Exited(i32),
    /// No live worker: killed, or its relaunch failed.
    Gone,
}

/// What one poll of the process finds at the current fake time (the real `send`'s `try_wait` and writer check).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Polled {
    /// The process has exited with this code; the `Exit` is at this index of the script.
    Exited { code: i32, at: usize },
    /// The process lives on but has closed its stdin: at this index of the script (the last `StdinClosed` due), or
    /// `None` when `recv` has already passed it.
    StdinClosed { at: Option<usize> },
    /// The process takes requests.
    TakingRequests,
}

/// A scripted `WorkerLink` (see the module doc).
pub struct FakeWorker {
    script: VecDeque<FakeReply>,
    /// Where the worker's timeline stands: the due time of the last item taken from the script, or the start of the
    /// timeline, from which the `Delay`s at the front of the script run. `None` until the engine's first call after a
    /// launch starts the timeline.
    anchor: Option<u64>,
    proc: Proc,
    /// The worker has closed its stdin (`recv` has passed its `StdinClosed`).
    stdin_closed: bool,
    ready: Option<Ready>,
    state: Arc<Mutex<FakeState>>,
    clock: Arc<FakeClock>,
    identity: Arc<Mutex<IdentityState>>,
    /// Why `send` refused the engine's last refused request, and its last refused solve: the panic of an `IdRef::Last`
    /// with nothing to resolve to names it.
    refused_request: Option<String>,
    refused_solve: Option<String>,
}

/// The end of a `Delay { ms }` that starts at `t`.
fn delay_end(t: u64, ms: u64) -> u64 {
    t.checked_add(ms).unwrap_or_else(|| panic!("FakeReply::Delay {{ ms: {ms} }} from {t} overflows the fake clock"))
}

/// The fix for the commonest cause of a request refused by an end already due (see the module doc).
const UNDELAYED_END: &str = "the timeline does not wait for requests: an end with no `Delay` before it is due at the engine's first call \
    after the worker's launch or restart; to place it after a request's replies, script `Delay { ms: 1 }` before it";

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
        let w = FakeWorker { script: script.into(), anchor: None, proc: Proc::Live, stdin_closed: false, ready: Some(Self::default_ready()),
            state: state.clone(), clock, identity, refused_request: None, refused_solve: None };
        (Box::new(w), state)
    }

    /// `IdRef::Last` resolved against what has been sent so far (see `IdRef`).
    fn resolve(&self, id: IdRef, reply: &str, of_solve: bool) -> String {
        match id {
            IdRef::Fixed(id) => id,
            IdRef::Last => {
                let s = lock(&self.state);
                let last = if of_solve { s.last_solve_id.clone() } else { s.sent.last().map(|m| request_id(m).to_string()) };
                last.unwrap_or_else(|| {
                    let refused = if of_solve { &self.refused_solve } else { &self.refused_request };
                    let why = refused.as_ref().map_or(String::new(), |r| format!(" ({r})"));
                    panic!("FakeReply::{reply} with IdRef::Last: no {} has been sent, so there is no id to answer{why}", if of_solve { "solve" } else { "request" })
                })
            }
        }
    }

    /// Spends the rest of the call's budget: the fake clock moves to the deadline. A call whose deadline the fake clock
    /// can never reach would wait forever, which a test can only mean by mistake.
    fn wait_out(&self, deadline: Option<u64>, why: &str) {
        let deadline = deadline.unwrap_or_else(|| panic!("FakeWorker::recv: {why}, and the timeout is too large for any fake-clock deadline: the call would never return"));
        self.clock.advance_to(deadline);
    }

    /// Starts the worker's timeline at the engine's first call after a launch (see the module doc).
    fn start_timeline(&mut self) {
        let now = self.clock.now_ms();
        self.anchor.get_or_insert(now);
    }

    /// Where the timeline stands (it has started: every `send` and `recv` on a worker process starts it first).
    fn anchor(&self) -> u64 { self.anchor.expect("the worker's timeline starts at the engine's first call") }

    /// The confirmation of an exit after stdout has ended, when the items from `from` are one: nothing but `Delay`s,
    /// `StdinClosed`s and identity markers (not the process's) up to an `Exit` (or a `Hang`: never confirmed), whose
    /// index this is. `None` otherwise: the process never confirms its exit, and those items are the next process's.
    fn confirmation_end(&self, from: usize) -> Option<usize> {
        for (i, item) in self.script.iter().enumerate().skip(from) {
            match item {
                FakeReply::Delay { .. } | FakeReply::StdinClosed | FakeReply::InvalidateIdentity => {}
                FakeReply::Exit { .. } | FakeReply::Hang => return Some(i),
                _ => return None,
            }
        }
        None
    }

    /// How many items at the front of the script are the running process's pending end, which a kill discards before
    /// its exit is due (see the module doc): the `Delay`s, `StdinClosed`s and identity markers leading to its
    /// terminator, the terminator, and after an `Eof` the confirmation of its exit. Zero when the process has something
    /// else to do first: short of a due exit or stdin closure (`end_process`), a reply is never discarded.
    fn pending_end(&self) -> usize {
        match self.proc {
            Proc::Live => {
                let lead = self.script.iter()
                    .take_while(|r| matches!(r, FakeReply::Delay { .. } | FakeReply::StdinClosed | FakeReply::InvalidateIdentity)).count();
                match self.script.get(lead) {
                    Some(FakeReply::Hang | FakeReply::Exit { .. }) => lead + 1,
                    Some(FakeReply::Eof) => self.confirmation_end(lead + 1).map_or(lead + 1, |end| end + 1),
                    _ => 0,
                }
            }
            Proc::Ended => self.confirmation_end(0).map_or(0, |end| end + 1),
            Proc::Exited(_) | Proc::Gone => 0,
        }
    }

    /// One poll of the process at `now`, as the real `send` makes it: has it exited by then, or closed its stdin (and
    /// where in the script)? It takes nothing from the script and never moves the clock; the replies due by then are
    /// written, and stay queued for `recv`.
    fn poll(&self, now: u64) -> Polled {
        let mut t = self.anchor();
        let mut closed_at = None;
        // The process's own items: the whole script while its stdout is open, only the confirmation of its exit once
        // stdout has ended.
        let mut own = match self.proc {
            Proc::Ended => self.confirmation_end(0).map_or(0, |end| end + 1),
            Proc::Live | Proc::Exited(_) | Proc::Gone => self.script.len(),
        };
        let mut i = 0;
        while i < own {
            match &self.script[i] {
                FakeReply::Delay { ms } => {
                    t = delay_end(t, *ms);
                    if t > now { break; }
                }
                FakeReply::StdinClosed => closed_at = Some(i),
                FakeReply::Exit { code } => return Polled::Exited { code: *code, at: i },
                FakeReply::Eof => own = self.confirmation_end(i + 1).map_or(i + 1, |end| end + 1),
                FakeReply::Hang | FakeReply::SpawnFails(_) => break,
                // Output written and not yet read, or a mutation (not the process's): the process goes on.
                FakeReply::Ack { .. } | FakeReply::Progress { .. } | FakeReply::Result { .. } | FakeReply::Malformed(_) | FakeReply::Oversized(_)
                | FakeReply::InvalidateIdentity => {}
            }
            i += 1;
        }
        if closed_at.is_some() || self.stdin_closed { Polled::StdinClosed { at: closed_at } } else { Polled::TakingRequests }
    }

    /// `recv` while stdout is open: the timeline up to the next line, the end of stdout, or the deadline.
    fn read_stdout(&mut self, deadline: Option<u64>) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        let mut invalidated = false;
        loop {
            let now = self.clock.now_ms();
            match self.script.front() {
                Some(FakeReply::Delay { ms }) => {
                    let end = delay_end(self.anchor(), *ms);
                    if let Some(d) = deadline.filter(|d| *d < end) {
                        self.clock.advance_to(d); // the rest of the delay is still ahead of the next call
                        return Ok(None);
                    }
                    self.clock.advance_to(end);
                    self.script.pop_front();
                    self.anchor = Some(end);
                    // The receive loop re-checks the identity here (see `InvalidateIdentity`); an item due exactly at
                    // the deadline is the next call's.
                    if invalidated || (end > now && Some(end) == deadline) { return Ok(None); }
                }
                Some(FakeReply::InvalidateIdentity) => {
                    lock(&self.identity).cancel_active();
                    self.script.pop_front();
                    invalidated = true;
                }
                Some(FakeReply::StdinClosed) => {
                    self.script.pop_front();
                    self.stdin_closed = true;
                }
                None | Some(FakeReply::Hang | FakeReply::SpawnFails(_)) => {
                    self.wait_out(deadline, "the worker has nothing more to say (a hang, or the end of the script)");
                    return Ok(None);
                }
                Some(FakeReply::Eof) => {
                    self.script.pop_front();
                    self.proc = Proc::Ended;
                    return self.confirm_exit(deadline);
                }
                Some(_) => return self.take_line(),
            }
        }
    }

    /// The line at the front of the script, taken: a reply as the real link decodes it, a faulty line's error, or the
    /// `Exit` that ends stdout, confirmed.
    fn take_line(&mut self) -> Result<Option<WorkerMessage>, WorkerLinkError> {
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
            FakeReply::Exit { code } => {
                self.proc = Proc::Exited(code);
                return Err(WorkerLinkError::Exit { code });
            }
            FakeReply::Delay { .. } | FakeReply::InvalidateIdentity | FakeReply::StdinClosed | FakeReply::Eof | FakeReply::Hang | FakeReply::SpawnFails(_) =>
                unreachable!("read_stdout handles the timeline before a line"),
        };
        Ok(Some(on_the_wire(msg)))
    }

    /// `recv` once stdout has ended: the exit, confirmed when it is due by the deadline (what is left of the call's
    /// budget), else the unconfirmed `Eof` after the whole budget. The identity markers within the confirmation that
    /// the call reaches on the timeline arrive on the way.
    fn confirm_exit(&mut self, deadline: Option<u64>) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        let now = self.clock.now_ms();
        // Already due, or due before the deadline; due exactly at the deadline is the next call's, as for a line.
        let reached = |due: u64| due <= now || deadline.is_none_or(|d| due < d);
        if let Some(mut end) = self.confirmation_end(0) {
            let (mut due, mut i) = (self.anchor(), 0);
            while i < end {
                match &self.script[i] {
                    FakeReply::Delay { ms } => {
                        due = delay_end(due, *ms);
                        if !reached(due) { break; }
                        i += 1;
                    }
                    FakeReply::InvalidateIdentity => {
                        self.script.remove(i);
                        end -= 1;
                        lock(&self.identity).cancel_active();
                    }
                    _ => i += 1,
                }
            }
            // Every delay before the end was reached, so the end is due by this call.
            if let (true, FakeReply::Exit { code }) = (i == end, &self.script[end]) {
                let code = *code;
                self.clock.advance_to(due);
                self.script.drain(..=end);
                self.anchor = Some(due);
                self.proc = Proc::Exited(code);
                return Err(WorkerLinkError::Exit { code });
            }
        }
        self.wait_out(deadline, "the worker's stdout has ended and its exit is not confirmed");
        Err(WorkerLinkError::Eof)
    }

    /// The kill of the running process (see the module doc): once its exit is due, everything scripted through that
    /// exit goes with it, the lines it wrote and the engine has not read included, as the real kill drops the dead
    /// process's queued output; its stdin closure, once due, likewise (it is that process's writer state), then what
    /// is left of its end; before either, its pending end. The identity markers among those items are not the
    /// process's: they stay, in order, at the front of the script. No worker is left.
    fn end_process(&mut self) {
        let mut kept = Vec::new();
        if let Proc::Live | Proc::Ended = self.proc {
            self.start_timeline(); // a kill before any other call is the engine's first call on this worker
            match self.poll(self.clock.now_ms()) {
                Polled::Exited { at, .. } => self.discard(at + 1, &mut kept),
                Polled::StdinClosed { at: Some(at) } => {
                    self.discard(at + 1, &mut kept);
                    // What is left of its end follows the closure (after an `Eof`, the rest of the confirmation).
                    let end = self.pending_end();
                    self.discard(end, &mut kept);
                }
                Polled::StdinClosed { at: None } | Polled::TakingRequests => {
                    let end = self.pending_end();
                    self.discard(end, &mut kept);
                }
            }
        } // a confirmed exit has already taken its process's items; a killed worker has none left
        for marker in kept.into_iter().rev() {
            self.script.push_front(marker);
        }
        self.proc = Proc::Gone;
        self.anchor = None;
        self.stdin_closed = false;
        self.ready = None;
    }

    /// Drops the first `n` items of the script with the killed process, keeping the identity markers among them (not
    /// the process's) in `kept`, in order.
    fn discard(&mut self, n: usize, kept: &mut Vec<FakeReply>) {
        kept.extend(self.script.drain(..n).filter(|r| matches!(r, FakeReply::InvalidateIdentity)));
    }

    /// Why the link refuses to send `msg` now, if it does: the error `send` answers, and what happened, for the panic
    /// of an `IdRef::Last` left with nothing to resolve to.
    fn refusal(&mut self, msg: &EngineMessage) -> Option<(WorkerLinkError, String)> {
        let id = request_id(msg);
        // The real link's check, before anything else: a request it cannot write is refused.
        if let Err(e) = encode_request(msg) {
            let why = format!("the link could not write request {id:?}: {e}");
            return Some((e, why));
        }
        match self.proc {
            Proc::Gone => return Some((WorkerLinkError::Eof, format!("there was no live worker (killed, or its relaunch failed) when the engine sent request {id:?}"))),
            Proc::Exited(code) => return Some((WorkerLinkError::Exit { code }, format!("the worker's exit (code {code}) was already confirmed when the engine sent request {id:?}"))),
            Proc::Live | Proc::Ended => {}
        }
        self.start_timeline();
        let now = self.clock.now_ms();
        let (err, end) = match self.poll(now) {
            Polled::Exited { code, .. } => (WorkerLinkError::Exit { code }, format!("`Exit {{ code: {code} }}`")),
            // It no longer takes requests and the poll found no exit: unconfirmed, and never recorded as an exit.
            Polled::StdinClosed { .. } => (WorkerLinkError::Eof, "`StdinClosed`".to_string()),
            Polled::TakingRequests => return None,
        };
        Some((err, format!("the worker's scripted {end} was already due at {now} ms when the engine sent request {id:?}, so `send` reported it and the \
            worker never took the request; {UNDELAYED_END}")))
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
        if let Some((err, why)) = self.refusal(msg) {
            if matches!(msg, EngineMessage::Solve(_)) { self.refused_solve = Some(why.clone()); }
            self.refused_request = Some(why);
            return Err(err);
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
        self.start_timeline();
        // One deadline for the whole call, in whole milliseconds; `None` when no fake-clock reading can reach it.
        let deadline = u64::try_from(timeout.as_millis()).ok().and_then(|ms| self.clock.now_ms().checked_add(ms));
        match self.proc {
            Proc::Live => self.read_stdout(deadline),
            Proc::Ended => self.confirm_exit(deadline),
            Proc::Exited(_) | Proc::Gone => unreachable!("answered before the timeline"),
        }
    }

    fn restart(&mut self) -> Result<(), WorkerLinkError> {
        self.end_process();
        lock(&self.state).restarts += 1;
        // The next item, past the identity markers a kill left at the front (they stay for the next worker).
        let next = self.script.iter().position(|r| !matches!(r, FakeReply::InvalidateIdentity));
        if let Some(i) = next.filter(|i| matches!(self.script[*i], FakeReply::SpawnFails(_))) {
            let Some(FakeReply::SpawnFails(reason)) = self.script.remove(i) else { unreachable!("the next item was a SpawnFails") };
            return Err(WorkerLinkError::Spawn(reason));
        }
        // A new process: stdin open, its timeline starting at the engine's next call.
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
    /// from the engine, and is not final (nor does it stop the worker taking requests); the scripted `Exit`, 300 ms
    /// after the end of stdout, confirms it, at once and for good, until a restart.
    #[test]
    fn an_unconfirmed_end_is_not_final_and_a_confirmed_exit_is() {
        let (clock, _identity, mut w, state) = rig(vec![ack("before"), FakeReply::Eof, FakeReply::Delay { ms: 300 }, FakeReply::Exit { code: 3 }, ack("next worker")]);
        assert_eq!(acked(w.recv(ms(50))), "before", "every line before the end comes first");
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)));
        assert_eq!(clock.now_ms(), 100, "the exit was waited for, for the whole budget");
        w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap(); // the end of stdout alone refuses no request
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)), "still unconfirmed at 200: the exit comes at 300");
        assert_eq!(clock.now_ms(), 200);
        assert!(matches!(w.recv(ms(500)), Err(WorkerLinkError::Exit { code: 3 })), "confirmed within the budget");
        assert_eq!(clock.now_ms(), 300);
        assert!(matches!(w.recv(ms(500)), Err(WorkerLinkError::Exit { code: 3 })), "final, and at once");
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "2".into() }), Err(WorkerLinkError::Exit { code: 3 })));
        assert_eq!(clock.now_ms(), 300);
        assert!(w.ready().is_some(), "ready lasts until a kill");
        assert_eq!(state.lock().unwrap().sent.len(), 1, "the exited worker was given nothing");
        w.restart().unwrap();
        assert_eq!(acked(w.recv(ms(10))), "next worker");
    }

    /// `kill` is idempotent and leaves no live worker until `restart`; it ends a hang and the silence leading into it,
    /// but a scripted reply is never discarded: it is the restarted worker's, and the delay before it runs from that
    /// worker's start.
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
        assert_eq!(clock.now_ms(), 700, "the reply was not discarded; its delay ran in full from the relaunch at 200");
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

    /// Ruling 19-I1: a process's end, from its stdout's end to the confirmation of its exit (the delays between them
    /// included), belongs to that process. A restart before the end is reached discards all of it, so the delayed
    /// confirmation of the killed process never reaches the new one (the reviewer's script).
    #[test]
    fn a_restart_before_the_end_discards_its_delayed_confirmation_too() {
        let (clock, _identity, mut w, _state) = rig(vec![FakeReply::Delay { ms: 500 }, FakeReply::Eof, FakeReply::Delay { ms: 200 }, FakeReply::Exit { code: 3 },
            FakeReply::Malformed("new process".into())]);
        assert!(w.recv(ms(100)).unwrap().is_none());
        w.restart().unwrap();
        let got = w.recv(ms(1_000));
        assert!(matches!(&got, Err(WorkerLinkError::Protocol(m)) if m == "new process"), "the killed process's end reached its successor: {got:?}");
        assert_eq!(clock.now_ms(), 100, "nothing of the killed process's end was waited for");
    }

    /// Ruling 19-I1: once the stdout has ended with the exit unconfirmed, a restart discards the pending confirmation,
    /// even while a receive is part-way through the delay leading to it.
    #[test]
    fn a_restart_after_an_unconfirmed_end_discards_its_pending_confirmation() {
        let (clock, _identity, mut w, _state) = rig(vec![ack("a"), FakeReply::Eof, FakeReply::Delay { ms: 200 }, FakeReply::Exit { code: 3 },
            FakeReply::Malformed("new process".into())]);
        assert_eq!(acked(w.recv(ms(10))), "a");
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)));
        assert!(matches!(w.recv(ms(50)), Err(WorkerLinkError::Eof)), "at 150 the exit (due at 200) is still unconfirmed");
        w.restart().unwrap();
        let got = w.recv(ms(1_000));
        assert!(matches!(&got, Err(WorkerLinkError::Protocol(m)) if m == "new process"), "the killed process's confirmation reached its successor: {got:?}");
        assert_eq!(clock.now_ms(), 150);
    }

    /// Ruling 19-I1: a confirmed exit has consumed its process's end; the restarted worker's own script, a delay
    /// included, is intact and measured from its launch.
    #[test]
    fn a_restart_after_a_confirmed_exit_leaves_the_next_worker_intact() {
        let (clock, _identity, mut w, _state) = rig(vec![ack("a"), FakeReply::Eof, FakeReply::Delay { ms: 200 }, FakeReply::Exit { code: 3 },
            FakeReply::Delay { ms: 50 }, FakeReply::Malformed("new process".into())]);
        assert_eq!(acked(w.recv(ms(10))), "a");
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)));
        assert!(matches!(w.recv(ms(500)), Err(WorkerLinkError::Exit { code: 3 })));
        assert_eq!(clock.now_ms(), 200, "the exit comes 200 ms after the end of stdout, however the receives fell");
        w.restart().unwrap();
        assert!(matches!(w.recv(ms(1_000)), Err(WorkerLinkError::Protocol(m)) if m == "new process"));
        assert_eq!(clock.now_ms(), 250, "the new worker's delay runs from its launch");
    }

    /// The real `recv` confirms the exit within what is left of the call's budget once stdout has ended, so an exit
    /// due within it is `Exit` from that same call; one due after it is `Eof` after the whole budget.
    #[test]
    fn an_exit_due_within_the_budget_is_confirmed_by_the_call_that_meets_the_end() {
        let (clock, _identity, mut w, _state) = rig(vec![ack("a"), FakeReply::Eof, FakeReply::Delay { ms: 50 }, FakeReply::Exit { code: 3 }]);
        assert_eq!(acked(w.recv(ms(10))), "a");
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Exit { code: 3 })));
        assert_eq!(clock.now_ms(), 50, "confirmed when it came, within the call");
        let (clock, _identity, mut w, _state) = rig(vec![FakeReply::Eof, FakeReply::Exit { code: 4 }]);
        assert!(matches!(w.recv(ms(0)), Err(WorkerLinkError::Exit { code: 4 })), "an exit already due is confirmed even with no budget");
        assert_eq!(clock.now_ms(), 0);
    }

    /// Ruling 19-I2 (the reviewer's script): `send` polls the process, so an exit already due on the fake clock is
    /// `Exit` from `send` at once, with no wait of its own, whatever `recv` has seen.
    #[test]
    fn send_observes_an_exit_already_due_on_the_fake_clock() {
        let (clock, _identity, mut w, state) = rig(vec![FakeReply::Eof, FakeReply::Delay { ms: 100 }, FakeReply::Exit { code: 7 }]);
        assert!(matches!(w.recv(ms(0)), Err(WorkerLinkError::Eof)));
        assert!(matches!(w.recv(ms(50)), Err(WorkerLinkError::Eof)));
        clock.set_ms(100);
        let sent = w.send(&EngineMessage::Shutdown { id: "1".into() });
        assert!(matches!(sent, Err(WorkerLinkError::Exit { code: 7 })), "send did not poll the due exit: {sent:?}");
        assert_eq!(clock.now_ms(), 100, "send never waits");
        assert!(matches!(w.recv(ms(0)), Err(WorkerLinkError::Exit { code: 7 })));
        assert!(state.lock().unwrap().sent.is_empty(), "an exited worker was given nothing");
    }

    /// Ruling 19-I2: the process exits on its own timeline, not when the engine has read its output. `send` may report
    /// the exit first, while replies written before it are still queued; `recv` returns every one of them, then the
    /// confirmed exit, which both calls keep answering.
    #[test]
    fn send_may_report_the_exit_before_recv_has_drained_the_replies_written_before_it() {
        let (clock, _identity, mut w, state) = rig(vec![ack("a"), FakeReply::Delay { ms: 100 }, ack("b"), FakeReply::Exit { code: 7 }]);
        assert_eq!(acked(w.recv(ms(0))), "a");
        clock.set_ms(150); // the engine is busy: the worker writes "b" at 100 and exits
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "1".into() }), Err(WorkerLinkError::Exit { code: 7 })));
        assert_eq!(acked(w.recv(ms(0))), "b", "the reply written before the exit is still there");
        assert!(matches!(w.recv(ms(0)), Err(WorkerLinkError::Exit { code: 7 })));
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "2".into() }), Err(WorkerLinkError::Exit { code: 7 })));
        assert_eq!(clock.now_ms(), 150);
        assert!(state.lock().unwrap().sent.is_empty());
        // No read at all before the send: the exit is still what the send reports, and nothing is lost.
        let (_clock, _identity, mut w, _state) = rig(vec![ack("x"), FakeReply::Exit { code: 9 }]);
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "1".into() }), Err(WorkerLinkError::Exit { code: 9 })));
        assert_eq!(acked(w.recv(ms(0))), "x");
        assert!(matches!(w.recv(ms(0)), Err(WorkerLinkError::Exit { code: 9 })));
    }

    /// Ruling 19-I2: the end of stdout alone does not stop the worker taking requests (its stdin is open), as with the
    /// real link's stand-in whose stdout ends first: `send` goes through until the exit, which `recv` then confirms.
    #[test]
    fn a_worker_whose_stdout_ended_still_takes_requests_until_it_exits() {
        let (clock, _identity, mut w, state) = rig(vec![ack("a"), FakeReply::Eof, FakeReply::Delay { ms: 1_000 }, FakeReply::Exit { code: 0 }]);
        assert_eq!(acked(w.recv(ms(10))), "a");
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)));
        w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap();
        assert_eq!(state.lock().unwrap().sent.len(), 1, "the worker was given the request");
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)), "still unconfirmed at 200");
        assert!(matches!(w.recv(ms(2_000)), Err(WorkerLinkError::Exit { code: 0 })));
        assert_eq!(clock.now_ms(), 1_000);
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "2".into() }), Err(WorkerLinkError::Exit { code: 0 })));
        assert_eq!(state.lock().unwrap().sent.len(), 1);
    }

    /// Ruling 19-I2: the other half-closed outcome, as with the real link's stand-in whose stdin breaks first. Once the
    /// worker has closed its stdin, `send` answers an unconfirmed `Eof` at once (nothing is given to the worker) while
    /// its stdout goes on; the exit, once due, is what both calls answer.
    #[test]
    fn a_worker_that_closed_its_stdin_refuses_requests_while_its_output_goes_on() {
        let (clock, _identity, mut w, state) = rig(vec![ack("a"), FakeReply::Delay { ms: 10 }, FakeReply::StdinClosed, FakeReply::Delay { ms: 90 }, ack("b"),
            FakeReply::Eof, FakeReply::Delay { ms: 50 }, FakeReply::Exit { code: 5 }]);
        w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap();
        assert_eq!(acked(w.recv(ms(5))), "a");
        clock.set_ms(10);
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "2".into() }), Err(WorkerLinkError::Eof)), "stdin closed at 10, the exit unconfirmed");
        assert_eq!(acked(w.recv(ms(200))), "b", "stdout is still open");
        assert_eq!(clock.now_ms(), 100);
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "3".into() }), Err(WorkerLinkError::Eof)), "still unconfirmed, still at once");
        assert!(matches!(w.recv(ms(200)), Err(WorkerLinkError::Exit { code: 5 })));
        assert_eq!(clock.now_ms(), 150);
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "4".into() }), Err(WorkerLinkError::Exit { code: 5 })));
        assert_eq!(state.lock().unwrap().sent.len(), 1, "only the request sent while stdin was open reached the worker");
    }

    /// Both halves closed, the process living on (`Eof`, then `StdinClosed`, then a `Hang`: it never exits): `recv`
    /// answers `Eof` after each whole budget, `send` answers `Eof` at once. The restart ends that process with its whole
    /// end: the new worker takes requests again, and its own script is intact.
    #[test]
    fn a_restart_ends_a_half_closed_worker_and_the_new_one_takes_requests() {
        let (clock, _identity, mut w, state) = rig(vec![FakeReply::Eof, FakeReply::Delay { ms: 10 }, FakeReply::StdinClosed, FakeReply::Hang, ack("new")]);
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)));
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "1".into() }), Err(WorkerLinkError::Eof)));
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Eof)));
        assert_eq!(clock.now_ms(), 200);
        w.restart().unwrap();
        w.send(&EngineMessage::Shutdown { id: "2".into() }).unwrap();
        assert_eq!(acked(w.recv(ms(100))), "new");
        assert_eq!((clock.now_ms(), state.lock().unwrap().sent.len()), (200, 1));
        // A kill before a stdin-closing end is reached discards it too: the relaunched worker's stdin is open.
        let (_clock, _identity, mut w, _state) = rig(vec![FakeReply::Delay { ms: 10 }, FakeReply::StdinClosed, FakeReply::Delay { ms: 10 }, FakeReply::Eof, ack("next")]);
        w.kill();
        w.restart().unwrap();
        w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap();
        assert_eq!(acked(w.recv(ms(100))), "next");
    }

    /// The ids of the requests the worker took, in order.
    fn sent_ids(state: &Arc<Mutex<FakeState>>) -> Vec<String> { state.lock().unwrap().sent.iter().map(|m| request_id(m).to_string()).collect() }
    fn ack_last() -> FakeReply { FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None } }
    /// A `result{ok}` for the last solve, with a solution for `solve`'s tree.
    fn ok_last() -> FakeReply {
        FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(uniform_solution(&tree(Street::River, "river_std_v1"), &[], 0.3)), error: None, elapsed_ms: 5 }
    }

    /// Ruling 19-R1-1 (the re-review's X1): `send` reports an exit already due while a line the process wrote before it
    /// is still queued, and the engine restarts, as it answers a send-side exit. What the exited process wrote goes with
    /// it, as the real kill drops the dead process's queued output: the new worker takes the request and answers from
    /// its own script, never with the dead worker's line or its exit. The same through `kill`, with an exit confirmed
    /// after the end of stdout.
    #[test]
    fn a_restart_after_send_reported_an_exit_leaves_none_of_the_dead_process_output_to_the_next_worker() {
        let (clock, _identity, mut w, state) = rig(vec![ack("a"), FakeReply::Delay { ms: 100 }, ack("b"), FakeReply::Exit { code: 7 },
            FakeReply::Delay { ms: 100 }, ack("c")]);
        assert_eq!(acked(w.recv(ms(0))), "a");
        clock.set_ms(150); // the worker writes "b" at 100 and exits
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "1".into() }), Err(WorkerLinkError::Exit { code: 7 })));
        w.restart().unwrap();
        w.send(&EngineMessage::Shutdown { id: "2".into() }).unwrap();
        let got = w.recv(ms(1_000));
        assert!(matches!(&got, Ok(Some(WorkerMessage::Ack { id, .. })) if id == "c"), "the dead worker's output reached its successor: {got:?}");
        assert_eq!(clock.now_ms(), 250, "the new worker's own delay, from its launch at 150");
        let after = w.recv(ms(100));
        assert!(matches!(after, Ok(None)), "the dead worker's exit reached its successor: {after:?}");
        assert_eq!(sent_ids(&state), ["2"]);
        // Through `kill` (the cancel-then-kill path): a line not read, then the end of stdout and an exit due by the kill.
        let (clock, _identity, mut w, _state) = rig(vec![ack("a"), ack("b"), FakeReply::Eof, FakeReply::Delay { ms: 50 }, FakeReply::Exit { code: 3 }, ack("next")]);
        assert_eq!(acked(w.recv(ms(0))), "a");
        clock.set_ms(100);
        w.kill();
        w.restart().unwrap();
        assert_eq!(acked(w.recv(ms(10))), "next", "the dead worker's unread line and its exit went with it");
    }

    /// Ruling 19-R1-1 (the re-review's X2): "crash, then a successful retry", driven as `run_attempt` drives it. The
    /// first `send` meets the exit (scripted with no `Delay`, so due at the engine's first call) and the request is not
    /// taken; the engine restarts and retries; the retry is taken and answered, and only it is recorded.
    #[test]
    fn a_crash_reported_by_send_then_a_restart_lets_the_retry_be_taken_and_answered() {
        let (_clock, _identity, mut w, state) = rig(vec![ack_last(), FakeReply::Exit { code: 3 }, ack_last(), ok_last()]);
        let first = w.send(&solve("solve-1"));
        assert!(matches!(first, Err(WorkerLinkError::Exit { code: 3 })), "{first:?}");
        w.restart().unwrap();
        let retry = w.send(&solve("solve-2"));
        assert!(retry.is_ok(), "the retry met the dead worker's exit again: {retry:?}");
        assert_eq!(acked(w.recv(ms(10))), "solve-2");
        match w.recv(ms(10)) { Ok(Some(WorkerMessage::Result { id, status: ResultStatus::Ok, .. })) => assert_eq!(id, "solve-2"), other => panic!("expected the retry's result, got {other:?}") }
        assert_eq!(sent_ids(&state), ["solve-2"]);
        assert_eq!(state.lock().unwrap().last_solve_id.as_deref(), Some("solve-2"));
    }

    /// Ruling 19-R1-1, before any call: a kill is the engine's first call on that worker, so an exit scripted with no
    /// `Delay` before it is due at the kill, and the line written before it goes with the process too.
    #[test]
    fn a_kill_before_any_call_takes_an_undelayed_exit_and_the_lines_before_it() {
        let (_clock, _identity, mut w, _state) = rig(vec![ack("a"), FakeReply::Exit { code: 3 }, ack("next")]);
        w.kill();
        w.restart().unwrap();
        assert_eq!(acked(w.recv(ms(10))), "next");
    }

    /// Ruling 19-R1-2: the documented idiom. The timeline does not wait for requests, so an exit meant to follow a
    /// request's replies is scripted after them with a `Delay` of at least 1 ms (the re-review's X7): the request is
    /// taken and answered, then the exit; the retry after the restart likewise. Without the `Delay` (X5) the exit is due
    /// at the engine's first call, and that call's `send` reports it before the request is taken.
    #[test]
    fn an_exit_after_a_one_ms_delay_follows_the_request_and_without_it_precedes_it() {
        let (clock, _identity, mut w, state) = rig(vec![ack_last(), FakeReply::Delay { ms: 1 }, FakeReply::Exit { code: 3 }, ack_last(), ok_last()]);
        w.send(&solve("solve-1")).unwrap();
        assert_eq!(acked(w.recv(ms(100))), "solve-1");
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Exit { code: 3 })));
        w.restart().unwrap();
        w.send(&solve("solve-2")).unwrap();
        assert_eq!(acked(w.recv(ms(100))), "solve-2");
        assert!(matches!(w.recv(ms(100)), Ok(Some(WorkerMessage::Result { id, .. })) if id == "solve-2"));
        assert_eq!((clock.now_ms(), sent_ids(&state)), (1, vec!["solve-1".to_string(), "solve-2".to_string()]));
        let (_clock, _identity, mut w, state) = rig(vec![ack_last(), ack_last(), FakeReply::Exit { code: 3 }]);
        assert!(matches!(w.send(&solve("solve-1")), Err(WorkerLinkError::Exit { code: 3 })));
        assert!(sent_ids(&state).is_empty());
    }

    /// Ruling 19-R1-2: a reply for `IdRef::Last` when the only request was refused because an end scripted with no
    /// `Delay` was due at the engine's first call panics with a message naming that cause and the idiom.
    #[test]
    #[should_panic(expected = "was already due at 0 ms when the engine sent request \"solve-1\"")]
    fn a_last_id_reply_after_a_send_refused_by_an_undelayed_exit_names_the_cause() {
        let (_clock, _identity, mut w, _state) = rig(vec![ack_last(), FakeReply::Exit { code: 3 }]);
        let _ = w.send(&solve("solve-1"));
        let _ = w.recv(ms(10)); // drain, as the link allows after a send-side exit: the ack has no request to answer
    }

    /// Ruling 19-R1-3 (the re-review's R-e): an `InvalidateIdentity` is not the worker's, so a kill reaches past it: the
    /// `Hang` scripted for the killed worker goes with it and the replacement answers the retry. The mutation itself
    /// still arrives, at the replacement's first receive.
    #[test]
    fn a_restart_reaches_past_an_identity_marker_so_the_killed_worker_hang_goes_with_it() {
        let (clock, identity, mut w, state) = rig(vec![FakeReply::Delay { ms: 7_000 }, FakeReply::InvalidateIdentity, FakeReply::Hang, ack_last()]);
        let decision = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
        w.send(&solve("1")).unwrap();
        assert!(w.recv(ms(5_000)).unwrap().is_none());
        assert!(identity.lock().unwrap().is_active(&decision), "the mutation is due at 7 000");
        w.restart().unwrap(); // the heartbeat's restart at 5 000
        w.send(&solve("retry")).unwrap();
        let got = w.recv(ms(8_650));
        assert!(matches!(&got, Ok(Some(WorkerMessage::Ack { id, .. })) if id == "retry"), "the killed worker's hang reached its successor: {got:?}");
        assert_eq!(clock.now_ms(), 5_000);
        assert!(!identity.lock().unwrap().is_active(&decision), "the mutation still arrived");
        assert_eq!(kills_and_restarts(&state), (0, 1));
    }

    /// Ruling 19-R1-3, after the end of stdout: an `InvalidateIdentity` does not interrupt the confirmation of the exit.
    /// A restart before the exit discards the confirmation behind it; a receive that reaches the exit confirms it, the
    /// mutation arriving within that call.
    #[test]
    fn an_identity_marker_does_not_interrupt_the_confirmation_of_an_exit() {
        let (clock, identity, mut w, _state) = rig(vec![ack("a"), FakeReply::Eof, FakeReply::Delay { ms: 50 }, FakeReply::InvalidateIdentity,
            FakeReply::Delay { ms: 50 }, FakeReply::Exit { code: 3 }, ack("next")]);
        let decision = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
        assert_eq!(acked(w.recv(ms(10))), "a");
        assert!(matches!(w.recv(ms(40)), Err(WorkerLinkError::Eof)));
        assert!(identity.lock().unwrap().is_active(&decision), "the mutation is due at 50");
        assert!(matches!(w.recv(ms(20)), Err(WorkerLinkError::Eof)), "the exit, due at 100, is not confirmed by 60");
        assert!(!identity.lock().unwrap().is_active(&decision), "the call reached the mutation at 50");
        w.restart().unwrap();
        let got = w.recv(ms(1_000));
        assert!(matches!(&got, Ok(Some(WorkerMessage::Ack { id, .. })) if id == "next"), "the killed worker's confirmation reached its successor: {got:?}");
        assert_eq!(clock.now_ms(), 60);
        let (clock, identity, mut w, _state) = rig(vec![FakeReply::Eof, FakeReply::InvalidateIdentity, FakeReply::Delay { ms: 100 }, FakeReply::Exit { code: 3 }]);
        let decision = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
        assert!(matches!(w.recv(ms(200)), Err(WorkerLinkError::Exit { code: 3 })));
        assert_eq!(clock.now_ms(), 100);
        assert!(!identity.lock().unwrap().is_active(&decision));
    }

    /// A scripted failed relaunch behind an `InvalidateIdentity` a kill passed over is still the next restart's.
    #[test]
    fn a_failed_relaunch_behind_a_kept_identity_marker_is_still_the_next_restart() {
        let (_clock, identity, mut w, _state) = rig(vec![FakeReply::Delay { ms: 100 }, FakeReply::InvalidateIdentity, FakeReply::Hang,
            FakeReply::SpawnFails("no binary".into()), ack("r")]);
        let decision = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
        assert!(w.recv(ms(50)).unwrap().is_none());
        assert!(matches!(w.restart(), Err(WorkerLinkError::Spawn(m)) if m == "no binary"));
        w.restart().unwrap();
        assert_eq!(acked(w.recv(ms(10))), "r");
        assert!(!identity.lock().unwrap().is_active(&decision));
    }

    /// Ruling 19-R2-1 (the round-2 probe Y1): the closure of its stdin is the process's writer state, so a kill or
    /// restart discards a due `StdinClosed` as it discards a due exit, with the lines written before it, and then what
    /// is left of that process's end: the replacement starts with an open stdin, takes requests, and is served its own
    /// script (what follows the closure, as after any kill point before the exit is due).
    #[test]
    fn a_restart_discards_the_dead_process_due_stdin_closure_so_the_replacement_takes_requests() {
        let shutdown = |id: &str| EngineMessage::Shutdown { id: id.into() };
        // Y1: `send` reports the closed stdin, and the engine restarts.
        let (clock, _identity, mut w, state) = rig(vec![ack("a"), FakeReply::Delay { ms: 10 }, FakeReply::StdinClosed, ack("b"), FakeReply::Delay { ms: 100 },
            FakeReply::Exit { code: 3 }, ack("next")]);
        assert_eq!(acked(w.recv(ms(0))), "a");
        clock.set_ms(50);
        assert!(matches!(w.send(&shutdown("1")), Err(WorkerLinkError::Eof)), "the dead process closed its stdin at 10");
        w.restart().unwrap();
        w.send(&shutdown("2")).unwrap();
        clock.set_ms(100);
        let later = w.send(&shutdown("3"));
        assert!(later.is_ok(), "the dead process's stdin closure reached its successor: {later:?}");
        assert_eq!(acked(w.recv(ms(100))), "b", "the replacement's script, from the item after the closure");
        assert!(matches!(w.recv(ms(100)), Err(WorkerLinkError::Exit { code: 3 })));
        assert_eq!(clock.now_ms(), 150, "the replacement's own delay, from its launch at 50");
        assert_eq!(sent_ids(&state), ["2", "3"]);
        // Through `kill`, with lines written before the closure unread: they go with it, and so does the end behind it.
        let (clock, _identity, mut w, state) = rig(vec![ack("a"), ack("b"), FakeReply::Delay { ms: 10 }, FakeReply::StdinClosed, FakeReply::Delay { ms: 100 },
            FakeReply::Exit { code: 3 }, ack("next")]);
        w.send(&shutdown("1")).unwrap();
        clock.set_ms(50);
        w.kill();
        w.restart().unwrap();
        w.send(&shutdown("2")).unwrap();
        let got = w.recv(ms(10));
        assert!(matches!(&got, Ok(Some(WorkerMessage::Ack { id, .. })) if id == "next"), "the dead worker's lines or end reached its successor: {got:?}");
        assert_eq!(sent_ids(&state), ["1", "2"]);
        // After the end of stdout, a closure within the confirmation of the exit: the rest of the confirmation goes too.
        let (clock, _identity, mut w, _state) = rig(vec![ack("a"), FakeReply::Eof, FakeReply::Delay { ms: 10 }, FakeReply::StdinClosed, FakeReply::Delay { ms: 100 },
            FakeReply::Exit { code: 3 }, ack("next")]);
        w.send(&shutdown("1")).unwrap();
        clock.set_ms(50);
        w.restart().unwrap();
        w.send(&shutdown("2")).unwrap();
        let got = w.recv(ms(10));
        assert!(matches!(&got, Ok(Some(WorkerMessage::Ack { id, .. })) if id == "next"), "the dead worker's end reached its successor: {got:?}");
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
