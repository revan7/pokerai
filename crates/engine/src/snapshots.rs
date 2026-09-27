//! The snapshot store of spec §9.2: validated street solutions keyed by the decision identity they were solved for.
//!
//! This is the single registration path for this plan's live river/turn results (cross-plan M15, M16, D1, D6). Plan 3
//! Task 14 replaces `SolvedStreet` with `core_replay::StreetSnapshot` and re-exports it from this module in one commit,
//! keeping `register(&DecisionIdentity, _) -> bool` and its identity rule unchanged. A stored node list may be a
//! truncated worker export (Task 26 Q1), so nothing here reconstructs hero-combo support from stored nodes; that comes
//! from the worker's available mask only.

use proto::worker::NodeStrategy;
use proto::{Action, ApproxReason, Card, DecisionIdentity, EffectiveTree, OrdinalPath, Seat, Street};

#[derive(Debug, Clone)]
pub struct SolvedStreet {
    pub identity_at_solve: DecisionIdentity,
    pub street: Street,
    pub board: Vec<Card>,
    pub tree: EffectiveTree,
    pub nodes: Vec<NodeStrategy>,
    pub ordinal_paths: Vec<OrdinalPath>,
    pub exploitability_chips: f32,
    pub reasons: Vec<ApproxReason>,
    pub solved_prefix: Vec<(Seat, Action)>,
}

/// Validated solutions keyed by identity: the single registration path of §9.2 for live river/turn results.
/// Plan 3 Task 14 replaces this with `core_replay::SnapshotStore`/`StreetSnapshot` in one commit and re-exports
/// it from this module; the identity rule below is the contract that survives that swap.
#[derive(Default)]
pub struct SnapshotStore {
    items: Vec<SolvedStreet>,
}

impl SnapshotStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rejects anything whose identity is not the active one (§4.4: stale results are never written). `active` is the
    /// identity the caller read from `IdentityState::active` at registration time; a result solved for any other
    /// identity, differing in any field, is refused and the store is left untouched. Re-registering the same decision
    /// (hand, street, decision id) replaces the earlier entry.
    pub fn register(&mut self, active: &DecisionIdentity, s: SolvedStreet) -> bool {
        if s.identity_at_solve != *active {
            return false;
        }
        self.items.retain(|x| {
            !(x.identity_at_solve.hand_id == s.identity_at_solve.hand_id
                && x.street == s.street
                && x.identity_at_solve.decision_id == s.identity_at_solve.decision_id)
        });
        self.items.push(s);
        true
    }

    /// Drops every snapshot of `hand_id` (finish, abandon or a mutation that invalidates the hand's replay).
    pub fn invalidate_hand(&mut self, hand_id: u64) {
        self.items.retain(|x| x.identity_at_solve.hand_id != hand_id);
    }

    /// The snapshots of `hand_id`, in registration order.
    pub fn for_hand(&self, hand_id: u64) -> Vec<&SolvedStreet> {
        self.items.iter().filter(|x| x.identity_at_solve.hand_id == hand_id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(hand: u64, rev: u32, dec: u64) -> DecisionIdentity { DecisionIdentity { hand_id: hand, hand_revision: rev, decision_id: dec, config_revision: 1, model_revision: 0 } }
    fn solved(i: DecisionIdentity, street: Street) -> SolvedStreet {
        SolvedStreet { identity_at_solve: i, street, board: vec![], tree: EffectiveTree { rules_version: 3, template_id: "t".into(), root_street: street, menus: Default::default(),
            add_allin_threshold: 0.0, force_allin_threshold: 0.0, merging_threshold: 0.0, wager_cap: 1, inserted: vec![], materialized: vec![] },
            nodes: vec![], ordinal_paths: vec![], exploitability_chips: 0.1, reasons: vec![], solved_prefix: vec![] }
    }
    #[test]
    fn only_the_active_identity_registers_and_a_hand_can_be_invalidated() {
        let mut s = SnapshotStore::new();
        let a = id(1, 7, 10);
        assert!(s.register(&a, solved(a.clone(), Street::River)));
        assert_eq!(s.for_hand(1).len(), 1);
        // a result whose identity is not the active one is refused outright (§4.4: stale results are never written)
        let stale = id(1, 6, 9);
        assert!(!s.register(&a, solved(stale, Street::River)));
        assert_eq!(s.for_hand(1).len(), 1);
        // a second decision of the same hand and street is kept alongside; re-registering the same decision replaces
        let b = id(1, 7, 11);
        assert!(s.register(&b, solved(b.clone(), Street::River)));
        assert!(s.register(&b, solved(b.clone(), Street::River)));
        assert_eq!(s.for_hand(1).len(), 2);
        let other = id(2, 8, 12);
        assert!(s.register(&other, solved(other.clone(), Street::Turn)));
        s.invalidate_hand(1);
        assert_eq!((s.for_hand(1).len(), s.for_hand(2).len()), (0, 1));
    }

    /// Re-registering a decision keeps the newest solution, not the first; an identity that differs from the active one
    /// in any single field (here only `config_revision`) is refused, and a refused result leaves the store untouched.
    #[test]
    fn re_registering_keeps_the_newest_solution_and_any_identity_difference_is_refused() {
        let mut s = SnapshotStore::new();
        let a = id(3, 2, 5);
        assert!(s.register(&a, solved(a.clone(), Street::Turn)));
        let mut newer = solved(a.clone(), Street::Turn);
        newer.exploitability_chips = 0.05;
        assert!(s.register(&a, newer));
        let kept = s.for_hand(3);
        assert_eq!((kept.len(), kept[0].exploitability_chips), (1, 0.05));
        let mut other_config = a.clone();
        other_config.config_revision = 2;
        let mut refused = solved(other_config, Street::Turn);
        refused.exploitability_chips = 9.0;
        assert!(!s.register(&a, refused));
        let kept = s.for_hand(3);
        assert_eq!((kept.len(), kept[0].exploitability_chips), (1, 0.05));
    }
}
