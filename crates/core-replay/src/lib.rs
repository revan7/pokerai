//! `core-replay`: Bayesian public-range replay across streets (spec section 9). Every seat's
//! public range is carried as the shared history-branch list of spec section 8.4 -- one ordered
//! list of branches, each with a branch weight `q` and every seat's un-normalized per-combo masses
//! -- whose types and conditioning kernel live in `core_preflop::branches` (so that `core-preflop`
//! never depends on this crate) and are re-exported here at the crate root.

pub mod branches;
pub use branches::*; // SeatMass, HistoryBranch, initial, marginal, posterior, condition, rescale, range_output
// added by later tasks in this plan:
// pub mod preflop; pub mod postflop; pub mod snapshot;
// pub use snapshot::*;              // SnapshotKey, SnapshotProvenance, StreetSnapshot,
//                                   // SnapshotStore, select_snapshot, covered_prefix
