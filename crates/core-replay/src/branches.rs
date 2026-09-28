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
//!
//! P3.T15 adds `cap_branches_with`, the kernel's cap with a side list (the postflop walk's
//! per-branch ordinal paths) kept aligned with the branches it caps.

pub use core_preflop::branches::*;

use proto::{Action, ApproxReason, Seat, Street};
use std::collections::BTreeMap;

/// [`cap_branches`] with a side list kept index-aligned with the branch list (P3.T15): `side[i]`
/// belongs to `bs[i]` before the cap; after it, every surviving live branch keeps its own entry,
/// reordered with it, the entries of the branches merged into the residual are dropped, and the
/// residual's entry is `residual()` (the residual has no history of its own, whether it existed
/// before the cap or the cap created it). The cap itself is [`cap_branches`], unchanged; entries are
/// matched to branches by id, which the cap never reassigns.
///
/// # Panics
/// Always, if `side` does not hold one entry per branch, if two branches share an id, or through
/// [`cap_branches`]'s own checks.
pub(crate) fn cap_branches_with<T>(bs: &mut Vec<HistoryBranch>, side: &mut Vec<T>, residual: impl Fn() -> T) {
    assert!(side.len() == bs.len(), "cap_branches_with: {} side entries for {} branches", side.len(), bs.len());
    let mut by_id: BTreeMap<u8, T> = BTreeMap::new();
    for (b, entry) in bs.iter().zip(side.drain(..)) {
        assert!(by_id.insert(b.id, entry).is_none(), "cap_branches_with: branch id {} appears more than once", b.id);
    }
    cap_branches(bs);
    side.extend(bs.iter().map(|b| {
        if b.residual {
            residual()
        } else {
            by_id.remove(&b.id).unwrap_or_else(|| panic!("cap_branches_with: live branch {} was not in the capped list", b.id))
        }
    }));
}

/// Stops branch `b` for the rest of the current street (spec section 9.3): records `cause`
/// (`"missing node <key>"`, or why the next action could not be mapped) and clears every seat's
/// node. `q` and every mass are left exactly as they are -- a stopped branch is frozen, which
/// [`condition`], [`split_action`] and [`split_batch`] then honour for every later action of the
/// street. It is the same stop [`BranchChoice::Stop`] applies inside a batch; the replay walk calls
/// it directly only on a rejected (zero-support) update, which keeps the pre-action list.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Branch `id` with weight `q`, uniform masses for two seats.
    fn branch(id: u8, q: f64) -> HistoryBranch {
        let mut b = initial(&[Seat(0), Seat(1)]).remove(0);
        b.id = id;
        b.q = q;
        b
    }

    /// Each surviving live branch keeps its own side entry in the capped order (by id), merged
    /// branches' entries are dropped, and the residual (created by this cap or already there) gets
    /// the residual entry.
    #[test]
    fn cap_branches_with_keeps_each_side_entry_with_its_branch() {
        let qs = [0.05, 0.2, 0.01, 0.15, 0.3, 0.1];
        let mut bs: Vec<HistoryBranch> = qs.iter().enumerate().map(|(i, &q)| branch(9 - i as u8, q)).collect();
        let mut side: Vec<String> = bs.iter().map(|b| format!("branch {}", b.id)).collect();
        cap_branches_with(&mut bs, &mut side, || "residual".to_string());
        let ids: Vec<(u8, bool)> = bs.iter().map(|b| (b.id, b.residual)).collect();
        // The four heaviest (q 0.3, 0.2, 0.15, 0.1: ids 5, 8, 6, 4) in id order, then the residual
        // created from the heaviest overflow (q 0.05, id 9), which absorbs id 7 (q 0.01).
        assert_eq!(ids, vec![(4, false), (5, false), (6, false), (8, false), (9, true)]);
        assert_eq!(side, vec!["branch 4", "branch 5", "branch 6", "branch 8", "residual"]);

        // A later cap with the residual already present merges the new lightest live branch (id 4,
        // q 0.1) into it and keeps it last, with the residual entry.
        let mut more = bs.clone();
        more.push(branch(10, 0.25));
        let mut side: Vec<String> = more.iter().map(|b| if b.residual { "stale".into() } else { format!("branch {}", b.id) }).collect();
        cap_branches_with(&mut more, &mut side, || "residual".to_string());
        assert_eq!(more.iter().map(|b| (b.id, b.residual)).collect::<Vec<_>>(), vec![(5, false), (6, false), (8, false), (10, false), (9, true)]);
        assert_eq!(side, vec!["branch 5", "branch 6", "branch 8", "branch 10", "residual"]);
    }

    #[test]
    #[should_panic(expected = "cap_branches_with: 1 side entries for 2 branches")]
    fn cap_branches_with_refuses_a_misaligned_side_list() {
        let mut bs = vec![branch(0, 0.5), branch(1, 0.5)];
        cap_branches_with(&mut bs, &mut vec![()], || ());
    }
}
