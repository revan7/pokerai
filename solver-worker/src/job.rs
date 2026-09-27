//! §4.5 / §10.3: one `solve` request end to end. Building (tree build, tree cross-check, memory check,
//! allocation, locks) -> Solving (the §7 `solve_step` loop) -> Extracting (`finalize`, the street export,
//! self-validation). A cancel is answered at the next checkpoint of the current stage (§4.5): after every
//! Building step, before the first iteration and at every iteration boundary (the solve loop's own polls),
//! at the Solving -> Extracting boundary, after the Extracting notification, `finalize`, the export (which
//! also polls around every node) and self-validation, so no expensive step starts once a cancel is
//! visible. Every failure is a typed §4.5 `WorkerError`, and a returned solution has passed
//! `proto::worker::validate_solution`, which stays strict.
//!
//! `out_of_memory` has no producer here (P2.T11 review ruling 1): `memory::admit` is the job's preventive
//! check (`tree_too_large`), and an allocation that fails anyway aborts the process, which the engine
//! sees as a worker exit (§10.3).
use crate::extract::NodeSite;
use crate::solve_loop::LoopSite;
use crate::{cards, extract, locks, memory, solve_loop, tree_build, win};
use postflop_solver::{finalize, CardConfig, PostFlopGame};
use proto::worker::{validate_solution, NodeLock, SolveRequest, Stage, StreetSolution, WorkerError, FAILURE_CODES};
use proto::{index_materialized, resolve_chip_path_indexed, Street};
use std::cell::RefCell;
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
/// NORMAL, not only the success path. The job holds it from before its first step until after its last
/// local (the game's buffers included) is dropped. `set` is the setter seam (`win::set_priority_class`
/// in production; P2.T11 review M2), called with `true` for BELOW_NORMAL.
struct Priority<'s> { set: &'s mut dyn FnMut(bool) }
impl<'s> Priority<'s> {
    fn set(set: &'s mut dyn FnMut(bool), background: bool) -> Priority<'s> {
        set(background);
        Priority { set }
    }
}
impl Drop for Priority<'_> {
    fn drop(&mut self) { (self.set)(false); }
}

/// The job's own cancel checkpoints: §4.5 "in `Building` after the current step (tree build, cross-check,
/// memory check, allocation, lock application are each a step)", plus one before the first step, one
/// after the staged lock set is validated, one at the Solving -> Extracting boundary, and in `Extracting`
/// one right after the `Extracting` notification (ahead of the uninterruptible `finalize`), one after
/// `finalize`, one after the export and one after self-validation, before the terminal is chosen
/// (P2.T11 review I1). The polls at iteration boundaries belong to `solve_loop::run` and those around
/// each node to `extract::street_solution`; the hooks see them as `Hooks::loop_site` and `Hooks::node_site`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkpoint { Start, TreeBuilt, TreeChecked, GameConfigured, MemoryChecked, Allocated, LocksValidated, LockApplied(u16), SolveEnded, ExtractingNotified, Finalized, Exported, Validated }

/// The job's expensive operations, as the seam observes them: `Hooks::enter` and `Hooks::leave` bracket
/// each one, so a test sees exactly which work started after a cancel. `LockApply(k)` applies the
/// `k`-th staged lock (from 1); `Solve` is the whole §7 loop, `Export` the whole street export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op { TreeBuild, TreeCheck, GameConfig, MemoryCheck, Allocate, LockValidate, LockApply(u16), Solve, Finalize, Export, Validate }

/// The hooks seam, deterministic for the tests (standing ruling: a hook places a cancel or a measurement
/// exactly, never a sleep racing the computation). Production (`Stderr`) keeps every default: nothing
/// at a checkpoint, at a poll or around an operation, the measurement unchanged, and diagnostics on
/// stderr (§4.5: stdout carries protocol only). Every hook is called on the job's own thread, never with
/// the protocol lock held, and none of them polls or decides anything: a hook that blocks holds the job
/// exactly where it is called while control goes on answering stdin (the executor's checkpoint barrier,
/// `protocol::executor_loop_with`).
pub trait Hooks {
    /// Called at each checkpoint, immediately before the cancel flag is polled.
    fn checkpoint(&mut self, _at: Checkpoint) {}
    /// Called immediately before an operation starts.
    fn enter(&mut self, _op: Op) {}
    /// Called as soon as an operation has returned, before its result is looked at.
    fn leave(&mut self, _op: Op) {}
    /// Called inside the §7 loop (`solve_loop::LoopSite`, with the iterations completed) immediately
    /// before each cancel poll (the iteration boundaries of `Solving`) and each solve step or
    /// measurement.
    fn loop_site(&mut self, _at: LoopSite) {}
    /// Called inside the street export (`extract::NodeSite`, with the nodes extracted) immediately
    /// before each cancel poll (the node boundaries of `Extracting`: one precedes every node and one
    /// follows the last) and each node's extraction.
    fn node_site(&mut self, _at: NodeSite) {}
    /// A raw measurement as the solve loop produced it, before it is reported.
    fn measured(&mut self, raw: f32) -> f32 { raw }
    fn log(&mut self, line: &str) { eprintln!("{line}"); }
}
/// The production hooks: every default.
pub struct Stderr;
impl Hooks for Stderr {}

/// Review ruling (c) of P2.T10 at the job's measurement boundary: every measurement the job emits (in a
/// progress report or the solution metadata) first passes `extract::report_exploitability` at the
/// request's pot (P2.T11 review I2: the noise tolerance is pot-relative). A noise value is emitted as
/// `+0.0` with its raw value on stderr (`Hooks::log`); a value the policy refuses (below the tolerance,
/// or non-finite) is never emitted: a progress report omits it (its refusal is logged) and, as the
/// final measurement, it fails the job as `internal`. The solve loop repeats its latest measurement
/// with every coalesced progress report, so a repeat of the value just reported is answered from
/// `last` rather than logged again. A negative or refused measurement is in practice also the final
/// one: a NaN persists through later iterations, and a negative value is at or below every target, so
/// the loop stops at it; a solve therefore logs one line.
struct Reporter { pot: u32, last: Option<(u32, Result<f32, String>)> }
impl Reporter {
    fn new(pot: u32) -> Reporter { Reporter { pot, last: None } }
    fn report(&mut self, raw: f32, hooks: &mut dyn Hooks) -> Result<f32, String> {
        if let Some((bits, reported)) = &self.last {
            if *bits == raw.to_bits() { return reported.clone(); }
        }
        let reported = match extract::report_exploitability(raw, self.pot) {
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

/// The loop seam: a solve loop receives the game, the loop parameters, the cancel flag, the job's per-report
/// callback and its per-site callback, exactly what `solve_loop::run` takes, and returns the loop's outcome.
pub type LoopSeam<'s> = &'s mut dyn FnMut(&PostFlopGame, &solve_loop::LoopParams, &AtomicBool, &mut dyn FnMut(u32, Option<f32>), &mut dyn FnMut(LoopSite)) -> solve_loop::LoopOutcome;

/// The job's three seams. Production (`run`) passes `Stderr` hooks, the real §7 loop (`real_loop`) and
/// `win::set_priority_class`; a test records events, scripts a loop's stop over real iterations
/// (P2.T11 review M1) or records the priority calls (review M2).
struct Seams<'s> {
    hooks: &'s mut dyn Hooks,
    solve: LoopSeam<'s>,
    set_priority: &'s mut dyn FnMut(bool),
}

/// The production loop seam: `solve_loop::run`, unchanged.
fn real_loop(game: &PostFlopGame, params: &solve_loop::LoopParams, cancel: &AtomicBool, progress: &mut dyn FnMut(u32, Option<f32>), at_site: &mut dyn FnMut(LoopSite)) -> solve_loop::LoopOutcome {
    solve_loop::run(game, params, cancel, progress, at_site)
}

/// Runs one `solve` (module docs). `staged` is the lock set this request consumes (§4.5; the executor has
/// matched its `spot`). The deadline is the absolute instant `deadline_ms` after entry: Building counts
/// against it (§7).
pub fn run(req: &SolveRequest, staged: Option<&[NodeLock]>, ctl: &mut JobControl) -> JobResult {
    run_hooked(req, staged, ctl, &mut Stderr)
}

/// `run` with the caller's hooks in place of `Stderr` (the executor's seam, `protocol::executor_loop_with`):
/// the real §7 loop and the real priority setter, every checkpoint, operation, poll and step reported to `hooks`.
/// With `Stderr` it is `run`.
pub fn run_hooked(req: &SolveRequest, staged: Option<&[NodeLock]>, ctl: &mut JobControl, hooks: &mut dyn Hooks) -> JobResult {
    run_with(req, staged, ctl, Seams { hooks, solve: &mut real_loop, set_priority: &mut win::set_priority_class })
}

/// `run_hooked` with the caller's loop in place of the real §7 loop: test support, never called by the worker (the
/// executor runs `run_hooked`), so it changes no production behaviour. The integration tests use it for a
/// deterministic solve schedule (P2.T16 review I1, I4, I5: a fixed count of real `solve_step`s, whatever the
/// machine's speed); everything else is the production job: every Building step, the reporting policy, `finalize`,
/// the export and self-validation, each reported to `hooks`, and the real priority setter.
pub fn run_scripted(req: &SolveRequest, staged: Option<&[NodeLock]>, ctl: &mut JobControl, hooks: &mut dyn Hooks, solve: LoopSeam<'_>) -> JobResult {
    run_with(req, staged, ctl, Seams { hooks, solve, set_priority: &mut win::set_priority_class })
}

fn run_with(req: &SolveRequest, staged: Option<&[NodeLock]>, ctl: &mut JobControl, seams: Seams<'_>) -> JobResult {
    let t0 = Instant::now();
    let Seams { hooks, solve, set_priority } = seams;
    let _priority = Priority::set(set_priority, req.background);
    let done = |o: JobOutcome| JobResult { outcome: o, elapsed_ms: ms(t0) };
    macro_rules! checkpoint {
        ($at:expr) => {{
            hooks.checkpoint($at);
            if ctl.cancel.load(Ordering::SeqCst) { return done(JobOutcome::Cancelled); }
        }};
    }
    // One expensive operation, bracketed for the seam; its result is matched only afterwards, so an
    // early return never skips `leave`.
    macro_rules! op {
        ($op:expr, $e:expr) => {{
            hooks.enter($op);
            let r = $e;
            hooks.leave($op);
            r
        }};
    }
    let invalid = |m: String| JobOutcome::Error(error("invalid_request", m, false, None));
    let lock_mismatch = |m: String| JobOutcome::Error(error("lock_mismatch", m, false, None));
    let internal = |m: String| JobOutcome::Error(error("internal", m, true, None));

    // Building: tree, cross-check, game, memory check, allocation, locks.
    (ctl.progress)(Stage::Building, 0, None, ms(t0), 0);
    checkpoint!(Checkpoint::Start);
    let eff = req.stack_oop.min(req.stack_ip);
    let built = op!(Op::TreeBuild, tree_build::build(&req.tree, req.pot, eff, req.rake_rate, req.rake_cap_mchips, &req.history));
    let mut tree = match built { Ok(t) => t, Err(e) => return done(invalid(e)) };
    checkpoint!(Checkpoint::TreeBuilt);
    let checked = op!(Op::TreeCheck, tree_build::enumerate(&mut tree, req.tree.root_street, req.pot).map(|lib| tree_build::cross_check(&lib, &req.tree.materialized)));
    match checked {
        Err(e) => return done(invalid(e)),
        Ok(Err(e)) => return done(JobOutcome::Error(error("tree_mismatch", e, false, None))),
        Ok(Ok(())) => {}
    }
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
    let configured = op!(Op::GameConfig, PostFlopGame::with_config(CardConfig { range: ranges, flop, turn, river }, tree));
    let mut game = match configured { Ok(g) => g, Err(e) => return done(invalid(e)) };
    checkpoint!(Checkpoint::GameConfigured);
    let admitted = op!(Op::MemoryCheck, {
        let (f32_bytes, i16_bytes) = game.memory_usage();
        memory::admit(f32_bytes, i16_bytes, req.memory_limit_bytes)
    });
    let adm = match admitted { Ok(a) => a, Err(e) => return done(JobOutcome::Error(e)) };
    (ctl.progress)(Stage::Building, 0, None, ms(t0), adm.estimate_bytes);
    checkpoint!(Checkpoint::MemoryChecked);
    op!(Op::Allocate, game.allocate_memory(adm.compressed));
    checkpoint!(Checkpoint::Allocated);
    let mut locks_applied = 0u16;
    if let Some(ls) = staged {
        // Validated at the protocol boundary already; checked again because `run` is public and
        // `locks_applied` must count distinct locked nodes: two locks naming one node would both apply
        // while only the last takes effect.
        if let Err(e) = op!(Op::LockValidate, locks::validate(ls)) { return done(lock_mismatch(e)); }
        if u16::try_from(ls.len()).is_err() { return done(lock_mismatch(format!("{} staged locks exceed the u16 locks_applied count", ls.len()))); }
        checkpoint!(Checkpoint::LocksValidated);
        for l in ls {
            // `apply` refuses a lock that would lock nothing (P2.T10 review I2): only a lock that took
            // effect is counted, and the length check above bounds the count.
            let k = locks_applied + 1;
            if let Err(e) = op!(Op::LockApply(k), locks::apply(&mut game, l, &req.tree.materialized)) { return done(lock_mismatch(e)); }
            locks_applied = k;
            checkpoint!(Checkpoint::LockApplied(locks_applied));
        }
    }

    // Solving: the §7 loop polls the cancel flag before the first iteration and at every boundary.
    (ctl.progress)(Stage::Solving, 0, None, ms(t0), adm.estimate_bytes);
    let target_chips = (f64::from(req.pot) * f64::from(req.target_bp) / 10_000.0) as f32;
    let params = solve_loop::LoopParams { deadline_ms: req.deadline_ms, extraction_margin_ms: req.extraction_margin_ms, target_chips, started: t0 };
    let cancel = Arc::clone(&ctl.cancel);
    let mut reporter = Reporter::new(req.pot);
    hooks.enter(Op::Solve);
    let out = {
        // Both loop callbacks reach the hooks: a report's measurement (`measured`, and `log` through the
        // noise policy) and each site (`loop_site`). The loop calls them one at a time, so the shared
        // borrow is never taken twice.
        let hooks = RefCell::new(&mut *hooks);
        let (progress, reporter) = (&mut ctl.progress, &mut reporter);
        let mut report = |iterations: u32, raw: Option<f32>| {
            let reported = match raw {
                None => None,
                Some(raw) => {
                    let mut h = hooks.borrow_mut();
                    let raw = h.measured(raw);
                    match reporter.report(raw, &mut **h) { Ok(chips) => Some(chips), Err(_) => return }
                }
            };
            progress(Stage::Solving, iterations, reported, ms(t0), adm.estimate_bytes);
        };
        let mut at_site = |at: LoopSite| hooks.borrow_mut().loop_site(at);
        solve(&game, &params, &cancel, &mut report, &mut at_site)
    };
    hooks.leave(Op::Solve);
    if out.cancelled { return done(JobOutcome::Cancelled); }
    checkpoint!(Checkpoint::SolveEnded);
    let Some(raw) = out.exploitability else { return done(JobOutcome::Error(error("no_iteration", "no exploitability measurement before stop point", false, None))); };
    let expl = match reporter.report(hooks.measured(raw), &mut *hooks) { Ok(chips) => chips, Err(e) => return done(internal(e)) };

    // Extracting: `finalize` is not interruptible; the export polls around every node; a checkpoint
    // follows each of the three operations, so a cancel raised by the notification or during any of
    // them never starts the next.
    (ctl.progress)(Stage::Extracting, out.iterations, Some(expl), ms(t0), adm.estimate_bytes);
    checkpoint!(Checkpoint::ExtractingNotified);
    op!(Op::Finalize, finalize(&mut game));
    checkpoint!(Checkpoint::Finalized);
    let meta = extract::SolutionMeta { exploitability_chips: expl, iterations: out.iterations, memory_bytes: adm.estimate_bytes, mode: adm.mode, locks_applied };
    let exported = op!(Op::Export, extract::street_solution_observed(&mut game, req, meta, &ctl.cancel, &mut |at| hooks.node_site(at)));
    let sol = match exported { Ok(Some(s)) => s, Ok(None) => return done(JobOutcome::Cancelled), Err(e) => return done(internal(e)) };
    // The export's last poll precedes its result-line sizing: a cancel during the sizing is answered here.
    checkpoint!(Checkpoint::Exported);
    // The last gate before a solution leaves the worker, and a strict one (a raw negative fails it).
    if let Err(e) = op!(Op::Validate, validate_solution(&sol, &req.tree.materialized)) { return done(internal(format!("self-validation failed: {e}"))); }
    checkpoint!(Checkpoint::Validated);
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
    use std::cell::RefCell;
    use std::rc::Rc;
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

    /// What the seams saw, in order: a checkpoint (just before its poll), an operation's entry or
    /// completion, a call of the priority setter, or (when the probe records them) a site of the §7
    /// loop or of the export (just before it: a poll, a step, a measurement, a node's extraction).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Event { At(Checkpoint), Enter(Op), Leave(Op), Priority(bool), Loop(LoopSite), Node(NodeSite) }
    type Log = Rc<RefCell<Vec<Event>>>;

    /// The job's deterministic hooks: records every event in the shared log, raises the cancel flag on
    /// one of them (at a checkpoint, or as an operation starts or completes: a cancel arriving during
    /// it), substitutes the raw exploitability measurement, and keeps the stderr lines. The loop's and
    /// the export's sites are recorded only when `sites` is set.
    struct Probe { cancel: Arc<AtomicBool>, cancel_at: Option<Event>, substitute: Option<f32>, log: Log, lines: Vec<String>, sites: bool }
    impl Probe {
        fn new(c: &JobControl) -> Probe { Probe { cancel: c.cancel.clone(), cancel_at: None, substitute: None, log: Log::default(), lines: Vec::new(), sites: false } }
        fn record(&mut self, e: Event) {
            self.log.borrow_mut().push(e);
            if self.cancel_at == Some(e) { self.cancel.store(true, Ordering::SeqCst); }
        }
    }
    impl Hooks for Probe {
        fn checkpoint(&mut self, at: Checkpoint) { self.record(Event::At(at)); }
        fn enter(&mut self, op: Op) { self.record(Event::Enter(op)); }
        fn leave(&mut self, op: Op) { self.record(Event::Leave(op)); }
        fn loop_site(&mut self, at: LoopSite) { if self.sites { self.record(Event::Loop(at)); } }
        fn node_site(&mut self, at: NodeSite) { if self.sites { self.record(Event::Node(at)); } }
        fn measured(&mut self, raw: f32) -> f32 { self.substitute.unwrap_or(raw) }
        fn log(&mut self, line: &str) { self.lines.push(line.to_string()); }
    }

    /// The loop seam's type, as a test writes a scripted loop.
    type Scripted<'s> = &'s mut dyn FnMut(&PostFlopGame, &solve_loop::LoopParams, &AtomicBool, &mut dyn FnMut(u32, Option<f32>), &mut dyn FnMut(LoopSite)) -> solve_loop::LoopOutcome;

    /// Runs the job through all three seams: `probe` as the hooks, `solve` as the loop (the real §7
    /// loop, as in production, when `None`), and a priority setter that records its calls in the
    /// probe's log instead of changing the process priority.
    fn run_seamed(req: &SolveRequest, staged: Option<&[NodeLock]>, c: &mut JobControl, probe: &mut Probe, solve: Option<Scripted<'_>>) -> JobResult {
        let log = Rc::clone(&probe.log);
        let mut set_priority = move |below_normal: bool| log.borrow_mut().push(Event::Priority(below_normal));
        let mut real = real_loop;
        let solve: Scripted<'_> = match solve { Some(s) => s, None => &mut real };
        run_with(req, staged, c, Seams { hooks: probe, solve, set_priority: &mut set_priority })
    }

    /// One job run through the seams: its outcome, the progress reports, the full seam log, the hook
    /// events alone (the log without the priority calls) and the stderr lines.
    struct Trace { outcome: JobOutcome, reports: Vec<Report>, log: Vec<Event>, events: Vec<Event>, lines: Vec<String> }
    impl Trace {
        fn new(r: JobResult, reports: &Reports, probe: Probe) -> Trace {
            let log = probe.log.borrow().clone();
            let events = log.iter().copied().filter(|e| !matches!(e, Event::Priority(_))).collect();
            Trace { outcome: r.outcome, reports: reports.lock().unwrap().clone(), log, events, lines: probe.lines }
        }
        fn checkpoints(&self) -> Vec<Checkpoint> { self.events.iter().filter_map(|e| match e { Event::At(c) => Some(*c), _ => None }).collect() }
        fn started(&self) -> Vec<Op> { self.events.iter().filter_map(|e| match e { Event::Enter(o) => Some(*o), _ => None }).collect() }
        fn stages(&self) -> Vec<Stage> { self.reports.iter().map(|r| r.0).collect() }
        fn priority(&self) -> Vec<bool> { self.log.iter().filter_map(|e| match e { Event::Priority(b) => Some(*b), _ => None }).collect() }
    }

    /// Runs `req` (the real loop) with a cancel raised on the first progress report `cancel_on`
    /// accepts, or on the seam event `cancel_at`, and the raw measurement replaced by `substitute`.
    fn traced(req: &SolveRequest, staged: Option<&[NodeLock]>, cancel_on: impl Fn(&Report) -> bool + Send + 'static, cancel_at: Option<Event>, substitute: Option<f32>) -> Trace {
        let (mut c, reports) = recording(cancel_on);
        let mut probe = Probe::new(&c);
        probe.cancel_at = cancel_at;
        probe.substitute = substitute;
        let r = run_seamed(req, staged, &mut c, &mut probe, None);
        Trace::new(r, &reports, probe)
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

    /// §4.5 cancel checkpoints (P2.T11 review I1). In `Building` a cancel is answered "after the current
    /// step" (tree build, cross-check, game configuration, memory check, allocation, the staged set's
    /// validation, each lock); in `Solving` by the loop's own polls; in `Extracting` right after the
    /// notification (before the uninterruptible `finalize`), after `finalize`, after the export (whose
    /// own polls answer a cancel before and between nodes) and after self-validation, before a terminal
    /// is chosen. The uncancelled run shows every operation completing into a checkpoint before anything
    /// else starts. A cancel placed on every event of that run (at a checkpoint, or as an operation
    /// starts or completes: a cancel arriving during it) then returns `Cancelled` with the trace ending
    /// exactly where it is answered, so no further expensive operation starts: in particular a cancel
    /// during the export's sizing or during self-validation never becomes a solution.
    #[test]
    fn a_cancel_at_or_during_any_step_starts_no_further_operation() {
        use Checkpoint::*;
        use Op::*;
        let (mut req, locks) = lock_job();
        req.target_bp = u16::MAX;                        // the first measurement reaches it: a short uncancelled run
        let full = traced(&req, Some(&locks), |_| false, None, None);
        assert!(matches!(full.outcome, JobOutcome::Ok(_)), "{:?}", full.outcome);
        for (k, e) in full.events.iter().enumerate() {
            if let Event::Leave(op) = e {
                assert!(matches!(full.events.get(k + 1), Some(Event::At(_))), "{op:?} completes into a checkpoint, not {:?}", full.events.get(k + 1));
            }
        }
        assert_eq!(full.checkpoints(), [Start, TreeBuilt, TreeChecked, GameConfigured, MemoryChecked, Allocated, LocksValidated, LockApplied(1), SolveEnded, ExtractingNotified, Finalized, Exported, Validated]);
        assert_eq!(full.started(), [TreeBuild, TreeCheck, GameConfig, MemoryCheck, Allocate, LockValidate, LockApply(1), Solve, Finalize, Export, Validate]);
        assert_eq!(full.events.last(), Some(&Event::At(Validated)), "the terminal is chosen after the last checkpoint");
        assert_eq!(full.stages().last(), Some(&Stage::Extracting));

        for (k, &at) in full.events.iter().enumerate() {
            // Where the cancel is answered: at that checkpoint; at the checkpoint that follows a completed
            // operation; inside the loop or the export, which poll before any iteration or node; after any
            // other operation (never interrupted) at the checkpoint that follows it.
            let end = match at {
                Event::At(_) => k,
                Event::Leave(_) | Event::Enter(Solve | Export) => k + 1,
                Event::Enter(_) => k + 2,
                Event::Priority(_) => unreachable!("`events` leaves out the priority calls"),
                Event::Loop(_) | Event::Node(_) => unreachable!("this probe does not record the loop's or the export's sites"),
            };
            let t = traced(&req, Some(&locks), |_| false, Some(at), None);
            assert!(matches!(t.outcome, JobOutcome::Cancelled), "cancel at {at:?}: {:?}", t.outcome);
            assert_eq!(t.events, full.events[..=end], "cancel at {at:?}: answered at {:?}, and nothing starts after it", full.events[end]);
            if at == Event::Enter(Solve) { assert!(t.reports.iter().all(|r| r.1 == 0), "cancel at {at:?}: no iteration: {:?}", t.reports); }
        }
        // the second Building report is the memory check: it carries the admitted estimate
        let t = traced(&req, Some(&locks), |_| false, Some(Event::At(MemoryChecked)), None);
        assert_eq!((t.reports[0].3, t.reports[1].2), (0, None));
        assert!(t.reports[1].3 > 0, "the memory check reports the estimate: {:?}", t.reports[1]);
    }

    /// A cancel raised synchronously by a progress notification (the callback runs inside the job) is
    /// answered before the next operation starts: on the first `Building` report at `Start`, before the
    /// tree build; on the memory-check report at `MemoryChecked`, before allocation; on the `Solving`
    /// transition by the loop's first poll, before any iteration; on the report carrying the
    /// target-reaching measurement (after the loop's last poll) at `SolveEnded`; on the `Extracting`
    /// transition at `ExtractingNotified`, before `finalize`. Nothing is reported after the cancelling
    /// notification, and never a solution.
    #[test]
    fn a_cancel_raised_by_a_notification_is_answered_before_the_next_operation() {
        use Checkpoint::*;
        let (mut req, _) = lock_job();
        req.target_bp = u16::MAX;
        let cases: [(&str, fn(&Report) -> bool, Event); 5] = [
            ("building transition", |r| r.0 == Stage::Building && r.3 == 0, Event::At(Start)),
            ("memory estimate", |r| r.0 == Stage::Building && r.3 > 0, Event::At(MemoryChecked)),
            ("solving transition", |r| r.0 == Stage::Solving && r.2.is_none(), Event::Leave(Op::Solve)),
            ("target-reaching measurement", |r| r.0 == Stage::Solving && r.2.is_some(), Event::At(SolveEnded)),
            ("extracting transition", |r| r.0 == Stage::Extracting, Event::At(ExtractingNotified)),
        ];
        for (what, cancel_on, answered) in cases {
            let t = traced(&req, None, cancel_on, None, None);
            assert!(matches!(t.outcome, JobOutcome::Cancelled), "{what}: {:?}", t.outcome);
            assert_eq!(t.events.last(), Some(&answered), "{what}: {:?}", t.events);
            let last = t.reports.last().unwrap();
            assert!(cancel_on(last), "{what}: nothing reported after the cancelling notification: {:?}", t.reports);
            if what == "solving transition" { assert!(t.reports.iter().all(|r| r.1 == 0), "{what}: no iteration: {:?}", t.reports); }
        }
    }

    /// Fix round 1 (review P2.T14-I2), the seam the executor's checkpoint barrier holds a job at: inside
    /// the §7 loop every solve step and the measurement sit between two polls (one before every step,
    /// one after it, one after the measurement), and inside the export every node's extraction sits
    /// between two polls (one before every node, one after the last); each site reaches the hooks
    /// immediately before it happens. A cancel raised at a poll is answered by that very poll; one raised
    /// as a step, a measurement or an extraction starts is answered by the poll right after it. Either
    /// way the run is the uncancelled one up to that poll and the one event that follows closes the
    /// operation it is in: no further step, measurement or node starts.
    #[test]
    fn a_cancel_at_any_loop_or_node_site_is_answered_by_the_next_poll() {
        use crate::extract::NodeSite::{Extract, Poll};
        use LoopSite::{Boundary, Iteration, Measured, Measurement, Stepped};
        let (mut req, locks) = lock_job();
        req.target_bp = u16::MAX;                        // the first measurement (iteration 10) reaches it
        let run_sited = |cancel_at: Option<Event>| {
            let (mut c, reports) = recording(|_| false);
            let mut probe = Probe::new(&c);
            (probe.sites, probe.cancel_at) = (true, cancel_at);
            let r = run_seamed(&req, Some(&locks), &mut c, &mut probe, None);
            Trace::new(r, &reports, probe)
        };
        let full = run_sited(None);
        assert!(matches!(full.outcome, JobOutcome::Ok(_)), "{:?}", full.outcome);
        let sites: Vec<(usize, Event)> = full.events.iter().copied().enumerate().filter(|(_, e)| matches!(e, Event::Loop(_) | Event::Node(_))).collect();
        let mut want: Vec<Event> = (0..10).flat_map(|i| [Boundary(i), Iteration(i + 1), Stepped(i + 1)]).chain([Measurement(10), Measured(10)]).map(Event::Loop).collect();
        want.extend([Poll(0), Extract(0), Poll(1), Extract(1), Poll(2), Extract(2), Poll(3)].map(Event::Node));
        assert_eq!(sites.iter().map(|(_, e)| *e).collect::<Vec<_>>(), want, "ten steps and a measurement, then three nodes, each between two polls");
        for (k, at) in sites {
            let t = run_sited(Some(at));
            assert!(matches!(t.outcome, JobOutcome::Cancelled), "cancel at {at:?}: {:?}", t.outcome);
            let answered = match at { Event::Loop(Iteration(_) | Measurement(_)) | Event::Node(Extract(_)) => k + 1, _ => k };
            assert!(matches!(full.events[answered], Event::Loop(Boundary(_) | Stepped(_) | Measured(_)) | Event::Node(Poll(_))), "cancel at {at:?}: answered at the poll {:?}", full.events[answered]);
            let closing = if matches!(at, Event::Loop(_)) { Event::Leave(Op::Solve) } else { Event::Leave(Op::Export) };
            assert_eq!(t.events[..=answered], full.events[..=answered], "cancel at {at:?}: the same run up to the poll that answers it");
            assert_eq!(t.events[answered + 1..], [closing], "cancel at {at:?}: nothing starts after that poll");
        }
    }

    /// Review ruling (c) of P2.T10 with the pot-relative tolerance of P2.T11 review I2, wired at the job's
    /// measurement boundary: every measured value passes `extract::report_exploitability(raw, pot)`
    /// before any progress report or the solution carries it. At this 100-chip pot the tolerance is
    /// `8 * f32::EPSILON * 100` = 9.5367431640625e-5 chips, a hundred times the old absolute 1e-6 floor:
    /// a value in `[-tolerance, 0)` is reported as exactly `+0.0` with the raw value in exactly ONE
    /// stderr line per solve (the solve loop repeats its latest measurement with every coalesced report,
    /// and the final measurement is the same sample); a positive value passes unchanged with no line; the
    /// next `f32` below the tolerance, a larger negative or a non-finite value is never emitted (its
    /// refusal is logged once) and, as the final measurement, fails the job as `internal`.
    /// `validate_solution` stays strict and accepts what is reported.
    #[test]
    fn measurements_are_reported_through_the_noise_floor_policy() {
        let req = solve_request("river_two_combo", 0);
        assert_eq!(req.pot, 100);
        let inside = -(8.0 * f32::EPSILON * 100.0);                 // exactly -tolerance: the inclusive edge
        assert_eq!(f64::from(inside), -9.5367431640625e-5);
        let outside = f32::from_bits(inside.to_bits() + 1);        // the next f32 below it
        let run_substituted = |raw: f32| {
            let t = traced(&req, None, |_| false, None, Some(raw));
            (t.outcome, t.reports, t.lines)
        };

        for raw in [inside, -5e-7, -f32::MIN_POSITIVE] {
            let (outcome, seen, lines) = run_substituted(raw);
            let sol = match outcome { JobOutcome::Ok(s) | JobOutcome::BestSoFar(s) => s, other => panic!("{raw:e}: {other:?}") };
            assert_eq!(sol.exploitability_chips.to_bits(), 0.0f32.to_bits(), "{raw:e}: noise is reported as +0.0");
            validate_solution(&sol, &req.tree.materialized).unwrap();
            let measured: Vec<f32> = seen.iter().filter_map(|r| r.2).collect();
            assert!(measured.len() >= 2 && measured.iter().all(|e| e.to_bits() == 0.0f32.to_bits()), "{raw:e}: progress carries the reported value only: {measured:?}");
            assert_eq!(*seen.last().unwrap(), (Stage::Extracting, sol.iterations, Some(sol.exploitability_chips), sol.memory_bytes));
            assert_eq!(lines.len(), 1, "{raw:e}: one diagnostic line per solve: {lines:?}");
            assert!(lines[0].contains(&format!("{raw:e}")), "the raw value is logged: {}", lines[0]);
        }

        let (outcome, seen, lines) = run_substituted(0.25);
        let sol = match outcome { JobOutcome::Ok(s) | JobOutcome::BestSoFar(s) => s, other => panic!("{other:?}") };
        assert_eq!(sol.exploitability_chips, 0.25);
        assert!(seen.iter().filter_map(|r| r.2).all(|e| e == 0.25));
        assert!(lines.is_empty(), "{lines:?}");

        for (raw, needle) in [(outside, "below"), (-0.01f32, "below"), (f32::NAN, "finite"), (f32::NEG_INFINITY, "finite")] {
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

    /// §7 at the stop point, on the full workload (P2.T11 review M1: behind `exhaustive`, since it
    /// allocates about 825 MB and depends on how much work fits in a real 2 s window; the default gate
    /// covers the same mapping with the scripted stop below): §13.0's `flop_best_so_far` fixture asks a
    /// 179 x 264-combo flop for 1 bp (0.018 chips) inside 2 s, which R8 puts far out of reach (0.5% alone
    /// takes 4-7 s on this size class), so the deadline ends the solve after at least one measurement
    /// and the job returns `BestSoFar`: every flop decision node exported, the reported exploitability
    /// above the target, and a solution `validate_solution` accepts.
    #[test]
    #[cfg_attr(not(feature = "exhaustive"), ignore = "enable the exhaustive feature for the full flop workload (about 825 MB)")]
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

    /// §7 at a non-target stop, in the default gate (P2.T11 review M1): the loop seam runs three real
    /// `solve_step` iterations and one real `compute_exploitability` on the river spot, reports the
    /// measurement, and stops as the deadline would, above the target. The job then really finalizes,
    /// exports and validates, and maps the stop to `BestSoFar`: every river decision node exported, the
    /// measured value reported unchanged, and a solution `validate_solution` accepts.
    #[test]
    fn a_non_target_stop_after_real_iterations_is_a_validated_best_so_far() {
        use postflop_solver::{compute_exploitability, solve_step};
        let req = solve_request("river_two_combo", 0);
        let target = (f64::from(req.pot) * f64::from(req.target_bp) / 10_000.0) as f32;
        let mut measured = None;
        let mut scripted = |game: &PostFlopGame, params: &solve_loop::LoopParams, cancel: &AtomicBool, progress: &mut dyn FnMut(u32, Option<f32>), _at_site: &mut dyn FnMut(LoopSite)| {
            assert_eq!(params.target_chips, target);
            for i in 0..3 {
                assert!(!cancel.load(Ordering::SeqCst));
                solve_step(game, i);
            }
            let e = compute_exploitability(game);
            assert!(e > target, "three iterations are far from the {target}-chip target: {e}");
            measured = Some(e);
            progress(3, Some(e));
            solve_loop::LoopOutcome { iterations: 3, exploitability: Some(e), reached_target: false, cancelled: false }
        };
        let (mut c, reports) = recording(|_| false);
        let mut probe = Probe::new(&c);
        let r = run_seamed(&req, None, &mut c, &mut probe, Some(&mut scripted));
        let t = Trace::new(r, &reports, probe);
        let e = measured.expect("the scripted loop ran");
        let sol = match &t.outcome { JobOutcome::BestSoFar(s) => s, other => panic!("{other:?}") };
        assert_eq!((sol.iterations, sol.exploitability_chips.to_bits()), (3, e.to_bits()));
        assert_eq!((sol.nodes.len(), sol.requested, sol.export.as_str(), sol.locks_applied), (3, 1, "street", 0));
        validate_solution(sol, &req.tree.materialized).unwrap();
        assert_eq!(t.started(), [Op::TreeBuild, Op::TreeCheck, Op::GameConfig, Op::MemoryCheck, Op::Allocate, Op::Solve, Op::Finalize, Op::Export, Op::Validate]);
        assert_eq!(t.events.last(), Some(&Event::At(Checkpoint::Validated)));
        assert_eq!(t.reports.iter().filter(|r| r.0 == Stage::Solving && r.2.is_some()).count(), 1);
        assert_eq!(*t.reports.last().unwrap(), (Stage::Extracting, 3, Some(e), sol.memory_bytes));
        assert!(t.lines.is_empty(), "{:?}", t.lines);
    }

    /// §3.4 (P2.T11 review M2): a `background: true` job runs at BELOW_NORMAL from before its first step
    /// until after its last, and every exit restores NORMAL exactly once: success, an early cancel, a
    /// Building error, and a panic unwinding out of a progress callback. A foreground job sets NORMAL
    /// and resets it the same way. The seam's setter records the calls in order with the job's other
    /// events; production passes `win::set_priority_class`.
    #[test]
    fn every_exit_restores_normal_priority() {
        let mut background = solve_request("river_two_combo", 0);
        background.background = true;
        background.target_bp = u16::MAX;                 // the first measurement reaches it: a short run
        let mut bad_pot = background.clone();
        bad_pot.pot = 0;
        let mut foreground = background.clone();
        foreground.background = false;

        let t = traced(&background, None, |_| false, None, None);
        assert!(matches!(t.outcome, JobOutcome::Ok(_)), "{:?}", t.outcome);
        assert_eq!(t.priority(), [true, false], "success");
        assert_eq!(t.log.first(), Some(&Event::Priority(true)), "set before the first step: {:?}", t.log);
        assert_eq!(t.log[t.log.len() - 2..], [Event::At(Checkpoint::Validated), Event::Priority(false)], "reset after the last step");

        let t = traced(&background, None, |_| false, Some(Event::At(Checkpoint::Start)), None);
        assert!(matches!(t.outcome, JobOutcome::Cancelled), "{:?}", t.outcome);
        assert_eq!(t.log, [Event::Priority(true), Event::At(Checkpoint::Start), Event::Priority(false)], "early cancel");

        let t = traced(&bad_pot, None, |_| false, None, None);
        match &t.outcome { JobOutcome::Error(e) => assert_eq!(e.code, "invalid_request"), other => panic!("{other:?}") }
        assert_eq!(t.priority(), [true, false], "Building error");
        assert_eq!(t.log.last(), Some(&Event::Priority(false)));

        // a panic in the progress callback, mid-solve, unwinds through the job and past the guard
        let (mut c, _) = recording(|r: &Report| if r.0 == Stage::Solving && r.2.is_some() { panic!("progress callback panics") } else { false });
        let mut probe = Probe::new(&c);
        let log = Rc::clone(&probe.log);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_seamed(&background, None, &mut c, &mut probe, None)));
        assert!(unwound.is_err(), "the callback's panic propagates");
        let log = log.borrow().clone();
        let calls: Vec<bool> = log.iter().filter_map(|e| match e { Event::Priority(b) => Some(*b), _ => None }).collect();
        assert_eq!(calls, [true, false], "unwinding: {log:?}");
        assert_eq!(log[log.len() - 2..], [Event::Enter(Op::Solve), Event::Priority(false)], "reset while unwinding out of the loop");

        let t = traced(&foreground, None, |_| false, None, None);
        assert!(matches!(t.outcome, JobOutcome::Ok(_)), "{:?}", t.outcome);
        assert_eq!(t.priority(), [false, false], "a foreground job sets NORMAL and resets it");
        assert_eq!(t.log.last(), Some(&Event::Priority(false)));
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
