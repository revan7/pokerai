//! §10.2: the effective tree of one street root — the template materialized at the snapshot's own
//! chips, with the observed history walked from the root and every off-menu wager inserted exactly.

use super::materialize::{materialize, MaterializeInput, Materialized};
use super::templates::{TemplateSpec, Templates};
use proto::{Action, EffectiveTree, OrdinalPath, Seat, StreetRootSnapshot, UnsupportedReason};

/// §4.6 `rules_version`; owned by `proto` (plan 1 Task 6) and re-exported so there is one value in the workspace.
pub use proto::RULES_VERSION;

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateSelection { pub template_id: String, pub observed: Vec<(Seat, Action)> }
impl TemplateSelection {
    /// The observed sizes to insert are the wagers of the current street's history (§10.2); checks/calls are kept for the walk.
    pub fn from_history(template_id: &str, history: &[(Seat, Action)]) -> Self { Self { template_id: template_id.to_string(), observed: history.to_vec() } }
}

#[derive(Debug, Clone)]
pub struct TreeBuild { pub tree: EffectiveTree, pub history: Vec<Action>, pub decision_path: OrdinalPath, pub pot: u32, pub eff: u32 }

fn engine_error(msg: impl Into<String>) -> UnsupportedReason { UnsupportedReason::EngineError { message: msg.into(), retryable: false } }

fn assemble(template: &TemplateSpec, m: Materialized, pot: u32, eff: u32) -> TreeBuild {
    let tree = EffectiveTree { rules_version: RULES_VERSION, template_id: template.id.to_string(), root_street: template.root_street, menus: template.menus.clone(),
        add_allin_threshold: template.add_allin_threshold, force_allin_threshold: template.force_allin_threshold, merging_threshold: template.merging_threshold,
        wager_cap: template.wager_cap, inserted: m.inserted, materialized: m.nodes };
    TreeBuild { tree, history: m.history, decision_path: m.decision_path, pot, eff }
}

/// Materializes a template at explicit chips with an actor-labelled prefix (0 = oop, 1 = ip); used by tests, `bench` and the fixture generator.
pub fn materialize_at(template: &TemplateSpec, pot: u32, eff: u32, prefix: &[(usize, Action)]) -> Result<TreeBuild, UnsupportedReason> {
    let m = materialize(&MaterializeInput { template, starting_pot: pot, eff, prefix })?;
    Ok(assemble(template, m, pot, eff))
}

/// §10.2: root inputs come from the snapshot only; `pot = pot_root + dead_this_street`, `eff = min(stacks)`; the history is
/// walked from the root and every off-menu wager is inserted exactly alongside the menu.
///
/// The walked history is `root.history`, never `sel.observed`: the snapshot is the sole authority for
/// the root's financial state and its history (§10.2), and the tree must describe the hand that is
/// actually on the table. `TemplateSelection::observed` is the history the *template* was selected
/// from; when it is non-empty it must agree with the snapshot, otherwise the caller selected a
/// template for one history and asked for a tree over another — an engine bug, reported as such
/// instead of being resolved silently in favour of either side.
pub fn build_tree_full(root: &StreetRootSnapshot, sel: &TemplateSelection) -> Result<TreeBuild, UnsupportedReason> {
    let template = Templates::get(&sel.template_id).ok_or_else(|| engine_error(format!("unknown template {}", sel.template_id)))?;
    if template.root_street != root.street { return Err(engine_error(format!("template {} is rooted at {:?}, street is {:?}", template.id, template.root_street, root.street))); }
    // A snapshot is a two-player root by construction (`core_model::street_root`), but it is also a
    // wire type: one seat filling both roles would silently map every history step onto `oop` and
    // label a tree whose two sides are the same player, so it is refused here rather than narrowed.
    if root.oop == root.ip { return Err(engine_error(format!("seat {} is both the oop and the ip player of the street root", root.oop.0))); }
    if !sel.observed.is_empty() && sel.observed != root.history {
        return Err(engine_error(format!("template {} was selected from a {}-step history that is not the snapshot's {}-step history", template.id, sel.observed.len(), root.history.len())));
    }
    let pot = root.pot_root.checked_add(root.dead_this_street).ok_or(UnsupportedReason::EngineError { message: "pot overflow".into(), retryable: false })?;
    let eff = root.stack_oop_root.min(root.stack_ip_root);
    let mut prefix = Vec::with_capacity(root.history.len());
    for (seat, a) in &root.history {
        let actor = if *seat == root.oop { 0 } else if *seat == root.ip { 1 } else { return Err(UnsupportedReason::UnsupportedHistory { reason: format!("seat {} is not a street-root player", seat.0) }); };
        prefix.push((actor, *a));
    }
    let m = materialize(&MaterializeInput { template, starting_pot: pot, eff, prefix: &prefix })?;
    Ok(assemble(template, m, pot, eff))
}

pub fn build_effective_tree(root: &StreetRootSnapshot, sel: &TemplateSelection) -> Result<EffectiveTree, UnsupportedReason> {
    build_tree_full(root, sel).map(|b| b.tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{Card, Street};

    fn snap(history: Vec<(Seat, Action)>) -> StreetRootSnapshot {
        StreetRootSnapshot { street: Street::Flop, board: "Kh 7d 2c".split(' ').map(|c| Card::parse(c).unwrap()).collect(),
            oop: Seat(2), ip: Seat(0), pot_root: 100, stack_oop_root: 500, stack_ip_root: 700, dead_this_street: 0, projected_from: 2, history, bb_chips: 2 }
    }

    /// §10.2: `pot = pot_root + dead_this_street` (the folded seats' same-street chips are dead money
    /// at the root) and `eff = min(stacks)`; the deeper stack's excess is never part of the tree.
    #[test]
    fn root_inputs_come_from_the_snapshot() {
        let mut s = snap(vec![]);
        s.dead_this_street = 25;
        let b = build_tree_full(&s, &TemplateSelection::from_history("flop_fast_v1", &[])).unwrap();
        assert_eq!((b.pot, b.eff), (125, 500));
        // pot 125 with the 0.5 menu is Bet(63) (round(62.5) away from zero), which is the tree's own witness
        assert_eq!(b.tree.materialized[0].actions, vec![Action::Check, Action::Bet { to: 63 }]);
        // an overflowing dead-money field is an EngineError, not a wrapped pot
        s.pot_root = u32::MAX;
        assert!(matches!(build_tree_full(&s, &TemplateSelection::from_history("flop_fast_v1", &[])), Err(UnsupportedReason::EngineError { .. })));
    }

    /// The assembled tree carries the template's nominal fields and the materializer's realized ones.
    #[test]
    fn assembled_tree_carries_template_and_materializer_fields() {
        let t = Templates::get("flop_fast_v1").unwrap();
        let b = build_tree_full(&snap(vec![(Seat(2), Action::Bet { to: 73 })]), &TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), Action::Bet { to: 73 })])).unwrap();
        assert_eq!(b.tree.rules_version, RULES_VERSION);
        assert_eq!((b.tree.template_id.as_str(), b.tree.root_street, b.tree.wager_cap), ("flop_fast_v1", Street::Flop, t.wager_cap));
        assert_eq!((b.tree.add_allin_threshold, b.tree.force_allin_threshold, b.tree.merging_threshold), (t.add_allin_threshold, t.force_allin_threshold, t.merging_threshold));
        assert_eq!(b.tree.menus, t.menus);
        assert_eq!(b.tree.inserted, vec![(vec![], "oop".to_string(), Action::Bet { to: 73 })]);
        assert!(!b.tree.materialized.is_empty());
    }

    /// An unknown template id is an engine bug (the selector's), not an unsupported history; so is a
    /// snapshot whose two sides are one seat.
    #[test]
    fn unknown_template_and_degenerate_seats_are_engine_errors() {
        assert!(matches!(build_tree_full(&snap(vec![]), &TemplateSelection::from_history("no_such_template", &[])), Err(UnsupportedReason::EngineError { .. })));
        let one_seat = StreetRootSnapshot { ip: Seat(2), ..snap(vec![]) };
        let e = build_tree_full(&one_seat, &TemplateSelection::from_history("flop_fast_v1", &[]));
        assert!(matches!(&e, Err(UnsupportedReason::EngineError { message, .. }) if message.contains("both the oop and the ip")), "{e:?}");
        // control: the same snapshot with two distinct seats builds
        assert!(build_tree_full(&snap(vec![]), &TemplateSelection::from_history("flop_fast_v1", &[])).is_ok());
    }

    /// A non-empty selection history that disagrees with the snapshot is refused rather than one of
    /// the two being silently preferred; an empty one leaves the snapshot in sole charge.
    #[test]
    fn a_selection_history_that_contradicts_the_snapshot_is_refused() {
        let s = snap(vec![(Seat(2), Action::Bet { to: 73 })]);
        let disagrees = TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), Action::Bet { to: 50 })]);
        assert!(matches!(build_tree_full(&s, &disagrees), Err(UnsupportedReason::EngineError { .. })));
        // agreeing and empty selections both build the snapshot's own tree
        let agrees = build_tree_full(&s, &TemplateSelection::from_history("flop_fast_v1", &s.history)).unwrap();
        let empty = build_tree_full(&s, &TemplateSelection::from_history("flop_fast_v1", &[])).unwrap();
        assert_eq!(agrees.tree, empty.tree);
        assert_eq!((agrees.history, agrees.decision_path), (vec![Action::Bet { to: 73 }], vec![2]));
    }

    /// `build_effective_tree` is `build_tree_full` without the build bookkeeping.
    #[test]
    fn build_effective_tree_returns_the_same_tree() {
        let s = snap(vec![(Seat(2), Action::Bet { to: 73 })]);
        let sel = TemplateSelection::from_history("flop_fast_v1", &s.history);
        assert_eq!(build_effective_tree(&s, &sel).unwrap(), build_tree_full(&s, &sel).unwrap().tree);
    }

    /// `materialize_at` addresses actors positionally (0 = oop, 1 = ip) for `bench` and the fixture
    /// generator, and reaches the same tree the snapshot path builds for the same chips.
    #[test]
    fn materialize_at_matches_the_snapshot_path() {
        let s = StreetRootSnapshot { stack_ip_root: 500, ..snap(vec![(Seat(2), Action::Bet { to: 73 })]) };
        let by_snapshot = build_tree_full(&s, &TemplateSelection::from_history("flop_fast_v1", &s.history)).unwrap();
        let direct = materialize_at(Templates::get("flop_fast_v1").unwrap(), 100, 500, &[(0, Action::Bet { to: 73 })]).unwrap();
        assert_eq!(by_snapshot.tree, direct.tree);
        assert_eq!((by_snapshot.history, by_snapshot.decision_path), (direct.history, direct.decision_path));
    }
}
