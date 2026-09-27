//! §4.5 / §4.6 / §7 worker deadline and memory contracts (P2.T17; fix round 1, review P2.T17-I1/I2). Two kinds of
//! check, kept apart (ruling 17-I1):
//!
//! - **Deterministic status coverage, in process** (`deadline_best_so_far_forced_in_process`,
//!   `deadline_no_iteration_forced_in_process`, `a_status_its_measurement_contradicts_is_refused`): the real control
//!   handlers, executor, job, §7 loop and export, with a checkpoint barrier as the executor's hooks
//!   (`job::Hooks`, the pattern of `locks_and_cancel.rs`). The barrier holds the job at a known site of the §7 loop and
//!   scripts the loop's clock (`Hooks::loop_clock`): time stands still at 0 ms until the test moves it past the
//!   deadline, so the stop point is placed by the test at a site, never by racing the solver against a clock.
//!   `best_so_far` is forced after real iterations and one real measurement, `no_iteration` with no measurement; each
//!   payload is strictly validated and the raw measurement disclosed.
//! - **Smoke checks of the spawned binary** on the committed fixtures (`deadline_best_so_far_bounded`,
//!   `deadline_no_iteration`, `memory_admission`). How much solving fits before a real deadline depends on the machine
//!   and its load (Building shares the absolute deadline; a step or a measurement can be descheduled), so the fixtures'
//!   deadlines are performance conditions here, not correctness gates: every wall-clock bound below is a liveness
//!   allowance, and the solve outcome is accepted on condition of the run's own measurements (`deadline_outcome`).
mod common;
use common::{fixture_lines, meets_raw_target, raw_target_chips, Worker};
use proto::worker::{validate_solution, StreetSolution};
use proto::EffectiveTree;
use serde_json::{json, Value};
use solver_worker::extract::NodeSite;
use solver_worker::job::{Checkpoint, Hooks, Op};
use solver_worker::protocol::{executor_loop_with, handle_line, Proto, Shared, WorkerState};
use solver_worker::solve_loop::LoopSite;
use solver_worker::writer::Out;
use std::sync::mpsc::{channel, sync_channel, Receiver};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const S: Duration = Duration::from_secs(1);
/// The liveness bound on every wait of the deadline tests below, and the allowance on every reported `elapsed_ms`:
/// generous headroom over every observed run, a liveness allowance, never a correctness condition (ruling 17-I1).
/// A request's own `deadline_ms` is the worker's performance target, not a test gate.
const LIVENESS: Duration = Duration::from_secs(60);
/// A reading of the §7 loop's clock past every deadline: `deadline_ms` is a `u32`, so `u32::MAX + 1` ms exceeds any,
/// and the stop rule `elapsed + 1.5 * max_iter + (a due measurement) + margin > deadline` (every term but `elapsed`
/// non-negative) holds at the loop's next boundary whatever the measured step costs.
const PAST_EVERY_DEADLINE_MS: f64 = u32::MAX as f64 + 1.0;

fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn ready(w: &Worker) { assert_eq!(w.recv(5 * S).unwrap()["type"], "ready"); }
/// A fixture's solve line with the §7 fields this file varies.
fn deadline_request(fixture: &str, id: &str, deadline_ms: u32, margin_ms: u32, target_bp: u16) -> String {
    edit(&fixture_lines(fixture)[0], |v| {
        v["id"] = json!(id);
        v["deadline_ms"] = json!(deadline_ms);
        v["extraction_margin_ms"] = json!(margin_ms);
        v["target_bp"] = json!(target_bp);
    })
}

/// Review P2.T17-I2 with follow-up P2.W1: the status boundary is spec 4.4's raw comparison (`common::meets_raw_target`,
/// the engine's exact `f64` form), which the §7 loop now evaluates itself (`solve_loop::meets_target`), never a target
/// narrowed to `f32` (that narrowing can round up past the raw target, 0.6f32 chips for 30 bp of a 200-chip pot). The
/// largest `f32` measurement that still meets `target_bp` of `pot`: the inclusive edge of that comparison.
fn target_edge(pot: u32, target_bp: u16) -> f32 {
    let t = raw_target_chips(pot, target_bp) as f32;
    let edge = if meets_raw_target(t, pot, target_bp) { t } else { f32::from_bits(t.to_bits() - 1) };
    assert!(meets_raw_target(edge, pot, target_bp) && !meets_raw_target(f32::from_bits(edge.to_bits() + 1), pot, target_bp), "{target_bp} bp of {pot}: edge {edge:e}");
    edge
}

/// How a deadline run ended, as its `result` reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Outcome { Ok { expl: f32, iterations: u32 }, BestSoFar { expl: f32, iterations: u32 }, NoIteration }

/// §7's stop-point outcome of one job, checked against the request's own target and the run's own measurements
/// (`msgs`: every message of the job, in order, its `result` last). The conditional acceptance of ruling 17-I1:
/// - `ok` iff the reported measurement meets the raw target (`meets_raw_target`), and `best_so_far` iff it misses it
///   (review P2.T17-I2, follow-up P2.W1): the loop decides on the raw measurement by the same comparison, and the
///   reported one differs from it only where a raw value in the noise band `[-tolerance, 0)` is reported as exactly 0.0
///   (constraints: noise-floor ruling), which meets every target; so the comparison is exact, with no tolerance. Either
///   carries a street export `validate_solution` accepts, and the measurement it reports is the one its `Extracting`
///   progress announced.
/// - `no_iteration` only when no measurement fit before the stop point: no progress of the job carried one, and
///   neither a solution nor a retryable error is reported.
///
/// Anything else (another status or error code) is refused. Diagnostics carry both numbers.
fn deadline_outcome(request: &str, msgs: &[Value]) -> Result<Outcome, String> {
    let req: Value = serde_json::from_str(request).unwrap();
    let id = req["id"].as_str().unwrap();
    let pot = u32::try_from(req["pot"].as_u64().unwrap()).unwrap();
    let target_bp = u16::try_from(req["target_bp"].as_u64().unwrap()).unwrap();
    let r = msgs.last().filter(|m| m["type"] == "result" && m["id"] == id).ok_or_else(|| format!("job {id}: no result at the end of its {} messages", msgs.len()))?;
    let progress: Vec<&Value> = msgs.iter().filter(|m| m["type"] == "progress" && m["id"] == id).collect();
    let status = r["status"].as_str().unwrap_or_default();
    match status {
        "ok" | "best_so_far" => {
            if !r["error"].is_null() { return Err(format!("{status} with an error {}", r["error"])); }
            let sol: StreetSolution = serde_json::from_value(r["solution"].clone()).map_err(|e| format!("{status} without a street solution: {e}"))?;
            let (expl, target) = (sol.exploitability_chips, raw_target_chips(pot, target_bp));
            if !expl.is_finite() { return Err(format!("{status} with exploitability {expl:e} chips")); }
            match (status, meets_raw_target(expl, pot, target_bp)) {
                ("ok", false) => return Err(format!("ok with exploitability {expl:e} chips above the target {target:e} chips ({target_bp} bp of pot {pot})")),
                ("best_so_far", true) => return Err(format!("best_so_far with exploitability {expl:e} chips at or below the target {target:e} chips ({target_bp} bp of pot {pot})")),
                _ => {}
            }
            let last = progress.last().ok_or_else(|| format!("{status} with no progress before it"))?;
            let announced = last["exploitability_chips"].as_f64().map(|e| e as f32);
            if last["stage"] != "extracting" || announced != Some(expl) || last["iterations"] != sol.iterations {
                return Err(format!("{status} reports {expl:e} chips after {} iterations, its last progress was {last}", sol.iterations));
            }
            let tree: EffectiveTree = serde_json::from_value(req["tree"].clone()).unwrap();
            validate_solution(&sol, &tree.materialized).map_err(|e| format!("{status}: the street export is invalid: {e}"))?;
            Ok(if status == "ok" { Outcome::Ok { expl, iterations: sol.iterations } } else { Outcome::BestSoFar { expl, iterations: sol.iterations } })
        }
        "error" => {
            let e = &r["error"];
            if (e["code"].as_str(), e["retryable"].as_bool()) != (Some("no_iteration"), Some(false)) { return Err(format!("error {e}, not a non-retryable no_iteration")); }
            if !r["solution"].is_null() { return Err("no_iteration with a solution".into()); }
            let measured: Vec<&&Value> = progress.iter().filter(|m| !m["exploitability_chips"].is_null()).collect();
            if !measured.is_empty() { return Err(format!("no_iteration although a measurement was reported: {measured:?}")); }
            Ok(Outcome::NoIteration)
        }
        other => Err(format!("status {other:?} (error {})", r["error"])),
    }
}

// ---- In process: the real control handlers and executor, the job held and its clock scripted by a barrier ----

/// A point of a job as the executor's hooks report it (`job::Hooks`), on the job's own thread, immediately before the
/// cancel poll or the work it names: a job checkpoint, an operation starting or completing, a site of the §7 loop (a
/// cancel poll, a solve step or a measurement, with the iterations completed), or a site of the export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Site { At(Checkpoint), Enter(Op), Leave(Op), Loop(LoopSite), Node(NodeSite) }

/// The barrier's state: every site passed, in order; the armed hold (a site predicate, once) and whether a job is held
/// there; the loop's scripted clock (`None`: the real one); the raw measurements the job reported, in order; and a
/// reported measurement substituted for the raw one (`None`: the raw one, as in production).
#[derive(Default)]
struct Gate { passed: Vec<Site>, hold: Option<fn(&Site) -> bool>, held: bool, clock_ms: Option<f64>, raw: Vec<f32>, substitute: Option<f32> }

/// The deterministic checkpoint barrier (the pattern of `locks_and_cancel.rs`), installed as the executor's hooks: it
/// records every site, holds the job at the first site the armed predicate accepts until the test releases it, and
/// answers the §7 loop's clock readings with the scripted time. Placement is by site, never by time. Hooks never run
/// under the protocol lock, so control goes on answering while a job is held.
#[derive(Clone, Default)]
struct Barrier(Arc<(Mutex<Gate>, Condvar)>);
impl Barrier {
    fn gate(&self) -> MutexGuard<'_, Gate> { self.0.0.lock().unwrap() }
    fn arm(&self, hold: fn(&Site) -> bool) {
        let mut g = self.gate();
        assert!(g.hold.is_none() && !g.held, "one hold at a time");
        g.hold = Some(hold);
    }
    /// Waits until a job is held, and returns the sites passed so far, the held one last.
    fn wait_held(&self) -> Vec<Site> {
        let (m, cv) = &*self.0;
        let (g, wait) = cv.wait_timeout_while(m.lock().unwrap(), LIVENESS, |g| !g.held).unwrap();
        assert!(!wait.timed_out(), "no job reached the armed site within {LIVENESS:?}; passed {:?}", g.passed.last());
        g.passed.clone()
    }
    fn release(&self) {
        self.gate().held = false;
        self.0.1.notify_all();
    }
    fn passed(&self) -> Vec<Site> { self.gate().passed.clone() }
    fn set_clock(&self, ms: Option<f64>) { self.gate().clock_ms = ms; }
    fn substitute(&self, reported: Option<f32>) { self.gate().substitute = reported; }
    fn take_raw(&self) -> Vec<f32> { std::mem::take(&mut self.gate().raw) }
    fn pass(&self, at: Site) {
        let (m, cv) = &*self.0;
        let mut g = m.lock().unwrap();
        g.passed.push(at);
        if g.hold.is_some_and(|hold| hold(&at)) {
            g.hold = None;
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
    fn loop_clock(&mut self, real_ms: f64) -> f64 { self.gate().clock_ms.unwrap_or(real_ms) }
    fn measured(&mut self, raw: f32) -> f32 {
        let mut g = self.gate();
        g.raw.push(raw);
        g.substitute.unwrap_or(raw)
    }
}

/// The worker in process, wired as `main` wires it (the shared state, the executor thread on the jobs channel, `out`
/// for stdout) with the barrier as the executor's hooks. The test is control and reads `out` in the writer's place.
struct InProcess { shared: Arc<Shared>, out: Receiver<Out>, barrier: Barrier }
impl InProcess {
    fn new() -> InProcess {
        let (out_tx, out) = sync_channel::<Out>(4096);
        let (jobs_tx, jobs) = channel();
        let shared = Arc::new(Shared { proto: Mutex::new(Proto::new()), out: out_tx, jobs: jobs_tx });
        let barrier = Barrier::default();
        let (exec, mut hooks) = (Arc::clone(&shared), barrier.clone());
        std::thread::spawn(move || executor_loop_with(exec, jobs, &mut hooks));
        InProcess { shared, out, barrier }
    }
    fn value(o: Out) -> Value { match o { Out::Msg(m) => serde_json::to_value(&m).unwrap(), Out::Exit(c) => panic!("unexpected Exit({c})") } }
    /// One line through control, exactly as `main` hands it over; everything queued up to its answer.
    fn send(&self, line: &str) -> Vec<Value> {
        handle_line(&self.shared, line);
        self.out.try_iter().map(InProcess::value).collect()
    }
    /// Everything queued up to and including job `id`'s terminal.
    fn until_result(&self, id: &str) -> Vec<Value> {
        let end = Instant::now() + LIVENESS;
        let mut got = Vec::new();
        loop {
            let o = self.out.recv_timeout(end.saturating_duration_since(Instant::now())).unwrap_or_else(|e| panic!("no result for {id} after {} messages: {e:?}", got.len()));
            let v = InProcess::value(o);
            let done = v["type"] == "result" && v["id"] == id;
            got.push(v);
            if done { return got; }
        }
    }
    fn state(&self) -> WorkerState { self.shared.proto.lock().unwrap().state }
}
/// A failed assertion never leaves a job held, nor solving on behind it: the clock moves past every deadline, so a
/// released loop stops at its next boundary.
impl Drop for InProcess {
    fn drop(&mut self) {
        self.barrier.set_clock(Some(PAST_EVERY_DEADLINE_MS));
        self.barrier.gate().hold = None;
        self.barrier.release();
    }
}

/// One in-process solve whose §7 stop point the test places. The loop's clock reads 0 ms from the start (Building's
/// real duration never reaches the loop); the barrier holds the job at the first site `hold` accepts, where the worker
/// must be `Solving`; the test then sets the loop's clock to `clock_after_ms` and releases the job. Returns every
/// message of the job (its ack first, its result last), the sites it passed, and the index of the held one.
fn run_placed(h: &InProcess, request: &str, hold: fn(&Site) -> bool, clock_after_ms: f64) -> (Vec<Value>, Vec<Site>, usize) {
    let id = serde_json::from_str::<Value>(request).unwrap()["id"].as_str().unwrap().to_string();
    let from = h.barrier.passed().len();
    h.barrier.take_raw();
    h.barrier.set_clock(Some(0.0));
    h.barrier.arm(hold);
    let mut msgs = h.send(request);
    assert_eq!((msgs[0]["type"].as_str(), msgs[0]["id"].as_str(), msgs[0]["status"].as_str()), (Some("ack"), Some(id.as_str()), Some("accepted")), "{id}");
    let held = h.barrier.wait_held().len() - 1 - from;
    assert_eq!(h.state(), WorkerState::Solving, "{id}: the worker's own state while the job is held");
    h.barrier.set_clock(Some(clock_after_ms));
    h.barrier.release();
    msgs.extend(h.until_result(&id));
    (msgs, h.barrier.passed()[from..].to_vec(), held)
}
fn loop_sites(sites: &[Site]) -> Vec<LoopSite> { sites.iter().filter_map(|s| match s { Site::Loop(l) => Some(*l), _ => None }).collect() }
fn is_measured(s: &Site) -> bool { matches!(s, Site::Loop(LoopSite::Measured(_))) }

/// Forced `best_so_far` (ruling 17-I1), deterministic: both spots of `deadline_best_so_far_bounded` at 1 bp and their
/// extraction margins, in process. The loop's clock stands at 0 ms while real `solve_step` iterations run, with
/// `deadline_ms = u32::MAX`: no stop-rule or measurement-budget comparison can end the loop early, whatever Building or
/// a step costs in any run that finishes within the liveness bound (the budget is 49 days of virtual time). The
/// barrier holds the job right after the loop's first real `compute_exploitability` (`LoopSite::Measured(n)`; §7's
/// cadence makes it the tenth iteration), the test moves the loop's clock past every deadline and releases it: the
/// loop's next boundary is its stop point (§7), with a measurement above the target, so `best_so_far` is the only
/// outcome §7 allows, and no further step or measurement runs. The job then finalizes, exports and self-validates.
/// The result reports the raw measurement unchanged (a positive value passes the noise-floor policy as it is),
/// disclosed below; it is a function of the iteration count, not of the clock, and far above 1 bp on both spots. The
/// payload passes `validate_solution`, and the status is consistent with the target the worker computes (I2).
#[test]
fn deadline_best_so_far_forced_in_process() {
    let h = InProcess::new();
    for (fixture, id, margin_ms, pot) in [("basic_turn_std_request", "140", 200, 200), ("flop_best_so_far", "141", 600, 180)] {
        let request = deadline_request(fixture, id, u32::MAX, margin_ms, 1);
        let (msgs, sites, held) = run_placed(&h, &request, is_measured, PAST_EVERY_DEADLINE_MS);
        let Site::Loop(LoopSite::Measured(n)) = sites[held] else { unreachable!("held at {:?}", sites[held]) };
        let before = loop_sites(&sites[..=held]);
        let steps = before.iter().filter(|l| matches!(l, LoopSite::Iteration(_))).count();
        let measurements: Vec<&LoopSite> = before.iter().filter(|l| matches!(l, LoopSite::Measurement(_))).collect();
        assert!(n >= 1 && steps == n as usize && measurements == [&LoopSite::Measurement(n)], "{fixture}: {n} real steps, then one measurement: {before:?}");
        assert_eq!(loop_sites(&sites[held + 1..]), [LoopSite::Boundary(n)], "{fixture}: the scripted deadline ends the loop at its next boundary");
        let raw = h.barrier.take_raw();
        let target = raw_target_chips(pot, 1);
        let outcome = deadline_outcome(&request, &msgs).unwrap_or_else(|e| panic!("{fixture}: {e}"));
        let measured = *raw.last().unwrap_or_else(|| panic!("{fixture}: the job reported no measurement"));
        assert_eq!(outcome, Outcome::BestSoFar { expl: measured, iterations: n }, "{fixture}: raw {raw:?} chips, target {target:e} chips");
        assert!(raw.iter().all(|r| r.to_bits() == measured.to_bits()), "{fixture}: one measurement, reported unchanged: {raw:?}");
        println!("{fixture}: best_so_far forced after {n} real iterations; raw exploitability {measured:e} chips (reported unchanged) above the 1 bp target {target:e} chips of pot {pot}");
    }
}

/// Forced `no_iteration` (ruling 17-I1), deterministic, in process on the turn spot: §7 maps a stop point with no
/// exploitability measurement to `error{no_iteration, retryable: false}`, whether or not an iteration ran.
/// - No iteration fits: margin 600 > deadline 300 (the shape of `deadline_no_iteration`). The loop's first check,
///   `elapsed + 1.5 * 10 (the first-iteration bound) + margin > deadline`, refuses with the clock at 0 ms, so at any
///   reading; the barrier holds the job at that boundary and releases it with the clock still at 0.
/// - One real iteration, then the deadline: with `deadline_ms = u32::MAX` and the clock at 0 the first step is
///   admitted and runs; held right after it (`Stepped(1)`, before any measurement decision), the clock moves past
///   every deadline, so the due measurement no longer fits (`deadline - elapsed - margin < 0 <= the step's cost`) and
///   the next boundary is the stop point.
///
/// Either way nothing is measured, the job ends at `SolveEnded` with nothing finalized or exported, and the result
/// carries no solution.
#[test]
fn deadline_no_iteration_forced_in_process() {
    use LoopSite::{Boundary, Iteration, Stepped};
    let h = InProcess::new();
    let cases: [(&str, u32, u32, fn(&Site) -> bool, f64, Vec<LoopSite>); 2] = [
        ("142", 300, 600, |s| *s == Site::Loop(Boundary(0)), 0.0, vec![Boundary(0)]),
        ("143", u32::MAX, 200, |s| *s == Site::Loop(Stepped(1)), PAST_EVERY_DEADLINE_MS, vec![Boundary(0), Iteration(1), Stepped(1), Boundary(1)]),
    ];
    for (id, deadline_ms, margin_ms, hold, clock_after_ms, loop_trace) in cases {
        let request = deadline_request("basic_turn_std_request", id, deadline_ms, margin_ms, 1);
        let (msgs, sites, held) = run_placed(&h, &request, hold, clock_after_ms);
        assert_eq!(loop_sites(&sites), loop_trace, "{id}: the loop's whole run, with no measurement");
        assert_eq!(sites[held + 1..].iter().filter(|s| !matches!(s, Site::Loop(_))).copied().collect::<Vec<_>>(), [Site::Leave(Op::Solve), Site::At(Checkpoint::SolveEnded)], "{id}: nothing finalized or exported");
        assert_eq!(deadline_outcome(&request, &msgs), Ok(Outcome::NoIteration), "{id}");
        assert_eq!(h.barrier.take_raw(), Vec::<f32>::new(), "{id}: no measurement reached the job");
    }
}

/// Review P2.T17-I2's scenario, produced by the worker itself: a result whose status its reported measurement
/// contradicts, with a payload the strict validator accepts (it sees neither the status nor the target), is refused
/// by `deadline_outcome`, with both numbers in the diagnostic. The job's `measured` hook substitutes the reported
/// measurement while the loop decides on the raw one: `best_so_far` (placed as in the forced test) reported at 0.01
/// chips, below the turn's 1 bp target, and at exactly the target's inclusive edge (`target_edge`); and `ok` (the raw
/// first measurement meets a `u16::MAX` bp target) reported above that target. The positive controls first: the same
/// `ok` run without a substitute is accepted, as `deadline_best_so_far_forced_in_process` accepts its `best_so_far`;
/// and (follow-up P2.W1) a `best_so_far` reported at 0.6f32 chips against 30 bp of the 200-chip pot is accepted, since
/// that `f32`, the old rounded threshold, lies above the raw 0.6-chip target: it misses it.
#[test]
fn a_status_its_measurement_contradicts_is_refused() {
    let h = InProcess::new();
    let one_bp = raw_target_chips(200, 1);
    let all_bp = raw_target_chips(200, u16::MAX);
    let request = deadline_request("basic_turn_std_request", "146", u32::MAX, 200, u16::MAX);
    let (msgs, sites, held) = run_placed(&h, &request, is_measured, 0.0);
    let Site::Loop(LoopSite::Measured(n)) = sites[held] else { unreachable!("held at {:?}", sites[held]) };
    let measured = *h.barrier.take_raw().last().expect("the job reported the loop's measurement");
    assert_eq!(deadline_outcome(&request, &msgs), Ok(Outcome::Ok { expl: measured, iterations: n }), "raw {measured:e} chips, target {all_bp:e} chips");
    let rounded = raw_target_chips(200, 30) as f32;
    assert!(f64::from(rounded) > raw_target_chips(200, 30) && !meets_raw_target(rounded, 200, 30), "0.6f32 is {:e} chips", f64::from(rounded));
    let request = deadline_request("basic_turn_std_request", "148", u32::MAX, 200, 30);
    h.barrier.substitute(Some(rounded));
    let (msgs, sites, held) = run_placed(&h, &request, is_measured, PAST_EVERY_DEADLINE_MS);
    h.barrier.substitute(None);
    let Site::Loop(LoopSite::Measured(n)) = sites[held] else { unreachable!("held at {:?}", sites[held]) };
    let raw = h.barrier.take_raw();
    assert!(!raw.is_empty() && raw.iter().all(|&r| !meets_raw_target(r, 200, 30)), "the loop's own measurement misses 30 bp: {raw:?}");
    assert_eq!(deadline_outcome(&request, &msgs), Ok(Outcome::BestSoFar { expl: rounded, iterations: n }), "raw {raw:?}: best_so_far at the old rounded threshold");
    let cases = [("144", 1, one_bp, 0.01f32, "best_so_far", PAST_EVERY_DEADLINE_MS), ("145", 1, one_bp, target_edge(200, 1), "best_so_far", PAST_EVERY_DEADLINE_MS), ("147", u16::MAX, all_bp, (4.0 * all_bp) as f32, "ok", 0.0)];
    for (id, target_bp, target, reported, status, clock_after_ms) in cases {
        let request = deadline_request("basic_turn_std_request", id, u32::MAX, 200, target_bp);
        h.barrier.substitute(Some(reported));
        let (msgs, _, _) = run_placed(&h, &request, is_measured, clock_after_ms);
        h.barrier.substitute(None);
        let raw = h.barrier.take_raw();
        let r = msgs.last().unwrap();
        assert_eq!(r["status"], status, "{id}: raw {raw:?}, target {target:e}");
        let sol: StreetSolution = serde_json::from_value(r["solution"].clone()).unwrap();
        assert_eq!(sol.exploitability_chips.to_bits(), reported.to_bits(), "{id}");
        let tree: EffectiveTree = serde_json::from_value(serde_json::from_str::<Value>(&request).unwrap()["tree"].clone()).unwrap();
        validate_solution(&sol, &tree.materialized).unwrap_or_else(|e| panic!("{id}: the payload alone is valid: {e}"));
        let refused = deadline_outcome(&request, &msgs).expect_err(&format!("{id}: {status} reported at {reported:e} chips against the target {target:e} chips"));
        assert!(refused.contains(&format!("{reported:e}")) && refused.contains(&format!("{target:e}")), "{id}: both numbers in {refused:?}");
    }
}

// ---- Through the spawned binary: smoke checks with liveness bounds and conditional outcomes ----

/// Every message of job `id`, in order, up to and including its `result`, within the liveness bound (the worker
/// runs one job at a time).
fn job_messages(w: &Worker, id: &str) -> Vec<Value> {
    let end = Instant::now() + LIVENESS;
    let mut got = Vec::new();
    loop {
        let v = w.recv(end.saturating_duration_since(Instant::now())).unwrap_or_else(|| panic!("no result for {id} within the liveness bound {LIVENESS:?} after {} messages", got.len()));
        let done = v["type"] == "result" && v["id"] == id;
        got.push(v);
        if done { return got; }
    }
}
/// The reported `elapsed_ms` against the liveness allowance (ruling 17-I1): the brief's exact bounds (1000 ms,
/// 2000 ms, 300 ms plus one build step) are the worker's performance targets, which a descheduled step, measurement
/// or `finalize` can overrun without any defect, so they are not asserted.
fn within_liveness(r: &Value) -> u64 {
    let elapsed_ms = r["elapsed_ms"].as_u64().unwrap();
    assert!(elapsed_ms <= LIVENESS.as_millis() as u64, "elapsed {elapsed_ms} ms beyond the liveness allowance {LIVENESS:?}");
    elapsed_ms
}

/// §13.2 `deadline_best_so_far_bounded`, a smoke check of the spawned binary: the turn spot (deadline 1000 ms, margin
/// 200 ms) and §13.0's `flop_best_so_far` fixture (deadline 2000 ms, margin 600 ms), each at 1 bp. Whether a
/// measurement fits, and whether it meets 1 bp, depends on this machine's speed and load (Building shares the
/// absolute deadline, and a first iteration can leave too little budget for a measurement), so the outcome is
/// conditional (`deadline_outcome`): `ok` iff the reported measurement meets the target, `best_so_far` iff it is
/// above it, each with a validated street export; `no_iteration` only when no measurement fit. The forced statuses are
/// `deadline_best_so_far_forced_in_process` and `deadline_no_iteration_forced_in_process`. The deadlines stay the
/// brief's; the wall-clock bounds are liveness allowances, and the outcome is printed.
#[test]
fn deadline_best_so_far_bounded() {
    let mut w = Worker::spawn(16);
    ready(&w);
    for (fixture, id, deadline_ms, margin_ms) in [("basic_turn_std_request", "130", 1000, 200), ("flop_best_so_far", "133", 2000, 600)] {
        let request = deadline_request(fixture, id, deadline_ms, margin_ms, 1);
        w.send(&request);
        let msgs = job_messages(&w, id);
        let elapsed_ms = within_liveness(msgs.last().unwrap());
        let outcome = deadline_outcome(&request, &msgs).unwrap_or_else(|e| panic!("{fixture}: {e}"));
        println!("{fixture}: {outcome:?} in {elapsed_ms} ms (deadline {deadline_ms} ms, a performance target, not asserted)");
    }
}

/// §13.2 `deadline_no_iteration`, a smoke check of the spawned binary on the flop spot: `deadline_ms = 300` with
/// `extraction_margin_ms = 600`. Its status is arithmetic, not timing: §7's first check, `elapsed + 1.5 * 10 (the
/// first-iteration bound) + margin > deadline`, already holds at `elapsed = 0` (`solve_loop`'s fake-clock test
/// `declines_all_work_when_even_the_first_iteration_cannot_fit`, and `deadline_no_iteration_forced_in_process`), so no
/// iteration runs, no measurement exists, and the job ends `no_iteration` with no solution. Building still runs first
/// (tree build, cross-check, game configuration, memory check, allocation), so "within 300 ms plus one build step" is
/// a performance target; the bound here is the liveness allowance.
#[test]
fn deadline_no_iteration() {
    let mut w = Worker::spawn(8);
    ready(&w);
    let request = edit(&fixture_lines("flop_cancel")[0], |v| {
        v["id"] = json!("131");
        v["deadline_ms"] = json!(300);
        v["extraction_margin_ms"] = json!(600);
    });
    w.send(&request);
    let msgs = job_messages(&w, "131");
    within_liveness(msgs.last().unwrap());
    assert_eq!(deadline_outcome(&request, &msgs), Ok(Outcome::NoIteration));
    assert!(msgs.iter().filter(|m| m["type"] == "progress").all(|m| m["iterations"] == 0), "no iteration ran: {:?}", msgs.iter().filter(|m| m["type"] == "progress").collect::<Vec<_>>());
}

/// §10.3 memory admission (`memory::admit`) runs in `Building`, after the game is configured and before allocation or
/// solving: a 64 MiB limit is refused against the flop tree's own memory estimate (the R8 FLOP-FAST size class, about
/// 825 MB uncompressed). The estimate is arithmetic over the tree shape, not a measurement of solve progress, so the
/// status does not depend on how fast this machine solves. The job answers `tree_too_large` with `estimate_bytes`
/// above the limit. The 2 s bound below is a liveness allowance, not a correctness condition, and this wire test does
/// not by itself prove that nothing was allocated: that order is the job's own (`job::run_with`, memory check before
/// `Op::Allocate`), pinned by its unit tests.
#[test]
fn memory_admission() {
    let mut w = Worker::spawn(4);
    ready(&w);
    let flop = &fixture_lines("flop_cancel")[0];
    w.send(&edit(flop, |v| {
        v["id"] = json!("132");
        v["memory_limit_bytes"] = json!(64 * 1024 * 1024);
    }));
    let msgs = job_messages(&w, "132");
    let r = msgs.last().unwrap();
    within_liveness(r);
    assert_eq!(
        (r["status"].as_str(), r["error"]["code"].as_str(), r["error"]["retryable"].as_bool()),
        (Some("error"), Some("tree_too_large"), Some(false))
    );
    assert!(r["error"]["estimate_bytes"].as_u64().unwrap() > 64 * 1024 * 1024);
}
