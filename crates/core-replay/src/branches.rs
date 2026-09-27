//! The shared history branches of spec sections 8.4 and 9.1 (`SeatMass`, `HistoryBranch`) and
//! their conditioning kernel (`initial`, `marginal`, `posterior`, `condition`, `rescale`,
//! `range_output`). The single definition lives in `core_preflop::branches`, which `core-preflop`
//! needs for its own translated-node assembly; `core-replay` re-exports it unchanged rather than
//! declaring a second copy or making `core-preflop` depend on this crate (plan 3 global
//! constraints: dependency direction is strictly downward).

pub use core_preflop::branches::*;
