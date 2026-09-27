//! The shared history branches of spec sections 8.4 and 9.1 (`SeatMass`, `HistoryBranch`) and
//! their conditioning kernel (`initial`, `marginal`, `posterior`, `condition`, `rescale`,
//! `range_output`). The single definition lives in `core_preflop::branches`, which `core-preflop`
//! needs for its own translated-node assembly; `core-replay` re-exports it unchanged rather than
//! declaring a second copy or making `core-preflop` depend on this crate (plan 3 global
//! constraints: dependency direction is strictly downward).
//!
//! P3.T13 adds the replay walk's branch-level stop and its reasons (spec section 9.3): a branch
//! that stops on the preflop street keeps `q` and every seat's masses frozen, and no seat has a
//! node in it; the reasons name the seat and the cause.

pub use core_preflop::branches::*;

use proto::{Action, ApproxReason, Seat, Street};

/// Stops branch `b` for the rest of the current street (spec section 9.3): records `cause`
/// ("missing node <key>", or why the next action could not be mapped) and clears every seat's
/// node. `q` and every mass are left exactly as they are -- a stopped branch is frozen, which
/// [`condition`] and [`split_action`] then honour for every later action of the street.
pub fn stop_branch(b: &mut HistoryBranch, cause: String) {
    b.stopped = Some(cause);
    for s in &mut b.seats {
        s.node = None;
    }
}

/// The reason a preflop branch stopped because `seat`'s node is absent (spec section 9.3):
/// `UnconditionedPriorStreet{Preflop, seat, cause: "missing node <key>"}`.
pub fn missing_reason(seat: Seat, key: &str) -> ApproxReason {
    ApproxReason::UnconditionedPriorStreet { street: Street::Preflop, seat, cause: format!("missing node {key}") }
}

/// The reason an observed action was rejected by the zero-support rule (spec section 9.2): no
/// branch that applied it had positive integrated likelihood, so every pre-action `q` and mass is
/// kept -- `UnconditionedPriorStreet{street, seat, cause: "zero support after <action>"}`.
pub fn zero_reason(street: Street, seat: Seat, action: &Action) -> ApproxReason {
    ApproxReason::UnconditionedPriorStreet { street, seat, cause: format!("zero support after {action:?}") }
}
