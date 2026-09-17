//! Decision identity of spec sections 4.4 and 5.

use proto::DecisionIdentity;

/// Session counters of §4.4/§5: revisions are never reused; a mutation, completion or
/// abandonment invalidates every earlier identity of the hand.
///
/// "Invalidates" is exact: after any of `set_config`, `begin_hand`, `mutate`, `invalidate_hand`
/// or `cancel_active`, `is_active` is false for every identity issued earlier, so an in-flight
/// result that arrives late can be dropped without ambiguity.
#[derive(Debug, Default)]
pub struct IdentityState {
    config_revision: u32,
    model_revision: u32,
    next_hand_id: u64,
    next_revision: u32,
    next_decision_id: u64,
    hand: Option<(u64, u32)>, // (hand_id, current hand_revision)
    active: Option<DecisionIdentity>,
}

/// Monotonic counters are the whole basis of "an identity is never reused", so the step is checked
/// in the counter's own width on every issue rather than being allowed to wrap in a release build.
fn step_u32(counter: &mut u32, what: &str) -> u32 {
    let value = *counter;
    *counter = value.checked_add(1).unwrap_or_else(|| panic!("{what} counter overflowed u32"));
    value
}

fn step_u64(counter: &mut u64, what: &str) -> u64 {
    let value = *counter;
    *counter = value.checked_add(1).unwrap_or_else(|| panic!("{what} counter overflowed u64"));
    value
}

impl IdentityState {
    pub fn new() -> Self {
        Self { next_hand_id: 1, next_revision: 1, next_decision_id: 1, ..Default::default() }
    }

    /// A config change (stakes, rake, profile) re-bases every later identity and makes any
    /// in-flight decision stale, exactly as a hand mutation does.
    pub fn set_config(&mut self) -> u32 {
        self.config_revision =
            self.config_revision.checked_add(1).expect("config revision counter overflowed u32");
        self.active = None;
        self.config_revision
    }

    pub fn config_revision(&self) -> u32 {
        self.config_revision
    }

    pub fn begin_hand(&mut self) -> (u64, u32) {
        let hand_id = step_u64(&mut self.next_hand_id, "hand id");
        let rev = step_u32(&mut self.next_revision, "revision");
        self.hand = Some((hand_id, rev));
        self.active = None;
        (hand_id, rev)
    }

    /// Every apply_action / set_board / undo: fresh revision, in-flight work invalidated.
    pub fn mutate(&mut self) -> u32 {
        let rev = step_u32(&mut self.next_revision, "revision");
        if let Some(h) = self.hand.as_mut() {
            h.1 = rev;
        }
        self.active = None;
        rev
    }

    pub fn current_revision(&self) -> Option<u32> {
        self.hand.map(|h| h.1)
    }

    pub fn next_decision(&mut self) -> Option<DecisionIdentity> {
        let (hand_id, hand_revision) = self.hand?;
        let decision_id = step_u64(&mut self.next_decision_id, "decision id");
        let id = DecisionIdentity {
            hand_id,
            hand_revision,
            decision_id,
            config_revision: self.config_revision,
            model_revision: self.model_revision,
        };
        self.active = Some(id.clone());
        Some(id)
    }

    /// True only for the one identity issued most recently and not yet invalidated.
    ///
    /// The revision cross-check is deliberate redundancy: every mutator already clears `active`,
    /// and this makes a stale identity read false even if a later mutator were to forget to.
    pub fn is_active(&self, id: &DecisionIdentity) -> bool {
        self.active.as_ref() == Some(id)
            && self.hand == Some((id.hand_id, id.hand_revision))
            && id.config_revision == self.config_revision
            && id.model_revision == self.model_revision
    }

    pub fn active(&self) -> Option<&DecisionIdentity> {
        self.active.as_ref()
    }

    /// finish_hand / abandon_hand: no decision can be requested until the next begin_hand.
    pub fn invalidate_hand(&mut self) {
        self.hand = None;
        self.active = None;
    }

    pub fn cancel_active(&mut self) {
        self.active = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutation_invalidates_and_revisions_never_repeat() {
        let mut s = IdentityState::new();
        let cfg = s.set_config();
        let (hand, rev1) = s.begin_hand();
        let id1 = s.next_decision().unwrap();
        assert_eq!((id1.hand_id, id1.hand_revision, id1.config_revision, id1.model_revision), (hand, rev1, cfg, 0));
        assert!(s.is_active(&id1));
        let rev2 = s.mutate();
        assert!(rev2 > rev1);
        assert!(!s.is_active(&id1));
        let id2 = s.next_decision().unwrap();
        let id3 = s.next_decision().unwrap();
        assert!(id3.decision_id > id2.decision_id && !s.is_active(&id2) && s.is_active(&id3));
        s.invalidate_hand();
        assert!(s.next_decision().is_none() && !s.is_active(&id3));
    }

    /// A decision issued before any of the four invalidating calls must read as stale afterwards,
    /// and no two decisions of a session ever share an identity.
    #[test]
    fn every_invalidating_call_makes_an_earlier_decision_stale() {
        type Mutator = (&'static str, fn(&mut IdentityState));
        let mutators: [Mutator; 4] = [
            ("set_config", |s| { s.set_config(); }),
            ("mutate", |s| { s.mutate(); }),
            ("invalidate_hand", IdentityState::invalidate_hand),
            ("cancel_active", IdentityState::cancel_active),
        ];
        for (name, apply) in mutators {
            let mut s = IdentityState::new();
            s.set_config();
            s.begin_hand();
            let id = s.next_decision().unwrap();
            assert!(s.is_active(&id), "{name}: the freshly issued decision must start active");
            apply(&mut s);
            assert!(!s.is_active(&id), "{name}: the earlier decision must no longer be active");
        }
    }

    #[test]
    fn consecutive_decisions_never_share_an_id() {
        let mut s = IdentityState::new();
        s.begin_hand();
        let first = s.next_decision().unwrap();
        let second = s.next_decision().unwrap();
        assert_ne!(first.decision_id, second.decision_id, "two consecutive decisions must not share a decision_id");
        assert_ne!(first, second, "two consecutive decisions must not share an identity");

        // Across a longer session, including across hands and revisions, decision ids stay strictly
        // increasing, so a decision id is never reused for a different decision.
        let mut seen = vec![first.decision_id, second.decision_id];
        for round in 0..4 {
            s.begin_hand();
            s.set_config();
            for _ in 0..3 {
                s.mutate();
                let id = s.next_decision().unwrap();
                assert!(id.decision_id > *seen.last().unwrap(), "round {round}: decision ids must increase");
                assert!(!seen.contains(&id.decision_id), "round {round}: decision id {} was reused", id.decision_id);
                seen.push(id.decision_id);
            }
        }
    }
}
