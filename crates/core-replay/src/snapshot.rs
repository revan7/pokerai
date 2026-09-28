//! The replay snapshot records of spec section 9.1 and their selection and retention (section 9.2):
//! one validated street solution as replay consumes it -- its compatibility key, its immutable
//! provenance, the materialized tree it was solved on, the exported node strategies and the ordinal
//! paths they cover -- and the store the engine registers them in.
//!
//! P3.T13 defined the records. P3.T14 adds the compatibility predicate ([`CompatKey`],
//! [`compatible`]), export coverage of an observed street line ([`covered_prefix`]), selection
//! ([`select_snapshot`]) and the store ([`SnapshotStore`]): the active-identity registration gate
//! and the prefix-based mutation invalidation. The store replaces Plan 2's temporary
//! `engine::snapshots::SolvedStreet` store; `engine::snapshots` re-exports it. The walk that
//! consumes a selected snapshot is Task 15.

use core_model::lifecycle::simulate;
use core_model::street_root;
use core_preflop::interpolate;
use proto::worker::NodeStrategy;
use proto::{
    index_materialized, resolve_chip_path_indexed, Action, ApproxReason, Card, DecisionIdentity, EffectiveTree, HandState, MaterializedIndex,
    MaterializedNode, OrdinalPath, Seat, Street, StreetRootSnapshot,
};
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

// ---------------------------------------------------------------------------------------------
// Compatibility and selection (spec section 9.2).
// ---------------------------------------------------------------------------------------------

/// Exactly the six fields spec section 9.2's compatibility predicate compares: the same hand,
/// config revision and model revision, the same street, an equal root board and equal incoming
/// public range hashes (OOP then IP). The tree signature is provenance in [`SnapshotKey`], never
/// compared -- different trees compete when their incoming public roots match -- and replay has
/// none to offer, so a caller never has to fabricate one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatKey {
    pub hand_id: u64,
    pub config_revision: u32,
    pub model_revision: u32,
    pub street: Street,
    pub root_board: Vec<Card>,
    pub root_range_hashes: [[u8; 32]; 2],
}

impl CompatKey {
    /// The compared fields of `k`, dropping its tree signature.
    pub fn of(k: &SnapshotKey) -> CompatKey {
        CompatKey {
            hand_id: k.hand_id,
            config_revision: k.config_revision,
            model_revision: k.model_revision,
            street: k.street,
            root_board: k.root_board.clone(),
            root_range_hashes: k.root_range_hashes,
        }
    }
}

/// Spec section 9.2's compatibility predicate: `a` names the same hand, config and model
/// revisions, street, root board and incoming range hashes as `b`. A later hand revision or
/// decision id, or another tree, never breaks it.
pub fn compatible(a: &SnapshotKey, b: &CompatKey) -> bool {
    CompatKey::of(a) == *b
}

/// Spec section 9.2's export coverage of an observed street line: how many consecutive observed
/// actions of `history`, from the street root, the snapshot covers. An action counts when every
/// node it is taken at along the mapped line is exported (its ordinal path is in
/// `covered_paths`); an off-menu wager maps to every menu child its section 8.4 interpolation gives
/// a nonzero coefficient (both bracketing sizes strictly between two menu sizes, the nearest one
/// alone at an exact size or a clamp), and each of those children must then be exported for the
/// next action to count. Counting stops at the first uncovered action; the real walk (Task 15)
/// continues past it, this only measures coverage for selection. It also stops where the line
/// leaves the skeleton (a terminal edge, or no materialized node) and at an off-menu action that is
/// not a wager or has no interpolation.
///
/// Every mapped line is a chip path resolved into its ordinal path by spec section 2's single rule
/// (`proto::resolve_chip_path_indexed`, over one index of `snapshot.tree.materialized` built per
/// call). An off-menu wager's children come from Task 10's `interpolate` over chip amounts beyond
/// the node's wager level (see `translated_children`), which selects exactly the children the
/// pot-fraction interpolation selects.
pub fn covered_prefix(snapshot: &StreetSnapshot, history: &[(Seat, Action)]) -> usize {
    let index = index_materialized(&snapshot.tree.materialized);
    let mut frontier: Vec<Vec<Action>> = vec![vec![]];
    let mut count = 0usize;
    for (_seat, action) in history {
        let mut next: Vec<Vec<Action>> = Vec::new();
        for chips in &frontier {
            // The mapped line must still name a decision node of the skeleton, and that node must be exported.
            let Some(ordinal) = resolve_chip_path_indexed(&index, chips) else { return count };
            if node_by_path(snapshot, &ordinal).is_none() {
                return count;
            }
            let node = *index.get(ordinal.as_slice()).expect("a resolved ordinal path names an indexed node");
            match node.actions.iter().position(|a| a == action) {
                Some(i) => next.push(extended(chips, node.actions[i])),
                None => {
                    let Some(level) = wager_level(&index, &ordinal, node.street) else { return count };
                    let Some(children) = translated_children(node, level, action) else { return count };
                    next.extend(children.into_iter().map(|i| extended(chips, node.actions[i])));
                }
            }
        }
        if next.is_empty() {
            return count;
        }
        frontier = next;
        count += 1;
    }
    count
}

/// Spec section 9.2's selection among `snapshots`: of those [`compatible`] with `key`, the longest
/// [`covered_prefix`] of the observed street `history` wins, then the lower raw
/// `exploitability_chips` (no display rounding), then the larger `decision_id`. Selection measures
/// export coverage, never a guessed likelihood of the whole hand, and never requires coverage of
/// every later action: a compatible snapshot covering none of the line is still a candidate.
/// `None` when no snapshot is compatible.
pub fn select_snapshot<'a>(snapshots: &'a [StreetSnapshot], key: &CompatKey, history: &[(Seat, Action)]) -> Option<&'a StreetSnapshot> {
    snapshots
        .iter()
        .filter(|s| compatible(&s.key, key))
        .map(|s| (covered_prefix(s, history), s))
        .max_by(|(pa, a), (pb, b)| {
            pa.cmp(pb)
                .then_with(|| b.exploitability_chips.total_cmp(&a.exploitability_chips))
                .then_with(|| a.provenance.identity_at_solve.decision_id.cmp(&b.provenance.identity_at_solve.decision_id))
        })
        .map(|(_, s)| s)
}

/// A street's position in the hand: preflop 0, flop 1, turn 2, river 3.
pub fn street_number(s: Street) -> u8 {
    match s {
        Street::Preflop => 0,
        Street::Flop => 1,
        Street::Turn => 2,
        Street::River => 3,
    }
}

/// The observed actions of `street`, in order, as `(seat, action)` pairs: every seat's, including
/// the actions of players who have folded. This is not the domain of a snapshot's `solved_prefix`,
/// which holds a street root's history (spec section 10.2 drops the actions of players who folded on
/// the street from a projected root; see `decision_roots`). Task 15's walk consumes it.
pub fn street_history(s: &HandState, street: Street) -> Vec<(Seat, Action)> {
    s.actions.iter().filter(|a| a.street == street).map(|a| (a.seat, a.action)).collect()
}

/// `street`'s root board as the state records it: the first `street.board_len()` cards, or every
/// card on record when the board is shorter (a street not dealt yet, which then matches no
/// snapshot's root board). Task 15's walk consumes it too.
pub fn root_board(s: &HandState, street: Street) -> Vec<Card> {
    s.board.iter().take(street.board_len()).copied().collect()
}

/// The street root of every hero decision on `street` that `state`'s history passes through, in
/// cutoff order: the model-based cutoff recovery that ruling 14-I1 makes the shared approach for
/// invalidation (rule (4) of [`SnapshotStore::invalidate`]) and for Task 15's `snapshot_root`.
///
/// For each cutoff of the street's observed actions, from none of them to all of them, the hand is
/// truncated there with the board on record up to `street`'s root board, and the model replays it
/// (`core_model::lifecycle::simulate`, which also sets the truncated state's phase and `Derived`).
/// Where that is hero's decision, `core_model::street_root` returns its genuine heads-up root or its
/// admitted projected root (spec section 10.2: the players who folded on the street removed, their
/// actions dropped from `history`, admitted only if the projection reproduces the decision). That
/// root's `history` is exactly the domain the engine registers a snapshot's `solved_prefix` in, so
/// the cutoff whose root history equals a solved prefix is the decision the snapshot was solved for.
/// A cutoff with no root contributes nothing: someone other than hero to act, the street closed, a
/// multiway decision, a projection that does not reproduce, a truncation that does not replay, or a
/// state without hero's cards (`core_model::is_decision_point` requires them). The seat pair is never
/// inferred from a prefix, and no folded player's actions are stripped: the model decides both.
/// Empty when `state` has not dealt `street`. A root recovered on another street than `street` would
/// be a model inconsistency and is asserted against (always on).
pub(crate) fn decision_roots(state: &HandState, street: Street) -> Vec<StreetRootSnapshot> {
    let board = root_board(state, street);
    if board.len() != street.board_len() {
        return vec![];
    }
    let before = state.actions.iter().take_while(|a| street_number(a.street) < street_number(street)).count();
    let count = state.actions[before..].iter().take_while(|a| a.street == street).count();
    (0..=count)
        .filter_map(|cut| {
            let mut at = state.clone();
            at.actions.truncate(before + cut);
            at.board = board.clone();
            let sim = simulate(&at).ok()?;
            at.derived = sim.derived();
            at.phase = sim.phase;
            let root = street_root(&at).ok()?;
            // A cutoff of this street's actions, on this street's board, replays on this street.
            assert!(root.street == street, "a cutoff of the {street:?} actions replayed to a {:?} root", root.street);
            Some(root)
        })
        .collect()
}

/// Task 15's `snapshot_node_at`, forward-declared under a private name: the exported strategy at
/// ordinal `path`, found through `covered_paths` (which lists the exported nodes in order).
fn node_by_path<'a>(s: &'a StreetSnapshot, path: &[u8]) -> Option<&'a NodeStrategy> {
    s.covered_paths.iter().position(|p| p.as_slice() == path).and_then(|i| s.nodes.get(i))
}

/// `chips` followed by `action`: a child's chip path.
fn extended(chips: &[Action], action: Action) -> Vec<Action> {
    let mut child = chips.to_vec();
    child.push(action);
    child
}

/// The amount a wager-sized action (`Bet`, `Raise`, `AllIn`) takes its actor's street contribution
/// to; `None` for `Fold`, `Check` and `Call`, which carry no size.
fn wager_to(a: &Action) -> Option<u32> {
    match a {
        Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => Some(*to),
        Action::Fold | Action::Check | Action::Call => None,
    }
}

/// The wager level at the node at `ordinal` on `street`: the `to` of the last wager taken on that
/// street along the path (0 at an opening node). It is the `own + call` of Task 10's
/// `wager_fraction` at that node: the actor's street contribution plus what it owes. `None` if the
/// path does not walk the skeleton (never for a path [`resolve_chip_path_indexed`] returned).
fn wager_level(index: &MaterializedIndex<'_>, ordinal: &[u8], street: Street) -> Option<u32> {
    let mut level = 0;
    for (k, &i) in ordinal.iter().enumerate() {
        let parent = index.get(&ordinal[..k])?;
        if parent.street != street {
            continue;
        }
        if let Some(to) = wager_to(parent.actions.get(usize::from(i))?) {
            level = to;
        }
    }
    Some(level)
}

/// The menu indices of `node` that an off-menu observed wager maps to with a nonzero spec section
/// 8.4 interpolation coefficient, at wager level `level`; `None` when the observed action has no
/// interpolation there: it is not a wager, it or a menu wager does not exceed the level (Task 10's
/// `wager_fraction` / `menu_fractions` are `None` in exactly those cases), or the node has no
/// wager-sized action at all.
///
/// Task 10's `interpolate` runs here on chip amounts beyond the level (`to - level`) instead of on
/// pot fractions, because a snapshot does not carry its root pot (Task 15's walk recovers it from
/// the hand state). At one node, every pot fraction is `(to - level) / (pot + call)` with the same
/// positive denominator, so the two representations order and equate the observed and menu sizes
/// identically, which is all `interpolate`'s choice of indices depends on: the same exact match, the
/// same clamp, the same bracketing pair. Strictly inside a bracket both coefficients are positive in
/// either representation (`0 < f_A < 1`), so the set of children with a nonzero coefficient is the
/// same; the coefficients themselves are the walk's, not needed to count coverage.
fn translated_children(node: &MaterializedNode, level: u32, observed: &Action) -> Option<Vec<usize>> {
    let beyond = |to: u32| to.checked_sub(level).map(f64::from);
    let s = beyond(wager_to(observed)?)?;
    let menu = node
        .actions
        .iter()
        .enumerate()
        .filter_map(|(i, a)| wager_to(a).map(|to| beyond(to).map(|x| (i, x))))
        .collect::<Option<Vec<(usize, f64)>>>()?;
    let split = interpolate(s, &menu)?;
    Some(split.choices.iter().filter(|(_, f)| *f > 0.0).map(|(i, _)| *i).collect())
}

// ---------------------------------------------------------------------------------------------
// The store (spec sections 4.4 and 9.2).
// ---------------------------------------------------------------------------------------------

/// The registered street snapshots: the single registration path of spec section 9.2 (the engine's
/// `engine::snapshots` re-exports this type). Only already validated solutions are registered, by
/// the engine, never raw worker results; a snapshot's provenance is never rewritten once stored.
#[derive(Debug, Default)]
pub struct SnapshotStore {
    entries: Vec<StreetSnapshot>,
}

impl SnapshotStore {
    pub fn new() -> Self {
        Self { entries: vec![] }
    }

    /// Registers `snap` if it was solved for the `active` decision, the identity the caller read as
    /// active at registration time; a snapshot solved for any other identity, differing in any
    /// field, is refused outright however well its prefix fits (spec section 4.4: stale results are
    /// never written), and so is one whose key names another hand, config or model than its
    /// identity. A refused snapshot leaves the store untouched. Re-registering the active decision
    /// on the same street replaces its earlier entry (a `Provisional` by the same decision's
    /// `Final`; the engine never lets a late `Provisional` replace a `Final`); every other decision
    /// stays a selection candidate. Returns whether `snap` was stored.
    pub fn register(&mut self, active: &DecisionIdentity, snap: StreetSnapshot) -> bool {
        if snap.provenance.identity_at_solve != *active {
            return false;
        }
        if snap.key.hand_id != active.hand_id || snap.key.config_revision != active.config_revision || snap.key.model_revision != active.model_revision {
            return false;
        }
        self.entries.retain(|s| s.key.street != snap.key.street || s.provenance.identity_at_solve != *active);
        self.entries.push(snap);
        true
    }

    /// The snapshots of `id`'s hand, config revision and model revision, in registration order:
    /// the slice the engine hands to replay. It intentionally does not require an equal hand
    /// revision or decision id (registration does): a snapshot kept across an append-only mutation
    /// or an undo stays usable under its original identity. After a config or model change it
    /// returns nothing for the old snapshots without deleting them, so an in-flight hand's frozen
    /// `HandConfig` is never retroactively reclassified.
    pub fn for_identity(&self, id: &DecisionIdentity) -> Vec<StreetSnapshot> {
        self.entries
            .iter()
            .filter(|s| s.key.hand_id == id.hand_id && s.key.config_revision == id.config_revision && s.key.model_revision == id.model_revision)
            .cloned()
            .collect()
    }

    /// The snapshots of `hand_id`, in registration order. Retained from Plan 2's store so its
    /// `identity_race_golden` keeps reading the store after the swap.
    pub fn for_hand(&self, hand_id: u64) -> Vec<&StreetSnapshot> {
        self.entries.iter().filter(|s| s.key.hand_id == hand_id).collect()
    }

    /// Drops every snapshot of `hand_id` (`begin_hand`, `finish_hand`, `abandon_hand`: the hand's
    /// identity ends).
    pub fn invalidate_hand(&mut self, hand_id: u64) {
        self.entries.retain(|s| s.key.hand_id != hand_id);
    }

    /// Spec section 9.2's mutation invalidation, prefix-based, against the new `state` of a mutation
    /// (`apply_action`, `set_board`, `set_hero_cards`, `undo`). Four rules, in this order: (1) a
    /// snapshot of another hand is dropped; (2) every snapshot of a street later than the state's
    /// current or awaited street is dropped; (3) a snapshot whose `root_board` no longer matches its
    /// street's board on record is dropped; (4) a snapshot survives iff its `solved_prefix` is still
    /// a prefix of the new history in the domain it was registered in (ruling 14-I1): the history of
    /// a street root recovered by the model at one of the street's cutoffs (`decision_roots`), which
    /// for a spec section 10.2 projection omits the actions of the players who folded on the street.
    /// So an append-only mutation or a change of hero's cards keeps the snapshot, and an undo across
    /// its solved decision (or across the projection that admitted it) removes it. Retained
    /// snapshots keep their original immutable provenance: an undo assigns a new hand revision but
    /// never rewrites `identity_at_solve`.
    pub fn invalidate(&mut self, state: &HandState) {
        let current = street_number(state.derived.street);
        // The recovered root histories of each street, computed once per call.
        let mut solved_at: [Option<Vec<Vec<(Seat, Action)>>>; 4] = Default::default();
        self.entries.retain(|s| {
            if s.key.hand_id != state.hand_id {
                return false; // (1)
            }
            if street_number(s.key.street) > current {
                return false; // (2)
            }
            if s.key.root_board != root_board(state, s.key.street) {
                return false; // (3)
            }
            let histories = solved_at[usize::from(street_number(s.key.street))]
                .get_or_insert_with(|| decision_roots(state, s.key.street).into_iter().map(|root| root.history).collect());
            histories.contains(&s.provenance.solved_prefix) // (4)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_preflop::{menu_fractions, wager_fraction};

    /// The children the pot-fraction interpolation of Task 10 gives a nonzero coefficient, at a node
    /// whose actor has put `own` in this street, owes `call` and faces a pot of `pot`.
    fn by_pot_fraction(node: &MaterializedNode, own: u32, call: u32, pot: u32, observed: &Action) -> Option<Vec<usize>> {
        let s = wager_fraction(wager_to(observed)?, own, call, pot)?;
        let menu = menu_fractions(&node.actions, own, call, pot)?;
        let split = interpolate(s, &menu)?;
        Some(split.choices.iter().filter(|(_, f)| *f > 0.0).map(|(i, _)| *i).collect())
    }

    fn node(actions: Vec<Action>) -> MaterializedNode {
        let terminal_pots = vec![None; actions.len()];
        MaterializedNode { path: vec![], street: Street::Flop, actor: "ip".into(), actions, terminal_pots }
    }

    /// The chip-amount children of `translated_children` are the pot-fraction children, for every
    /// observed size across opening and facing nodes, several pots, duplicate sizes and an all-in.
    #[test]
    fn chip_amount_children_equal_the_pot_fraction_children() {
        let opening = node(vec![Action::Check, Action::Bet { to: 33 }, Action::Bet { to: 50 }, Action::Bet { to: 100 }, Action::AllIn { to: 970 }]);
        let duplicate = node(vec![Action::Check, Action::Bet { to: 50 }, Action::AllIn { to: 50 }, Action::Bet { to: 75 }]);
        let facing = node(vec![Action::Fold, Action::Call, Action::Raise { to: 150 }, Action::Raise { to: 260 }, Action::AllIn { to: 970 }]);
        let mut compared = 0;
        for pot in [1u32, 2, 100, 137, 5_000, 1 << 29] {
            for (n, own, call) in [(&opening, 0u32, 0u32), (&duplicate, 0, 0), (&facing, 0, 50), (&facing, 20, 30)] {
                let level = own + call;
                for to in (0..=1_000u32).chain([u32::MAX / 2, u32::MAX]) {
                    for observed in [Action::Bet { to }, Action::Raise { to }, Action::AllIn { to }] {
                        assert_eq!(
                            translated_children(n, level, &observed),
                            by_pot_fraction(n, own, call, pot, &observed),
                            "pot {pot}, level {level}, observed {observed:?}, menu {:?}",
                            n.actions
                        );
                        compared += 1;
                    }
                }
            }
        }
        assert_eq!(compared, 6 * 4 * 1_003 * 3);
        // Not wagers: no interpolation in either representation.
        for observed in [Action::Fold, Action::Check, Action::Call] {
            assert_eq!(translated_children(&facing, 50, &observed), None);
            assert_eq!(by_pot_fraction(&facing, 0, 50, 100, &observed), None);
        }
        // A menu without any wager-sized action has no interpolation either.
        let passive = node(vec![Action::Fold, Action::Call]);
        assert_eq!(translated_children(&passive, 50, &Action::Raise { to: 120 }), None);
    }

    /// Sanity for the equivalence test above: inside a bracket both sizes are children, at the
    /// edges and on an exact size only one is.
    #[test]
    fn a_bracketed_wager_maps_to_both_neighbours_and_an_edge_or_exact_size_to_one() {
        let opening = node(vec![Action::Check, Action::Bet { to: 50 }, Action::Bet { to: 100 }]);
        assert_eq!(translated_children(&opening, 0, &Action::Bet { to: 73 }), Some(vec![1, 2]));
        assert_eq!(translated_children(&opening, 0, &Action::Bet { to: 50 }), Some(vec![1]));
        assert_eq!(translated_children(&opening, 0, &Action::Bet { to: 20 }), Some(vec![1]));
        assert_eq!(translated_children(&opening, 0, &Action::Bet { to: 400 }), Some(vec![2]));
        let facing = node(vec![Action::Fold, Action::Call, Action::Raise { to: 150 }, Action::Raise { to: 260 }]);
        assert_eq!(translated_children(&facing, 50, &Action::Raise { to: 200 }), Some(vec![2, 3]));
        assert_eq!(translated_children(&facing, 50, &Action::Raise { to: 40 }), None, "below the level: not a wager here");
        assert_eq!(translated_children(&facing, 200, &Action::Raise { to: 300 }), None, "a menu raise below the level has no fraction");
    }

    /// The wager level of a node is the last wager's `to` on the node's own street, and restarts at
    /// 0 on the next street.
    #[test]
    fn the_wager_level_follows_the_last_wager_of_the_nodes_street() {
        let n = |path: Vec<u8>, street: Street, actions: Vec<Action>| {
            let terminal_pots = vec![None; actions.len()];
            MaterializedNode { path, street, actor: "oop".into(), actions, terminal_pots }
        };
        let tree = vec![
            n(vec![], Street::Flop, vec![Action::Check, Action::Bet { to: 40 }]),
            n(vec![1], Street::Flop, vec![Action::Fold, Action::Call, Action::Raise { to: 120 }]),
            n(vec![1, 2], Street::Flop, vec![Action::Fold, Action::Call]),
            n(vec![1, 2, 1], Street::Turn, vec![Action::Check, Action::Bet { to: 90 }]),
            n(vec![1, 2, 1, 1], Street::Turn, vec![Action::Fold, Action::Call]),
        ];
        let index = index_materialized(&tree);
        assert_eq!(wager_level(&index, &[], Street::Flop), Some(0));
        assert_eq!(wager_level(&index, &[1], Street::Flop), Some(40));
        assert_eq!(wager_level(&index, &[1, 2], Street::Flop), Some(120));
        assert_eq!(wager_level(&index, &[1, 2, 1], Street::Turn), Some(0));
        assert_eq!(wager_level(&index, &[1, 2, 1, 1], Street::Turn), Some(90));
        assert_eq!(wager_level(&index, &[3], Street::Flop), None, "an index off the menu");
    }
}
