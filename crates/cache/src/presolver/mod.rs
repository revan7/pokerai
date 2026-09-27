//! Background pre-solver (spec section 10.5): the scenario tiers and canonical-flop enumeration
//! order (`scenarios`, task 13); the durable, resumable job queue persisted as `queue.json`,
//! iterated in `(tier, flop_order, scenario_order)`, with progress keyed by each slot's normalized
//! game identity, retry deadlines persisted as UTC, and completion verified by reading the cache
//! entry back (`queue`, task 14); and the `presolver` thread that runs queue jobs while the table is
//! idle and gives the worker back the moment live work arrives (`scheduler`, task 15).

pub mod queue;
pub mod scenarios;
pub mod scheduler;

/// Plan 5 and `Engine::presolver_status` name the parent path, so the status type is
/// re-exported here; `scheduler::PresolverStatus` remains its single definition.
pub use scheduler::PresolverStatus;

/// Defined here, before `scheduler` calls it (the scheduler's status publication uses
/// `crate::presolver::remaining_seconds`). `engine::log` re-exports it in Task 16.
pub fn remaining_seconds(pending:u32,measured_p50:Option<f64>)->Option<f64> {
    measured_p50.filter(|x|x.is_finite()&&*x>0.0).map(|x|pending as f64*x)
}

#[cfg(test)]
mod tests {
    #[test]
    fn remaining_seconds_needs_a_finite_positive_measurement() {
        assert_eq!(super::remaining_seconds(7,Some(27.0)),Some(189.0));
        assert_eq!(super::remaining_seconds(7,None),None);
        assert_eq!(super::remaining_seconds(7,Some(0.0)),None);
        assert_eq!(super::remaining_seconds(7,Some(f64::NAN)),None);
    }
}
