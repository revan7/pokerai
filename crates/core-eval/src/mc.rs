//! Monte Carlo equity (Task 25); this task ships exact enumeration only.
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use crate::equity::{EquityRequest, EquityResult};

/// Complete, compiling dispatcher target for this task; Task 25 replaces the whole file.
/// It panics rather than returning a plausible status so a mis-ordered execution fails loudly
/// instead of silently reporting a budget overrun for every Monte Carlo request.
pub fn monte_carlo(_req: &EquityRequest, _seed: u64, _max_samples: u32, _budget: Duration, _cancel: &AtomicBool) -> EquityResult {
    unreachable!("Monte Carlo lands in Task 25; no test of Task 24 requests EquityMode::MonteCarlo")
}
