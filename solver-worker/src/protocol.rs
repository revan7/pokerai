//! §4.5's worker side, shared by the three threads of `main`: `control` (reads stdin in every state and
//! answers each line), `executor` (runs one job at a time) and `writer` (the only thread that writes
//! stdout, `crate::writer`). Control reaches a running job only through its `LiveJob::cancel` flag, the
//! same `Arc` the job polls, so a cancel never waits for the executor. Task 12 provides the shared state,
//! the bounded line reader, the executor loop and the terminal; Task 13 replaces the placeholder
//! `handle_line` / `handle_eof` with the §4.5 state machine (admission, ack rules, cancel, shutdown).
use crate::job::{self, JobControl, JobOutcome};
use crate::writer::Out;
use proto::worker::{AckStatus, NodeLock, ResultStatus, SolveRequest, Stage, WorkerMessage};
use std::collections::VecDeque;
use std::io::{self, BufRead};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// §4.5's 1 MiB request-line limit, owned by `proto::worker` (plan 1 Task 7); re-exported, never redefined.
pub use proto::worker::REQUEST_LINE_MAX as MAX_REQUEST_LINE;
const FINISHED_IDS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState { Idle, Building, Solving, Extracting, Stopping }

pub struct LiveJob { pub id: String, pub cancel: Arc<AtomicBool> }
pub struct Job { pub req: SolveRequest, pub locks: Option<Vec<NodeLock>>, pub cancel: Arc<AtomicBool> }
pub struct Proto { pub state: WorkerState, pub live: Option<LiveJob>, pub finished: VecDeque<String>, pub staged: Option<(String, Vec<NodeLock>)>, pub stopping: bool }
pub struct Shared { pub proto: Mutex<Proto>, pub out: SyncSender<Out>, pub jobs: Sender<Job> }

impl Proto { pub fn new() -> Self { Self { state: WorkerState::Idle, live: None, finished: VecDeque::new(), staged: None, stopping: false } } }
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

fn send(shared: &Shared, m: WorkerMessage) { let _ = shared.out.send(Out::Msg(m)); }
fn ack(shared: &Shared, id: &str, status: AckStatus, reason: Option<String>, replaced: Option<bool>) {
    send(shared, WorkerMessage::Ack { id: id.to_string(), status, reason, replaced });
}
fn rejected(shared: &Shared, id: &str, reason: impl Into<String>) { ack(shared, id, AckStatus::Rejected, Some(reason.into()), None); }

/// Task 13 replaces this with the §4.5 state machine.
pub fn handle_line(shared: &Shared, line: &str) {
    if line.trim().is_empty() { return; }
    let id = serde_json::from_str::<serde_json::Value>(line).ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_str().map(String::from))).unwrap_or_else(|| "unknown".into());
    rejected(shared, &id, "not implemented yet");
}
/// stdin EOF: exit 0 without an ack (§4.5). Task 13 replaces this with `begin_stop`.
pub fn handle_eof(shared: &Shared) { let _ = shared.out.send(Out::Exit(0)); }

/// A job's single terminal (§4.5): under the protocol lock the job is retired (its id joins the remembered
/// finished ids) and the worker returns to `Idle` (or stays `Stopping`); only then, still under the lock, is
/// the `result` queued, followed by `Exit(0)` when stopping.
fn terminal(shared: &Shared, id: &str, outcome: JobOutcome, elapsed_ms: u32) {
    terminal_via(shared, id, outcome, elapsed_ms, |o| { let _ = shared.out.send(o); });
}

/// `terminal` with its publication step passed in: `publish` is `shared.out` in production, and a test's
/// probe that observes the protocol lock and state at the instant each item is published.
fn terminal_via(shared: &Shared, id: &str, outcome: JobOutcome, elapsed_ms: u32, mut publish: impl FnMut(Out)) {
    let (status, solution, error) = match outcome {
        JobOutcome::Ok(s) => (ResultStatus::Ok, Some(s), None), JobOutcome::BestSoFar(s) => (ResultStatus::BestSoFar, Some(s), None),
        JobOutcome::Cancelled => (ResultStatus::Cancelled, None, None), JobOutcome::Error(e) => (ResultStatus::Error, None, Some(e)),
    };
    let result = WorkerMessage::Result { id: id.to_string(), status, elapsed_ms, solution, error };
    let mut p = shared.proto.lock().unwrap();
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
    for job in jobs {
        let id = job.req.id.clone();
        let mut ctl = JobControl { cancel: job.cancel.clone(), progress: progress_to(shared.clone(), id.clone()) };
        let started = Instant::now();
        let (outcome, elapsed) = caught(started, || job::run(&job.req, job.locks.as_deref(), &mut ctl));
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
}
