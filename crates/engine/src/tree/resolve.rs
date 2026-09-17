//! Node addressing: the §2 chip-path rule (re-exported from `proto`) and the ordinal lookup.

use proto::MaterializedNode;

/// The single implementation of the §2 chip-path rule lives in `proto`; every consumer re-exports it
/// so cache, replay and worker can never disagree (cross-plan M21/D2).
pub use proto::resolve_chip_path;

pub fn node_at<'a>(materialized: &'a [MaterializedNode], path: &[u8]) -> Option<&'a MaterializedNode> {
    materialized.iter().find(|n| n.path.as_slice() == path)
}
