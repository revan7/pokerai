//! §4.5 / §10.3: one `solve` request end to end. Building (tree build, tree cross-check, memory check,
//! allocation, locks) -> Solving (the §7 `solve_step` loop) -> Extracting (`finalize`, the street export,
//! self-validation). A cancel is answered at the next checkpoint of the current stage (§4.5): after every
//! Building step, before the first iteration and at every iteration boundary (the solve loop's own polls),
//! at the Solving -> Extracting boundary, and after every extracted node (the export's own polls). Every
//! failure is a typed §4.5 `WorkerError`, and a returned solution has passed
//! `proto::worker::validate_solution`, which stays strict.
use crate::{cards, extract, locks, memory, solve_loop, tree_build, win};
use postflop_solver::{finalize, CardConfig, PostFlopGame};
use proto::worker::{validate_solution, NodeLock, SolveRequest, Stage, StreetSolution, WorkerError, FAILURE_CODES};
use proto::{index_materialized, resolve_chip_path_indexed, Street};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// The executor's handle on a running job: `cancel` is polled at every checkpoint; `progress` receives
/// `(stage, iterations, exploitability_chips, elapsed_ms, memory_bytes)` at every stage transition and at
/// the solve loop's coalesced iteration boundaries (§4.5), always with the *reported* exploitability.
pub struct JobControl { pub cancel: Arc<AtomicBool>, pub progress: Box<dyn FnMut(Stage, u32, Option<f32>, u32, u64) + Send> }

/// One job's terminal (§4.5 `result`): `Ok` when a measurement reached the target before the stop point,
/// `BestSoFar` when the §7 stop point came first with at least one measurement; both carry a solution that
/// `validate_solution` accepted. No measurement before the stop point is `Error(no_iteration)`.
#[derive(Debug)]
pub enum JobOutcome { Ok(StreetSolution), BestSoFar(StreetSolution), Cancelled, Error(WorkerError) }
#[derive(Debug)]
pub struct JobResult { pub outcome: JobOutcome, pub elapsed_ms: u32 }

/// A §4.5 `WorkerError`. `code` must be one of `proto::worker::FAILURE_CODES` (always-on assert): a
/// misspelt code would otherwise reach the engine as a failure it cannot classify.
pub fn error(code: &str, message: impl Into<String>, retryable: bool, estimate_bytes: Option<u64>) -> WorkerError {
    assert!(FAILURE_CODES.contains(&code), "{code:?} is not a section 4.5 failure code {FAILURE_CODES:?}");
    WorkerError { code: code.into(), message: message.into(), retryable, estimate_bytes }
}

/// Milliseconds since `t` for the `u32` wire fields: a measured duration, not an input, which would only
/// saturate past `u32::MAX` ms (49.7 days).
fn ms(t: Instant) -> u32 { t.elapsed().as_millis().min(u32::MAX as u128) as u32 }

/// §3.4: BELOW_NORMAL while a `background: true` job runs, NORMAL otherwise. Restored on drop, so every
/// exit (success, cancel, error, or a panic unwinding to the executor's boundary) returns the worker to
/// NORMAL, not only the success path.
struct Priority;
impl Priority {
    fn set(background: bool) -> Priority { win::set_priority_class(background); Priority }
}
impl Drop for Priority {
    fn drop(&mut self) { win::set_priority_class(false); }
}

/// The job's own cancel checkpoints: §4.5 "in `Building` after the current step (tree build, cross-check,
/// memory check, allocation, lock application are each a step)", plus one before the first step and one at
/// the Solving -> Extracting boundary, ahead of the uninterruptible `finalize`. The polls at iteration
/// boundaries belong to `solve_loop::run` and those after each node to `extract::street_solution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Checkpoint { Start, TreeBuilt, TreeChecked, GameConfigured, MemoryChecked, Allocated, LockApplied(u16), SolveEnded }

/// Deterministic seams for the tests (standing ruling: a hook places a cancel or a measurement exactly,
/// never a sleep racing the computation). Production (`Stderr`) keeps every default: nothing at a
/// checkpoint, the measurement unchanged, and diagnostics on stderr (§4.5: stdout carries protocol only).
trait Hooks {
    /// Called at each checkpoint, immediately before the cancel flag is polled.
    fn checkpoint(&mut self, _at: Checkpoint) {}
    /// A raw measurement as the solve loop produced it, before it is reported.
    fn measured(&mut self, raw: f32) -> f32 { raw }
    fn log(&mut self, line: &str) { eprintln!("{line}"); }
}
struct Stderr;
impl Hooks for Stderr {}

/// Review ruling (c) of P2.T10 at the job's measurement boundary: every measurement the job emits (in a
/// progress report or the solution metadata) first passes `extract::report_exploitability`. A noise-floor
/// value is emitted as `+0.0` with its raw value on stderr (`Hooks::log`); a value the policy refuses
/// (below the floor, or non-finite) is never emitted: a progress report omits it (its refusal is logged)
/// and, as the final measurement, it fails the job as `internal`. The solve loop repeats its latest
/// measurement with every coalesced progress report, so a repeat of the value just reported is answered
/// from `last` rather than logged again. A refused measurement is in practice also the final one: a NaN
/// persists through later iterations, and a negative value is at or below every target, so the loop
/// stops at it.
#[derive(Default)]
struct Reporter { last: Option<(u32, Result<f32, String>)> }
impl Reporter {
    fn report(&mut self, raw: f32, hooks: &mut dyn Hooks) -> Result<f32, String> {
        if let Some((bits, reported)) = &self.last {
            if *bits == raw.to_bits() { return reported.clone(); }
        }
        let reported = match extract::report_exploitability(raw) {
            Ok(r) => {
                if let Some(line) = &r.log_line { hooks.log(line); }
                Ok(r.chips)
            }
            Err(e) => {
                hooks.log(&format!("exploitability: not reported: {e}"));
                Err(e)
            }
        };
        self.last = Some((raw.to_bits(), reported.clone()));
        reported
    }
}

/// Runs one `solve` (module docs). `staged` is the lock set this request consumes (§4.5; the executor has
/// matched its `spot`). The deadline is the absolute instant `deadline_ms` after entry: Building counts
/// against it (§7).
pub fn run(req: &SolveRequest, staged: Option<&[NodeLock]>, ctl: &mut JobControl) -> JobResult { run_with(req, staged, ctl, &mut Stderr) }

fn run_with(req: &SolveRequest, staged: Option<&[NodeLock]>, ctl: &mut JobControl, hooks: &mut dyn Hooks) -> JobResult {
    let t0 = Instant::now();
    let _priority = Priority::set(req.background);
    let done = |o: JobOutcome| JobResult { outcome: o, elapsed_ms: ms(t0) };
    macro_rules! checkpoint {
        ($at:expr) => {{
            hooks.checkpoint($at);
            if ctl.cancel.load(Ordering::SeqCst) { return done(JobOutcome::Cancelled); }
        }};
    }
    let invalid = |m: String| JobOutcome::Error(error("invalid_request", m, false, None));
    let lock_mismatch = |m: String| JobOutcome::Error(error("lock_mismatch", m, false, None));
    let internal = |m: String| JobOutcome::Error(error("internal", m, true, None));

    // Building: tree, cross-check, game, memory check, allocation, locks.
    (ctl.progress)(Stage::Building, 0, None, ms(t0), 0);
    checkpoint!(Checkpoint::Start);
    let eff = req.stack_oop.min(req.stack_ip);
    let mut tree = match tree_build::build(&req.tree, req.pot, eff, req.rake_rate, req.rake_cap_mchips, &req.history) { Ok(t) => t, Err(e) => return done(invalid(e)) };
    checkpoint!(Checkpoint::TreeBuilt);
    let lib = match tree_build::enumerate(&mut tree, req.tree.root_street, req.pot) { Ok(l) => l, Err(e) => return done(invalid(e)) };
    if let Err(e) = tree_build::cross_check(&lib, &req.tree.materialized) { return done(JobOutcome::Error(error("tree_mismatch", e, false, None))); }
    checkpoint!(Checkpoint::TreeChecked);
    let street_nodes = req.tree.materialized.iter().filter(|n| n.street == req.tree.root_street).count();
    if street_nodes > extract::MAX_EXPORTED_NODES { return done(invalid(format!("{street_nodes} street nodes exceed the export limit"))); }
    // The history must name a decision node of the solved street: the node the export reports as
    // `requested`. The library accepts a history that ends at a terminal or runs past the street's
    // chance deal, which would otherwise surface only after the whole solve, as a retryable `internal`.
    let index = index_materialized(&req.tree.materialized);
    match resolve_chip_path_indexed(&index, &req.history).and_then(|o| index.get(o.as_slice()).copied()) {
        Some(node) if node.street == req.tree.root_street => {}
        _ => return done(invalid(format!("history {:?} is not a decision node of the {:?} street", req.history, req.tree.root_street))),
    }
    let expected_cards = match req.tree.root_street { Street::Flop => 3, Street::Turn => 4, Street::River => 5, Street::Preflop => 0 };
    if req.board.len() != expected_cards { return done(invalid(format!("{} board cards for a {:?} root", req.board.len(), req.tree.root_street))); }
    let (flop, turn, river) = match cards::board_to_lib(&req.board) { Ok(b) => b, Err(e) => return done(invalid(e)) };
    let ranges = match (cards::range_to_lib(&req.oop_range), cards::range_to_lib(&req.ip_range)) { (Ok(o), Ok(i)) => [o, i], (Err(e), _) | (_, Err(e)) => return done(invalid(e)) };
    let mut game = match PostFlopGame::with_config(CardConfig { range: ranges, flop, turn, river }, tree) { Ok(g) => g, Err(e) => return done(invalid(e)) };
    checkpoint!(Checkpoint::GameConfigured);
    let (f32_bytes, i16_bytes) = game.memory_usage();
    let adm = match memory::admit(f32_bytes, i16_bytes, req.memory_limit_bytes) { Ok(a) => a, Err(e) => return done(JobOutcome::Error(e)) };
    (ctl.progress)(Stage::Building, 0, None, ms(t0), adm.estimate_bytes);
    checkpoint!(Checkpoint::MemoryChecked);
    game.allocate_memory(adm.compressed);
    checkpoint!(Checkpoint::Allocated);
    let mut locks_applied = 0u16;
    if let Some(ls) = staged {
        // Validated at the protocol boundary already; checked again because `run` is public and
        // `locks_applied` must count distinct locked nodes: two locks naming one node would both apply
        // while only the last takes effect.
        if let Err(e) = locks::validate(ls) { return done(lock_mismatch(e)); }
        if u16::try_from(ls.len()).is_err() { return done(lock_mismatch(format!("{} staged locks exceed the u16 locks_applied count", ls.len()))); }
        for l in ls {
            // `apply` refuses a lock that would lock nothing (P2.T10 review I2): only a lock that took
            // effect is counted, and the length check above bounds the count.
            if let Err(e) = locks::apply(&mut game, l, &req.tree.materialized) { return done(lock_mismatch(e)); }
            locks_applied += 1;
            checkpoint!(Checkpoint::LockApplied(locks_applied));
        }
    }

    // Solving: the §7 loop polls the cancel flag at every iteration boundary.
    (ctl.progress)(Stage::Solving, 0, None, ms(t0), adm.estimate_bytes);
    let target_chips = (f64::from(req.pot) * f64::from(req.target_bp) / 10_000.0) as f32;
    let params = solve_loop::LoopParams { deadline_ms: req.deadline_ms, extraction_margin_ms: req.extraction_margin_ms, target_chips, started: t0 };
    let cancel = Arc::clone(&ctl.cancel);
    let mut reporter = Reporter::default();
    let out = {
        let (progress, reporter, hooks) = (&mut ctl.progress, &mut reporter, &mut *hooks);
        solve_loop::run(&game, &params, &cancel, |iterations, raw| {
            let reported = match raw {
                None => None,
                Some(raw) => match reporter.report(hooks.measured(raw), &mut *hooks) { Ok(chips) => Some(chips), Err(_) => return },
            };
            progress(Stage::Solving, iterations, reported, ms(t0), adm.estimate_bytes);
        })
    };
    if out.cancelled { return done(JobOutcome::Cancelled); }
    checkpoint!(Checkpoint::SolveEnded);
    let Some(raw) = out.exploitability else { return done(JobOutcome::Error(error("no_iteration", "no exploitability measurement before stop point", false, None))); };
    let expl = match reporter.report(hooks.measured(raw), &mut *hooks) { Ok(chips) => chips, Err(e) => return done(internal(e)) };

    // Extracting: `finalize` is not interruptible; the export polls after every node.
    (ctl.progress)(Stage::Extracting, out.iterations, Some(expl), ms(t0), adm.estimate_bytes);
    finalize(&mut game);
    let meta = extract::SolutionMeta { exploitability_chips: expl, iterations: out.iterations, memory_bytes: adm.estimate_bytes, mode: adm.mode, locks_applied };
    let sol = match extract::street_solution(&mut game, req, meta, &ctl.cancel) { Ok(Some(s)) => s, Ok(None) => return done(JobOutcome::Cancelled), Err(e) => return done(internal(e)) };
    // The last gate before a solution leaves the worker, and a strict one (a raw negative fails it).
    if let Err(e) = validate_solution(&sol, &req.tree.materialized) { return done(internal(format!("self-validation failed: {e}"))); }
    done(if out.reached_target { JobOutcome::Ok(sol) } else { JobOutcome::BestSoFar(sol) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::solve_request;
    use proto::worker::validate_solution;
    use proto::{combo_index, Card};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn ctl() -> JobControl { JobControl { cancel: Arc::new(AtomicBool::new(false)), progress: Box::new(|_, _, _, _, _| {}) } }
    fn combo(a: &str, b: &str) -> usize { combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize }

    #[test]
    fn river_two_combo_solves_to_the_analytic_solution() {
        let req = solve_request("river_two_combo", 0);
        let r = run(&req, None, &mut ctl());
        let sol = match r.outcome { JobOutcome::Ok(s) => s, other => panic!("{other:?}") };
        // every decision node of the street is exported: [] oop, [check] ip, [check, allin] oop; the requested node is the one reached by history
        assert_eq!((sol.nodes.len(), sol.requested, sol.mode.as_str(), sol.export.as_str()), (3, 1, "f32", "street"));
        assert!(sol.exploitability_chips <= 0.1, "exploitability {}", sol.exploitability_chips);
        validate_solution(&sol, &req.tree.materialized).unwrap();
        let ip = &sol.nodes[1];
        assert_eq!(ip.actor, "ip");
        for qq in [combo("Qc", "Qd"), combo("Qc", "Qh"), combo("Qd", "Qh")] { assert!(ip.available[qq] && ip.probs[qq][1] > 0.97, "QQ bets: {:?}", ip.probs[qq]); }
        let bluff = ip.probs[combo("5c", "4d")][1];
        assert!((bluff - 0.5).abs() <= 0.03, "54o bluffs {bluff}");
        let oop = &sol.nodes[2];
        let call = oop.probs[combo("Ac", "Ad")][1];
        assert!((call - 0.5).abs() <= 0.03, "AA calls {call}");
        assert_eq!(oop.ev_chips[combo("Ac", "Ad")][0], 0.0);                  // fold = 0 by the identity of §10.3
        assert!(!ip.available[combo("Ac", "Ad")] && ip.probs[combo("Ac", "Ad")] == vec![0.0, 0.0]);
    }

    #[test]
    fn cancel_before_building_and_no_iteration() {
        let req = solve_request("river_two_combo", 0);
        let mut c = ctl();
        c.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(run(&req, None, &mut c).outcome, JobOutcome::Cancelled));
        let mut short = req.clone();
        short.deadline_ms = 300; short.extraction_margin_ms = 600;
        match run(&short, None, &mut ctl()).outcome { JobOutcome::Error(e) => assert_eq!((e.code.as_str(), e.retryable), ("no_iteration", false)), other => panic!("{other:?}") }
    }

    // ---- Beyond the brief's two tests: the checkpoints, the exploitability policy, locks and error codes ----

    use crate::testutil::fixture_lines;
    use proto::worker::EngineMessage;
    use proto::Action;
    use std::sync::Mutex;

    const CHECK: Action = Action::Check;
    const JAM: Action = Action::AllIn { to: 100 };

    /// One progress report as the job emitted it: stage, iterations, exploitability, memory bytes.
    type Report = (Stage, u32, Option<f32>, u64);
    type Reports = Arc<Mutex<Vec<Report>>>;

    /// A control whose progress callback records every report and raises the job's own cancel flag on
    /// the first report `cancel_on` accepts: the callback runs synchronously inside the job, so this
    /// places a cancel exactly between two steps with no thread and no sleep.
    fn recording(cancel_on: impl Fn(&Report) -> bool + Send + 'static) -> (JobControl, Reports) {
        let cancel = Arc::new(AtomicBool::new(false));
        let reports: Reports = Arc::default();
        let (flag, log) = (cancel.clone(), reports.clone());
        let progress = Box::new(move |stage, iterations, exploitability, _elapsed_ms, memory_bytes| {
            let report = (stage, iterations, exploitability, memory_bytes);
            if cancel_on(&report) { flag.store(true, Ordering::SeqCst); }
            log.lock().unwrap().push(report);
        });
        (JobControl { cancel, progress }, reports)
    }
    fn stages(reports: &Reports) -> Vec<Stage> { reports.lock().unwrap().iter().map(|r| r.0).collect() }

    /// The job's deterministic seams: records every checkpoint, raises the cancel flag at one of them,
    /// substitutes the raw exploitability measurement, and keeps the stderr lines.
    struct Probe { cancel: Arc<AtomicBool>, cancel_at: Option<Checkpoint>, substitute: Option<f32>, checkpoints: Vec<Checkpoint>, lines: Vec<String> }
    impl Probe {
        fn new(c: &JobControl) -> Probe { Probe { cancel: c.cancel.clone(), cancel_at: None, substitute: None, checkpoints: Vec::new(), lines: Vec::new() } }
    }
    impl Hooks for Probe {
        fn checkpoint(&mut self, at: Checkpoint) {
            self.checkpoints.push(at);
            if self.cancel_at == Some(at) { self.cancel.store(true, Ordering::SeqCst); }
        }
        fn measured(&mut self, raw: f32) -> f32 { self.substitute.unwrap_or(raw) }
        fn log(&mut self, line: &str) { self.lines.push(line.to_string()); }
    }

    /// `lock_river`: line 0 stages IP's lock at `[check]` (QQ bets always, 54o one time in five), line 1
    /// is the solve it belongs to (OOP holds QQ and 66).
    fn lock_job() -> (SolveRequest, Vec<NodeLock>) {
        let locks = match serde_json::from_str::<EngineMessage>(&fixture_lines("lock_river")[0]).expect("parse lock line") {
            EngineMessage::Lock { locks, .. } => locks,
            other => panic!("line 0 of lock_river is not a lock: {other:?}"),
        };
        (solve_request("lock_river", 1), locks)
    }

    /// §4.5 `cancel` in `Building`: answered "after the current step (tree build, cross-check, memory
    /// check, allocation, lock application are each a step)"; the job also polls before its first step
    /// and at the Solving -> Extracting boundary, so a cancel that lands after the last iteration never
    /// pays for `finalize`. The checkpoints come in this order, and a cancel raised at any one of them is
    /// answered there: nothing past it runs (no later checkpoint, no later stage).
    #[test]
    fn a_cancel_at_every_building_step_and_stage_boundary_stops_the_job_there() {
        use Checkpoint::*;
        let (mut req, locks) = lock_job();
        req.target_bp = u16::MAX;                        // the first measurement reaches it: a short uncancelled run
        let (mut c, reports) = recording(|_| false);
        let mut probe = Probe::new(&c);
        let full = run_with(&req, Some(&locks), &mut c, &mut probe);
        assert!(matches!(full.outcome, JobOutcome::Ok(_)), "{:?}", full.outcome);
        let all = probe.checkpoints;
        assert_eq!(all, [Start, TreeBuilt, TreeChecked, GameConfigured, MemoryChecked, Allocated, LockApplied(1), SolveEnded]);
        assert_eq!(stages(&reports).last(), Some(&Stage::Extracting));

        for (k, &at) in all.iter().enumerate() {
            let (mut c, reports) = recording(|_| false);
            let mut probe = Probe::new(&c);
            probe.cancel_at = Some(at);
            let r = run_with(&req, Some(&locks), &mut c, &mut probe);
            assert!(matches!(r.outcome, JobOutcome::Cancelled), "cancel at {at:?}: {:?}", r.outcome);
            assert_eq!(probe.checkpoints, all[..=k], "cancel at {at:?}: no step after it");
            let seen = stages(&reports);
            match at {
                Start | TreeBuilt | TreeChecked | GameConfigured => assert_eq!(seen, [Stage::Building], "cancel at {at:?}"),
                MemoryChecked | Allocated | LockApplied(_) => assert_eq!(seen, [Stage::Building, Stage::Building], "cancel at {at:?}"),
                SolveEnded => assert!(seen.contains(&Stage::Solving) && !seen.contains(&Stage::Extracting), "cancel at {at:?}: {seen:?}"),
            }
        }
        // the second Building report is the memory check: it carries the admitted estimate
        let (mut c, reports) = recording(|_| false);
        let mut probe = Probe::new(&c);
        probe.cancel_at = Some(MemoryChecked);
        run_with(&req, Some(&locks), &mut c, &mut probe);
        let r = reports.lock().unwrap();
        assert_eq!((r[0].3, r[1].2), (0, None));
        assert!(r[1].3 > 0, "the memory check reports the estimate: {:?}", r[1]);
    }

    /// §4.5 `cancel` in `Solving` (at the next iteration boundary) and in `Extracting` (after the node
    /// being extracted), placed deterministically from inside the progress callback: raised on the
    /// Solving transition it is seen before the first iteration; raised on the report that carries the
    /// target-reaching measurement (after the loop's last poll) it is answered at the Solving ->
    /// Extracting boundary without `finalize`; raised on the Extracting transition it is answered by the
    /// export before its first node. Never a solution.
    #[test]
    fn a_cancel_in_solving_or_extracting_is_answered_at_the_next_checkpoint() {
        let (mut req, _) = lock_job();
        req.target_bp = u16::MAX;
        let cases: [(&str, fn(&Report) -> bool); 3] = [
            ("solving transition", |r| r.0 == Stage::Solving && r.2.is_none()),
            ("target-reaching measurement", |r| r.0 == Stage::Solving && r.2.is_some()),
            ("extracting transition", |r| r.0 == Stage::Extracting),
        ];
        for (what, cancel_on) in cases {
            let (mut c, reports) = recording(cancel_on);
            let mut probe = Probe::new(&c);
            let r = run_with(&req, None, &mut c, &mut probe);
            assert!(matches!(r.outcome, JobOutcome::Cancelled), "{what}: {:?}", r.outcome);
            let seen = reports.lock().unwrap().clone();
            let last = *seen.last().unwrap();
            match what {
                "solving transition" => assert_eq!((last.0, last.1, last.2), (Stage::Solving, 0, None), "{what}: nothing after the transition"),
                "target-reaching measurement" => {
                    assert!(last.0 == Stage::Solving && last.2.is_some(), "{what}: {last:?}");
                    assert_eq!(probe.checkpoints.last(), Some(&Checkpoint::SolveEnded), "{what}");
                }
                _ => assert_eq!(last.0, Stage::Extracting, "{what}"),
            }
        }
    }

    /// Review ruling (c) of P2.T10, wired at the job's measurement boundary: every measured value passes
    /// `extract::report_exploitability` before any progress report or the solution carries it. A value in
    /// `[-1e-6, 0)` is reported as exactly `+0.0` with the raw value in ONE stderr line (the solve loop
    /// repeats its latest measurement with every coalesced report); a positive value passes unchanged
    /// with no line; a value below the floor or a non-finite one is never emitted (its refusal is logged
    /// once) and, as the final measurement, fails the job as `internal`. `validate_solution` stays strict
    /// and accepts what is reported.
    #[test]
    fn measurements_are_reported_through_the_noise_floor_policy() {
        let req = solve_request("river_two_combo", 0);
        let run_substituted = |raw: f32| {
            let (mut c, reports) = recording(|_| false);
            let mut probe = Probe::new(&c);
            probe.substitute = Some(raw);
            let r = run_with(&req, None, &mut c, &mut probe);
            let seen = reports.lock().unwrap().clone();
            (r.outcome, seen, probe.lines)
        };

        let (outcome, seen, lines) = run_substituted(-5e-7);
        let sol = match outcome { JobOutcome::Ok(s) | JobOutcome::BestSoFar(s) => s, other => panic!("{other:?}") };
        assert_eq!(sol.exploitability_chips.to_bits(), 0.0f32.to_bits(), "noise is reported as +0.0");
        validate_solution(&sol, &req.tree.materialized).unwrap();
        let measured: Vec<f32> = seen.iter().filter_map(|r| r.2).collect();
        assert!(!measured.is_empty() && measured.iter().all(|e| e.to_bits() == 0.0f32.to_bits()), "progress carries the reported value only: {measured:?}");
        assert_eq!(*seen.last().unwrap(), (Stage::Extracting, sol.iterations, Some(sol.exploitability_chips), sol.memory_bytes));
        assert_eq!(lines.len(), 1, "one line per raw measurement: {lines:?}");
        assert!(lines[0].contains("-5e-7"), "the raw value is logged: {}", lines[0]);

        let (outcome, seen, lines) = run_substituted(0.25);
        let sol = match outcome { JobOutcome::Ok(s) | JobOutcome::BestSoFar(s) => s, other => panic!("{other:?}") };
        assert_eq!(sol.exploitability_chips, 0.25);
        assert!(seen.iter().filter_map(|r| r.2).all(|e| e == 0.25));
        assert!(lines.is_empty(), "{lines:?}");

        for (raw, needle) in [(-0.01f32, "below"), (f32::NAN, "finite"), (f32::NEG_INFINITY, "finite")] {
            let (outcome, seen, lines) = run_substituted(raw);
            match outcome {
                JobOutcome::Error(e) => {
                    assert_eq!((e.code.as_str(), e.retryable, e.estimate_bytes), ("internal", true, None), "{raw:e}");
                    assert!(e.message.contains(needle), "{raw:e}: {needle:?} in {}", e.message);
                }
                other => panic!("{raw:e}: {other:?}"),
            }
            assert!(seen.iter().all(|r| r.2.is_none() && r.0 != Stage::Extracting), "{raw:e}: never emitted: {seen:?}");
            // the target-reaching report carried it first (omitted, refusal logged once), then the result
            assert!(lines.len() == 1 && lines[0].contains("not reported") && lines[0].contains(needle), "{raw:e}: {lines:?}");
        }
    }

    /// §13.2 `ev_convention_non_root_payoffs` through the job: the staged lock is applied before the first
    /// iteration and counted; its rows come back unchanged; OOP's call against the locked range is worth
    /// `equity * 300 - 100` = +200 with QQ (IP's QQ shares a queen with every OOP QQ, so only 54o bets
    /// into it) and -50 with 66 (equity 0.6 / 3.6); fold is exactly +0.0. The library's best response
    /// honours locks (`utility.rs`, "when the node is locked"), so the locked game converges like any
    /// other: `Ok` once a measurement reaches the target, `BestSoFar` otherwise, either way a solution
    /// `validate_solution` accepts. An empty staged set locks nothing.
    #[test]
    fn a_staged_lock_is_applied_counted_and_honoured() {
        let (req, locks) = lock_job();
        let target = req.pot as f32 * req.target_bp as f32 / 10_000.0;
        let sol = match run(&req, Some(&locks), &mut ctl()).outcome {
            JobOutcome::Ok(s) => { assert!(s.exploitability_chips <= target, "Ok at {} over target {target}", s.exploitability_chips); s }
            JobOutcome::BestSoFar(s) => { assert!(s.exploitability_chips > target, "BestSoFar at {} within target {target}", s.exploitability_chips); s }
            other => panic!("{other:?}"),
        };
        validate_solution(&sol, &req.tree.materialized).unwrap();
        assert_eq!((sol.locks_applied, sol.nodes.len(), sol.requested), (1, 3, 1));
        assert_eq!(sol.covered_paths, vec![vec![], vec![CHECK], vec![CHECK, JAM]]);
        let (ip, facing) = (&sol.nodes[1], &sol.nodes[2]);
        for c in 0..1326 {
            assert_eq!(ip.available[c], locks[0].probs[c].iter().any(|p| *p > 0.0), "combo {c}");
            if ip.available[c] {
                for a in 0..2 { assert!((ip.probs[c][a] - locks[0].probs[c][a]).abs() < 1e-6, "locked row {c}: {:?} vs {:?}", ip.probs[c], locks[0].probs[c]); }
            }
        }
        let queens = [combo("Qc", "Qd"), combo("Qc", "Qh"), combo("Qd", "Qh")];
        let sixes = [combo("6c", "6d"), combo("6c", "6h"), combo("6c", "6s"), combo("6d", "6h"), combo("6d", "6s"), combo("6h", "6s")];
        for (combos, call) in [(&queens[..], 200.0f32), (&sixes[..], -50.0)] {
            for &c in combos {
                assert!(facing.available[c], "combo {c}");
                assert_eq!(facing.ev_chips[c][0].to_bits(), 0.0f32.to_bits(), "fold EV of combo {c} is exactly +0.0");
                assert!((facing.ev_chips[c][1] - call).abs() <= 1e-3, "combo {c}: call EV {} expected {call}", facing.ev_chips[c][1]);
            }
        }

        let mut quick = req.clone();
        quick.target_bp = u16::MAX;
        match run(&quick, Some(&[]), &mut ctl()).outcome { JobOutcome::Ok(s) => assert_eq!(s.locks_applied, 0), other => panic!("{other:?}") }
    }

    /// §7 at the stop point: §13.0's `flop_best_so_far` fixture asks a 179 x 264-combo flop for 1 bp
    /// (0.018 chips) inside 2 s, which R8 puts far out of reach (0.5% alone takes 4-7 s on this size
    /// class), so the deadline ends the solve after at least one measurement and the job returns
    /// `BestSoFar`: every flop decision node exported, the reported exploitability above the target, and
    /// a solution `validate_solution` accepts.
    #[test]
    fn the_flop_best_so_far_fixture_returns_a_validated_best_so_far() {
        let req = solve_request("flop_best_so_far", 0);
        let (mut c, reports) = recording(|_| false);
        let r = run(&req, None, &mut c);
        let sol = match r.outcome { JobOutcome::BestSoFar(s) => s, other => panic!("{other:?}") };
        let target = req.pot as f32 * req.target_bp as f32 / 10_000.0;
        assert!(sol.exploitability_chips > target && sol.iterations > 0, "{} chips after {} iterations", sol.exploitability_chips, sol.iterations);
        validate_solution(&sol, &req.tree.materialized).unwrap();
        let flop_nodes = req.tree.materialized.iter().filter(|n| n.street == proto::Street::Flop).count();
        assert_eq!((sol.nodes.len(), sol.requested, sol.export.as_str(), sol.locks_applied), (flop_nodes, 0, "street", 0));
        assert_eq!(*reports.lock().unwrap().last().unwrap(), (Stage::Extracting, sol.iterations, Some(sol.exploitability_chips), sol.memory_bytes));
    }

    /// Every failure is a typed §4.5 `WorkerError` with the spec's code and retryability, returned before
    /// any iteration: an unrepresentable history, a history that names no decision node of the solved
    /// street (a terminal, or a node past the street's chance deal), a board that does not fit the root
    /// street and a zero pot are `invalid_request` (never a retryable `internal` from the export); a
    /// realized tree that differs from `tree.materialized` is `tree_mismatch`; an estimate whose 1.25x
    /// headroom exceeds the limit is `tree_too_large` with `estimate_bytes`, before the estimate is
    /// reported or anything allocated; a lock that locks nothing, and two locks naming one node (only one
    /// could take effect, so `locks_applied` would overstate it), are `lock_mismatch`.
    #[test]
    fn failures_are_typed_worker_errors() {
        let base = solve_request("river_two_combo", 0);
        let (lock_req, locks) = lock_job();
        let mut history = base.clone();
        history.history = vec![Action::Bet { to: 50 }];
        let mut terminal = base.clone();
        terminal.history = vec![CHECK, CHECK];
        let mut past_street = crate::extract::test_games::turn_request();
        past_street.history = vec![CHECK, CHECK];
        assert!(past_street.tree.materialized.iter().any(|n| n.street == proto::Street::River), "the turn tree reaches the river");
        let mut board = base.clone();
        board.board.pop();
        let mut pot = base.clone();
        pot.pot = 0;
        let mut mismatch = base.clone();
        let ip = mismatch.tree.materialized.iter_mut().find(|n| n.path == [0]).unwrap();
        assert_eq!(ip.terminal_pots[0], Some(100));
        ip.terminal_pots[0] = Some(99);
        let mut memory = base.clone();
        memory.memory_limit_bytes = 1;
        let mut free = locks.clone();
        for row in free[0].probs.iter_mut() { *row = vec![0.0, 0.0]; }
        let twice = vec![locks[0].clone(), locks[0].clone()];

        let cases: [(&str, &SolveRequest, Option<&[NodeLock]>, &str, bool, &str); 9] = [
            ("history", &history, None, "invalid_request", false, "history"),
            ("terminal history", &terminal, None, "invalid_request", false, "history"),
            ("history past the street", &past_street, None, "invalid_request", false, "decision node of the Turn street"),
            ("board", &board, None, "invalid_request", false, "board"),
            ("pot", &pot, None, "invalid_request", false, "pot"),
            ("mismatch", &mismatch, None, "tree_mismatch", false, "path [0]"),
            ("memory", &memory, None, "tree_too_large", false, "memory_limit_bytes"),
            ("no-op lock", &lock_req, Some(&free), "lock_mismatch", false, "no hand"),
            ("duplicate lock", &lock_req, Some(&twice), "lock_mismatch", false, "duplicates"),
        ];
        for (what, req, staged, code, retryable, needle) in cases {
            let (mut c, reports) = recording(|_| false);
            let e = match run(req, staged, &mut c).outcome { JobOutcome::Error(e) => e, other => panic!("{what}: {other:?}") };
            assert_eq!((e.code.as_str(), e.retryable), (code, retryable), "{what}: {}", e.message);
            assert!(e.message.contains(needle), "{what}: {needle:?} in {}", e.message);
            assert_eq!(e.estimate_bytes.is_some(), code == "tree_too_large", "{what}: estimate_bytes {:?}", e.estimate_bytes);
            let seen = stages(&reports);
            assert!(!seen.contains(&Stage::Solving), "{what}: failed in Building");
            if code != "lock_mismatch" { assert_eq!(seen, [Stage::Building], "{what}: nothing reported past the failed step"); }
        }
    }

    /// `error` builds only the §4.5 failure codes (always-on assert): a typo would otherwise reach the
    /// wire as a code the engine cannot classify.
    #[test]
    fn error_accepts_exactly_the_section_4_5_failure_codes() {
        for code in proto::worker::FAILURE_CODES {
            let e = error(code, "m", code == "internal" || code == "out_of_memory", None);
            assert_eq!((e.code.as_str(), e.message.as_str(), e.estimate_bytes), (code, "m", None));
        }
        assert_eq!(error("tree_too_large", "m", false, Some(7)).estimate_bytes, Some(7));
        let refused = std::panic::catch_unwind(|| error("oom", "m", true, None));
        assert!(refused.is_err(), "an unknown code is refused");
    }
}
