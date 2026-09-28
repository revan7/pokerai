//! The snapshot store of spec §9.2: validated street solutions keyed by the decision identity they were solved for.
//!
//! Plan 3 Task 14 replaced this plan's temporary `SolvedStreet` store with `core_replay`'s, re-exported here, in one
//! commit: `StreetSnapshot` carries every `SolvedStreet` field (`board` as `key.root_board`, `ordinal_paths` as
//! `covered_paths`, `identity_at_solve`/`solved_prefix` in `provenance`, `street` in `key`), and
//! `SnapshotStore::register(&DecisionIdentity, StreetSnapshot) -> bool` keeps the identity rule: only the active
//! identity registers, a stale one is refused outright (§4.4). This is the single registration path for live results
//! (cross-plan M15, M16, D1, D6). A stored node list may be a truncated worker export (Task 26 Q1), so nothing here
//! reconstructs hero-combo support from stored nodes; that comes from the worker's available mask only.

pub use core_replay::{SnapshotKey, SnapshotProvenance, SnapshotStore, StreetSnapshot};
