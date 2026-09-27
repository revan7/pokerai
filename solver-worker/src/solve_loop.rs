use postflop_solver::{compute_exploitability, solve_step, PostFlopGame};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub struct LoopParams { pub deadline_ms: u32, pub extraction_margin_ms: u32, pub target_chips: f32, pub started: Instant }
#[derive(Debug, Clone, Copy)]
pub struct LoopOutcome { pub iterations: u32, pub exploitability: Option<f32>, pub reached_target: bool, pub cancelled: bool }

/// §7 stop rule, as pure arithmetic so it can be tested without a solve: stop when
/// `elapsed + 1.5 * max_iteration_so_far + (one more iteration if an exploitability pass is due) + margin > deadline`.
/// `compute_exploitability` costs about one iteration (measured, R8).
pub fn should_stop(elapsed_ms: f64, max_iter_ms: f64, expl_due: bool, margin_ms: f64, deadline_ms: f64) -> bool {
    elapsed_ms + 1.5 * max_iter_ms + if expl_due { max_iter_ms } else { 0.0 } + margin_ms > deadline_ms
}
/// §7 cadence: every 10 iterations, and additionally whenever fewer than 10 iterations still fit before the stop point.
pub fn expl_due(next_iteration: u32, fits: f64) -> bool { next_iteration % 10 == 0 || fits < 10.0 }

/// R1 (fix round 1): the conservative nonzero cost bound assumed for the very first iteration,
/// before any real per-iteration cost has been measured. Without this, `max_iter_ms` starts at
/// exactly `0.0` and the §7 stop formula's `1.5 * max_iter_ms` safety term vanishes, so admission
/// of the first (non-interruptible) `solve_step` was effectively unconditional even against an
/// almost-exhausted budget. This cannot be a true worst-case bound -- the solver's own cost is
/// opaque until measured, so an unexpectedly slow first iteration can still overrun -- but it
/// stops the loop from ever treating an unmeasured iteration as free.
const FIRST_ITERATION_COST_BOUND_MS: f64 = 10.0;

fn fits(deadline_ms: f64, elapsed_ms: f64, margin_ms: f64, iter_cost_ms: f64) -> f64 {
    if iter_cost_ms > 0.0 { (deadline_ms - elapsed_ms - margin_ms) / iter_cost_ms } else { f64::INFINITY }
}

/// Where the loop is, as `run` reports it to its caller immediately before it happens (the job's hooks seam,
/// `job::Hooks::loop_site`), each with the number of iterations completed so far. Its three cancel polls:
/// `Boundary` at the top of every pass, before the first iteration and between two (§4.5's "next iteration
/// boundary"); `Stepped` right after a solve step, before any measurement; `Measured` right after a measurement,
/// before its outcome is published. The work between them: `Iteration(n)`, the n-th solve step, about to run;
/// `Measurement(n)`, an exploitability measurement after n iterations, about to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopSite { Boundary(u32), Stepped(u32), Measured(u32), Iteration(u32), Measurement(u32) }

/// The operations and clock the scheduling loop drives, factored out of `run` so the deadline and
/// cancellation policy in `run_loop` -- the part under test for R1/R2 -- can be exercised
/// deterministically against scripted costs and a fake clock (`testutil::FakeOps`), never a real
/// solve and never a timed sleep. `run` is the only production caller, via `RealOps`.
trait LoopOps {
    fn elapsed_ms(&self) -> f64;
    fn run_iteration(&mut self, iteration: u32) -> f64;
    fn measure_exploitability(&mut self) -> (f32, f64);
    /// Called immediately before each poll and each operation (`LoopSite`); it neither polls nor decides
    /// anything. `RealOps` hands it to `run`'s caller; the fakes keep this no-op.
    fn at_site(&mut self, _at: LoopSite) {}
}

struct RealOps<'g, 'h> { game: &'g PostFlopGame, started: Instant, at_site: &'h mut dyn FnMut(LoopSite) }
impl LoopOps for RealOps<'_, '_> {
    fn at_site(&mut self, at: LoopSite) { (self.at_site)(at) }
    fn elapsed_ms(&self) -> f64 { self.started.elapsed().as_secs_f64() * 1000.0 }
    fn run_iteration(&mut self, iteration: u32) -> f64 {
        let t = Instant::now();
        solve_step(self.game, iteration);
        t.elapsed().as_secs_f64() * 1000.0
    }
    fn measure_exploitability(&mut self) -> (f32, f64) {
        let t = Instant::now();
        let e = compute_exploitability(self.game);
        (e, t.elapsed().as_secs_f64() * 1000.0)
    }
}

/// The §7 loop on `game`. `at_site` is called immediately before each of its cancel polls and each of its
/// operations (`LoopSite`); the job passes its hooks' `loop_site`, a no-op in production.
pub fn run(game: &PostFlopGame, p: &LoopParams, cancel: &AtomicBool, progress: impl FnMut(u32, Option<f32>), at_site: &mut dyn FnMut(LoopSite)) -> LoopOutcome {
    let mut ops = RealOps { game, started: p.started, at_site };
    run_loop(&mut ops, p.deadline_ms as f64, p.extraction_margin_ms as f64, p.target_chips, cancel, progress)
}

/// The §7/§10.3 scheduling core, generic over `LoopOps` (see its doc above). Takes `&mut O` (not
/// an owned `O`) so tests can keep their own `FakeOps` binding and inspect its call counts after
/// the loop returns. `run` is the only production caller.
fn run_loop<O: LoopOps>(
    ops: &mut O,
    deadline_ms: f64,
    margin_ms: f64,
    target_chips: f32,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u32, Option<f32>),
) -> LoopOutcome {
    let mut iters = 0u32;
    let mut max_iter_ms = 0.0f64;
    let mut expl: Option<f32> = None;
    let mut last_progress_ms = 0.0f64;
    loop {
        // Checkpoint: nothing further starts once cancellation is observed (§4.5).
        ops.at_site(LoopSite::Boundary(iters));
        if cancel.load(Ordering::SeqCst) {
            return LoopOutcome { iterations: iters, exploitability: expl, reached_target: false, cancelled: true };
        }
        // R1: the first iteration has no measured cost yet -- use the conservative bound instead
        // of treating it as free.
        let next_cost_bound = if iters == 0 { FIRST_ITERATION_COST_BOUND_MS } else { max_iter_ms };
        let elapsed = ops.elapsed_ms();
        let due_guess = expl_due(iters + 1, fits(deadline_ms, elapsed, margin_ms, next_cost_bound));
        if should_stop(elapsed, next_cost_bound, due_guess, margin_ms, deadline_ms) { break; }

        ops.at_site(LoopSite::Iteration(iters + 1));
        let step_cost = ops.run_iteration(iters);
        iters += 1;
        max_iter_ms = max_iter_ms.max(step_cost);

        // R2: poll cancellation immediately after the (expensive, non-interruptible) solve step,
        // before starting a measurement.
        ops.at_site(LoopSite::Stepped(iters));
        if cancel.load(Ordering::SeqCst) {
            return LoopOutcome { iterations: iters, exploitability: expl, reached_target: false, cancelled: true };
        }

        let elapsed = ops.elapsed_ms();
        if expl_due(iters, fits(deadline_ms, elapsed, margin_ms, max_iter_ms)) {
            // R1: recheck the absolute remaining budget with the just-updated per-iteration cost
            // estimate, reserving that cost plus the extraction margin, before spending it on a
            // measurement. Skip the measurement (keep `expl` as whatever it already was, `None`
            // if never measured) when it cannot fit -- the top-of-loop check above ends the loop
            // on the next pass if nothing changes.
            let remaining_after_margin = deadline_ms - elapsed - margin_ms;
            if remaining_after_margin >= max_iter_ms {
                ops.at_site(LoopSite::Measurement(iters));
                let (value, _measure_cost) = ops.measure_exploitability();
                expl = Some(value);

                // R2: poll cancellation immediately after the measurement and before publishing
                // a target-reached outcome -- an early return here must not bypass this checkpoint.
                ops.at_site(LoopSite::Measured(iters));
                if cancel.load(Ordering::SeqCst) {
                    return LoopOutcome { iterations: iters, exploitability: expl, reached_target: false, cancelled: true };
                }
                if value <= target_chips {
                    progress(iters, expl);
                    return LoopOutcome { iterations: iters, exploitability: expl, reached_target: true, cancelled: false };
                }
            }
        }
        let elapsed = ops.elapsed_ms();
        if elapsed - last_progress_ms >= 100.0 { progress(iters, expl); last_progress_ms = elapsed; }
    }
    LoopOutcome { iterations: iters, exploitability: expl, reached_target: false, cancelled: false }
}

/// Deterministic fakes for `run_loop`'s scheduling policy (R1/R2 regressions), never a real solve
/// and never a timed sleep -- see `LoopOps`'s doc.
#[cfg(test)]
mod testutil {
    use super::LoopOps;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Scripted costs (and, for measurements, values) drive a fake clock that only advances when
    /// an operation runs, so every budget/cadence decision in `run_loop` is fully deterministic.
    /// `cancel_after_step`/`cancel_after_measurement` are the "stepping hook": the fake flips the
    /// shared cancellation flag itself, as a side effect of the Nth call, letting a test place
    /// cancellation exactly between two operations without a real thread or a timed sleep.
    pub struct FakeOps<'c> {
        clock_ms: Cell<f64>,
        step_costs: Vec<f64>,
        step_calls: Cell<usize>,
        measurements: Vec<(f32, f64)>,
        measure_calls: Cell<usize>,
        cancel: &'c AtomicBool,
        cancel_after_step: Option<usize>,
        cancel_after_measurement: Option<usize>,
    }

    impl<'c> FakeOps<'c> {
        pub fn new(cancel: &'c AtomicBool) -> Self {
            FakeOps {
                clock_ms: Cell::new(0.0),
                step_costs: Vec::new(),
                step_calls: Cell::new(0),
                measurements: Vec::new(),
                measure_calls: Cell::new(0),
                cancel,
                cancel_after_step: None,
                cancel_after_measurement: None,
            }
        }
        /// Costs (ms) returned by successive `run_iteration` calls, in order; the last entry
        /// repeats if more calls happen than scripted.
        pub fn with_step_costs(mut self, costs: &[f64]) -> Self { self.step_costs = costs.to_vec(); self }
        /// (exploitability value, cost ms) returned by successive `measure_exploitability` calls.
        pub fn with_measurements(mut self, values: &[(f32, f64)]) -> Self { self.measurements = values.to_vec(); self }
        /// Sets the shared cancel flag immediately after the Nth (1-indexed) `run_iteration` call.
        pub fn cancel_after_step(mut self, n: usize) -> Self { self.cancel_after_step = Some(n); self }
        /// Sets the shared cancel flag immediately after the Nth (1-indexed) `measure_exploitability` call.
        pub fn cancel_after_measurement(mut self, n: usize) -> Self { self.cancel_after_measurement = Some(n); self }
        pub fn step_call_count(&self) -> usize { self.step_calls.get() }
        pub fn measure_call_count(&self) -> usize { self.measure_calls.get() }
    }

    impl LoopOps for FakeOps<'_> {
        fn elapsed_ms(&self) -> f64 { self.clock_ms.get() }
        fn run_iteration(&mut self, _iteration: u32) -> f64 {
            let call = self.step_calls.get() + 1;
            self.step_calls.set(call);
            let cost = *self.step_costs.get(call - 1).or_else(|| self.step_costs.last()).expect("scripted step cost");
            self.clock_ms.set(self.clock_ms.get() + cost);
            if self.cancel_after_step == Some(call) { self.cancel.store(true, Ordering::SeqCst); }
            cost
        }
        fn measure_exploitability(&mut self) -> (f32, f64) {
            let call = self.measure_calls.get() + 1;
            self.measure_calls.set(call);
            let (value, cost) = *self.measurements.get(call - 1).or_else(|| self.measurements.last()).expect("scripted measurement");
            self.clock_ms.set(self.clock_ms.get() + cost);
            if self.cancel_after_measurement == Some(call) { self.cancel.store(true, Ordering::SeqCst); }
            (value, cost)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stop_rule_of_section_7() {
        // stop when elapsed + 1.5 * max_iteration + (exploitability pass if due) + margin > deadline
        assert!(!should_stop(1000.0, 200.0, false, 200.0, 2000.0));            // 1000 + 300 + 200 = 1500 <= 2000
        assert!(!should_stop(1500.0, 200.0, false, 200.0, 2000.0));            // 1500 + 300 + 200 = 2000, not strictly greater
        assert!(should_stop(1501.0, 200.0, false, 200.0, 2000.0));
        assert!(should_stop(1500.0, 200.0, true, 200.0, 2000.0));              // the due exploitability pass costs one iteration
        assert!(!should_stop(0.0, 0.0, false, 200.0, 300.0));                  // before the first iteration only the margin counts
        assert!(should_stop(0.0, 0.0, false, 600.0, 300.0));                   // margin alone exceeds the deadline: no_iteration
    }
    #[test]
    fn exploitability_cadence() {
        // every 10 iterations ...
        assert!(expl_due(10, 1000.0) && expl_due(20, 1000.0) && !expl_due(11, 1000.0));
        // ... and additionally whenever fewer than 10 iterations still fit
        assert!(expl_due(3, 9.5) && expl_due(1, 0.0));
        assert!(!expl_due(3, 10.0));
    }

    use super::testutil::FakeOps;
    use std::sync::atomic::AtomicBool;

    // R1 (fix round 1), "an already-consumed budget": margin alone already exceeds the deadline,
    // so no iteration -- not even the first -- can be admitted, and no expensive operation runs.
    #[test]
    fn declines_all_work_when_even_the_first_iteration_cannot_fit() {
        let cancel = AtomicBool::new(false);
        let mut ops = FakeOps::new(&cancel).with_step_costs(&[80.0]);
        let outcome = run_loop(&mut ops, 100.0, 150.0, 0.0, &cancel, |_, _| {});
        assert_eq!((outcome.iterations, outcome.exploitability, outcome.reached_target, outcome.cancelled), (0, None, false, false));
        assert_eq!((ops.step_call_count(), ops.measure_call_count()), (0, 0));
    }

    // R1, "the first iteration": before any real cost is measured, the loop must still refuse to
    // start when even the conservative nonzero cost bound cannot fit -- treating the unmeasured
    // first iteration as free (the pre-fix `max_iter_ms = 0.0`) would have proceeded here, since
    // `0 + 1.5*0 + 90 = 90 <= 100`; the conservative bound correctly declines instead, and
    // `run_iteration` is never called.
    #[test]
    fn first_iteration_uses_a_conservative_cost_bound_not_zero() {
        let cancel = AtomicBool::new(false);
        let mut ops = FakeOps::new(&cancel).with_step_costs(&[5.0]);
        let outcome = run_loop(&mut ops, 100.0, 90.0, 0.0, &cancel, |_, _| {});
        assert_eq!((outcome.iterations, outcome.exploitability, outcome.cancelled), (0, None, false));
        assert_eq!(ops.step_call_count(), 0);
    }

    // R1, "an unexpectedly slow iteration" + "a newly due measurement": the reviewer's own worked
    // example (300 ms deadline, 200 ms margin, 80 ms first iteration). After that iteration only
    // 20 ms remain past the margin, less than the now-known 80 ms iteration cost, so the due
    // measurement must be skipped rather than started unconditionally: `exploitability` stays
    // `None` and `measure_exploitability` is never called.
    #[test]
    fn measurement_is_skipped_when_the_updated_cost_estimate_no_longer_fits() {
        let cancel = AtomicBool::new(false);
        let mut ops = FakeOps::new(&cancel).with_step_costs(&[80.0]);
        let outcome = run_loop(&mut ops, 300.0, 200.0, 0.0, &cancel, |_, _| {});
        assert_eq!((outcome.iterations, outcome.exploitability, outcome.reached_target, outcome.cancelled), (1, None, false, false));
        assert_eq!((ops.step_call_count(), ops.measure_call_count()), (1, 0));
    }

    // R1, "a newly due measurement" (positive control): same shape, but with enough budget left
    // after the first iteration that the measurement fits; it runs, reaches target, and is
    // reported.
    #[test]
    fn measurement_proceeds_and_reaches_target_when_budget_allows() {
        let cancel = AtomicBool::new(false);
        let mut ops = FakeOps::new(&cancel).with_step_costs(&[95.0]).with_measurements(&[(5.0, 5.0)]);
        let outcome = run_loop(&mut ops, 1000.0, 100.0, 1e9, &cancel, |_, _| {});
        assert_eq!((outcome.iterations, outcome.exploitability, outcome.reached_target, outcome.cancelled), (1, Some(5.0), true, false));
        assert_eq!((ops.step_call_count(), ops.measure_call_count()), (1, 1));
    }

    // R2: cancellation observed immediately after a completed (non-interruptible) solve step must
    // stop the loop before any further expensive work -- even though a measurement would
    // otherwise have been due right after this step. `measure_exploitability` is never called.
    #[test]
    fn cancellation_after_solve_step_stops_before_any_measurement() {
        let cancel = AtomicBool::new(false);
        let mut ops = FakeOps::new(&cancel).with_step_costs(&[80.0]).cancel_after_step(1);
        let outcome = run_loop(&mut ops, 200.0, 100.0, 0.0, &cancel, |_, _| {});
        assert_eq!((outcome.iterations, outcome.exploitability, outcome.reached_target, outcome.cancelled), (1, None, false, true));
        assert_eq!((ops.step_call_count(), ops.measure_call_count()), (1, 0));
    }

    // R2: cancellation observed immediately after a measurement must stop the loop before
    // publishing a target-reached outcome -- the early return on `value <= target_chips` must not
    // bypass this checkpoint even though the measured value did reach target, and no second
    // measurement or iteration follows.
    #[test]
    fn cancellation_after_measurement_stops_before_publishing_target_reached() {
        let cancel = AtomicBool::new(false);
        let mut ops = FakeOps::new(&cancel).with_step_costs(&[95.0]).with_measurements(&[(5.0, 5.0)]).cancel_after_measurement(1);
        let outcome = run_loop(&mut ops, 1000.0, 100.0, 1e9, &cancel, |_, _| {});
        assert_eq!((outcome.iterations, outcome.exploitability, outcome.reached_target, outcome.cancelled), (1, Some(5.0), false, true));
        assert_eq!((ops.step_call_count(), ops.measure_call_count()), (1, 1));
    }
}
