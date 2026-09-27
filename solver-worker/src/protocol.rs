//! §4.5's worker side, shared by the three threads of `main`: `control` (reads stdin in every state and
//! answers each line), `executor` (runs one job at a time) and `writer` (the only thread that writes
//! stdout, `crate::writer`). Control reaches a running job only through its `LiveJob::cancel` flag, the
//! same `Arc` the job polls, so a cancel never waits for the executor. Task 12 provides the shared state,
//! the bounded line reader, the executor loop and the terminal; Task 13 adds the §4.5 state machine
//! (admission, ack rules, cancel, shutdown) in `handle_line` / `handle_message` / `handle_eof`; Task 14 adds
//! lock staging (`stage`) and the staged set's consumption by the next `solve` (`admit`).
//!
//! The state machine (a job's own progress moves `Building -> Solving -> Extracting`; its terminal returns the
//! worker to `Idle`, or leaves it `Stopping`):
//!
//! | Message | `Idle` | `Building` / `Solving` / `Extracting` | `Stopping` |
//! |---|---|---|---|
//! | `solve` | `duplicate` if its id is remembered as finished; else `precheck`'s reason, or `accepted` -> `Building`; either way it takes the staged lock set (below), as does a line naming a `solve` that the wire codec refuses | `duplicate` for the live id or a finished id, else `busy` | `stopping` |
//! | `cancel` | a finished id: `already_finished`; else `unknown_target` | the live id: `accepted`, the job's flag raised; else as `Idle` | the live id (if any): `accepted`; else as `Idle` |
//! | `shutdown` | `accepted`, `Exit(0)` -> `Stopping` (a staged set discarded) | `accepted`, the job cancelled -> `Stopping`; `Exit(0)` follows its terminal | `accepted`, nothing else |
//! | stdin EOF | `Exit(0)` -> `Stopping` (a staged set discarded) | the job cancelled -> `Stopping` | nothing |
//! | `lock` | `locks::validate`'s reason, or `staged` (`replaced: true` iff a set was staged, for any spot) | `solve_in_progress` | `stopping` |
//!
//! Lock staging (§4.5). At most one lock set is staged (`Proto::staged`, with the spot it belongs to), and only
//! while the worker is `Idle`: a `lock` is refused in any other state, a stop discards the set, and a `solve`
//! always takes it, so a live job and a staged set never coexist (asserted). The set belongs to exactly one
//! `solve`, the next one decided. Any request whose envelope identifies as `type: "solve"` discards the staged set
//! when it is rejected, whichever layer rejects it: the typed wire decode (valid JSON whose top-level object has
//! `"type": "solve"` but that is not a valid `solve`, e.g. `rake_rate: 1.0`; `handle_line_via`), `precheck`, or
//! admission (`duplicate`; `busy` and `stopping` never meet a staged set). The discard and the rejection ack happen
//! under the protocol lock in one decision. Unidentifiable input (invalid JSON, invalid UTF-8, an over-limit line,
//! an unknown or missing `type`) leaves staging untouched, as does every other message's rejection. Accepted with
//! another spot, the job ends `result{error{lock_mismatch}}` at once (§4.5) and the set is discarded; accepted with
//! the same spot, the set is checked against that solve's `tree.materialized` by the engine's own validator
//! (`proto::worker::validate_locks`, which needs the tree, so it cannot run at staging), and either goes to the
//! job, which validates it again and applies it in `Building` (`locks::apply`), or ends the job `lock_mismatch` at
//! once. The job's own end (ok, error, cancel) is the set's end too: nothing is ever staged again but by a new
//! `lock`.
//!
//! Every rejection is `ack{rejected, reason}` and changes no state, with the one exception above: a rejected
//! `solve` still takes the staged set with it. A line the wire codec refuses is rejected with the id `lenient_id`
//! recovers. Every decision is a pure function of the request and the state (no clock is read), taken and acked
//! under one hold of the protocol lock.
use crate::job::{self, JobControl, JobOutcome};
use crate::locks;
use crate::writer::Out;
use proto::worker::{validate_locks, AckStatus, EngineMessage, NodeLock, ResultStatus, SolveRequest, Stage, WorkerMessage, MAX_EXPORTED_NODES};
use proto::Street;
use std::collections::{HashSet, VecDeque};
use std::io::{self, BufRead};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// §4.5's 1 MiB request-line limit, owned by `proto::worker` (plan 1 Task 7); re-exported, never redefined.
pub use proto::worker::REQUEST_LINE_MAX as MAX_REQUEST_LINE;
/// How many finished job ids are remembered for `duplicate` and `already_finished` (the oldest is forgotten first).
const FINISHED_IDS: usize = 4096;
/// The `lock_mismatch` message of a solve whose spot is not the staged set's (§4.5).
const OTHER_SPOT: &str = "staged lock belongs to another spot";
/// The `lock_mismatch` message prefix of a solve of the staged spot whose tree the staged set does not fit.
const DOES_NOT_FIT: &str = "staged lock does not fit the solve's tree";
/// §4.5: after `shutdown` or stdin EOF "the process then exits 0 within 2 s". A live job normally ends at its next
/// cancel checkpoint well inside that (measured: an iteration 0.08-0.3 s, `finalize` 0.2-0.35 s), and its terminal is
/// followed by `Exit(0)`. If it has not ended after this grace, the stop watchdog queues `Exit(0)` behind whatever is
/// already queued, leaving the writer the rest of the 2 s to drain; the engine synthesizes the terminal of a job
/// that dies this way (§4.5's process-death exception, §10.3).
pub const STOP_GRACE: Duration = Duration::from_millis(1900);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState { Idle, Building, Solving, Extracting, Stopping }

pub struct LiveJob { pub id: String, pub cancel: Arc<AtomicBool> }
pub struct Job { pub req: SolveRequest, pub locks: Option<Vec<NodeLock>>, pub cancel: Arc<AtomicBool> }

/// `begin_stop`'s watchdog thread creation, as a seam: production always installs `spawn_watchdog`; a
/// test installs a `fn` that returns `Err` instead, to exercise the recoverable failure path (review I2)
/// without ever actually exhausting the OS thread scheduler.
type WatchdogSpawn = fn(Duration, SyncSender<Out>) -> io::Result<std::thread::JoinHandle<()>>;
/// The real watchdog: sleeps `grace`, then queues `Exit(0)` behind whatever is already queued.
fn spawn_watchdog(grace: Duration, out: SyncSender<Out>) -> io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new().name("stop-watchdog".into()).spawn(move || {
        std::thread::sleep(grace);
        let _ = out.send(Out::Exit(0));
    })
}

/// The protocol state, behind `Shared::proto`. `stop_grace` is how long a stop lets a live job run on before the
/// watchdog exits the process: `STOP_GRACE` in every worker (`new`); a test lengthens it to observe a stop without
/// the watchdog. `watchdog_spawn` is `spawn_watchdog` in every worker; a test replaces it to inject a spawn failure.
pub struct Proto { pub state: WorkerState, pub live: Option<LiveJob>, pub finished: VecDeque<String>, pub staged: Option<(String, Vec<NodeLock>)>, pub stopping: bool, pub stop_grace: Duration, watchdog_spawn: WatchdogSpawn }
pub struct Shared { pub proto: Mutex<Proto>, pub out: SyncSender<Out>, pub jobs: Sender<Job> }

impl Proto { pub fn new() -> Self { Self { state: WorkerState::Idle, live: None, finished: VecDeque::new(), staged: None, stopping: false, stop_grace: STOP_GRACE, watchdog_spawn: spawn_watchdog } } }
impl Default for Proto { fn default() -> Self { Self::new() } }

/// One read from stdin: a complete line (its terminator removed), a line over the limit or a line that is not
/// UTF-8 (each consumed whole and discarded unparsed, so neither carries a readable id), or the end of input.
pub enum Incoming { Line(String), TooLong, InvalidUtf8, Eof }

/// Bounded line read: a line over `MAX_REQUEST_LINE` is consumed to its newline and reported as `TooLong`.
/// The limit counts the line's bytes with its terminator (LF, and the CR of a CRLF), the convention
/// `extract::result_line_len` uses for the result line; at most `MAX_REQUEST_LINE` plus one read chunk is
/// ever buffered. A line within the limit is decoded as checked UTF-8 once it is complete (§4.5's lines are
/// UTF-8): one that is not is `InvalidUtf8`, never repaired into a different request. An `Interrupted` read is
/// retried, as `std`'s line readers do: `main` answers any other read error like EOF, by stopping the worker.
pub fn read_line(reader: &mut impl BufRead) -> io::Result<Incoming> {
    let mut buf: Vec<u8> = Vec::new();
    let mut too_long = false;
    loop {
        let chunk = match reader.fill_buf() { Ok(c) => c, Err(e) if e.kind() == io::ErrorKind::Interrupted => continue, Err(e) => return Err(e) };
        if chunk.is_empty() { return Ok(if buf.is_empty() && !too_long { Incoming::Eof } else if too_long { Incoming::TooLong } else { decoded(buf) }); }
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') { Some(i) => (i + 1, true), None => (chunk.len(), false) };
        if !too_long { buf.extend_from_slice(&chunk[..take]); if buf.len() > MAX_REQUEST_LINE { too_long = true; buf.clear(); } }
        reader.consume(take);
        if done {
            if too_long { return Ok(Incoming::TooLong); }
            while matches!(buf.last(), Some(b'\n' | b'\r')) { buf.pop(); }   // ASCII, so never part of a multi-byte sequence
            return Ok(decoded(buf));
        }
    }
}

/// A complete line's bytes as text, by checked UTF-8 decoding only: a lossy decode would turn an invalid byte
/// inside, say, a quoted `id` into U+FFFD and hand the parser a different, well-formed request.
fn decoded(line: Vec<u8>) -> Incoming {
    match String::from_utf8(line) { Ok(text) => Incoming::Line(text), Err(_) => Incoming::InvalidUtf8 }
}

/// An `ack` as a queue item: `reason` is present iff `rejected`, `replaced` iff `staged` (§4.5).
fn ack(id: &str, status: AckStatus, reason: Option<String>, replaced: Option<bool>) -> Out {
    assert_eq!(reason.is_some(), status == AckStatus::Rejected, "an ack carries a reason iff it is rejected");
    assert_eq!(replaced.is_some(), status == AckStatus::Staged, "an ack carries `replaced` iff it is staged");
    Out::Msg(WorkerMessage::Ack { id: id.to_string(), status, reason, replaced })
}
fn answer(id: &str, status: AckStatus) -> Out { ack(id, status, None, None) }
fn rejection(id: &str, reason: impl Into<String>) -> Out { ack(id, AckStatus::Rejected, Some(reason.into()), None) }

/// The sender's id for the ack of a line that is not a valid message: the ack must carry it (§4.5; review M2 of the
/// plan), including for a line that is not JSON at all (§13.2's `"pot":NaN`). A line that is valid JSON gives its
/// top-level `"id"` when that is a string; a line that is not is scanned (`scan_id`) for the same member. Anything
/// else is "unknown". It only ever names a rejection, so a wrong recovery costs an unmatched ack, never an admission.
pub fn lenient_id(line: &str) -> String {
    let id = match serde_json::from_str::<serde_json::Value>(line) {
        Ok(v) => v.get("id").and_then(|i| i.as_str()).map(String::from),
        Err(_) => scan_id(line),
    };
    id.unwrap_or_else(|| "unknown".into())
}

/// `lenient_id`'s scan of a line that is not valid JSON: the value of the top-level object's `"id"` key when it is
/// a complete JSON string, decoded (escapes resolved). The scan skips over every string (escapes included) and
/// tracks the nesting of objects and arrays, so an `"id"` inside a nested value or inside another string is never
/// taken for the sender's; it gives up (None) at a non-string id, at an unterminated string, at a closer without an
/// opener, or when the top-level object closes without one.
fn scan_id(line: &str) -> Option<String> {
    let b = line.as_bytes();
    let mut i = skip_ws(b, 0);
    if b.get(i) != Some(&b'{') { return None; }
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let end = string_end(b, i)?;
                if depth == 1 && &b[i..=end] == br#""id""# {
                    let colon = skip_ws(b, end + 1);
                    if b.get(colon) == Some(&b':') {
                        let value = skip_ws(b, colon + 1);
                        if b.get(value) != Some(&b'"') { return None; }
                        return serde_json::from_str(&line[value..=string_end(b, value)?]).ok();
                    }
                }
                i = end + 1;
            }
            b'{' | b'[' => { depth += 1; i += 1; }
            b'}' | b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 { return None; }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// The index of the quote that closes the JSON string opened at `b[open]`, None when it is unterminated. Every byte
/// of a multi-byte UTF-8 character is >= 0x80, so none is ever taken for a quote or a backslash.
fn string_end(b: &[u8], open: usize) -> Option<usize> {
    let mut k = open + 1;
    while k < b.len() {
        match b[k] { b'\\' => k += 2, b'"' => return Some(k), _ => k += 1 }
    }
    None
}
/// The first index at or after `k` that is not JSON whitespace.
fn skip_ws(b: &[u8], mut k: usize) -> usize {
    while matches!(b.get(k), Some(b' ' | b'\t' | b'\n' | b'\r')) { k += 1; }
    k
}

/// §4.5's cheap structural checks on a `solve`, answered `ack{rejected, reason}` with no work. The deeper ones (the
/// tree build and its cross-check, the history, the board against the root street, the ranges, the locks) run in
/// `Building` and end the accepted job `result{error{invalid_request}}`, or `tree_mismatch` for a realized tree that
/// differs from `tree.materialized`. §4.6 / §13.2: a `None` donk option for a street strictly after `root_street`
/// is invalid input rejected here, never a tree mismatch; the root street's own `None` is legal (its player never
/// faces a donk node).
pub fn precheck(req: &SolveRequest) -> Result<(), String> {
    let t = &req.tree;
    if t.materialized.is_empty() { return Err("materialized tree is empty".into()); }
    if t.root_street == Street::Preflop { return Err("root_street must be flop, turn or river".into()); }
    for (s, name) in [(Street::Turn, "turn"), (Street::River, "river")] {
        if s > t.root_street {
            match t.menus.get(&s).and_then(|m| m.donk.as_ref()) {
                Some(d) if d.is_empty() => {}
                Some(_) => return Err(format!("{s:?} donk sizes must be empty")),
                None => return Err(format!("donk option missing for {name}")),
            }
        }
    }
    let street_nodes = t.materialized.iter().filter(|n| n.street == t.root_street).count();
    if street_nodes > MAX_EXPORTED_NODES { return Err(format!("street node count exceeds {MAX_EXPORTED_NODES}")); }
    let mut seen = HashSet::new();
    if !(3..=5).contains(&req.board.len()) || !req.board.iter().all(|c| seen.insert(c.0)) { return Err("board must be 3 to 5 distinct cards".into()); }
    if req.pot == 0 || req.stack_oop == 0 || req.stack_ip == 0 { return Err("pot and stacks must be positive".into()); }
    // Fix round 1 (review I1): half-open at 1, the same domain the shared wire codec enforces
    // (`proto::numeric::domain_rake_rate`) -- a direct in-process `SolveRequest` must not accept a
    // value the wire parser would refuse.
    if !req.rake_rate.is_finite() || req.rake_rate < 0.0 || req.rake_rate >= 1.0 { return Err("rake_rate outside [0, 1)".into()); }
    Ok(())
}

/// One stdin line: parsed as an `EngineMessage` and handled, or rejected with the parser's reason and the id
/// `lenient_id` recovers. A blank line is ignored.
pub fn handle_line(shared: &Shared, line: &str) {
    handle_line_via(shared, line, &mut |o| { let _ = shared.out.send(o); });
}

/// `handle_line` with its publication step passed in (`handle_message_via`'s seam). A line the typed decode refuses is
/// rejected under one hold of the protocol lock, like every other ack; when the line still identifies itself as a
/// `solve` (`names_solve`), that same hold discards the staged lock set (module docs), so the discard and the
/// rejection are one decision. Any other refused line changes nothing.
fn handle_line_via(shared: &Shared, line: &str, publish: &mut dyn FnMut(Out)) {
    if line.trim().is_empty() { return; }
    match serde_json::from_str::<EngineMessage>(line) {
        Ok(msg) => handle_message_via(shared, msg, publish),
        Err(e) => {
            // Both read the line (up to 1 MiB) before the lock is taken, so the hold stays as short as any other.
            let (refusal, solve) = (rejection(&lenient_id(line), format!("invalid message: {e}")), names_solve(line));
            let mut p = shared.proto.lock().unwrap();
            if solve {
                assert!(p.staged.is_none() || p.state == WorkerState::Idle, "a lock set is staged while the worker is {:?}", p.state);
                p.staged = None;
            }
            publish(refusal);
        }
    }
}

/// Whether a line the typed decode refused still identifies itself as a `solve`: valid JSON whose top-level object has
/// `"type": "solve"`. Anything else is unidentifiable (not JSON, which includes a line that is not UTF-8 or is over the
/// line limit and never reaches the decoder; not an object; no `type`; another or an unknown type) and names no solve.
fn names_solve(line: &str) -> bool {
    matches!(serde_json::from_str::<serde_json::Value>(line), Ok(serde_json::Value::Object(o)) if o.get("type").and_then(|t| t.as_str()) == Some("solve"))
}

/// §4.5's state machine for one message (the module's table).
pub fn handle_message(shared: &Shared, msg: EngineMessage) {
    handle_message_via(shared, msg, &mut |o| { let _ = shared.out.send(o); });
}

/// `handle_message` with its publication step passed in: `shared.out` in production, and a test's probe that
/// observes the protocol lock at the instant each item is published. The decision, its state change and its ack
/// all happen under one hold of the protocol lock, the lock under which the executor retires a job and publishes
/// its terminal (`terminal_via`); so acks and terminals reach stdout in the order of the decisions: a cancel's
/// `accepted` ahead of the job's terminal, an `already_finished` behind it. The writer never takes this lock, so a
/// full `out` still drains.
fn handle_message_via(shared: &Shared, msg: EngineMessage, publish: &mut dyn FnMut(Out)) {
    let mut p = shared.proto.lock().unwrap();
    match msg {
        EngineMessage::Solve(req) => admit(shared, &mut p, req, publish),
        EngineMessage::Lock { id, spot, locks } => stage(&mut p, &id, spot, locks, publish),
        EngineMessage::Cancel { id, target } => {
            // Only the job named by `target` is ever touched: an unknown or finished target leaves the live job alone.
            let status = match p.live.as_ref().filter(|l| l.id == target) {
                Some(live) => { live.cancel.store(true, Ordering::SeqCst); AckStatus::Accepted }
                None if p.finished.contains(&target) => AckStatus::AlreadyFinished,
                None => AckStatus::UnknownTarget,
            };
            publish(answer(&id, status));
        }
        EngineMessage::Shutdown { id } => {
            publish(answer(&id, AckStatus::Accepted));
            begin_stop(shared, &mut p, publish);
        }
    }
}

/// §4.5 admission of a `solve`, tested in the order stopping, duplicate (the live id or a remembered finished id),
/// busy (`state != Idle`), precheck. Duplicate comes before busy (review M1 of the plan): a duplicate of the live id
/// implies `state != Idle`, so busy first would make "duplicate" unreachable. An admitted job becomes the live job
/// in `Building`, is acked `accepted`, and only then is handed to the executor, so its ack precedes everything the
/// job produces.
///
/// The staged lock set belongs to this solve whatever its fate (module docs), so it is taken before any test: a
/// rejection discards it (the one state change a rejection ever makes, here or, for a line naming a `solve` that the
/// wire codec refuses, in `handle_line_via`); an admitted job carries it when it is for
/// this solve's spot and fits its tree, and otherwise ends `lock_mismatch` right after its `accepted` ack, retired
/// under this same hold of the protocol lock (`retire`) with no job handed over, like a terminal from the executor.
fn admit(shared: &Shared, p: &mut Proto, req: SolveRequest, publish: &mut dyn FnMut(Out)) {
    assert!(p.staged.is_none() || p.state == WorkerState::Idle, "a lock set is staged while the worker is {:?}", p.state);
    let staged = p.staged.take();
    if p.stopping { return publish(rejection(&req.id, "stopping")); }
    if p.live.as_ref().is_some_and(|l| l.id == req.id) || p.finished.contains(&req.id) { return publish(rejection(&req.id, "duplicate")); }
    if p.state != WorkerState::Idle { return publish(rejection(&req.id, "busy")); }
    assert!(p.live.is_none(), "the worker is Idle with job {:?} live", p.live.as_ref().map(|l| &l.id));
    if let Err(reason) = precheck(&req) { return publish(rejection(&req.id, reason)); }
    let cancel = Arc::new(AtomicBool::new(false));
    p.state = WorkerState::Building;
    p.live = Some(LiveJob { id: req.id.clone(), cancel: Arc::clone(&cancel) });
    publish(answer(&req.id, AckStatus::Accepted));
    let mismatch = |message: String| JobOutcome::Error(job::error("lock_mismatch", message, false, None));
    let locks = match staged {
        None => None,
        Some((spot, _)) if spot != req.spot => return retire(p, &req.id, mismatch(OTHER_SPOT.into()), 0, publish),
        Some((_, locks)) => match validate_locks(&locks, &req.tree.materialized) {
            Ok(_) => Some(locks),
            Err(e) => return retire(p, &req.id, mismatch(format!("{DOES_NOT_FIT}: {e}")), 0, publish),
        },
    };
    // The executor lives as long as the process, so a closed channel means it died outside its panic boundary: an
    // accepted job must not be left live with nothing to run it, so control panics, the process exits non-zero, and
    // the engine synthesizes the terminal (§4.5's process-death exception).
    shared.jobs.send(Job { req, locks, cancel }).expect("the executor thread has stopped");
}

/// §4.5 `lock`, tested in the order stopping, not `Idle` (`solve_in_progress`: a lock never reaches, nor waits for,
/// a live job), the matrix rules (`locks::validate`, the tree-free rules, all of which it shares with the engine's
/// `proto::worker::validate_locks`, plus actor names, one width per lock and one lock per node). Stopping comes
/// first as it does for `solve`: a stopping worker, live job or not, never solves again. A rejection changes
/// nothing, a staged set included. An accepted set replaces any staged one, for any spot (`replaced`).
fn stage(p: &mut Proto, id: &str, spot: String, locks: Vec<NodeLock>, publish: &mut dyn FnMut(Out)) {
    if p.stopping { return publish(rejection(id, "stopping")); }
    if p.state != WorkerState::Idle { return publish(rejection(id, "solve_in_progress")); }
    assert!(p.live.is_none(), "the worker is Idle with job {:?} live", p.live.as_ref().map(|l| &l.id));
    if let Err(reason) = locks::validate(&locks) { return publish(rejection(id, reason)); }
    let replaced = p.staged.replace((spot, locks)).is_some();
    publish(ack(id, AckStatus::Staged, None, Some(replaced)));
}

/// §4.5 `Stopping`, on `shutdown` (after its ack) or stdin EOF: nothing more is admitted, a staged lock set is
/// discarded, and the process exits 0 once nothing runs. With no live job, `Exit(0)` is published now, behind
/// everything already queued. A live job is cancelled; its terminal, published at its next checkpoint, is followed
/// by `Exit(0)` (`terminal_via`), and should it not end within `stop_grace` the watchdog queues `Exit(0)` anyway
/// (`STOP_GRACE`). A second stop (a second `shutdown`, or EOF after one) changes nothing.
fn begin_stop(shared: &Shared, p: &mut Proto, publish: &mut dyn FnMut(Out)) {
    if p.stopping { return; }
    p.stopping = true;
    p.state = WorkerState::Stopping;
    p.staged = None;
    match p.live.as_ref() {
        None => publish(Out::Exit(0)),
        Some(live) => {
            live.cancel.store(true, Ordering::SeqCst);
            let (out, grace, spawn) = (shared.out.clone(), p.stop_grace, p.watchdog_spawn);
            // Fix round 1 (review I2): a failed thread spawn is a recoverable control-path error, not
            // license to drop the exit bound the watchdog exists to provide. Publish `Exit(0)` right now
            // instead -- through `publish`, so it is queued under this same lock hold, right behind the
            // ack already published above -- rather than waiting indefinitely on a job that may never
            // reach a checkpoint, or exiting the process directly while still holding the protocol lock.
            // The writer still drains everything queued ahead of it; a job that later finishes anyway is
            // the same process-death exception §4.5 already grants a fired watchdog (the engine
            // synthesizes any terminal that never arrives).
            if spawn(grace, out).is_err() { publish(Out::Exit(0)); }
        }
    }
}

/// stdin EOF: `shutdown` without the ack (§4.5).
pub fn handle_eof(shared: &Shared) {
    let mut p = shared.proto.lock().unwrap();
    begin_stop(shared, &mut p, &mut |o| { let _ = shared.out.send(o); });
}

/// A job's single terminal (§4.5): under the protocol lock the job is retired (its id joins the remembered
/// finished ids) and the worker returns to `Idle` (or stays `Stopping`); only then, still under the lock, is
/// the `result` queued, followed by `Exit(0)` when stopping.
fn terminal(shared: &Shared, id: &str, outcome: JobOutcome, elapsed_ms: u32) {
    terminal_via(shared, id, outcome, elapsed_ms, |o| { let _ = shared.out.send(o); });
}

/// `terminal` with its publication step passed in: `publish` is `shared.out` in production, and a test's
/// probe that observes the protocol lock and state at the instant each item is published.
fn terminal_via(shared: &Shared, id: &str, outcome: JobOutcome, elapsed_ms: u32, mut publish: impl FnMut(Out)) {
    let mut p = shared.proto.lock().unwrap();
    retire(&mut p, id, outcome, elapsed_ms, &mut publish);
}

/// The live job `id`'s terminal, under a hold of the protocol lock the caller already has: the executor's
/// (`terminal_via`), or control's own for a job it ends at admission (`admit`'s `lock_mismatch`).
fn retire(p: &mut Proto, id: &str, outcome: JobOutcome, elapsed_ms: u32, publish: &mut dyn FnMut(Out)) {
    let (status, solution, error) = match outcome {
        JobOutcome::Ok(s) => (ResultStatus::Ok, Some(s), None), JobOutcome::BestSoFar(s) => (ResultStatus::BestSoFar, Some(s), None),
        JobOutcome::Cancelled => (ResultStatus::Cancelled, None, None), JobOutcome::Error(e) => (ResultStatus::Error, None, Some(e)),
    };
    let result = WorkerMessage::Result { id: id.to_string(), status, elapsed_ms, solution, error };
    // The executor runs only the job control admitted, and only control's admission makes a job live.
    assert_eq!(p.live.as_ref().map(|l| l.id.as_str()), Some(id), "the job ending is not the live job");
    // A lock set is staged only while no job is live (module docs), so none can outlive the job that took it.
    assert!(p.staged.is_none(), "a lock set is staged while job {id} is live");
    p.live = None;
    p.finished.push_back(id.to_string());
    if p.finished.len() > FINISHED_IDS { p.finished.pop_front(); }
    p.state = if p.stopping { WorkerState::Stopping } else { WorkerState::Idle };
    // The result is the last effect of retirement and is published under the same lock: once the engine has seen
    // it, control cannot find the job still live or the worker not yet `Idle` (and answer the engine's next
    // `solve` "busy"), nor queue an `already_finished` ack ahead of it. The writer never takes this lock, so a
    // full `out` still drains. `Exit(0)` follows the result, in queue order.
    publish(Out::Msg(result));
    if p.stopping { publish(Out::Exit(0)); }
}

/// The executor's panic boundary (§4.5 `internal`, retryable): a job that panics becomes an error carrying
/// the panic's message when it has one, timed from `started` (a measured duration, which would saturate
/// only past `u32::MAX` ms, as `job`'s own timings do); a job that returns is passed through unchanged.
fn caught(started: Instant, run: impl FnOnce() -> job::JobResult) -> (JobOutcome, u32) {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(r) => (r.outcome, r.elapsed_ms),
        Err(p) => {
            let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
            (JobOutcome::Error(job::error("internal", msg, true, None)), u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX))
        }
    }
}

/// Job `id`'s progress sink: each report moves the worker to the reported stage (never out of `Stopping`)
/// and queues the `progress` message.
fn progress_to(sh: Arc<Shared>, pid: String) -> Box<dyn FnMut(Stage, u32, Option<f32>, u32, u64) + Send> {
    Box::new(move |stage, iterations, exploitability_chips, elapsed_ms, memory_bytes| {
        { let mut p = sh.proto.lock().unwrap(); if !p.stopping { p.state = match stage { Stage::Building => WorkerState::Building, Stage::Solving => WorkerState::Solving, Stage::Extracting => WorkerState::Extracting }; } }
        let _ = sh.out.send(Out::Msg(WorkerMessage::Progress { id: pid.clone(), stage, iterations, exploitability_chips, elapsed_ms, memory_bytes }));
    })
}

/// The executor thread: one job at a time, panics caught at the boundary (`internal`, retryable).
pub fn executor_loop(shared: Arc<Shared>, jobs: Receiver<Job>) {
    executor_loop_with(shared, jobs, &mut job::Stderr);
}

/// `executor_loop` with the jobs' hooks passed in (`job::Hooks`, via `job::run_hooked`). Production passes
/// `job::Stderr`, whose every hook is a no-op but for its stderr diagnostics, so both are the same loop. A test
/// passes hooks that record where each job goes and can hold it at a chosen Building checkpoint, Solving iteration
/// boundary or Extracting node boundary (a checkpoint barrier): the hooks run on this thread, never with the
/// protocol lock held, so control keeps answering while a job is held.
pub fn executor_loop_with(shared: Arc<Shared>, jobs: Receiver<Job>, hooks: &mut dyn job::Hooks) {
    for job in jobs {
        let id = job.req.id.clone();
        let mut ctl = JobControl { cancel: job.cancel.clone(), progress: progress_to(shared.clone(), id.clone()) };
        let started = Instant::now();
        let (outcome, elapsed) = caught(started, || job::run_hooked(&job.req, job.locks.as_deref(), &mut ctl, &mut *hooks));
        terminal(&shared, &id, outcome, elapsed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;
    fn read_all(input: &[u8]) -> Vec<String> {
        let mut r = BufReader::with_capacity(64, input);
        let mut out = Vec::new();
        loop {
            match read_line(&mut r).unwrap() {
                Incoming::Line(l) => out.push(l),
                Incoming::TooLong => out.push("<too long>".into()),
                Incoming::InvalidUtf8 => out.push("<not utf-8>".into()),
                Incoming::Eof => return out,
            }
        }
    }
    #[test]
    fn bounded_line_reading() {
        assert_eq!(read_all(b"{\"a\":1}\n{\"b\":2}\n"), vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(read_all(b"{\"a\":1}\r\n"), vec!["{\"a\":1}"]);      // CRLF is trimmed
        assert_eq!(read_all(b"{\"a\":1}"), vec!["{\"a\":1}"]);          // a final line without a newline still arrives
        assert_eq!(read_all(b"\n\n"), vec!["", ""]);                    // blank lines are lines; handle_line ignores them
        let over = format!("{}\n{{\"b\":2}}\n", "x".repeat(MAX_REQUEST_LINE + 1));
        assert_eq!(read_all(over.as_bytes()), vec!["<too long>", "{\"b\":2}"]);   // the oversized line is consumed to its newline
    }

    // ---- Beyond the brief's test: the limit's exact boundary, EINTR, the executor and its panic boundary ----

    /// The limit counts the line's bytes with its terminator, the convention `extract::result_line_len`
    /// uses for the result line: `MAX_REQUEST_LINE - 1` content bytes plus LF is the longest line accepted.
    #[test]
    fn the_request_line_limit_counts_the_terminator() {
        let fits = "x".repeat(MAX_REQUEST_LINE - 1);
        let over = "y".repeat(MAX_REQUEST_LINE);
        assert_eq!(read_all(format!("{fits}\n{over}\nz\n").as_bytes()), vec![fits.clone(), "<too long>".into(), "z".into()]);
        // CRLF: the CR is part of the line, so two content bytes fewer fit
        let crlf_fits = "x".repeat(MAX_REQUEST_LINE - 2);
        let crlf_over = "y".repeat(MAX_REQUEST_LINE - 1);
        assert_eq!(read_all(format!("{crlf_fits}\r\n{crlf_over}\r\nz\r\n").as_bytes()), vec![crlf_fits, "<too long>".into(), "z".into()]);
        // a final line without a terminator may use the whole limit; one byte more is too long, then EOF
        assert_eq!(read_all("x".repeat(MAX_REQUEST_LINE).as_bytes()), vec!["x".repeat(MAX_REQUEST_LINE)]);
        assert_eq!(read_all("x".repeat(MAX_REQUEST_LINE + 1).as_bytes()), vec!["<too long>".to_string()]);
    }

    /// A line that runs far past the limit before its newline (here over 2 MiB, so the overflow is seen
    /// many reads before the newline arrives) is still skipped whole: its tail never surfaces as a line.
    #[test]
    fn a_line_far_over_the_limit_is_skipped_to_its_newline() {
        let long = "w".repeat(2 * MAX_REQUEST_LINE + 7);
        assert_eq!(read_all(format!("{long}\nz\n").as_bytes()), vec!["<too long>", "z"]);
        assert_eq!(read_all(format!("{long}\r\n{long}").as_bytes()), vec!["<too long>", "<too long>"]);
    }

    /// Review I2: §4.5's lines are UTF-8. A line that is not is reported as exactly that, consumed whole,
    /// terminated or not, and never repaired with U+FFFD into a different request; the lines around it are
    /// unaffected.
    #[test]
    fn a_line_that_is_not_utf8_is_reported_as_such_never_repaired() {
        // 0xFF inside the quoted id: a lossy decoder would hand the parser the valid request {"id":"\u{FFFD}"}
        assert_eq!(read_all(b"{\"id\":\"\xff\"}\n{\"b\":2}\n"), vec!["<not utf-8>", "{\"b\":2}"]);
        assert_eq!(read_all(b"{\"id\":\"\xff\"}\r\n{\"b\":2}\r\n"), vec!["<not utf-8>", "{\"b\":2}"]);
        // a final line without a terminator: a truncated two-byte sequence
        assert_eq!(read_all(b"{\"b\":2}\n{\"id\":\"\xc3\"}"), vec!["{\"b\":2}", "<not utf-8>"]);
        // an overlong encoding and a UTF-16 surrogate are not UTF-8 either
        assert_eq!(read_all(b"\xc0\xaf\n\xed\xa0\x80\n"), vec!["<not utf-8>", "<not utf-8>"]);
        // valid multi-byte text that straddles two read chunks (the reader's buffer is 64 bytes) decodes whole
        let split = format!("{}\u{e9}\u{1F0A1}", "x".repeat(63));
        assert_eq!(read_all(format!("{split}\n").as_bytes()), vec![split]);
    }

    /// A reader that fails its first `fill_buf` with `Interrupted`, as a signal-interrupted read does.
    struct InterruptOnce<R> { inner: R, interrupted: bool }
    impl<R: io::Read> io::Read for InterruptOnce<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.inner.read(buf) }
    }
    impl<R: BufRead> BufRead for InterruptOnce<R> {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if !self.interrupted { self.interrupted = true; return Err(io::Error::from(io::ErrorKind::Interrupted)); }
            self.inner.fill_buf()
        }
        fn consume(&mut self, n: usize) { self.inner.consume(n) }
    }

    /// `main` answers a read error by stopping the worker (like EOF), so an `Interrupted` read, which
    /// `std`'s own line readers retry, must be retried here too rather than surface as an error.
    #[test]
    fn an_interrupted_read_is_retried() {
        let mut r = InterruptOnce { inner: BufReader::new(&b"{\"a\":1}\n"[..]), interrupted: false };
        assert!(matches!(read_line(&mut r).unwrap(), Incoming::Line(l) if l == "{\"a\":1}"));
        assert!(matches!(read_line(&mut r).unwrap(), Incoming::Eof));
    }

    use crate::testutil::solve_request;
    use proto::worker::{ResultStatus, WorkerError};
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::{channel, sync_channel};

    /// The proto state after the executor returned: `(state, live job present, finished ids)`.
    type After = (WorkerState, bool, Vec<String>);

    /// What `handle_message` (Task 13) does on admission: `Building`, a live job, one cancel flag shared by
    /// the live-job record and the queued job. The executor thread owns the only `Shared` and the jobs
    /// channel is closed after this one job, so `executor_loop` returns, the thread snapshots the state and
    /// drops `Shared`, and `out` disconnects: the test collects every queued item with no timeout.
    fn run_one(id: &str, stopping: bool, cancel_first: bool) -> (Vec<Out>, After) {
        let (out_tx, out_rx) = sync_channel::<Out>(256);
        let (unused_jobs, _unused_rx) = channel::<Job>();
        let shared = Arc::new(Shared { proto: Mutex::new(Proto::new()), out: out_tx, jobs: unused_jobs });
        let mut req = solve_request("river_two_combo", 0);
        req.id = id.to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut p = shared.proto.lock().unwrap();
            p.state = if stopping { WorkerState::Stopping } else { WorkerState::Building };
            p.live = Some(LiveJob { id: id.to_string(), cancel: cancel.clone() });
            p.stopping = stopping;
            // the control thread's cancel: through the live-job record, never through the executor
            if cancel_first { p.live.as_ref().unwrap().cancel.store(true, Ordering::SeqCst); }
        }
        let (jobs_tx, jobs_rx) = channel::<Job>();
        jobs_tx.send(Job { req, locks: None, cancel }).unwrap();
        drop(jobs_tx);
        let t = std::thread::spawn(move || {
            executor_loop(shared.clone(), jobs_rx);
            let p = shared.proto.lock().unwrap();
            (p.state, p.live.is_some(), p.finished.iter().cloned().collect::<Vec<_>>())
        });
        let got: Vec<Out> = out_rx.iter().collect();
        (got, t.join().expect("executor thread"))
    }

    fn messages(got: &[Out]) -> Vec<&WorkerMessage> { got.iter().filter_map(|o| match o { Out::Msg(m) => Some(m), Out::Exit(_) => None }).collect() }
    fn results(got: &[Out]) -> Vec<(&str, ResultStatus, bool, Option<&WorkerError>)> {
        messages(got).into_iter().filter_map(|m| match m { WorkerMessage::Result { id, status, solution, error, .. } => Some((id.as_str(), *status, solution.is_some(), error.as_ref())), _ => None }).collect()
    }

    #[test]
    fn the_executor_runs_a_job_to_exactly_one_terminal_and_returns_to_idle() {
        let (got, after) = run_one("21", false, false);
        let msgs = messages(&got);
        assert!(matches!(msgs.first(), Some(WorkerMessage::Progress { id, stage: Stage::Building, exploitability_chips: None, .. }) if id == "21"), "first message {:?}", msgs.first());
        assert_eq!(results(&got), vec![("21", ResultStatus::Ok, true, None)]);
        assert!(matches!(msgs.last(), Some(WorkerMessage::Result { .. })), "the terminal is the last message");
        assert!(!got.iter().any(|o| matches!(o, Out::Exit(_))), "no exit while not stopping");
        assert_eq!(after, (WorkerState::Idle, false, vec!["21".to_string()]));
    }

    #[test]
    fn a_cancel_through_the_live_job_flag_ends_the_job_cancelled() {
        let (got, after) = run_one("22", false, true);
        assert_eq!(results(&got), vec![("22", ResultStatus::Cancelled, false, None)]);
        assert_eq!(after, (WorkerState::Idle, false, vec!["22".to_string()]));
    }

    /// A job that ends while the worker is stopping is followed by `Exit(0)`, queued after its terminal.
    #[test]
    fn a_job_ending_while_stopping_is_followed_by_exit_zero() {
        let (got, after) = run_one("23", true, true);
        assert_eq!(results(&got), vec![("23", ResultStatus::Cancelled, false, None)]);
        assert!(matches!(got.last(), Some(Out::Exit(0))), "Exit(0) is the last item queued");
        assert!(matches!(got[got.len() - 2], Out::Msg(WorkerMessage::Result { .. })), "the terminal immediately precedes it");
        assert_eq!(after, (WorkerState::Stopping, false, vec!["23".to_string()]));
    }

    /// A job that completes while the worker is stopping still delivers its full result, then `Exit(0)`.
    #[test]
    fn a_job_completing_while_stopping_keeps_its_result_then_exits() {
        let (got, after) = run_one("24", true, false);
        assert_eq!(results(&got), vec![("24", ResultStatus::Ok, true, None)]);
        assert!(matches!(got.last(), Some(Out::Exit(0))));
        assert_eq!(after, (WorkerState::Stopping, false, vec!["24".to_string()]));
    }

    /// Each progress report moves the state to its stage and queues its message; once stopping, the state
    /// stays `Stopping` while the messages still go out.
    #[test]
    fn progress_moves_the_state_but_never_out_of_stopping() {
        let (out_tx, out_rx) = sync_channel::<Out>(16);
        let (unused_jobs, _unused_rx) = channel::<Job>();
        let shared = Arc::new(Shared { proto: Mutex::new(Proto::new()), out: out_tx, jobs: unused_jobs });
        let mut report = progress_to(shared.clone(), "25".into());
        for (stage, state) in [(Stage::Building, WorkerState::Building), (Stage::Solving, WorkerState::Solving), (Stage::Extracting, WorkerState::Extracting)] {
            report(stage, 3, Some(0.5), 7, 9);
            assert_eq!(shared.proto.lock().unwrap().state, state);
        }
        { let mut p = shared.proto.lock().unwrap(); p.stopping = true; p.state = WorkerState::Stopping; }
        for stage in [Stage::Building, Stage::Solving, Stage::Extracting] {
            report(stage, 4, None, 8, 10);
            assert_eq!(shared.proto.lock().unwrap().state, WorkerState::Stopping);
        }
        let got: Vec<WorkerMessage> = out_rx.try_iter().map(|o| match o { Out::Msg(m) => m, Out::Exit(c) => panic!("unexpected Exit({c})") }).collect();
        assert_eq!(got.len(), 6);
        assert_eq!(got[0], WorkerMessage::Progress { id: "25".into(), stage: Stage::Building, iterations: 3, exploitability_chips: Some(0.5), elapsed_ms: 7, memory_bytes: 9 });
        assert_eq!(got[5], WorkerMessage::Progress { id: "25".into(), stage: Stage::Extracting, iterations: 4, exploitability_chips: None, elapsed_ms: 8, memory_bytes: 10 });
    }

    /// The panic boundary: any payload becomes `internal`, retryable, with the panic's message when it has one.
    #[test]
    fn a_panicking_job_is_an_internal_retryable_error() {
        let internal = |r: (JobOutcome, u32)| match r.0 { JobOutcome::Error(e) => (e.code, e.message, e.retryable), other => panic!("{other:?}") };
        assert_eq!(internal(caught(std::time::Instant::now(), || panic!("static payload"))), ("internal".to_string(), "static payload".to_string(), true));
        assert_eq!(internal(caught(std::time::Instant::now(), || panic!("formatted {}", 7))), ("internal".to_string(), "formatted 7".to_string(), true));
        assert_eq!(internal(caught(std::time::Instant::now(), || std::panic::panic_any(7u8))), ("internal".to_string(), "panic".to_string(), true));
        // a job that returns is passed through with its own elapsed time
        let r = caught(std::time::Instant::now(), || job::JobResult { outcome: JobOutcome::Cancelled, elapsed_ms: 17 });
        assert!(matches!(r, (JobOutcome::Cancelled, 17)));
    }

    // ---- Fix round 1 (review I3): no terminal is visible before the retirement that makes it true ----

    use std::sync::TryLockError;

    /// What another thread (control) could observe right now: the protocol lock held, so it can act on nothing
    /// yet, or free with the state it would act on: (state, a live job, the id remembered as finished). It never
    /// blocks: `try_lock` reports a held lock as `WouldBlock` whoever holds it, including the calling thread.
    #[derive(Debug, PartialEq)]
    enum Seen { LockHeld, Free(WorkerState, bool, bool) }
    fn seen_now(shared: &Shared, id: &str) -> Seen {
        match shared.proto.try_lock() {
            Ok(p) => Seen::Free(p.state, p.live.is_some(), p.finished.iter().any(|f| f == id)),
            Err(TryLockError::WouldBlock) => Seen::LockHeld,
            Err(TryLockError::Poisoned(e)) => panic!("poisoned: {e}"),
        }
    }
    fn published(o: &Out) -> String {
        match o { Out::Msg(WorkerMessage::Result { id, status, .. }) => format!("result {id} {status:?}"), Out::Msg(m) => format!("{m:?}"), Out::Exit(c) => format!("exit {c}") }
    }
    /// The state the executor finds at its terminal: job `id` live, the worker extracting, or stopping.
    fn live_job(id: &str, stopping: bool, out: SyncSender<Out>) -> Arc<Shared> {
        let (unused_jobs, _) = channel::<Job>();
        let shared = Arc::new(Shared { proto: Mutex::new(Proto::new()), out, jobs: unused_jobs });
        {
            let mut p = shared.proto.lock().unwrap();
            p.state = if stopping { WorkerState::Stopping } else { WorkerState::Extracting };
            p.live = Some(LiveJob { id: id.into(), cancel: Arc::new(AtomicBool::new(false)) });
            p.stopping = stopping;
        }
        shared
    }

    /// Deterministic: the probe runs at the instant each item is published. The result (and, when stopping, the
    /// `Exit(0)` behind it) is published only under the protocol lock and after the job is retired, so once the
    /// engine has seen the result, control can never find the job still live, the worker not yet back to `Idle`,
    /// or the id not yet finished (which would answer the engine's next `solve` "busy", §4.5).
    #[test]
    fn the_terminal_is_published_under_the_lock_after_the_job_is_retired() {
        for stopping in [false, true] {
            let (out_tx, _out_rx) = sync_channel::<Out>(1);
            let shared = live_job("31", stopping, out_tx);
            let mut log = Vec::new();
            terminal_via(&shared, "31", JobOutcome::Cancelled, 5, |o| log.push((seen_now(&shared, "31"), published(&o))));
            let mut want = vec![(Seen::LockHeld, "result 31 Cancelled".to_string())];
            if stopping { want.push((Seen::LockHeld, "exit 0".to_string())); }
            assert_eq!(log, want, "stopping: {stopping}");
            let end = if stopping { WorkerState::Stopping } else { WorkerState::Idle };
            assert_eq!(seen_now(&shared, "31"), Seen::Free(end, false, true), "the lock is released with the job retired");
        }
    }

    /// The same through the real `terminal` and `out` (a rendezvous channel; this test is the writer and then
    /// control). Once the result is taken off the channel, control's next admission finds the job retired; when
    /// stopping, the executor still holds the lock until its `Exit(0)` is taken as well.
    #[test]
    fn control_never_finds_the_job_live_once_its_result_is_out() {
        for stopping in [false, true] {
            let (out_tx, out_rx) = sync_channel::<Out>(0);
            let shared = live_job("32", stopping, out_tx);
            let sh = shared.clone();
            let executor = std::thread::spawn(move || terminal(&sh, "32", JobOutcome::Cancelled, 5));
            assert_eq!(published(&out_rx.recv().unwrap()), "result 32 Cancelled");
            if stopping {
                assert_eq!(seen_now(&shared, "32"), Seen::LockHeld, "the executor holds the lock from its result to its exit");
                assert_eq!(published(&out_rx.recv().unwrap()), "exit 0");
            }
            let p = shared.proto.lock().unwrap();   // control's admission, right after the engine saw the result
            let end = if stopping { WorkerState::Stopping } else { WorkerState::Idle };
            assert_eq!((p.state, p.live.is_some(), p.finished.iter().any(|f| f == "32")), (end, false, true), "stopping: {stopping}");
            drop(p);
            executor.join().expect("executor thread");
        }
    }

    // ---- Task 13: the §4.5 state machine (admission, ack rules, cancel, shutdown) ----

    use crate::testutil::cases;
    use proto::{MenuSize, Street};

    /// Control in isolation: the test holds the writer's and the executor's ends of the two channels, so no other
    /// thread runs and every effect of a call is visible when it returns: what it queued for stdout, in order, and
    /// the jobs it handed to the executor. The stop grace is lengthened so the stop watchdog never fires inside
    /// these tests (its own test uses the production grace).
    struct Control { shared: Arc<Shared>, out: Receiver<Out>, jobs: Receiver<Job> }
    fn control() -> Control {
        let (out_tx, out) = sync_channel::<Out>(256);
        let (jobs_tx, jobs) = channel::<Job>();
        let mut proto = Proto::new();
        proto.stop_grace = Duration::from_secs(3600);
        Control { shared: Arc::new(Shared { proto: Mutex::new(proto), out: out_tx, jobs: jobs_tx }), out, jobs }
    }
    /// One queued item in comparable form: a message, or `Err(code)` for `Out::Exit(code)`.
    type Item = Result<WorkerMessage, i32>;
    /// Every piece of protocol state: the state, the live job's id and whether its cancel flag is raised, the
    /// remembered finished ids, the spot of the staged lock set (if one is staged), and the stopping flag.
    #[derive(Debug, Clone, PartialEq)]
    struct Snap { state: WorkerState, live: Option<(String, bool)>, finished: Vec<String>, staged: Option<String>, stopping: bool }
    impl Control {
        fn queued(&self) -> Vec<Item> { self.out.try_iter().map(|o| match o { Out::Msg(m) => Ok(m), Out::Exit(c) => Err(c) }).collect() }
        fn msg(&self, m: EngineMessage) -> Vec<Item> { handle_message(&self.shared, m); self.queued() }
        fn line(&self, l: &str) -> Vec<Item> { handle_line(&self.shared, l); self.queued() }
        fn eof(&self) -> Vec<Item> { handle_eof(&self.shared); self.queued() }
        fn handed(&self) -> Vec<Job> { self.jobs.try_iter().collect() }
        fn snap(&self) -> Snap {
            let p = self.shared.proto.lock().unwrap();
            Snap { state: p.state, live: p.live.as_ref().map(|l| (l.id.clone(), l.cancel.load(Ordering::SeqCst))), finished: p.finished.iter().cloned().collect(), staged: p.staged.as_ref().map(|(spot, _)| spot.clone()), stopping: p.stopping }
        }
        /// The executor's side of job `id` ending: its terminal, published exactly as in production.
        fn end(&self, id: &str) -> Vec<Item> { terminal(&self.shared, id, JobOutcome::Cancelled, 1); self.queued() }
    }

    fn solve(fixture: &str, id: &str) -> EngineMessage { solve_with(fixture, id, |_| {}) }
    fn solve_with(fixture: &str, id: &str, f: impl FnOnce(&mut SolveRequest)) -> EngineMessage {
        let mut r = solve_request(fixture, 0);
        r.id = id.into();
        f(&mut r);
        EngineMessage::Solve(r)
    }
    /// A solve that fails `precheck` alone: `flop_cancel` with its turn donk option removed.
    fn unchecked(id: &str) -> EngineMessage { solve_with("flop_cancel", id, |r| r.tree.menus.get_mut(&Street::Turn).unwrap().donk = None) }
    const DONK_MISSING: &str = "donk option missing for turn";
    fn cancel(id: &str, target: &str) -> EngineMessage { EngineMessage::Cancel { id: id.into(), target: target.into() } }
    fn shutdown(id: &str) -> EngineMessage { EngineMessage::Shutdown { id: id.into() } }
    fn acked(id: &str, status: AckStatus) -> Item { Ok(WorkerMessage::Ack { id: id.into(), status, reason: None, replaced: None }) }
    fn refused(id: &str, reason: &str) -> Item { Ok(WorkerMessage::Ack { id: id.into(), status: AckStatus::Rejected, reason: Some(reason.into()), replaced: None }) }
    fn cancelled(id: &str) -> Item { Ok(WorkerMessage::Result { id: id.into(), status: ResultStatus::Cancelled, elapsed_ms: 1, solution: None, error: None }) }
    /// Admits `id` (a valid river solve) and takes its ack and its job off the channels.
    fn running(c: &Control, id: &str) {
        assert_eq!(c.msg(solve("river_two_combo", id)), vec![acked(id, AckStatus::Accepted)]);
        assert_eq!(c.handed().len(), 1);
    }

    // Task 14: lock staging. `lock_river`'s line 0 stages IP's lock at `[check]` and its line 1 is the solve it
    // belongs to (the same spot); `river_two_combo` is the same river tree under another spot.
    /// The lock set of `lock_river`'s line 0.
    fn river_locks() -> Vec<NodeLock> {
        match serde_json::from_str::<EngineMessage>(&crate::testutil::fixture_lines("lock_river")[0]).expect("parse lock line") {
            EngineMessage::Lock { locks, .. } => locks,
            other => panic!("line 0 of lock_river is not a lock: {other:?}"),
        }
    }
    /// The spot `lock_river`'s lock set belongs to (its line 1's `spot`).
    fn lock_spot() -> String { solve_request("lock_river", 1).spot }
    fn lock_with(id: &str, spot: &str, f: impl FnOnce(&mut Vec<NodeLock>)) -> EngineMessage {
        let mut locks = river_locks();
        f(&mut locks);
        EngineMessage::Lock { id: id.into(), spot: spot.into(), locks }
    }
    fn river_lock(id: &str, spot: &str) -> EngineMessage { lock_with(id, spot, |_| {}) }
    /// A lock set `locks::validate` refuses (its first row sums to 0.4), and the reason it gives.
    fn bad_lock(id: &str) -> EngineMessage { lock_with(id, &lock_spot(), |l| l[0].probs[0] = vec![0.1, 0.3]) }
    fn bad_lock_reason() -> String {
        let EngineMessage::Lock { locks, .. } = bad_lock("x") else { unreachable!() };
        crate::locks::validate(&locks).expect_err("the bad lock is invalid")
    }
    /// `lock_river`'s own solve (line 1), under `id`.
    fn locked_solve(id: &str) -> EngineMessage {
        let mut r = solve_request("lock_river", 1);
        r.id = id.into();
        EngineMessage::Solve(r)
    }
    fn staged(id: &str, replaced: bool) -> Item { Ok(WorkerMessage::Ack { id: id.into(), status: AckStatus::Staged, reason: None, replaced: Some(replaced) }) }
    /// The terminal of an accepted solve whose staged lock set is refused before any work: `lock_mismatch`,
    /// not retryable, decided without reading a clock (elapsed 0).
    fn mismatched(id: &str, message: &str) -> Item {
        Ok(WorkerMessage::Result { id: id.into(), status: ResultStatus::Error, elapsed_ms: 0, solution: None, error: Some(WorkerError { code: "lock_mismatch".into(), message: message.into(), retryable: false, estimate_bytes: None }) })
    }

    /// Review M2 of the plan: the ack of a line that is not a valid message carries the sender's id whenever the
    /// line names one at its top level, valid JSON or not; anything else is "unknown".
    #[test]
    fn lenient_id_recovers_the_top_level_id() {
        let cases: [(&str, &str); 19] = [
            // valid JSON: the top-level string id, else "unknown" (never a nested one)
            (r#"{"type":"bogus","id":"1"}"#, "1"),
            (r#"{"type":"cancel","target":"x","id":"2","extra":1}"#, "2"),
            (r#"{"type":"cancel","id":7}"#, "unknown"),
            (r#"{"type":"cancel","x":{"id":"inner"}}"#, "unknown"),
            (r#"["id","5"]"#, "unknown"),
            // not JSON (§13.2's `NaN`): the scan
            (r#"{"type":"solve","id":"4","pot":NaN}"#, "4"),
            (r#"{"pot":NaN,"id":"4"}"#, "4"),
            ("{\"pot\":NaN,\r\n \"id\" \t:  \"4\"}", "4"),
            (r#"{"x":{"id":"inner"},"pot":NaN,"id":"outer"}"#, "outer"),
            (r#"{"x":[{"id":"inner"},"id"],"pot":NaN}"#, "unknown"),
            (r#"{"s":"\"id\":\"fake\"","pot":NaN,"id":"real"}"#, "real"),
            (r#"{"k":"id","pot":NaN}"#, "unknown"),
            (r#"{"id":"a\"b\\c","pot":NaN}"#, "a\"b\\c"),
            (r#"{"id":"A","pot":NaN}"#, "A"),
            (r#"{"pot":NaN,"id":"4"#, "unknown"),
            (r#"{"id":4,"pot":NaN}"#, "unknown"),
            (r#"{"type":"solve""#, "unknown"),
            (r#"{"pot":NaN}{"id":"second"}"#, "unknown"),
            (r#"xx"id":"5""#, "unknown"),
        ];
        for (line, want) in cases { assert_eq!(lenient_id(line), want, "{line}"); }
        assert_eq!(lenient_id(""), "unknown");
    }

    /// Every committed solve fixture passes the admission checks.
    #[test]
    fn precheck_accepts_every_committed_solve_fixture() {
        for (fixture, line) in [("river_two_combo", 0), ("flop_cancel", 0), ("flop_best_so_far", 0), ("lock_river", 1)] {
            assert_eq!(precheck(&solve_request(fixture, line)), Ok(()), "{fixture}");
        }
    }

    /// §4.6 / §13.2: a `None` donk option on a street strictly after `root_street` is invalid input, rejected with
    /// its street named; the root street's own `None` (and any earlier street's) is legal.
    #[test]
    fn precheck_rejects_a_missing_donk_option_only_after_the_root() {
        let flop = solve_request("flop_cancel", 0);
        let with = |f: &dyn Fn(&mut SolveRequest)| { let mut r = flop.clone(); f(&mut r); precheck(&r) };
        assert_eq!(flop.tree.menus[&Street::Flop].donk, None, "the flop-rooted fixture's own root donk is None");
        assert_eq!(with(&|r| r.tree.menus.get_mut(&Street::Turn).unwrap().donk = None), Err("donk option missing for turn".into()));
        assert_eq!(with(&|r| r.tree.menus.get_mut(&Street::River).unwrap().donk = None), Err("donk option missing for river".into()));
        assert_eq!(with(&|r| { r.tree.menus.remove(&Street::Turn); }), Err("donk option missing for turn".into()));
        assert_eq!(with(&|r| r.tree.menus.get_mut(&Street::Turn).unwrap().donk = Some(vec![MenuSize::Pot(0.5)])), Err("Turn donk sizes must be empty".into()));
        assert_eq!(with(&|r| r.tree.menus.get_mut(&Street::Flop).unwrap().donk = Some(vec![])), Ok(()));
        // a turn root: the river's None is invalid, the turn's own None is legal
        let turn_case = cases().into_iter().find(|c| c.case == "turn_std_v1_100_100").expect("turn_std_v1 case");
        let mut turn = flop.clone();
        turn.tree = turn_case.tree;
        assert_eq!((turn.tree.root_street, turn.tree.menus[&Street::Turn].donk.clone()), (Street::Turn, None));
        assert_eq!(precheck(&turn), Ok(()));
        turn.tree.menus.get_mut(&Street::River).unwrap().donk = None;
        assert_eq!(precheck(&turn), Err("donk option missing for river".into()));
        // a river root: no later street, and an earlier street's None is ignored
        let mut river = solve_request("river_two_combo", 0);
        assert_eq!(river.tree.menus[&Street::River].donk, None);
        let side = river.tree.menus[&Street::River].clone();
        river.tree.menus.insert(Street::Turn, proto::PlayerMenus { donk: None, ..side });
        assert_eq!(precheck(&river), Ok(()));
    }

    /// The other admission checks, each with its exact reason.
    #[test]
    fn precheck_rejects_the_other_structural_faults() {
        let river = solve_request("river_two_combo", 0);
        let with = |f: &dyn Fn(&mut SolveRequest)| { let mut r = river.clone(); f(&mut r); precheck(&r) };
        let err = |s: &str| Err::<(), String>(s.into());
        assert_eq!(with(&|r| r.tree.materialized.clear()), err("materialized tree is empty"));
        assert_eq!(with(&|r| r.tree.root_street = Street::Preflop), err("root_street must be flop, turn or river"));
        // the root street's node count is bounded by the export limit, inclusive; other streets do not count
        let node = river.tree.materialized[0].clone();
        let other = MaterializedNode { street: Street::Turn, ..node.clone() };
        assert_eq!(with(&|r| { r.tree.materialized = vec![node.clone(); MAX_REQUEST_NODES]; r.tree.materialized.extend(vec![other.clone(); 5]); }), Ok(()));
        assert_eq!(with(&|r| r.tree.materialized = vec![node.clone(); MAX_REQUEST_NODES + 1]), err("street node count exceeds 100000"));
        let card = |s: &str| proto::Card::parse(s).unwrap();
        for board in [vec!["Qs", "Jd"], vec!["Qs", "Jd", "7h", "3c", "2d", "2c"], vec!["Qs", "Jd", "Qs"], vec!["Qs", "Jd", "7h", "3c", "Qs"]] {
            let b: Vec<proto::Card> = board.iter().map(|s| card(s)).collect();
            assert_eq!(with(&|r| r.board = b.clone()), err("board must be 3 to 5 distinct cards"), "{board:?}");
        }
        for board in [vec!["Qs", "Jd", "7h"], vec!["Qs", "Jd", "7h", "3c"]] {
            let b: Vec<proto::Card> = board.iter().map(|s| card(s)).collect();
            assert_eq!(with(&|r| r.board = b.clone()), Ok(()), "{board:?}");
        }
        assert_eq!(with(&|r| r.pot = 0), err("pot and stacks must be positive"));
        assert_eq!(with(&|r| r.stack_oop = 0), err("pot and stacks must be positive"));
        assert_eq!(with(&|r| r.stack_ip = 0), err("pot and stacks must be positive"));
        // Fix round 1 (review I1): the domain is half-open at 1, matching the shared wire codec
        // (`proto::numeric::domain_rake_rate`), so 1.0 itself is a rejection, not the upper bound.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e-9, 1.0, 1.000_001] {
            assert_eq!(with(&|r| r.rake_rate = bad), err("rake_rate outside [0, 1)"), "{bad}");
        }
        for good in [0.0, 0.05, 0.999_999_94] { assert_eq!(with(&|r| r.rake_rate = good), Ok(()), "{good}"); }
    }
    const MAX_REQUEST_NODES: usize = proto::worker::MAX_EXPORTED_NODES;
    use proto::MaterializedNode;

    #[derive(Clone, Copy, Debug)]
    enum Start { Idle, Finished9, Live(WorkerState), StoppingIdle, StoppingLive, Staged }
    /// A worker in state `s`, reached through the handlers themselves (the executor's progress, which moves a live
    /// job's state, is played by setting it); the setup's own output is taken off the channels. `Staged` is
    /// `Finished9` with `lock_river`'s lock set then staged for its own spot.
    fn start(s: Start) -> Control {
        let c = control();
        match s {
            Start::Idle => {}
            Start::Finished9 => { running(&c, "9"); c.end("9"); }
            Start::Live(state) => { running(&c, "11"); c.shared.proto.lock().unwrap().state = state; }
            Start::StoppingIdle => { c.msg(shutdown("0")); }
            Start::StoppingLive => { running(&c, "11"); c.eof(); }
            Start::Staged => {
                running(&c, "9");
                c.end("9");
                assert_eq!(c.msg(river_lock("8", &lock_spot())), vec![staged("8", false)]);
            }
        }
        c.queued();
        c.handed();
        c
    }
    enum In { M(EngineMessage), L(String), Eof }

    // Fix round 1 (review P2.T14-I1): lines the typed decode refuses. A line that still identifies itself as a
    // `solve` (valid JSON, top-level `"type": "solve"`) takes the staged set with it; anything unidentifiable does not.
    /// `msg` as its wire line, edited as a JSON value by `f`.
    fn wire_with(msg: &EngineMessage, f: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut v = serde_json::to_value(msg).expect("the message serializes");
        f(&mut v);
        v.to_string()
    }
    /// The rejection `handle_line` gives `line`, which the typed decode must refuse: the codec's own reason.
    fn codec_refusal(id: &str, line: &str) -> Item {
        let e = serde_json::from_str::<EngineMessage>(line).expect_err("the wire codec refuses the line");
        refused(id, &format!("invalid message: {e}"))
    }
    /// `lock_river`'s own solve under `id`, with `rake_rate: 1.0`: valid JSON naming a `solve`, refused by the codec's
    /// half-open rake domain (the reviewer's replay).
    fn rake_one(id: &str) -> String { wire_with(&locked_solve(id), |v| v["rake_rate"] = serde_json::json!(1.0)) }
    /// `lock_river`'s own solve under `id` with its pot written `NaN`: not JSON at all (§13.2), so it names no type.
    fn nan_pot(id: &str) -> String {
        let msg = locked_solve(id);
        let EngineMessage::Solve(r) = &msg else { unreachable!() };
        let line = serde_json::to_string(&msg).unwrap().replacen(&format!("\"pot\":{}", r.pot), "\"pot\":NaN", 1);
        assert!(line.contains("\"pot\":NaN") && line.contains("\"type\":\"solve\""), "{line}");
        line
    }

    /// §4.5's state table, row by row: from a start state, one input queues exactly the listed items, hands the
    /// executor the listed number of jobs, and changes the state exactly as listed (a rejection changes nothing).
    /// The solve rows pin the admission order stopping, duplicate, busy, precheck: `unchecked` fails precheck, so
    /// every reason other than its own shows an earlier test winning.
    #[test]
    fn the_state_table() {
        use AckStatus::*;
        use WorkerState::{Building, Extracting, Solving, Stopping};
        let same: fn(&mut Snap) = |_| {};
        let admitted: fn(&mut Snap) = |s| { s.state = Building; s.live = Some(("21".into(), false)); };
        let raised: fn(&mut Snap) = |s| s.live.as_mut().unwrap().1 = true;
        let stopped: fn(&mut Snap) = |s| { s.state = Stopping; s.stopping = true; };
        let stopped_raised: fn(&mut Snap) = |s| { s.state = Stopping; s.stopping = true; s.live.as_mut().unwrap().1 = true; };
        // Task 14: lock staging and the staged set's consumption
        let staged_here: fn(&mut Snap) = |s| s.staged = Some(lock_spot());
        let staged_abcd: fn(&mut Snap) = |s| s.staged = Some("abcd".into());
        let admitted_with_lock: fn(&mut Snap) = |s| { s.state = Building; s.live = Some(("21".into(), false)); s.staged = None; };
        let mismatch_retired: fn(&mut Snap) = |s| { s.finished.push("21".into()); s.staged = None; };
        let discarded: fn(&mut Snap) = |s| s.staged = None;
        let stopped_discarded: fn(&mut Snap) = |s| { s.state = Stopping; s.stopping = true; s.staged = None; };
        let bad = bad_lock_reason();
        let spot = lock_spot();
        // Fix round 1 (review P2.T14-I1): lines the typed decode refuses, each with the codec's own reason.
        let extra_field = wire_with(&locked_solve("21"), |v| v["extra"] = serde_json::json!(1));
        let untyped = wire_with(&locked_solve("21"), |v| { v.as_object_mut().unwrap().remove("type"); });
        let mistyped = wire_with(&locked_solve("21"), |v| v["type"] = serde_json::json!("solves"));
        let wide_lock = wire_with(&river_lock("19", &spot), |v| v["locks"][0]["probs"][0] = serde_json::json!([-0.1, 1.1]));
        let wide_cancel = wire_with(&cancel("13", "nope"), |v| v["extra"] = serde_json::json!(1));
        let line = |l: &String| In::L(l.clone());
        let rows: Vec<(&str, Start, In, Vec<Item>, usize, fn(&mut Snap))> = vec![
            ("a solve the wire codec refuses (rake_rate 1) discards the staged set", Start::Staged, line(&rake_one("21")), vec![codec_refusal("21", &rake_one("21"))], 0, discarded),
            ("a solve the wire codec refuses (an unknown field) discards the staged set", Start::Staged, line(&extra_field), vec![codec_refusal("21", &extra_field)], 0, discarded),
            ("a solve the wire codec refuses, idle: nothing staged, nothing changes", Start::Idle, line(&rake_one("21")), vec![codec_refusal("21", &rake_one("21"))], 0, same),
            ("a solve the wire codec refuses while solving: nothing changes", Start::Live(Solving), line(&rake_one("12")), vec![codec_refusal("12", &rake_one("12"))], 0, same),
            ("a solve the wire codec refuses while stopping: nothing changes", Start::StoppingIdle, line(&rake_one("21")), vec![codec_refusal("21", &rake_one("21"))], 0, same),
            ("a line that is not JSON names no type: the staged set stays", Start::Staged, line(&nan_pot("21")), vec![codec_refusal("21", &nan_pot("21"))], 0, same),
            ("a message without a type: the staged set stays", Start::Staged, line(&untyped), vec![codec_refusal("21", &untyped)], 0, same),
            ("a message of an unknown type: the staged set stays", Start::Staged, line(&mistyped), vec![codec_refusal("21", &mistyped)], 0, same),
            ("a lock the wire codec refuses: the staged set stays", Start::Staged, line(&wide_lock), vec![codec_refusal("19", &wide_lock)], 0, same),
            ("a cancel the wire codec refuses: the staged set stays", Start::Staged, line(&wide_cancel), vec![codec_refusal("13", &wide_cancel)], 0, same),
            ("admit", Start::Idle, In::M(solve("river_two_combo", "21")), vec![acked("21", Accepted)], 1, admitted),
            ("admit after a finished job", Start::Finished9, In::M(solve("river_two_combo", "21")), vec![acked("21", Accepted)], 1, admitted),
            ("precheck", Start::Idle, In::M(unchecked("21")), vec![refused("21", DONK_MISSING)], 0, same),
            ("duplicate of a finished id, before precheck", Start::Finished9, In::M(unchecked("9")), vec![refused("9", "duplicate")], 0, same),
            ("duplicate of the live id, before busy", Start::Live(Building), In::M(unchecked("11")), vec![refused("11", "duplicate")], 0, same),
            ("duplicate while solving", Start::Live(Solving), In::M(solve("river_two_combo", "11")), vec![refused("11", "duplicate")], 0, same),
            ("busy, before precheck", Start::Live(Building), In::M(unchecked("12")), vec![refused("12", "busy")], 0, same),
            ("busy while solving", Start::Live(Solving), In::M(solve("river_two_combo", "12")), vec![refused("12", "busy")], 0, same),
            ("busy while extracting", Start::Live(Extracting), In::M(solve("river_two_combo", "12")), vec![refused("12", "busy")], 0, same),
            ("stopping, before duplicate", Start::StoppingLive, In::M(unchecked("11")), vec![refused("11", "stopping")], 0, same),
            ("stopping, idle", Start::StoppingIdle, In::M(solve("river_two_combo", "21")), vec![refused("21", "stopping")], 0, same),
            ("cancel the live job", Start::Live(Building), In::M(cancel("13", "11")), vec![acked("13", Accepted)], 0, raised),
            ("cancel the live job while solving", Start::Live(Solving), In::M(cancel("13", "11")), vec![acked("13", Accepted)], 0, raised),
            ("cancel the live job while stopping", Start::StoppingLive, In::M(cancel("13", "11")), vec![acked("13", Accepted)], 0, same),
            ("cancel a finished id", Start::Finished9, In::M(cancel("13", "9")), vec![acked("13", AlreadyFinished)], 0, same),
            ("cancel an unknown id", Start::Idle, In::M(cancel("13", "nope")), vec![acked("13", UnknownTarget)], 0, same),
            ("cancel an unknown id beside a live job", Start::Live(Building), In::M(cancel("13", "nope")), vec![acked("13", UnknownTarget)], 0, same),
            ("cancel's own id is not its target", Start::Live(Building), In::M(cancel("11", "12")), vec![acked("11", UnknownTarget)], 0, same),
            ("shutdown, idle", Start::Idle, In::M(shutdown("17")), vec![acked("17", Accepted), Err(0)], 0, stopped),
            ("shutdown, live", Start::Live(Solving), In::M(shutdown("17")), vec![acked("17", Accepted)], 0, stopped_raised),
            ("shutdown again, idle", Start::StoppingIdle, In::M(shutdown("18")), vec![acked("18", Accepted)], 0, same),
            ("shutdown again, live", Start::StoppingLive, In::M(shutdown("18")), vec![acked("18", Accepted)], 0, same),
            ("eof, idle", Start::Idle, In::Eof, vec![Err(0)], 0, stopped),
            ("eof, live", Start::Live(Extracting), In::Eof, vec![], 0, stopped_raised),
            ("eof after a stop", Start::StoppingLive, In::Eof, vec![], 0, same),
            ("lock, idle", Start::Idle, In::M(river_lock("19", &spot)), vec![staged("19", false)], 0, staged_here),
            ("lock after a finished job", Start::Finished9, In::M(river_lock("19", &spot)), vec![staged("19", false)], 0, staged_here),
            ("a second lock replaces the staged set, for any spot", Start::Staged, In::M(river_lock("19", "abcd")), vec![staged("19", true)], 0, staged_abcd),
            ("an invalid lock matrix, idle", Start::Idle, In::M(bad_lock("19")), vec![refused("19", &bad)], 0, same),
            ("an invalid lock matrix leaves the staged set staged", Start::Staged, In::M(bad_lock("19")), vec![refused("19", &bad)], 0, same),
            ("lock while building", Start::Live(Building), In::M(river_lock("19", &spot)), vec![refused("19", "solve_in_progress")], 0, same),
            ("lock while solving", Start::Live(Solving), In::M(river_lock("19", &spot)), vec![refused("19", "solve_in_progress")], 0, same),
            ("lock while extracting", Start::Live(Extracting), In::M(river_lock("19", &spot)), vec![refused("19", "solve_in_progress")], 0, same),
            ("lock while solving, before its matrix", Start::Live(Solving), In::M(bad_lock("19")), vec![refused("19", "solve_in_progress")], 0, same),
            ("lock while stopping, idle", Start::StoppingIdle, In::M(river_lock("19", &spot)), vec![refused("19", "stopping")], 0, same),
            ("lock while stopping, live", Start::StoppingLive, In::M(river_lock("19", &spot)), vec![refused("19", "stopping")], 0, same),
            ("lock while stopping, before its matrix", Start::StoppingIdle, In::M(bad_lock("19")), vec![refused("19", "stopping")], 0, same),
            ("a solve of the staged spot takes the set to its job", Start::Staged, In::M(locked_solve("21")), vec![acked("21", Accepted)], 1, admitted_with_lock),
            ("a solve of another spot: accepted, lock_mismatch, the set discarded", Start::Staged, In::M(solve("river_two_combo", "21")), vec![acked("21", Accepted), mismatched("21", OTHER_SPOT)], 0, mismatch_retired),
            ("a solve rejected by precheck discards the staged set", Start::Staged, In::M(unchecked("21")), vec![refused("21", DONK_MISSING)], 0, discarded),
            ("a duplicate solve discards the staged set", Start::Staged, In::M(locked_solve("9")), vec![refused("9", "duplicate")], 0, discarded),
            ("a cancel leaves the staged set staged", Start::Staged, In::M(cancel("13", "nope")), vec![acked("13", UnknownTarget)], 0, same),
            ("a cancel of a finished id leaves the staged set staged", Start::Staged, In::M(cancel("13", "9")), vec![acked("13", AlreadyFinished)], 0, same),
            ("shutdown discards the staged set", Start::Staged, In::M(shutdown("17")), vec![acked("17", Accepted), Err(0)], 0, stopped_discarded),
            ("eof discards the staged set", Start::Staged, In::Eof, vec![Err(0)], 0, stopped_discarded),
        ];
        for (label, from, input, want, jobs, change) in rows {
            let c = start(from);
            let mut after = c.snap();
            change(&mut after);
            let got = match input { In::M(m) => c.msg(m), In::L(l) => c.line(&l), In::Eof => c.eof() };
            assert_eq!(got, want, "{label} ({from:?}): queued");
            let handed = c.handed();
            assert_eq!(handed.len(), jobs, "{label} ({from:?}): jobs handed to the executor");
            assert_eq!(c.snap(), after, "{label} ({from:?}): state");
        }
    }

    /// An admitted job is handed to the executor with exactly the request it was admitted with, no lock set when
    /// none is staged, and the very cancel flag of the live-job record, so a cancel raised through control reaches it.
    #[test]
    fn an_admitted_job_carries_the_live_jobs_cancel_flag() {
        let c = control();
        let msg = solve("flop_cancel", "21");
        let EngineMessage::Solve(req) = msg.clone() else { unreachable!() };
        assert_eq!(c.msg(msg), vec![acked("21", AckStatus::Accepted)]);
        let job = c.handed().pop().expect("a job");
        assert_eq!(job.req, req);
        assert!(job.locks.is_none());
        let p = c.shared.proto.lock().unwrap();
        assert!(Arc::ptr_eq(&job.cancel, &p.live.as_ref().unwrap().cancel), "one flag, shared");
        drop(p);
        assert!(!job.cancel.load(Ordering::SeqCst));
        assert_eq!(c.msg(cancel("22", "21")), vec![acked("22", AckStatus::Accepted)]);
        assert!(job.cancel.load(Ordering::SeqCst), "control's cancel reaches the job");
    }

    // ---- Task 14: lock staging and its consumption by the next solve ----

    /// A solve of the staged spot hands the staged set to the executor with its job, unchanged, and nothing stays
    /// staged: the set belongs to exactly this one solve.
    #[test]
    fn a_solve_of_the_staged_spot_hands_the_staged_set_to_its_job() {
        let c = start(Start::Staged);
        let msg = locked_solve("21");
        let EngineMessage::Solve(req) = msg.clone() else { unreachable!() };
        assert_eq!(c.msg(msg), vec![acked("21", AckStatus::Accepted)]);
        let job = c.handed().pop().expect("a job");
        assert_eq!((job.req, job.locks), (req, Some(river_locks())));
        assert_eq!(c.snap().staged, None);
    }

    /// §4.5: the staged set is consumed by a solve of its own spot. One that does not fit that solve's tree (the
    /// engine's own `proto::worker::validate_locks` against the solve's `tree.materialized` refuses it: a path that
    /// names no decision node, another actor, another menu width) cannot be what the engine meant for this solve:
    /// the solve is accepted and ends `lock_mismatch` at once, with no job handed over, and the set is discarded.
    #[test]
    fn a_staged_set_that_does_not_fit_its_solves_tree_is_a_lock_mismatch() {
        let flop = solve_request("flop_cancel", 0);
        let edits: [(&str, fn(&mut NodeLock)); 3] = [
            ("a path that names no decision node", |l| l.path = vec![proto::Action::Bet { to: 50 }]),
            ("another actor", |l| l.actor = "oop".into()),
            ("another menu width", |l| for row in l.probs.iter_mut() { row.push(0.0) }),
        ];
        for (what, edit) in edits {
            let c = control();
            let mut locks = river_locks();
            edit(&mut locks[0]);
            crate::locks::validate(&locks).unwrap_or_else(|e| panic!("{what}: a valid matrix at staging: {e}"));
            let why = proto::worker::validate_locks(&locks, &flop.tree.materialized).expect_err(what);
            assert_eq!(c.msg(EngineMessage::Lock { id: "20".into(), spot: flop.spot.clone(), locks }), vec![staged("20", false)], "{what}");
            let got = c.msg(solve("flop_cancel", "21"));
            assert_eq!(got, vec![acked("21", AckStatus::Accepted), mismatched("21", &format!("{DOES_NOT_FIT}: {why}"))], "{what}");
            assert_eq!(c.handed().len(), 0, "{what}: no job handed to the executor");
            assert_eq!(c.snap(), Snap { state: WorkerState::Idle, live: None, finished: vec!["21".into()], staged: None, stopping: false }, "{what}");
        }
        // the unedited set fits the flop tree (`[check]` is IP's node there too, with two actions): its job gets it
        let c = control();
        assert_eq!(c.msg(river_lock("20", &flop.spot)), vec![staged("20", false)]);
        assert_eq!(c.msg(solve("flop_cancel", "21")), vec![acked("21", AckStatus::Accepted)]);
        assert_eq!(c.handed().pop().expect("a job").locks, Some(river_locks()));
    }

    /// The staged set's lifecycle across a job's three stages, with this test as the executor (deterministic: the
    /// stage is set by the executor's own progress report, the job ends only when the test ends it). In each stage a
    /// `lock` is rejected `solve_in_progress` and changes nothing; a cancel is accepted and raises the flag of the job
    /// that carried the set; the job's single terminal follows, a second cancel is `already_finished`, and the set
    /// consumed by the cancelled solve is gone: the next solve of the same spot runs with no lock.
    #[test]
    fn the_staged_set_is_gone_once_a_cancelled_solve_consumed_it() {
        for stage in [Stage::Building, Stage::Solving, Stage::Extracting] {
            let c = control();
            let spot = lock_spot();
            assert_eq!(c.msg(river_lock("20", &spot)), vec![staged("20", false)]);
            assert_eq!(c.msg(locked_solve("21")), vec![acked("21", AckStatus::Accepted)]);
            let job = c.handed().pop().expect("a job");
            assert_eq!(job.locks, Some(river_locks()), "{stage:?}");
            progress_to(c.shared.clone(), "21".into())(stage, 1, None, 5, 7);
            assert!(matches!(c.queued().as_slice(), [Ok(WorkerMessage::Progress { id, .. })] if id == "21"), "{stage:?}");
            let during = c.snap();
            assert_eq!(c.msg(river_lock("22", &spot)), vec![refused("22", "solve_in_progress")], "{stage:?}");
            assert_eq!(c.snap(), during, "{stage:?}: a rejected lock changes nothing");
            assert_eq!(c.msg(cancel("23", "21")), vec![acked("23", AckStatus::Accepted)], "{stage:?}");
            assert!(job.cancel.load(Ordering::SeqCst), "{stage:?}: the job that carried the set is cancelled");
            assert_eq!(c.end("21"), vec![cancelled("21")], "{stage:?}");
            assert_eq!(c.msg(cancel("24", "21")), vec![acked("24", AckStatus::AlreadyFinished)], "{stage:?}");
            assert_eq!(c.msg(locked_solve("25")), vec![acked("25", AckStatus::Accepted)], "{stage:?}");
            assert_eq!(c.handed().pop().expect("a job").locks, None, "{stage:?}: the consumed set is gone");
            assert_eq!(c.queued(), vec![], "{stage:?}: exactly one terminal");
        }
    }

    /// A `lock_mismatch` decided at admission is published like every terminal: after the `accepted` ack, under the
    /// same hold of the protocol lock, with the job already retired (its id finished, the worker `Idle`), and no job
    /// handed to the executor at any point.
    #[test]
    fn a_lock_mismatch_is_published_under_the_protocol_lock_with_the_job_retired() {
        let c = control();
        assert_eq!(c.msg(river_lock("20", &lock_spot())), vec![staged("20", false)]);
        let mut log = Vec::new();
        handle_message_via(&c.shared, solve("river_two_combo", "21"), &mut |o| log.push((seen_now(&c.shared, "21"), c.jobs.try_recv().is_ok(), published(&o))));
        let want = vec![
            (Seen::LockHeld, false, format!("{:?}", WorkerMessage::Ack { id: "21".into(), status: AckStatus::Accepted, reason: None, replaced: None })),
            (Seen::LockHeld, false, "result 21 Error".to_string()),
        ];
        assert_eq!(log, want);
        assert_eq!(seen_now(&c.shared, "21"), Seen::Free(WorkerState::Idle, false, true));
        assert!(c.queued().is_empty() && c.handed().is_empty());
    }

    /// Fix round 1 (review I1): `admit` rejects `rake_rate == 1.0` exactly like the shared wire codec's
    /// half-open domain (`proto::numeric::domain_rake_rate`) -- a direct handler rejection, not merely a
    /// `precheck` unit result: the worker's state is unchanged and nothing is handed to the executor.
    #[test]
    fn admission_rejects_a_rake_rate_of_exactly_one() {
        let c = control();
        let before = c.snap();
        assert_eq!(c.msg(solve_with("river_two_combo", "21", |r| r.rake_rate = 1.0)), vec![refused("21", "rake_rate outside [0, 1)")]);
        assert_eq!(c.handed().len(), 0, "no job handed to the executor");
        assert_eq!(c.snap(), before, "no state change");
    }

    /// A cancel never reaches another job: after a cancel of an unknown or finished id, a live job's flag is down.
    #[test]
    fn a_cancel_never_touches_a_different_job() {
        let c = start(Start::Finished9);
        running(&c, "11");
        let flag = c.shared.proto.lock().unwrap().live.as_ref().unwrap().cancel.clone();
        assert_eq!(c.msg(cancel("13", "9")), vec![acked("13", AckStatus::AlreadyFinished)]);
        assert_eq!(c.msg(cancel("14", "nope")), vec![acked("14", AckStatus::UnknownTarget)]);
        assert_eq!(c.msg(cancel("11", "10")), vec![acked("11", AckStatus::UnknownTarget)]);
        assert!(!flag.load(Ordering::SeqCst));
    }

    /// A stop with a live job: the job is cancelled, and its terminal is followed by `Exit(0)`; nothing is admitted in
    /// between, a lock set staged before the stop is discarded, and the live id is remembered as finished afterwards.
    #[test]
    fn a_stop_cancels_the_live_job_and_exits_after_its_terminal() {
        for by_eof in [false, true] {
            let c = start(Start::Live(WorkerState::Solving));
            c.shared.proto.lock().unwrap().staged = Some(("spot".into(), vec![]));
            let acks = if by_eof { c.eof() } else { c.msg(shutdown("17")) };
            assert_eq!(acks, if by_eof { vec![] } else { vec![acked("17", AckStatus::Accepted)] });
            assert_eq!(c.snap(), Snap { state: WorkerState::Stopping, live: Some(("11".into(), true)), finished: vec![], staged: None, stopping: true });
            assert_eq!(c.msg(solve("river_two_combo", "12")), vec![refused("12", "stopping")]);
            assert_eq!(c.end("11"), vec![cancelled("11"), Err(0)], "the terminal, then the exit");
            assert_eq!(c.snap(), Snap { state: WorkerState::Stopping, live: None, finished: vec!["11".into()], staged: None, stopping: true });
            assert_eq!(c.msg(cancel("13", "11")), vec![acked("13", AckStatus::AlreadyFinished)]);
        }
    }

    /// A watchdog spawn that always fails, injected through `Proto::watchdog_spawn` (review I2's seam).
    fn failing_watchdog_spawn(_: Duration, _: SyncSender<Out>) -> io::Result<std::thread::JoinHandle<()>> {
        Err(io::Error::from(io::ErrorKind::Other))
    }

    /// Fix round 1 (review I2): should the watchdog thread itself fail to start, the exit bound it exists to
    /// provide must not be silently lost. `Exit(0)` is queued right away -- behind the shutdown ack, when there
    /// is one -- rather than the worker waiting indefinitely on a job that may never reach a checkpoint, or
    /// control exiting the process directly while still holding the protocol lock. The state still ends exactly
    /// as a successful stop leaves it: `Stopping`, the live job's flag raised, no staged lock.
    #[test]
    fn a_failed_watchdog_spawn_still_queues_the_exit_bound() {
        for by_eof in [false, true] {
            let c = start(Start::Live(WorkerState::Solving));
            c.shared.proto.lock().unwrap().staged = Some(("spot".into(), vec![]));
            c.shared.proto.lock().unwrap().watchdog_spawn = failing_watchdog_spawn;
            let acks = if by_eof { c.eof() } else { c.msg(shutdown("17")) };
            let want = if by_eof { vec![Err(0)] } else { vec![acked("17", AckStatus::Accepted), Err(0)] };
            assert_eq!(acks, want, "by_eof: {by_eof}: exactly an ordered ack/exit");
            assert_eq!(c.snap(), Snap { state: WorkerState::Stopping, live: Some(("11".into(), true)), finished: vec![], staged: None, stopping: true });
        }
    }

    /// A line is parsed then handled as its message; a line that does not parse is rejected with the reason the
    /// parser gave and the id `lenient_id` recovers; a blank line is ignored.
    #[test]
    fn handle_line_parses_or_rejects_with_the_recovered_id() {
        let c = control();
        assert_eq!(c.line(""), vec![]);
        assert_eq!(c.line("  \t "), vec![]);
        let reason_of = |items: Vec<Item>, id: &str| match items.as_slice() {
            [Ok(WorkerMessage::Ack { id: got, status: AckStatus::Rejected, reason: Some(r), replaced: None })] if got == id => r.clone(),
            other => panic!("expected one rejection of {id}: {other:?}"),
        };
        let r = reason_of(c.line(r#"{"type":"bogus","id":"1"}"#), "1");
        assert!(r.starts_with("invalid message: ") && r.contains("bogus"), "{r}");
        let r = reason_of(c.line(r#"{"type":"cancel","id":"2","target":"x","extra":1}"#), "2");
        assert!(r.starts_with("invalid message: ") && r.contains("extra"), "{r}");
        let line = serde_json::to_string(&solve("river_two_combo", "4")).unwrap().replacen("\"pot\":100", "\"pot\":NaN", 1);
        assert!(line.contains("\"pot\":NaN"));
        assert!(reason_of(c.line(&line), "4").starts_with("invalid message: "));
        assert_eq!(c.line(r#"{"id":7,"type":"shutdown"}"#).len(), 1, "one rejection");
        assert_eq!(c.snap(), start(Start::Idle).snap(), "no rejection changed the state");
        assert_eq!(c.line(r#"{"type":"cancel","id":"5","target":"nope"}"#), vec![acked("5", AckStatus::UnknownTarget)]);
        assert_eq!(c.line(&serde_json::to_string(&solve("river_two_combo", "6")).unwrap()), vec![acked("6", AckStatus::Accepted)]);
        assert_eq!(c.handed().len(), 1);
    }

    /// Every ack (and a stop's `Exit(0)`) is queued while control still holds the protocol lock, the lock the executor
    /// needs to publish a terminal (`terminal`, fix round 1 of Task 12): so a cancel's `accepted` always reaches
    /// stdout ahead of the job's terminal (§4.5: "`ack{accepted}`, then `result{cancelled}`"), an `already_finished`
    /// behind it, and a solve's `accepted` ahead of everything its job produces (no job is handed over yet when it is
    /// queued). Deterministic, like the terminal's own test: the probe runs at the instant each item is published.
    #[test]
    fn every_ack_is_queued_under_the_protocol_lock() {
        let rows: [(&str, bool, EngineMessage, usize); 11] = [
            ("cancel accepted", true, cancel("13", "11"), 1),
            ("cancel already_finished", false, cancel("13", "9"), 1),
            ("cancel unknown_target", true, cancel("13", "nope"), 1),
            ("solve accepted", false, solve("river_two_combo", "21"), 1),
            ("solve busy", true, solve("river_two_combo", "21"), 1),
            ("solve rejected by precheck", false, unchecked("21"), 1),
            ("shutdown, live", true, shutdown("17"), 1),
            ("shutdown, idle: the ack, then the exit", false, shutdown("17"), 2),
            ("lock staged", false, river_lock("19", &lock_spot()), 1),
            ("lock rejected by its matrix", false, bad_lock("19"), 1),
            ("lock rejected, solve_in_progress", true, river_lock("19", &lock_spot()), 1),
        ];
        for (label, live, msg, n) in rows {
            let c = control();
            {
                let mut p = c.shared.proto.lock().unwrap();
                p.finished.push_back("9".into());
                if live { p.state = WorkerState::Solving; p.live = Some(LiveJob { id: "11".into(), cancel: Arc::new(AtomicBool::new(false)) }); }
            }
            let mut log = Vec::new();
            handle_message_via(&c.shared, msg, &mut |o| log.push((seen_now(&c.shared, "11"), c.jobs.try_recv().is_ok(), published(&o))));
            assert_eq!(log.len(), n, "{label}: {log:?}");
            for (seen, handed, item) in &log {
                assert_eq!(*seen, Seen::LockHeld, "{label}: {item} queued under the lock");
                assert!(!handed, "{label}: no job handed over before {item}");
            }
            assert!(c.queued().is_empty(), "{label}: everything was published through the seam");
        }
    }

    /// Fix round 1 (review P2.T14-I1): a line the typed decode refuses is rejected under the protocol lock like every
    /// other ack. When it still identifies itself as a `solve`, that same hold discards the staged set: the rejection is
    /// published with the lock held and, once the lock is free, nothing is staged. Unidentifiable input leaves the set.
    #[test]
    fn a_refused_line_is_rejected_under_the_protocol_lock_and_a_named_solve_takes_the_staged_set() {
        let spot = lock_spot();
        let rows: [(&str, String, &str, Option<String>); 4] = [
            ("a solve the wire codec refuses", rake_one("21"), "21", None),
            ("a line that is not JSON", nan_pot("21"), "21", Some(spot.clone())),
            ("a message of an unknown type", wire_with(&locked_solve("21"), |v| v["type"] = serde_json::json!("solves")), "21", Some(spot.clone())),
            ("a lock the wire codec refuses", wire_with(&river_lock("19", &spot), |v| v["locks"][0]["probs"][0] = serde_json::json!([-0.1, 1.1])), "19", Some(spot.clone())),
        ];
        for (label, line, id, left) in rows {
            let c = start(Start::Staged);
            let mut log = Vec::new();
            handle_line_via(&c.shared, &line, &mut |o| log.push((seen_now(&c.shared, "21"), o)));
            let want = match codec_refusal(id, &line) { Ok(m) => m, Err(_) => unreachable!() };
            assert!(matches!(log.as_slice(), [(Seen::LockHeld, Out::Msg(m))] if *m == want), "{label}: one rejection, published under the lock: {:?}", log.iter().map(|(s, o)| (s, published(o))).collect::<Vec<_>>());
            assert_eq!(c.snap().staged, left, "{label}: the staged set once the lock is free");
            assert!(c.queued().is_empty() && c.handed().is_empty(), "{label}");
        }
    }

    /// §4.5: after a stop the process exits 0 within 2 s. A live job that never reaches a checkpoint (here nothing
    /// runs it) is exited past by the watchdog: `Exit(0)` is queued after the production grace, never before it.
    #[test]
    fn the_stop_watchdog_exits_zero_when_a_live_job_never_ends() {
        assert!(STOP_GRACE < Duration::from_secs(2), "the grace leaves the writer time to drain inside §4.5's 2 s");
        let c = start(Start::Live(WorkerState::Solving));
        c.shared.proto.lock().unwrap().stop_grace = STOP_GRACE;
        let t0 = Instant::now();
        handle_eof(&c.shared);
        let item = c.out.recv_timeout(Duration::from_secs(30)).expect("the watchdog's exit (liveness bound)");
        assert!(matches!(item, Out::Exit(0)), "{}", published(&item));
        assert!(t0.elapsed() >= STOP_GRACE, "not before the grace: {:?}", t0.elapsed());
        assert!(c.out.try_recv().is_err(), "a single exit");
    }

    /// The remembered finished ids are bounded: the oldest is forgotten once `FINISHED_IDS` newer ones exist, and is
    /// then an unknown target and admissible again; every remembered id is still a duplicate and already finished.
    #[test]
    fn finished_ids_are_remembered_up_to_the_bound() {
        let c = control();
        for k in 0..=FINISHED_IDS {
            let id = k.to_string();
            {
                let mut p = c.shared.proto.lock().unwrap();
                p.state = WorkerState::Solving;
                p.live = Some(LiveJob { id: id.clone(), cancel: Arc::new(AtomicBool::new(false)) });
            }
            assert_eq!(c.end(&id), vec![cancelled(&id)]);
        }
        let s = c.snap();
        assert_eq!((s.finished.len(), s.finished.first().map(String::as_str), s.finished.last().cloned()), (FINISHED_IDS, Some("1"), Some(FINISHED_IDS.to_string())));
        assert_eq!(c.msg(cancel("x", "1")), vec![acked("x", AckStatus::AlreadyFinished)]);
        assert_eq!(c.msg(solve("river_two_combo", "1")), vec![refused("1", "duplicate")]);
        assert_eq!(c.msg(cancel("x", "0")), vec![acked("x", AckStatus::UnknownTarget)]);
        assert_eq!(c.msg(solve("river_two_combo", "0")), vec![acked("0", AckStatus::Accepted)]);
    }
}
