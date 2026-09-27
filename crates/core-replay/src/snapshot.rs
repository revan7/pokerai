//! The replay snapshot records of spec section 9.1: one validated street solution as replay
//! consumes it -- its compatibility key, its immutable provenance, the materialized tree it was
//! solved on, the exported node strategies and the ordinal paths they cover.
//!
//! P3.T13 defines the records only, so that [`crate::ReplayInput::snapshots`] is typed; selection
//! (`SnapshotStore`, `select_snapshot`, `covered_prefix`) is Task 14 and the walk that consumes a
//! snapshot is Task 15. The fields are spec section 9.1's, verbatim.

use proto::worker::NodeStrategy;
use proto::{Action, ApproxReason, Card, DecisionIdentity, EffectiveTree, OrdinalPath, Seat, Street};
use serde::{Deserialize, Serialize};

/// What a snapshot is compatible with (spec section 9.2): the hand and revisions it was solved
/// under, the street, the street's root board, the hashes of the two public ranges it was solved
/// from, and the effective tree's signature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotKey {
    pub hand_id: u64,
    pub config_revision: u32,
    pub model_revision: u32,
    pub street: Street,
    pub root_board: Vec<Card>,
    pub root_range_hashes: [[u8; 32]; 2],
    pub tree_signature: String,
}

/// Where a snapshot came from (spec section 9.1): the identity under which the solution was
/// validated (immutable), the street history inserted when it was solved, and its origin
/// (`live`, `cache_exact`, `cache_approximate` or `cache_provisional`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotProvenance {
    pub identity_at_solve: DecisionIdentity,
    pub solved_prefix: Vec<(Seat, Action)>,
    pub origin: String,
}

/// One registered street solution (spec section 9.1): the materialized tree, the exported node
/// strategies, the ordinal path of every exported node (resolved from the wire chip paths at
/// registration, spec section 2), the solve's exploitability and the reasons it carries into every
/// result that consumes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreetSnapshot {
    pub key: SnapshotKey,
    pub provenance: SnapshotProvenance,
    pub tree: EffectiveTree,
    pub nodes: Vec<NodeStrategy>,
    pub covered_paths: Vec<OrdinalPath>,
    pub exploitability_chips: f32,
    pub reasons: Vec<ApproxReason>,
}
