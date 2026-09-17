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

pub fn run(game: &PostFlopGame, p: &LoopParams, cancel: &AtomicBool, mut progress: impl FnMut(u32, Option<f32>)) -> LoopOutcome {
    let deadline = p.deadline_ms as f64;
    let margin = p.extraction_margin_ms as f64;
    let elapsed = || p.started.elapsed().as_secs_f64() * 1000.0;
    let fits_now = |max_iter_ms: f64| if max_iter_ms > 0.0 { (deadline - elapsed() - margin) / max_iter_ms } else { f64::INFINITY };
    let mut iters = 0u32;
    let mut max_iter_ms = 0.0f64;
    let mut expl: Option<f32> = None;
    let mut last_progress = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) { return LoopOutcome { iterations: iters, exploitability: expl, reached_target: false, cancelled: true }; }
        let due = expl_due(iters + 1, fits_now(max_iter_ms));
        if should_stop(elapsed(), max_iter_ms, due, margin, deadline) { break; }
        let t = Instant::now();
        solve_step(game, iters);
        iters += 1;
        max_iter_ms = max_iter_ms.max(t.elapsed().as_secs_f64() * 1000.0);
        if expl_due(iters, fits_now(max_iter_ms)) {
            let e = compute_exploitability(game);
            expl = Some(e);
            if e <= p.target_chips { progress(iters, expl); return LoopOutcome { iterations: iters, exploitability: expl, reached_target: true, cancelled: false }; }
        }
        if last_progress.elapsed().as_millis() >= 100 { progress(iters, expl); last_progress = Instant::now(); }
    }
    LoopOutcome { iterations: iters, exploitability: expl, reached_target: false, cancelled: false }
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
}
