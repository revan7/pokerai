//! `core-replay`: Bayesian public-range replay across streets (spec section 9). Every seat's
//! public range is carried as the shared history-branch list of spec section 8.4 -- one ordered
//! list of branches, each with a branch weight `q` and every seat's un-normalized per-combo masses
//! -- whose types and conditioning kernel live in `core_preflop::branches` (so that `core-preflop`
//! never depends on this crate) and are re-exported here at the crate root.

pub mod branches;
pub mod preflop;
pub mod snapshot;
pub use branches::*; // SeatMass, HistoryBranch, initial, marginal, posterior, condition, rescale, range_output,
                     // cap_branches, split_action, split_batch, BranchChoice, BatchSplit, residual_reason,
                     // stop_branch, missing_reason, zero_reason
pub use preflop::*; // ReplayInput, ReplayOutput, replay, walk_preflop, apply_preflop_action, query_translated,
                    // board_mask, block_and_rescale, publish
pub use snapshot::*; // SnapshotKey, SnapshotProvenance, StreetSnapshot
// added by later tasks in this plan:
// pub mod postflop;                 // Task 15: walk_postflop
// snapshot.rs gains SnapshotStore, select_snapshot, covered_prefix (Task 14)
// P3.T16: the branch-supported assembly of hero's current preflop decision (spec section 8.4).
pub use core_preflop::{mix_action, mix_nodes, range_mix_weight, BranchNode, MixedNode};
