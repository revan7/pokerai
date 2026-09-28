//! The production `WorkerLink`: the `solver-worker` binary as a child process (§3.1, §3.4, §4.5, §10.3, §12).
//!
//! One live process at a time, with three threads of its own (§3.4):
//! - `worker-stdin` writes the queued request lines in order, so `send` never blocks on a worker that stopped
//!   reading; a failed write ends the thread and the next `send` reports the process's state at once: `send` never
//!   waits, its budget for confirming an exit is zero (one non-blocking poll).
//! - `worker-stdout` reads bounded lines (`read_line`: at most `MAX_RESULT_LINE` bytes counting the LF, the worker's
//!   own convention for both of its limits), decodes each into a `WorkerMessage` and queues it for `recv`. A line that
//!   is too long, not UTF-8 or not a message is queued as an error and reading goes on at the next line: a fault is
//!   reported (the caller restarts the worker, §12), never a panic, and never repaired into a different message.
//! - `worker-stderr` drains diagnostics into a ring of the last `STDERR_RING` bytes (`stderr_tail`).
//!
//! Start (`spawn`, `restart`): launch with `--threads N`, place the child in the job object (`job_object`; a failed
//! assignment fails the launch), and wait up to `STARTUP_TIMEOUT` for the first line, which must be a `ready` that
//! `validate_ready` accepts. A launch that fails otherwise (the binary does not start, exits, stays silent, or writes
//! something else first) is killed and retried once, then `Spawn` (§4.5). A `ready` that fails validation is refused
//! at once, without the retry: the same binary reports the same values (§12, "until rebuilt"), and the refusal is
//! returned typed, `ReadyRefused { exe, refusal }`, naming the check that failed and the value reported (follow-up
//! P2.W2). So every `spawn` and every `restart` launches at most twice, each launch bounded by the startup timeout, and
//! any other failure is returned as `Spawn` with every attempt's reason; `restarts()` counts the restarts.
//!
//! End: `recv` reports the end of stdout only after every line before it. One deadline, the caller's `timeout` from
//! the call's start, covers the whole call, the exit confirmation included: the exit is confirmed only within what is
//! left of that budget (a single non-blocking poll when nothing is left), never with a wait of its own. A confirmed
//! exit is `Exit{code}`, and the link keeps giving that answer. An exit not confirmed in time is `Eof` for that call
//! only, never recorded as the link's end: the next `recv` tries again within its own budget (a process can close its
//! stdout and live on; the caller that cannot wait kills it, §7/§12). A line cut off by the end of stdout (a process
//! that died mid-write) is dropped: the death itself is what gets reported. A second `ready` is a protocol error
//! (`ready` is written once, §4.5). `kill` is idempotent: it ends the stdin writer, terminates and reaps the child
//! (bounded), closes the job, waits (bounded) for the three pipe threads to end and joins them (ruling 29-I2), so the
//! stderr ring is complete afterwards and no thread of the link outlives it. Dropping a
//! `ProcessWorker` kills its worker.
//!
//! Win32 here (kernel32): `WaitForSingleObject` on the child's process handle (the budgeted exit confirmation, the
//! bounded reap) and `K32GetProcessMemoryInfo` (`PeakWorkingSetSize`, measured from the engine, which is why the
//! worker has no memory query of its own); `CREATE_NO_WINDOW` as the creation flag, so the console worker never opens
//! a console window under the GUI app. The job object's calls are all in `job_object`.
use super::job_object::{self, JobHandle};
use super::link::{WorkerLink, WorkerLinkError};
use super::ready::{validate_ready, ReadyRefusal};
use proto::worker::{EngineMessage, Ready, WorkerMessage, REQUEST_LINE_MAX};
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// §4.5's result-line limit, owned by `proto::worker`; re-exported, never redefined. It counts the line's bytes
/// with its LF terminator (the worker's `extract::result_line_len` convention).
pub use proto::worker::RESULT_LINE_MAX as MAX_RESULT_LINE;
/// §4.5: the worker's stderr is drained into a 64 KiB ring.
pub const STDERR_RING: usize = 64 << 10;
/// §4.5: `ready` within 5 s of the launch.
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
/// §4.5: a launch that does not reach a valid `ready` is retried once.
pub const START_ATTEMPTS: u32 = 2;
/// Liveness bound on reaping a terminated child; termination is immediate, the bound only keeps `kill` from blocking.
const REAP: Duration = Duration::from_secs(10);
/// Liveness bound on the end of the three pipe threads after the child is gone (its pipes are at EOF and the request
/// queue is closed by then, so they end at once; the bound only keeps `kill` from blocking on a pipe held open elsewhere).
const THREADS_END: Duration = Duration::from_secs(2);
/// How much of an undecodable line a `Protocol` error quotes.
const EXCERPT: usize = 200;
/// The longest single OS wait of the link (final review M1): `clock::WAIT_SLICE_MS`.
const SLICE: Duration = Duration::from_millis(crate::clock::WAIT_SLICE_MS);

type Incoming = Result<WorkerMessage, WorkerLinkError>;

/// The live process and the ends of its three threads' channels.
struct Live {
    child: Child,
    stdin: Sender<Vec<u8>>,
    lines: Receiver<Incoming>,
    /// Disconnects when all three pipe threads have ended (each drops its sender as it ends).
    threads_done: Receiver<()>,
    /// The three pipe threads, joined by `kill` once they have ended (ruling 29-I2: nothing the engine starts is
    /// detached).
    threads: Vec<JoinHandle<()>>,
    /// Held for its `Drop`: closing it kills whatever is still in the job.
    _job: JobHandle,
    /// The confirmed exit code, once `recv` has read every line before the end of stdout and confirmed the exit
    /// within a call's budget. An end whose exit was not confirmed is never recorded (see the module doc).
    exit: Option<i32>,
}

pub struct ProcessWorker {
    exe: PathBuf,
    threads: u8,
    startup: Duration,
    live: Option<Live>,
    ready: Option<Ready>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    restarts: u32,
    /// Unit tests only: a suspend's effect on the link's clock (`seam`).
    #[cfg(test)]
    seam: Arc<seam::Seam>,
}

/// Why one launch failed: `Refused` (a `ready` that fails validation) is final, `Failed` is retried once.
enum Launch { Refused(ReadyRefusal), Failed(WorkerLinkError) }

impl ProcessWorker {
    /// Spawns with `--threads N`, assigns the job object, validates `ready` within 5 s; one retry, then `Spawn`.
    pub fn spawn(exe: &Path, threads: u8) -> Result<ProcessWorker, WorkerLinkError> { Self::spawn_with(exe, threads, STARTUP_TIMEOUT) }

    /// `spawn` with another startup timeout: the seam the startup-timeout test uses (a stand-in that never writes
    /// `ready` fails either way; the seam only keeps the test from waiting 2 x 5 s).
    pub(crate) fn spawn_with(exe: &Path, threads: u8, startup: Duration) -> Result<ProcessWorker, WorkerLinkError> {
        let mut w = ProcessWorker { exe: exe.to_path_buf(), threads, startup, live: None, ready: None, stderr: Arc::new(Mutex::new(VecDeque::new())), restarts: 0,
            #[cfg(test)] seam: Arc::default() };
        w.start()?;
        Ok(w)
    }

    /// How many times `restart` has been called on this link (each call launches at most `START_ATTEMPTS` times).
    pub fn restarts(&self) -> u32 { self.restarts }

    /// At most `START_ATTEMPTS` launches; a refused `ready` ends it at once.
    fn start(&mut self) -> Result<(), WorkerLinkError> {
        let mut failures = Vec::new();
        for attempt in 1..=START_ATTEMPTS {
            match self.launch() {
                Ok(()) => return Ok(()),
                Err(Launch::Refused(refusal)) => {
                    self.kill();
                    return Err(WorkerLinkError::ReadyRefused { exe: self.exe.display().to_string(), refusal });
                }
                Err(Launch::Failed(e)) => { self.kill(); failures.push(format!("attempt {attempt}: {e}")); }
            }
        }
        Err(WorkerLinkError::Spawn(format!("{} did not become ready after {START_ATTEMPTS} attempts ({})", self.exe.display(), failures.join("; "))))
    }

    fn launch(&mut self) -> Result<(), Launch> {
        assert!(self.live.is_none() && self.ready.is_none(), "launch: the previous worker was not killed first");
        let failed = |what: String| Launch::Failed(WorkerLinkError::Spawn(what));
        let mut cmd = Command::new(&self.exe);
        cmd.args(["--threads", &self.threads.to_string()]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        no_console_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| failed(format!("{}: {e}", self.exe.display())))?;
        let job = match job_object::assign(&child) {
            Ok(job) => job,
            Err(e) => { terminate(&mut child); return Err(failed(format!("job object: {e}"))); }
        };
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let pipes = start_threads(stdin, stdout, stderr, self.stderr.clone());
        let (stdin, lines, threads_done, threads) = match pipes {
            Ok(p) => p,
            Err(e) => { terminate(&mut child); return Err(failed(format!("worker threads: {e}"))); }
        };
        self.live = Some(Live { child, stdin, lines, threads_done, threads, _job: job, exit: None });
        match self.recv_any(self.startup) {
            Ok(Some(WorkerMessage::Ready(r))) => {
                validate_ready(&r, self.threads).map_err(Launch::Refused)?;
                self.ready = Some(r);
                Ok(())
            }
            Ok(Some(other)) => Err(Launch::Failed(WorkerLinkError::Protocol(format!("expected ready, got {}", kind(&other))))),
            Ok(None) => Err(Launch::Failed(WorkerLinkError::Protocol(format!("no ready within {} ms", self.startup.as_millis())))),
            Err(e) => Err(Launch::Failed(e)),
        }
    }

    /// The next message of any kind (`ready` included, for `launch`). One deadline, `timeout` from now, governs the
    /// whole call: the wait for a line and, once stdout has ended, the exit confirmation, which gets only what is left
    /// (a single poll when nothing is). A confirmed exit is recorded; an unconfirmed one is `Eof` for this call only.
    ///
    /// Bounded slices (final review M1, `clock`'s "Bounded slices"): the wait for a line is made of OS waits of at most
    /// `WAIT_SLICE_MS`, the deadline compared with the link's clock after each, so a suspend (whose time Windows leaves
    /// out of wait timeouts) makes the call at most one slice late. A line already queued is returned even with a zero
    /// timeout.
    fn recv_any(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        #[cfg(test)]
        let seam = self.seam.clone();
        // The link's monotonic clock; unit tests add a suspend's jump to it.
        let now = || {
            let t = Instant::now();
            #[cfg(test)]
            let t = t + Duration::from_millis(seam.skew_ms.load(std::sync::atomic::Ordering::SeqCst));
            t
        };
        let deadline = now().checked_add(timeout); // `None`: a timeout too large to be a deadline at all
        let Some(live) = self.live.as_mut() else { return Err(WorkerLinkError::Eof) };
        if let Some(code) = live.exit { return Err(WorkerLinkError::Exit { code }); }
        let left = |at: Instant| deadline.map_or(Duration::MAX, |d| d.saturating_duration_since(at));
        loop {
            #[cfg(test)]
            seam.note_wait();
            match live.lines.recv_timeout(left(now()).min(SLICE)) {
                Ok(item) => return item.map(Some),
                Err(RecvTimeoutError::Timeout) => {
                    if left(now()).is_zero() { return Ok(None); }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return match wait_for_exit(&mut live.child, left(now())) {
                        Some(st) => {
                            let code = exit_code(st);
                            live.exit = Some(code);
                            Err(WorkerLinkError::Exit { code })
                        }
                        None => Err(WorkerLinkError::Eof),
                    };
                }
            }
        }
    }
}

impl WorkerLink for ProcessWorker {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> {
        let line = encode_request(msg)?;
        let Some(live) = self.live.as_mut() else { return Err(WorkerLinkError::Eof) };
        if let Some(code) = live.exit { return Err(WorkerLinkError::Exit { code }); }
        if let Ok(Some(st)) = live.child.try_wait() { return Err(WorkerLinkError::Exit { code: exit_code(st) }); }
        // The writer thread only ends at a failed write: the process closed its stdin, so it is exiting or lives on
        // without reading. `send`'s budget for confirming that is zero: one poll, `Exit{code}` if the process has
        // exited, else `Eof`. Neither is recorded in `exit`, which `recv` sets only once it has read every line the
        // process wrote; `recv` confirms an exit this poll missed, within its own budget.
        live.stdin.send(line).map_err(|_| match live.child.try_wait() { Ok(Some(st)) => WorkerLinkError::Exit { code: exit_code(st) }, _ => WorkerLinkError::Eof })
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        match self.recv_any(timeout)? {
            Some(WorkerMessage::Ready(_)) => Err(WorkerLinkError::Protocol("a second ready (ready is written once, §4.5)".into())),
            other => Ok(other),
        }
    }
    fn restart(&mut self) -> Result<(), WorkerLinkError> {
        self.kill();
        self.restarts += 1;
        self.start()
    }
    fn kill(&mut self) {
        self.ready = None;
        let Some(Live { mut child, stdin, lines, threads_done, threads, _job: job, .. }) = self.live.take() else { return };
        drop(stdin);           // the writer thread ends at its next receive (or at its failed write, below)
        terminate(&mut child); // TerminateProcess and a bounded reap
        drop(job);             // the last handle to the job: anything still in it is killed
        drop(lines);           // the reader thread ends at the pipe's EOF, or at its next send
        // The stderr drain has ended too once all three have (the ring is complete then), and they are joined (ruling
        // 29-I2). Only a pipe held open outside the job could outlast the bound, and its thread is then left to end.
        if let Err(RecvTimeoutError::Disconnected) = threads_done.recv_timeout(THREADS_END) {
            for h in threads {
                let _ = h.join();
            }
        }
    }
    fn ready(&self) -> Option<&Ready> { self.ready.as_ref() }
    fn peak_working_set_bytes(&self) -> u64 { self.live.as_ref().map_or(0, |l| peak_ws(&l.child)) }
    /// The last `STDERR_RING` bytes the worker wrote to stderr, across launches (lossy UTF-8): complete for a killed
    /// process, whose drain thread `kill` joins.
    fn stderr_tail(&self) -> String {
        let ring = lock(&self.stderr);
        let (a, b) = ring.as_slices();
        String::from_utf8_lossy(&[a, b].concat()).into_owned()
    }
}

impl Drop for ProcessWorker {
    fn drop(&mut self) { self.kill(); }
}

/// A request as the exact bytes of its line: the JSON object and its LF, within §4.5's 1 MiB request-line limit
/// counted with the LF (the worker's reader counts the same way, and answers an over-limit line with an ack it can
/// only address to "unknown", so the engine never sends one).
pub(crate) fn encode_request(msg: &EngineMessage) -> Result<Vec<u8>, WorkerLinkError> {
    let mut line = serde_json::to_vec(msg).map_err(|e| WorkerLinkError::Protocol(format!("the request does not serialize: {e}")))?;
    assert!(!line.contains(&b'\n'), "serde_json escapes every newline inside a JSON value");
    line.push(b'\n');
    if line.len() > REQUEST_LINE_MAX { return Err(WorkerLinkError::LineTooLong(line.len())); }
    Ok(line)
}

/// The request queue, the decoded lines, the receiver that disconnects once all three threads have ended, and their
/// handles.
type Pipes = (Sender<Vec<u8>>, Receiver<Incoming>, Receiver<()>, Vec<JoinHandle<()>>);

fn start_threads(stdin: impl Write + Send + 'static, stdout: impl Read + Send + 'static, stderr: impl Read + Send + 'static, ring: Arc<Mutex<VecDeque<u8>>>) -> io::Result<Pipes> {
    let (req_tx, req_rx) = channel::<Vec<u8>>();
    let (line_tx, line_rx) = channel::<Incoming>();
    let (done_tx, done_rx) = channel::<()>();
    // Each thread holds a `done` sender, dropped as it ends.
    let (stdin_done, stdout_done) = (done_tx.clone(), done_tx.clone());
    let threads = vec![
        std::thread::Builder::new().name("worker-stdin".into()).spawn(move || { let _done = stdin_done; pump_stdin(stdin, req_rx) })?,
        std::thread::Builder::new().name("worker-stdout".into()).spawn(move || { let _done = stdout_done; pump_stdout(stdout, MAX_RESULT_LINE, line_tx) })?,
        std::thread::Builder::new().name("worker-stderr".into()).spawn(move || drain_stderr(stderr, &ring, done_tx))?,
    ];
    Ok((req_tx, line_rx, done_rx, threads))
}

fn pump_stdin(mut stdin: impl Write, requests: Receiver<Vec<u8>>) {
    for line in requests {
        if stdin.write_all(&line).and_then(|()| stdin.flush()).is_err() { return; }
    }
}

/// Reads `stdout` line by line until its end (or until `recv`'s side is gone), queuing each line decoded.
fn pump_stdout(stdout: impl Read, max: usize, lines: Sender<Incoming>) {
    let mut reader = BufReader::with_capacity(1 << 16, stdout);
    loop {
        let item = match read_line(&mut reader, max) {
            Ok(Some(line)) => decode(line),
            // The end of stdout; a read error on the pipe is its end too. Either way `recv` then confirms the exit.
            Ok(None) | Err(_) => return,
        };
        if lines.send(item).is_err() { return; }
    }
}

/// Keeps the last `STDERR_RING` bytes; `_done` is dropped when the pipe ends, which is what `kill` waits for.
fn drain_stderr(mut stderr: impl Read, ring: &Mutex<VecDeque<u8>>, _done: Sender<()>) {
    let mut buf = [0u8; 4096];
    loop {
        match stderr.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => push_ring(&mut lock(ring), &buf[..n], STDERR_RING),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

fn push_ring(ring: &mut VecDeque<u8>, bytes: &[u8], cap: usize) {
    ring.extend(&bytes[bytes.len().saturating_sub(cap)..]);
    let excess = ring.len().saturating_sub(cap);
    ring.drain(..excess);
}

/// One complete stdout line, classified. The lengths count the terminator.
#[derive(Debug, PartialEq)]
enum Line { Text(String), TooLong(usize), NotUtf8(usize) }

/// Bounded line read: at most `max` bytes counting the LF are buffered; a longer line is consumed to its LF and
/// reported with its full length. A complete line is decoded as checked UTF-8, never repaired. `None` at the end
/// of input, including after an unterminated tail (see the module doc). `Interrupted` reads are retried.
fn read_line(reader: &mut impl BufRead, max: usize) -> io::Result<Option<Line>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut len = 0usize;
    loop {
        let chunk = match reader.fill_buf() { Ok(c) => c, Err(e) if e.kind() == io::ErrorKind::Interrupted => continue, Err(e) => return Err(e) };
        if chunk.is_empty() { return Ok(None); }
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') { Some(i) => (i + 1, true), None => (chunk.len(), false) };
        len = len.saturating_add(take);
        if len <= max { buf.extend_from_slice(&chunk[..take]); } else if buf.capacity() > 0 { buf = Vec::new(); }
        reader.consume(take);
        if done {
            if len > max { return Ok(Some(Line::TooLong(len))); }
            buf.pop();                                   // the LF
            if buf.last() == Some(&b'\r') { buf.pop(); } // a CRLF's CR (ASCII, so never part of a multi-byte sequence)
            return Ok(Some(match String::from_utf8(buf) { Ok(text) => Line::Text(text), Err(_) => Line::NotUtf8(len) }));
        }
    }
}

fn decode(line: Line) -> Incoming {
    match line {
        Line::Text(text) => serde_json::from_str::<WorkerMessage>(&text).map_err(|e| WorkerLinkError::Protocol(format!("{e}: {}", excerpt(&text)))),
        Line::TooLong(len) => Err(WorkerLinkError::LineTooLong(len)),
        Line::NotUtf8(len) => Err(WorkerLinkError::Protocol(format!("a {len}-byte line is not valid UTF-8"))),
    }
}

/// At most `EXCERPT` bytes of `text`, cut at a character boundary.
fn excerpt(text: &str) -> &str {
    let mut end = text.len().min(EXCERPT);
    while !text.is_char_boundary(end) { end -= 1; }
    &text[..end]
}

fn kind(m: &WorkerMessage) -> &'static str {
    match m { WorkerMessage::Ready(_) => "ready", WorkerMessage::Ack { .. } => "ack", WorkerMessage::Progress { .. } => "progress", WorkerMessage::Result { .. } => "result" }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) }

fn exit_code(st: ExitStatus) -> i32 { st.code().unwrap_or(-1) }

/// Terminates the child (a no-op if it has exited) and reaps it within `REAP`.
fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = wait_for_exit(child, REAP);
}

#[cfg(windows)]
fn no_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
fn no_console_window(_cmd: &mut Command) {}

/// The child's exit status if it has exited or exits within `bound`: OS waits of at most `SLICE`, the bound compared
/// with the clock after each (final review M1). A zero bound is one poll.
#[cfg(windows)]
fn wait_for_exit(child: &mut Child, bound: Duration) -> Option<ExitStatus> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" { fn WaitForSingleObject(handle: isize, ms: u32) -> u32; }
    let end = Instant::now().checked_add(bound); // `None`: no bound a clock can reach
    loop {
        let left = end.map_or(SLICE, |e| e.saturating_duration_since(Instant::now()));
        // Fits a u32 (and is never INFINITE): at most one slice.
        let ms = u32::try_from(left.min(SLICE).as_millis()).expect("a slice of milliseconds fits u32");
        // SAFETY: the child's process handle is open for as long as `child` lives; the call only waits on it.
        unsafe { WaitForSingleObject(child.as_raw_handle() as isize, ms); }
        let exited = child.try_wait().ok().flatten();
        if exited.is_some() || end.is_some_and(|e| Instant::now() >= e) {
            return exited;
        }
    }
}
#[cfg(not(windows))]
fn wait_for_exit(child: &mut Child, bound: Duration) -> Option<ExitStatus> {
    let end = std::time::Instant::now() + bound;
    loop {
        if let Ok(Some(st)) = child.try_wait() { return Some(st); }
        if std::time::Instant::now() >= end { return None; }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(windows)]
fn peak_ws(child: &Child) -> u64 {
    use std::os::windows::io::AsRawHandle;
    /// `PROCESS_MEMORY_COUNTERS` (psapi.h).
    #[repr(C)]
    struct Counters { cb: u32, page_fault_count: u32, peak_working_set_size: usize, rest: [usize; 7] }
    #[link(name = "kernel32")]
    extern "system" { fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32; }
    let mut c = Counters { cb: std::mem::size_of::<Counters>() as u32, page_fault_count: 0, peak_working_set_size: 0, rest: [0; 7] };
    // SAFETY: the process handle is open while `child` lives; `c` is a writable PROCESS_MEMORY_COUNTERS of `cb` bytes.
    if unsafe { K32GetProcessMemoryInfo(child.as_raw_handle() as isize, &mut c, c.cb) } == 0 { 0 } else { c.peak_working_set_size as u64 }
}
#[cfg(not(windows))]
fn peak_ws(_child: &Child) -> u64 { 0 }

/// Unit tests only: a suspend as the link sees it (the monotonic clock jumps by `skew_ms`, no OS wait timeout moves and
/// nothing is woken), and an acknowledgement of each OS wait a receive enters.
#[cfg(test)]
pub(crate) mod seam {
    use std::sync::atomic::AtomicU64;
    use std::sync::{Condvar, Mutex};
    use std::time::Instant;

    #[derive(Default)]
    pub(crate) struct Seam {
        pub(crate) skew_ms: AtomicU64,
        waits: Mutex<u64>,
        entered: Condvar,
    }

    impl Seam {
        pub(crate) fn note_wait(&self) {
            *self.waits.lock().unwrap() += 1;
            self.entered.notify_all();
        }

        /// How many waits were entered so far.
        pub(crate) fn waits(&self) -> u64 {
            *self.waits.lock().unwrap()
        }

        /// Blocks until `n` waits were entered; fails after the tests' liveness allowance.
        pub(crate) fn wait_for_waits(&self, n: u64) {
            let deadline = Instant::now() + crate::testing::ACK_LIVENESS;
            let mut waits = self.waits.lock().unwrap();
            while *waits < n {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "ProcessWorker: {n} wait(s) awaited, {} entered, no acknowledgement within the liveness bound", *waits);
                waits = self.entered.wait_timeout(waits, left).unwrap().0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::worker::{AckStatus, NodeLock, ADAPTER_VERSION, PROTO_VERSION, SOLVER_COMMIT};
    use std::io::Cursor;

    fn lines(bytes: &[u8], chunk: usize, max: usize) -> Vec<Line> {
        let mut r = BufReader::with_capacity(chunk, Cursor::new(bytes.to_vec()));
        let mut out = Vec::new();
        while let Some(l) = read_line(&mut r, max).unwrap() { out.push(l); }
        out
    }

    /// The limit counts the LF: a line of exactly `max` bytes with it is read, one byte more is refused with its full
    /// length, and reading resumes at the next line. Small reader chunks put every boundary inside a chunk.
    #[test]
    fn read_line_counts_the_terminator_and_resumes_after_an_over_long_line() {
        let long = format!("{}\n", "x".repeat(100));
        let input = format!("1234567\n12345678\n{long}ok\n");
        for chunk in [1, 3, 8, 64] {
            assert_eq!(lines(input.as_bytes(), chunk, 8), vec![Line::Text("1234567".into()), Line::TooLong(9), Line::TooLong(101), Line::Text("ok".into())], "chunk {chunk}");
        }
    }

    /// A line that is not UTF-8 is classified, not repaired; CRLF is accepted; an unterminated tail at the end of
    /// input is dropped (the process died mid-write, which the exit reports).
    #[test]
    fn read_line_classifies_utf8_crlf_and_the_torn_tail() {
        let input: &[u8] = b"a\xff\"b\n\xc3\xa9\r\nnext\ntorn";
        assert_eq!(lines(input, 2, 64), vec![Line::NotUtf8(5), Line::Text("\u{e9}".into()), Line::Text("next".into())]);
    }

    fn ready_line() -> String {
        serde_json::to_string(&WorkerMessage::Ready(Ready { proto_version: PROTO_VERSION, solver_commit: SOLVER_COMMIT.into(), adapter_version: ADAPTER_VERSION, threads: 4,
            build_features: vec!["avx2".into()], cpu_features: vec!["avx2".into()], capabilities: vec!["solve".into()] })).unwrap()
    }

    /// The stdout pump turns every fault into a queued error and keeps reading: over-long, not UTF-8, not JSON, not a
    /// message; the messages around them arrive intact and in order, and the queue ends with the input.
    #[test]
    fn stdout_faults_are_errors_in_order_never_panics() {
        let mut input = Vec::new();
        input.extend_from_slice(format!("{}\n", ready_line()).as_bytes());
        input.extend_from_slice(format!("{}\n", "y".repeat(2_000)).as_bytes());
        input.extend_from_slice(b"\xfe\xff\n");
        input.extend_from_slice(format!("{}{{not json\n", "\u{e9}".repeat(150)).as_bytes());
        input.extend_from_slice(b"{\"type\":\"ack\",\"id\":\"7\",\"status\":\"accepted\",\"extra\":1}\n");
        input.extend_from_slice(b"{\"type\":\"ack\",\"id\":\"8\",\"status\":\"accepted\"}\n");
        let (tx, rx) = channel();
        pump_stdout(Cursor::new(input), 1_024, tx);
        let got: Vec<Incoming> = rx.try_iter().collect();
        assert_eq!(got.len(), 6, "{got:?}");
        assert!(matches!(&got[0], Ok(WorkerMessage::Ready(r)) if r.threads == 4));
        assert!(matches!(&got[1], Err(WorkerLinkError::LineTooLong(2_001))));
        assert!(matches!(&got[2], Err(WorkerLinkError::Protocol(m)) if m.contains("not valid UTF-8")));
        // 150 two-byte characters: the 200-byte excerpt limit falls on a boundary check, never inside a character
        assert!(matches!(&got[3], Err(WorkerLinkError::Protocol(m)) if m.contains('\u{e9}')));
        assert!(matches!(&got[4], Err(WorkerLinkError::Protocol(m)) if m.contains("extra")), "unknown fields are refused: {:?}", got[4]);
        assert!(matches!(&got[5], Ok(WorkerMessage::Ack { id, status: AckStatus::Accepted, .. }) if id == "8"));
        assert!(rx.try_recv().is_err(), "the pump ended with its input and dropped its sender");
    }

    #[test]
    fn excerpt_cuts_at_a_character_boundary() {
        let s = format!("{}\u{e9}tail", "a".repeat(199));   // the two-byte character spans bytes 199..201
        assert_eq!(excerpt(&s), "a".repeat(199));
        assert_eq!(excerpt("short"), "short");
    }

    /// §4.5's 1 MiB request limit counts the LF: a line of exactly the limit is sent, one byte more is refused.
    #[test]
    fn request_lines_are_bounded_by_the_request_limit_with_their_terminator() {
        let base = encode_request(&EngineMessage::Shutdown { id: String::new() }).unwrap();
        assert_eq!(base.last(), Some(&b'\n'));
        assert_eq!(base.iter().filter(|b| **b == b'\n').count(), 1);
        let fits = EngineMessage::Shutdown { id: "1".repeat(REQUEST_LINE_MAX - base.len()) };
        assert_eq!(encode_request(&fits).unwrap().len(), REQUEST_LINE_MAX);
        let over = EngineMessage::Shutdown { id: "1".repeat(REQUEST_LINE_MAX - base.len() + 1) };
        assert!(matches!(encode_request(&over), Err(WorkerLinkError::LineTooLong(n)) if n == REQUEST_LINE_MAX + 1));
        // a request the checked wire codecs refuse is a protocol error, never a partial line
        let bad = EngineMessage::Lock { id: "1".into(), spot: "s".into(), locks: vec![NodeLock { path: vec![], actor: "oop".into(), probs: vec![vec![f32::NAN]] }] };
        assert!(matches!(encode_request(&bad), Err(WorkerLinkError::Protocol(_))));
    }

    #[test]
    fn the_stderr_ring_keeps_the_last_bytes() {
        let mut ring = VecDeque::new();
        push_ring(&mut ring, b"abcdef", 4);
        assert_eq!(ring.iter().copied().collect::<Vec<u8>>(), b"cdef");
        push_ring(&mut ring, b"gh", 4);
        assert_eq!(ring.iter().copied().collect::<Vec<u8>>(), b"efgh");
        push_ring(&mut ring, &[b'z'; 10], 4);
        assert_eq!(ring.iter().copied().collect::<Vec<u8>>(), b"zzzz");
    }
}

/// The launch, validation and exit paths against stand-in workers: batch files run by `cmd.exe` (std's `Command`
/// runs a `.cmd` file through it). Each launch appends a line to `launches.txt` next to the script before anything
/// else, so a test counts launches exactly; `set /p` blocks on stdin, so a stand-in that reaches it lives until it is
/// killed or sent a line. Every outcome here is decided by what the stand-in writes, never by timing.
#[cfg(all(test, windows))]
mod stand_in_tests {
    use super::*;
    use crate::worker::ready::ReadyRefusal;
    use proto::worker::{ADAPTER_VERSION, PROTO_VERSION, SOLVER_COMMIT};

    struct StandIn { dir: PathBuf, script: PathBuf }
    impl StandIn {
        fn new(tag: &str, body: &[String]) -> StandIn {
            let dir = std::env::temp_dir().join(format!("pokerai-stand-in-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let script = dir.join("worker.cmd");
            let mut text = String::from("@echo off\r\necho x>>\"%~dp0launches.txt\"\r\n");
            for line in body { text.push_str(line); text.push_str("\r\n"); }
            std::fs::write(&script, text).unwrap();
            StandIn { dir, script }
        }
        fn launches(&self) -> usize { std::fs::read_to_string(self.dir.join("launches.txt")).map(|s| s.lines().count()).unwrap_or(0) }
    }
    impl Drop for StandIn { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.dir); } }

    fn valid() -> Ready {
        Ready { proto_version: PROTO_VERSION, solver_commit: SOLVER_COMMIT.into(), adapter_version: ADAPTER_VERSION, threads: 4,
            build_features: vec!["avx2".into()], cpu_features: vec!["avx2".into()], capabilities: vec!["solve".into()] }
    }
    fn echo_ready(r: Ready) -> String { format!("echo {}", serde_json::to_string(&WorkerMessage::Ready(r)).unwrap()) }
    const WAIT: &str = "set /p _=";

    fn spawn_err(s: &StandIn, startup: Duration) -> String {
        match ProcessWorker::spawn_with(&s.script, 4, startup) {
            Err(WorkerLinkError::Spawn(msg)) => msg,
            Err(other) => panic!("expected a spawn error, got {other:?}"),
            Ok(_) => panic!("the stand-in was accepted"),
        }
    }

    /// The engine never trusts a `ready` that fails any §4.5 rule, and does not retry it: one launch, refused with the
    /// typed refusal (follow-up P2.W2) naming the binary, the check that failed and the value the worker reported.
    #[test]
    fn a_ready_failing_any_rule_is_refused_without_a_retry() {
        let cases: [(&str, fn(&mut Ready), ReadyRefusal, &str); 5] = [
            ("proto", |r| r.proto_version = PROTO_VERSION + 1, ReadyRefusal::ProtoVersion { reported: PROTO_VERSION + 1 }, "proto_version"),
            ("commit", |r| r.solver_commit = "deadbeef".into(), ReadyRefusal::SolverCommit { reported: "deadbeef".into() }, "solver commit \"deadbeef\""),
            ("adapter", |r| r.adapter_version = ADAPTER_VERSION + 1, ReadyRefusal::AdapterVersion { reported: ADAPTER_VERSION + 1 }, "adapter_version"),
            ("threads", |r| r.threads = 8, ReadyRefusal::Threads { reported: 8, requested: 4 }, "threads 8 != requested 4"),
            ("avx2", |r| r.build_features = vec!["fma".into()], ReadyRefusal::NoAvx2 { build_features: vec!["fma".into()] }, "worker built without AVX2 (build_features [\"fma\"])"),
        ];
        for (tag, edit, expected, text) in cases {
            let mut r = valid();
            edit(&mut r);
            let s = StandIn::new(&format!("refused-{tag}"), &[echo_ready(r), WAIT.into()]);
            let err = match ProcessWorker::spawn_with(&s.script, 4, STARTUP_TIMEOUT) {
                Err(e) => e,
                Ok(_) => panic!("{tag}: the stand-in was accepted"),
            };
            let msg = err.to_string();
            assert_eq!(msg, format!("{}: ready refused: {}", s.script.display(), expected), "{tag}");
            match err {
                WorkerLinkError::ReadyRefused { exe, refusal } => assert_eq!((exe, refusal), (s.script.display().to_string(), expected), "{tag}"),
                other => panic!("{tag}: expected the typed ready refusal, got {other:?}"),
            }
            assert!(msg.contains(text), "{tag}: {msg}");
            assert_eq!(s.launches(), 1, "{tag}: a refused ready is not retried");
        }
    }

    /// A process that exits before `ready` is retried once, then reported with its confirmed exit code.
    #[test]
    fn an_exit_before_ready_is_retried_once_and_reports_its_code() {
        let s = StandIn::new("exit-early", &["exit /b 7".into()]);
        let msg = spawn_err(&s, STARTUP_TIMEOUT);
        assert!(msg.contains("after 2 attempts") && msg.contains("worker exited with code 7"), "{msg}");
        assert_eq!(s.launches(), 2);
    }

    /// A first line that is not a message, or a message that is not `ready`, is a protocol error: retried once.
    #[test]
    fn a_first_line_other_than_ready_is_retried_once() {
        for (tag, first, expected) in [("garbage", "echo this is not json", "protocol: "), ("ack", r#"echo {"type":"ack","id":"1","status":"accepted"}"#, "expected ready, got ack")] {
            let s = StandIn::new(&format!("first-{tag}"), &[first.into(), WAIT.into()]);
            let msg = spawn_err(&s, STARTUP_TIMEOUT);
            assert!(msg.contains("after 2 attempts") && msg.contains(expected), "{tag}: {msg}");
            assert_eq!(s.launches(), 2, "{tag}");
        }
    }

    /// A silent process is killed at the startup timeout and retried once (the stand-in never writes, so the outcome
    /// does not depend on the timeout's length; the seam only shortens the test).
    ///
    /// The link's count is exact and is what the message asserts. `start` launches at most `START_ATTEMPTS` times,
    /// and every failed launch is killed and reaped before the next, so there is neither a third launch nor overlap.
    /// The message carries one "no ready within 300 ms" per launch, i.e. per process that was spawned and lived to
    /// the timeout.
    ///
    /// The stand-ins' own count (`launches.txt`) is only an upper bound, a race in the observation, not in the link.
    /// Each stand-in counts itself with its first batch line, so the file counts stand-ins whose cmd.exe got that far
    /// before the kill, not processes the link started. On a loaded machine cmd.exe start-up has exceeded 300 ms,
    /// leaving 1. Seen with temporary diagnostics: two spawned pids, both killed and reaped, one line, empty stderr.
    /// With a 20 ms seam: two pids and zero lines in 10 of 10 runs. The other tests' counts stay exact, because what
    /// decides their outcome is written by their stand-ins after that first line.
    #[test]
    fn a_silent_process_times_out_twice() {
        let s = StandIn::new("silent", &[WAIT.into()]);
        let msg = spawn_err(&s, Duration::from_millis(300));
        let timed_out = "protocol: no ready within 300 ms";
        assert!(msg.ends_with(&format!("did not become ready after 2 attempts (attempt 1: {timed_out}; attempt 2: {timed_out})")), "{msg}");
        assert!(s.launches() <= 2, "never a third launch: {}", s.launches());
    }

    /// A live link: the child is in the engine's job; stderr reaches the ring (complete once `kill` returns); a second
    /// `ready` is a protocol error; the link stays usable after it.
    #[test]
    fn a_live_stand_in_is_jobbed_drains_stderr_and_refuses_a_second_ready() {
        let s = StandIn::new("live", &["echo diag-marker-1 1>&2".into(), echo_ready(valid()), echo_ready(valid()), WAIT.into()]);
        let mut w = ProcessWorker::spawn_with(&s.script, 4, STARTUP_TIMEOUT).expect("the stand-in's ready is valid");
        {
            let live = w.live.as_ref().unwrap();
            assert!(live._job.contains(&live.child), "the worker runs in the engine's job");
        }
        match w.recv(STARTUP_TIMEOUT) { Err(WorkerLinkError::Protocol(m)) => assert!(m.contains("second ready"), "{m}"), other => panic!("{other:?}") }
        // the protocol error did not end the link: it still sends (the stand-in is blocked on stdin, so alive)
        w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap();
        w.kill();
        let tail = w.stderr_tail();
        assert!(tail.contains("diag-marker-1"), "{tail:?}");
        assert_eq!(s.launches(), 1);
    }

    /// Final review M1 (spec 12, suspend/resume): Windows excludes suspended time from wait timeouts while the monotonic
    /// clock counts it, so a receive re-reads the clock after every bounded slice instead of trusting one OS timeout.
    /// A live stand-in writes nothing after its `ready`; a receive given ten liveness allowances is inside its OS wait
    /// when the link's clock jumps past its deadline without waking anything (a suspend). The receive notices at its
    /// next slice and returns `Ok(None)`, well within one liveness allowance.
    #[test]
    fn a_receive_rereads_the_clock_after_each_bounded_slice() {
        let s = StandIn::new("suspend", &[echo_ready(valid()), WAIT.into()]);
        let w = ProcessWorker::spawn_with(&s.script, 4, STARTUP_TIMEOUT).unwrap();
        let seam = w.seam.clone();
        let launch_waits = seam.waits(); // the launch's own receives of `ready`
        let span = 10 * crate::testing::ACK_LIVENESS;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut w = w;
            let got = w.recv(span).map(|m| m.is_none());
            let _ = tx.send((got, w));
        });
        seam.wait_for_waits(launch_waits + 1);
        seam.skew_ms.store(u64::try_from(span.as_millis()).unwrap(), std::sync::atomic::Ordering::SeqCst);
        let (got, mut w) = rx.recv_timeout(crate::testing::ACK_LIVENESS).expect("the receive noticed the clock's jump within one slice, not at its OS timeout");
        w.kill();
        assert!(matches!(got, Ok(true)), "the receive timed out with nothing to report: {got:?}");
    }

    /// A worker that exits with a non-zero code mid-session (the writer-fault code 3, say) is reported as
    /// `Exit{code}` by `recv` and then by `send`; a restart relaunches it.
    #[test]
    fn a_non_zero_exit_is_reported_with_its_code_and_restart_relaunches() {
        let s = StandIn::new("exit-3", &[echo_ready(valid()), WAIT.into(), "exit /b 3".into()]);
        let mut w = ProcessWorker::spawn_with(&s.script, 4, STARTUP_TIMEOUT).unwrap();
        w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap();   // releases `set /p`; the stand-in exits 3
        match w.recv(Duration::from_secs(10)) { Err(WorkerLinkError::Exit { code: 3 }) => {}, other => panic!("expected exit 3, got {other:?}") }
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "2".into() }), Err(WorkerLinkError::Exit { code: 3 })));
        w.restart().unwrap();
        assert_eq!((w.restarts(), s.launches(), w.ready().map(|r| r.threads)), (1, 2, Some(4)));
    }

    /// The short receive budget of the timing tests below, and the scheduling allowance on top of it. The allowance is
    /// liveness only (a busy machine wakes a thread late); it is a quarter of the fixed 2 s wait the I1 defect added.
    const SHORT: Duration = Duration::from_millis(10);
    const ALLOWANCE: Duration = Duration::from_millis(500);

    /// `recv(SHORT)`, asserting that the whole call, exit confirmation included, returned within its budget.
    fn recv_short(w: &mut ProcessWorker) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        let t = Instant::now();
        let got = w.recv(SHORT);
        let took = t.elapsed();
        assert!(took <= SHORT + ALLOWANCE, "recv({SHORT:?}) took {took:?} and returned {got:?}");
        got
    }

    /// `recv_short` until it returns something other than a timeout: `Ok(None)` only means the reader thread has not
    /// yet queued the next item. Bounded by a call count, never by a sleep.
    fn next_short(w: &mut ProcessWorker) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        for _ in 0..2_000 {
            match recv_short(w) { Ok(None) => continue, other => return other }
        }
        panic!("2,000 receives of {SHORT:?} saw nothing");
    }

    /// The stand-in `s` with piped stdin and stderr and the given stdout.
    fn spawn_stand_in(s: &StandIn, stdout: Stdio) -> Child {
        let mut cmd = Command::new(&s.script);
        cmd.stdin(Stdio::piped()).stdout(stdout).stderr(Stdio::piped());
        no_console_window(&mut cmd);
        cmd.spawn().unwrap()
    }

    /// A link around `child`, built as `launch` builds one (job, the three threads) minus the `ready` exchange, with
    /// the stdin sink and the stdout source the test chooses.
    fn link_around(s: &StandIn, mut child: Child, stdin: impl Write + Send + 'static, stdout: impl Read + Send + 'static) -> ProcessWorker {
        let job = job_object::assign(&child).unwrap();
        let ring = Arc::new(Mutex::new(VecDeque::new()));
        let (stdin, lines, threads_done, threads) = start_threads(stdin, stdout, child.stderr.take().unwrap(), ring.clone()).unwrap();
        let live = Live { child, stdin, lines, threads_done, threads, _job: job, exit: None };
        ProcessWorker { exe: s.script.clone(), threads: 4, startup: STARTUP_TIMEOUT, live: Some(live), ready: Some(valid()), stderr: ring, restarts: 0, seam: Arc::default() }
    }

    /// A link around the live stand-in `s` whose stdout is a pipe the TEST holds the write end of: the test ends
    /// stdout by dropping the writer, while the child lives on, as a process that closes its stdout does. The
    /// stand-in's own stdout goes nowhere; its stdin is the link's.
    fn link_with_test_stdout(s: &StandIn) -> (ProcessWorker, io::PipeWriter) {
        let (reader, writer) = io::pipe().unwrap();
        let mut child = spawn_stand_in(s, Stdio::null());
        let stdin = child.stdin.take().unwrap();
        (link_around(s, child, stdin, reader), writer)
    }

    /// The link's stdin in `link_with_test_stdin`: the write end of a pipe whose read end the test holds. `_released`
    /// disconnects when the link's writer thread lets go of it, which is only after that thread's request queue is
    /// closed (`pump_stdin`'s `for` loop owns the queue and is left before its `stdin` parameter drops), so from then
    /// on `send` finds the writer gone.
    struct TestStdin { pipe: io::PipeWriter, _released: Sender<()> }
    impl Write for TestStdin {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> { self.pipe.write(buf) }
        fn flush(&mut self) -> io::Result<()> { self.pipe.flush() }
    }

    /// A link around the live stand-in `s` whose stdin is a pipe the TEST holds the read end of: the test breaks the
    /// link's stdin by dropping that reader, while the child lives on, as a process that closes its stdin does. The
    /// child's own stdin is returned as the barrier (`set /p` returns once it is written to); its stdout is the
    /// link's, so its exit ends the link's stdout.
    fn link_with_test_stdin(s: &StandIn) -> (ProcessWorker, io::PipeReader, std::process::ChildStdin, Receiver<()>) {
        let (reader, writer) = io::pipe().unwrap();
        let (released_tx, released) = channel();
        let mut child = spawn_stand_in(s, Stdio::piped());
        let barrier = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        (link_around(s, child, TestStdin { pipe: writer, _released: released_tx }, stdout), reader, barrier, released)
    }

    /// `send`, asserting it returned within its budget: zero (one poll of the process), plus the allowance.
    fn send_timed(w: &mut ProcessWorker, id: &str) -> Result<(), WorkerLinkError> {
        let t = Instant::now();
        let got = w.send(&EngineMessage::Shutdown { id: id.into() });
        let took = t.elapsed();
        assert!(took <= ALLOWANCE, "send took {took:?} and returned {got:?}");
        got
    }

    /// 18-Q1: `send` never waits for the worker. The link's stdin breaks while the child lives on, blocked on its own
    /// stdin (the barrier). The writer thread ends at its first failed write, and the test waits for it to let go of
    /// the pipe (a barrier with a liveness bound, not a sleep). Every `send` after that returns at once with an
    /// unconfirmed `Eof`, which is not kept: once the child is released and exits, `recv` confirms `Exit{5}`, and
    /// `send` answers that confirmed exit too.
    #[test]
    fn a_broken_stdin_is_reported_by_send_at_once_and_an_unconfirmed_exit_is_not_kept() {
        let s = StandIn::new("stdin-breaks-first", &[WAIT.into(), "exit /b 5".into()]);
        let (mut w, stdin_reader, mut barrier, released) = link_with_test_stdin(&s);
        // The pipe carries requests until the test breaks it.
        send_timed(&mut w, "1").unwrap();
        let mut reader = BufReader::new(stdin_reader);
        let mut first = Vec::new();
        reader.read_until(b'\n', &mut first).unwrap();
        assert_eq!(first, encode_request(&EngineMessage::Shutdown { id: "1".into() }).unwrap());
        drop(reader); // the link's stdin is broken; the child is still blocked on `set /p`
        send_timed(&mut w, "2").unwrap(); // queued: the writer thread's write of it fails, and the thread ends
        assert_eq!(released.recv_timeout(Duration::from_secs(10)), Err(RecvTimeoutError::Disconnected), "the writer thread let go of the broken pipe");
        assert!(matches!(send_timed(&mut w, "3"), Err(WorkerLinkError::Eof)), "a writer gone with the exit unconfirmed is Eof");
        assert!(w.live.as_mut().unwrap().child.try_wait().unwrap().is_none(), "the child is alive: the Eof was rightly unconfirmed");
        assert!(matches!(send_timed(&mut w, "4"), Err(WorkerLinkError::Eof)), "still unconfirmed, still at once");
        // Release the barrier: the stand-in reads the line and exits 5; its stdout (the link's) ends with it.
        barrier.write_all(b"go\r\n").unwrap();
        drop(barrier);
        // `recv` confirms the exit (10 s is a liveness bound; the child exits at once), and `send` then answers it.
        match w.recv(Duration::from_secs(10)) { Err(WorkerLinkError::Exit { code: 5 }) => {}, other => panic!("expected the confirmed exit 5, got {other:?}") }
        assert!(matches!(send_timed(&mut w, "5"), Err(WorkerLinkError::Exit { code: 5 })));
    }

    /// I1: one deadline governs the whole receive, exit confirmation included. Stdout ends while the child lives on,
    /// blocked on stdin (the barrier: nothing here is placed by a sleep). Every 10 ms receive returns within its budget
    /// (plus the allowance); the line written before the end arrives first; the end is an unconfirmed `Eof`, and it
    /// is not remembered as the link's final answer: once the child is released and exits, a receive with room
    /// confirms `Exit{5}`, and that confirmed exit is what the link keeps answering.
    #[test]
    fn an_end_of_stdout_is_confirmed_only_within_the_receive_budget_and_an_unconfirmed_one_is_not_kept() {
        let s = StandIn::new("stdout-ends-first", &[WAIT.into(), "exit /b 5".into()]);
        let (mut w, mut stdout) = link_with_test_stdout(&s);
        stdout.write_all(b"{\"type\":\"ack\",\"id\":\"1\",\"status\":\"accepted\"}\n").unwrap();
        drop(stdout); // the end of stdout; the child is still blocked on `set /p`
        assert!(matches!(next_short(&mut w), Ok(Some(WorkerMessage::Ack { id, .. })) if id == "1"), "the line before the end comes first");
        assert!(matches!(next_short(&mut w), Err(WorkerLinkError::Eof)), "an end whose exit is not confirmed within the budget is Eof");
        assert!(w.live.as_mut().unwrap().child.try_wait().unwrap().is_none(), "the child is alive: the Eof was rightly unconfirmed");
        assert!(matches!(recv_short(&mut w), Err(WorkerLinkError::Eof)), "still unconfirmed, still within the budget");
        // Release the barrier: the stand-in reads the line and exits 5. The unconfirmed Eof did not end the link.
        w.send(&EngineMessage::Shutdown { id: "2".into() }).unwrap();
        // A receive with room confirms the exit (10 s is a liveness bound; the child exits at once)...
        match w.recv(Duration::from_secs(10)) { Err(WorkerLinkError::Exit { code: 5 }) => {}, other => panic!("expected the confirmed exit 5, got {other:?}") }
        // ...and the confirmed exit is kept: answered at once by `recv` and by `send`.
        assert!(matches!(recv_short(&mut w), Err(WorkerLinkError::Exit { code: 5 })));
        assert!(matches!(w.send(&EngineMessage::Shutdown { id: "3".into() }), Err(WorkerLinkError::Exit { code: 5 })));
    }

    /// Normal exits keep their confirmed codes (0 after `shutdown`, 3 the worker's writer fault) through both calls,
    /// each within its budget once the process has exited: the test waits for the exit itself (the barrier), then
    /// `send` (budget zero) answers the confirmed `Exit{code}`, every 10 ms receive returns within its budget, the end
    /// of stdout is the confirmed `Exit{code}`, and both calls keep answering it.
    #[test]
    fn a_normal_exit_is_confirmed_with_its_code_by_send_and_recv_within_their_budgets() {
        for code in [0, 3] {
            let s = StandIn::new(&format!("normal-exit-{code}"), &[echo_ready(valid()), WAIT.into(), format!("exit /b {code}")]);
            let mut w = ProcessWorker::spawn_with(&s.script, 4, STARTUP_TIMEOUT).unwrap();
            w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap(); // releases `set /p`
            let st = w.live.as_mut().unwrap().child.wait().unwrap();
            assert_eq!(st.code(), Some(code), "the stand-in exited");
            assert!(matches!(send_timed(&mut w, "2"), Err(WorkerLinkError::Exit { code: c }) if c == code), "code {code}: send confirms the exit");
            match next_short(&mut w) { Err(WorkerLinkError::Exit { code: c }) if c == code => {}, other => panic!("code {code}: expected the confirmed exit, got {other:?}") }
            assert!(matches!(recv_short(&mut w), Err(WorkerLinkError::Exit { code: c }) if c == code), "code {code}: the confirmed exit is kept");
            assert!(matches!(send_timed(&mut w, "3"), Err(WorkerLinkError::Exit { code: c }) if c == code), "code {code}: and send answers it");
        }
    }
}
