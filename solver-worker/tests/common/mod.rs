#![allow(dead_code)]
use postflop_solver::{compute_exploitability, solve_step, PostFlopGame};
use proto::worker::{SolveRequest, Stage};
use serde_json::Value;
use solver_worker::extract::NodeSite;
use solver_worker::job::{self, Checkpoint, Hooks, JobControl, JobOutcome, Op};
use solver_worker::protocol::{executor_loop_with, handle_line, Proto, Shared, WorkerState};
use solver_worker::solve_loop::{LoopOutcome, LoopParams, LoopSite};
use solver_worker::writer::Out;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub struct Worker { child: Child, stdin: Option<ChildStdin>, lines: Receiver<String> }

impl Worker {
    pub fn spawn(threads: u8) -> Worker {
        let mut child = Command::new(env!("CARGO_BIN_EXE_solver-worker"))
            .args(["--threads", &threads.to_string()])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .spawn().expect("spawn solver-worker");
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || { for line in BufReader::new(stdout).lines() { match line { Ok(l) => { if tx.send(l).is_err() { break; } } Err(_) => break } } });
        Worker { stdin: child.stdin.take(), child, lines: rx }
    }
    pub fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(line.trim_end().as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }
    pub fn recv(&self, timeout: Duration) -> Option<Value> {
        match self.lines.recv_timeout(timeout) { Ok(l) => Some(serde_json::from_str(&l).unwrap_or_else(|e| panic!("non-JSON stdout line {l:?}: {e}"))), Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None }
    }
    /// Skips messages until `pred` holds; returns None at the deadline.
    pub fn recv_until(&self, timeout: Duration, pred: impl Fn(&Value) -> bool) -> Option<Value> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() { return None; }
            let v = self.recv(left)?;
            if pred(&v) { return Some(v); }
        }
    }
    pub fn close_stdin(&mut self) { self.stdin.take(); }
    pub fn wait_exit(&mut self, timeout: Duration) -> Option<i32> {
        let end = Instant::now() + timeout;
        loop {
            if let Ok(Some(st)) = self.child.try_wait() { return st.code(); }
            if Instant::now() >= end { return None; }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn kill(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}
impl Drop for Worker { fn drop(&mut self) { if self.child.try_wait().ok().flatten().is_none() { self.kill(); } } }

pub fn fixture_lines(name: &str) -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/worker").join(format!("{name}.jsonl"));
    let hint = if name.starts_with("basic_") {
        "run: cargo run --release -p solver-worker --example gen_basic_fixture"
    } else {
        "run: tools/gen_worker_fixtures.py"
    };
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e} ({})", path.display(), hint)).lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}

// ---- In process: the worker's own control handlers and executor, each job's hooks a recording checkpoint barrier ----
//
// The deterministic in-process harness on the job runner's hook seam (`job::Hooks`: public no-op hooks at every job
// checkpoint, around every operation, before every solve step, measurement, node extraction and cancel poll), built
// on the barrier pattern of `tests/locks_and_cancel.rs` (P2.T14 fix round 1, review P2.T14-I2) for the contract
// suites whose checks must not depend on the machine's speed (P2.T16 fix round 1, review P2T16R-I1). Nothing here
// sleeps or reads a clock to place a job: the barrier places it by site, and a timeout is only a liveness bound.

/// A liveness bound on every wait for an in-process job, never a placement.
pub const LIVENESS: Duration = Duration::from_secs(60);

/// A point of a job as its hooks report it, on the job's own thread, immediately before the cancel poll or the work
/// it names: a job checkpoint, an operation starting or completing, a site of the §7 loop (a cancel poll, a solve step
/// or a measurement, with the iterations completed), or a site of the export (a cancel poll or a node's extraction,
/// with the nodes extracted). Every unit of work a job does is one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Site { At(Checkpoint), Enter(Op), Leave(Op), Loop(LoopSite), Node(NodeSite) }

/// The barrier's state: every site the jobs passed, in order; the armed site; whether a job is held there.
#[derive(Default)]
struct Gate { passed: Vec<Site>, armed: Option<Site>, held: bool }

/// The deterministic checkpoint barrier, installed as a job's hooks: it records every site and holds the job at the
/// armed site (once) until the test releases it. Unarmed, it only records. Hooks never run under the protocol lock,
/// so control goes on answering while a job is held.
#[derive(Clone, Default)]
pub struct Barrier(Arc<(Mutex<Gate>, Condvar)>);
impl Barrier {
    pub fn arm(&self, at: Site) {
        let mut g = self.0.0.lock().unwrap();
        assert!(g.armed.is_none() && !g.held, "one hold at a time");
        g.armed = Some(at);
    }
    /// Waits until a job is held at the armed site, and returns the sites passed so far, the held one last.
    pub fn wait_held(&self) -> Vec<Site> {
        let (m, cv) = &*self.0;
        let (g, wait) = cv.wait_timeout_while(m.lock().unwrap(), LIVENESS, |g| !g.held).unwrap();
        assert!(!wait.timed_out(), "no job reached {:?}", g.armed);
        g.passed.clone()
    }
    pub fn release(&self) {
        let (m, cv) = &*self.0;
        m.lock().unwrap().held = false;
        cv.notify_all();
    }
    /// Every site passed so far, in order.
    pub fn passed(&self) -> Vec<Site> { self.0.0.lock().unwrap().passed.clone() }
    fn pass(&self, at: Site) {
        let (m, cv) = &*self.0;
        let mut g = m.lock().unwrap();
        g.passed.push(at);
        if g.armed == Some(at) {
            g.armed = None;
            g.held = true;
            cv.notify_all();
            while g.held { g = cv.wait(g).unwrap(); }
        }
    }
}
impl Hooks for Barrier {
    fn checkpoint(&mut self, at: Checkpoint) { self.pass(Site::At(at)) }
    fn enter(&mut self, op: Op) { self.pass(Site::Enter(op)) }
    fn leave(&mut self, op: Op) { self.pass(Site::Leave(op)) }
    fn loop_site(&mut self, at: LoopSite) { self.pass(Site::Loop(at)) }
    fn node_site(&mut self, at: NodeSite) { self.pass(Site::Node(at)) }
}

/// The operations a job started, in order, from a slice of recorded sites.
pub fn started(sites: &[Site]) -> Vec<Op> { sites.iter().filter_map(|s| match s { Site::Enter(o) => Some(*o), _ => None }).collect() }

/// The worker in process, wired as `main` wires it (the shared state, the executor thread on the jobs channel, `out`
/// for stdout) with a `Barrier` as the executor's hooks. The test is control (`send`) and reads `out` in the writer's
/// place; every message is kept, in order, none dropped (`seen`).
pub struct Harness { pub shared: Arc<Shared>, out: Receiver<Out>, pub barrier: Barrier, pub seen: Vec<Value> }
impl Harness {
    pub fn new() -> Harness {
        let (out_tx, out) = sync_channel::<Out>(4096);
        let (jobs_tx, jobs) = channel();
        let shared = Arc::new(Shared { proto: Mutex::new(Proto::new()), out: out_tx, jobs: jobs_tx });
        let barrier = Barrier::default();
        let (exec, mut hooks) = (Arc::clone(&shared), barrier.clone());
        std::thread::spawn(move || executor_loop_with(exec, jobs, &mut hooks));
        Harness { shared, out, barrier, seen: Vec::new() }
    }
    fn keep(&mut self, o: Out) -> Value {
        let v = match o { Out::Msg(m) => serde_json::to_value(&m).unwrap(), Out::Exit(c) => panic!("unexpected Exit({c})") };
        self.seen.push(v.clone());
        v
    }
    /// One line through control, exactly as `main` hands it over; returns everything queued by the time
    /// `handle_line` returns: its answer (queued under the protocol lock before `handle_line` returns), possibly
    /// followed by the first progress reports of a job it admitted.
    pub fn send(&mut self, line: &str) -> Vec<Value> {
        handle_line(&self.shared, line);
        let queued: Vec<Out> = self.out.try_iter().collect();
        queued.into_iter().map(|o| self.keep(o)).collect()
    }
    /// Everything queued up to and including job `id`'s terminal, within the liveness bound.
    pub fn until_result(&mut self, id: &str) -> Vec<Value> {
        let end = Instant::now() + LIVENESS;
        let mut got = Vec::new();
        loop {
            let o = self.out.recv_timeout(end.saturating_duration_since(Instant::now())).unwrap_or_else(|e| panic!("no result for {id} after {got:?}: {e:?}"));
            let v = self.keep(o);
            let done = v["type"] == "result" && v["id"] == id;
            got.push(v);
            if done { return got; }
        }
    }
    /// One request line and every message it produces, in order, up to and including job `id`'s terminal: a job
    /// that ends before `send` drains the queue (a fast Building failure) has its terminal among `send`'s messages.
    pub fn exchange(&mut self, line: &str, id: &str) -> Vec<Value> {
        let mut got = self.send(line);
        if !got.iter().any(|m| m["type"] == "result" && m["id"] == id) { got.extend(self.until_result(id)); }
        got
    }
    pub fn state(&self) -> WorkerState { self.shared.proto.lock().unwrap().state }
    pub fn staged(&self) -> bool { self.shared.proto.lock().unwrap().staged.is_some() }
}
/// A failed assertion never leaves a job held behind it.
impl Drop for Harness { fn drop(&mut self) { self.barrier.release(); } }

// ---- In process: one job through the job runner's seam on a fixed solve schedule ----

/// One progress report as the job emitted it: stage, iterations, reported exploitability, memory bytes.
pub type Report = (Stage, u32, Option<f32>, u64);

/// What `run_fixed` observed: the job's outcome, every progress report in order, and every raw measurement of the
/// schedule with the iterations completed when it was taken.
pub struct Fixed { pub outcome: JobOutcome, pub reports: Vec<Report>, pub measured: Vec<(u32, f32)> }

/// `req` through the production job (`job::run_scripted`: tree build and cross-check, game configuration, memory
/// admission, allocation, the reporting policy, `finalize`, the street export and self-validation) with a fixed solve
/// schedule in the loop seam's place, so that how far the solve gets never depends on the machine's speed (review
/// P2T16R-I1/I4/I5): exactly `steps` real `solve_step`s, a real `compute_exploitability` after every tenth (§7's
/// cadence whenever the deadline does not bind, and the V1 generator's), each measurement reported through the job's
/// progress, every loop site reported to `hooks` immediately before it, and the cancel flag polled where §7's loop
/// polls it. The outcome's `reached_target` is the last measurement against the request's target (the job's own
/// arithmetic, `LoopParams::target_chips`), so a caller that needs §7's stop point asserts from `measured` that no
/// earlier measurement met it. The job runs on a dedicated rayon pool of `threads` threads: a pinned thread count,
/// as `--threads` pins the spawned worker's. No clock is read and nothing sleeps.
pub fn run_fixed(req: &SolveRequest, threads: usize, steps: u32, hooks: &mut Barrier) -> Fixed {
    assert!(steps > 0 && steps % 10 == 0, "a fixed schedule ends on a measurement: {steps} steps");
    let reports: Arc<Mutex<Vec<Report>>> = Arc::default();
    let log = Arc::clone(&reports);
    let mut ctl = JobControl {
        cancel: Arc::new(AtomicBool::new(false)),
        progress: Box::new(move |stage, iterations, expl, _elapsed_ms, memory_bytes| log.lock().unwrap().push((stage, iterations, expl, memory_bytes))),
    };
    let mut measured: Vec<(u32, f32)> = Vec::new();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("rayon pool");
    let result = pool.install(|| {
        let mut schedule = |game: &PostFlopGame, params: &LoopParams, cancel: &AtomicBool, progress: &mut dyn FnMut(u32, Option<f32>), at_site: &mut dyn FnMut(LoopSite), _clock: &mut dyn FnMut(f64) -> f64| {
            let stop = |iterations: u32, exploitability: Option<f32>| LoopOutcome { iterations, exploitability, reached_target: false, cancelled: true };
            let mut expl = None;
            for i in 0..steps {
                at_site(LoopSite::Boundary(i));
                if cancel.load(Ordering::SeqCst) { return stop(i, expl); }
                at_site(LoopSite::Iteration(i + 1));
                solve_step(game, i);
                at_site(LoopSite::Stepped(i + 1));
                if cancel.load(Ordering::SeqCst) { return stop(i + 1, expl); }
                if (i + 1) % 10 == 0 {
                    at_site(LoopSite::Measurement(i + 1));
                    let e = compute_exploitability(game);
                    measured.push((i + 1, e));
                    expl = Some(e);
                    at_site(LoopSite::Measured(i + 1));
                    if cancel.load(Ordering::SeqCst) { return stop(i + 1, expl); }
                    progress(i + 1, expl);
                }
            }
            LoopOutcome { iterations: steps, exploitability: expl, reached_target: expl.is_some_and(|e| e <= params.target_chips), cancelled: false }
        };
        job::run_scripted(req, None, &mut ctl, hooks, &mut schedule)
    });
    let reports = reports.lock().unwrap().clone();
    Fixed { outcome: result.outcome, reports, measured }
}
