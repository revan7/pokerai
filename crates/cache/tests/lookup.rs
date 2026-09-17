mod support;

use cache::entry::{CacheEntry, SourceInputs};
use cache::key::{KeyFields, Model, RakeKey, Rational};
use cache::lookup::{action_to, cap_agrees, compare, Comparison};
use proto::{Action, EffectiveTree, MaterializedNode, Street};
use std::collections::BTreeMap;

// --- step 1: independent rake boundary -------------------------------------------------------

#[test]
fn terminal_rake_cap_changes_without_topology_changes() {
    let r = 0.05_f32 as f64;
    assert!(r * 1100.0 < 55.2);
    assert!(r * 1108.0 >= 55.2);
    assert!(!cache::lookup::cap_agrees(r, 55200, 1100, 55200, 1108));
}

// --- test fixtures -----------------------------------------------------------------------------

/// A minimal, otherwise-arbitrary `CacheEntry` carrying only the fields `compare` reads: the
/// rake rate (`key.rake`), the financial `source` inputs, and `tree.materialized`. Every other
/// field is a harmless placeholder -- `compare` never touches it, and this file never calls
/// `validate_entry`, so the placeholders do not need to satisfy its invariants.
fn minimal_entry(pot: u32, stack_oop: u32, stack_ip: u32, cap_mchips: u32, rate: f32, materialized: Vec<MaterializedNode>) -> CacheEntry {
    CacheEntry {
        key: KeyFields {
            schema_version: 3,
            solver_commit: "solver".into(),
            adapter_version: 1,
            rules_version: 3,
            canonical_board: vec![],
            root_street: Street::Flop,
            spr_bucket: 0,
            tree_signature: "sig".into(),
            rake: RakeKey::new(rate, Rational::new(0, 1).unwrap(), 1).unwrap(),
            range_hash_oop: [0u8; 32],
            range_hash_ip: [0u8; 32],
            model: Model::Baseline,
        },
        source: SourceInputs {
            pot,
            stack_oop,
            stack_ip,
            spr: Rational::new(1, 1).unwrap(),
            bb_chips: 2,
            quantum_over_p: Rational::new(1, 100).unwrap(),
            cap_mchips,
            ranges: [proto::Range1326::zero(), proto::Range1326::zero()],
        },
        tree: query_tree(materialized),
        fractions: vec![],
        nodes: vec![],
        covered_paths: vec![],
        exploitability_over_P: 0.0,
        target_bp: 50,
        iterations: 0,
        elapsed_ms: 0,
        memory_bytes: 0,
        mode: "f32".into(),
        locks_applied: 0,
        export: "street".into(),
        reasons: vec![],
        created: 0,
        last_hit: 0,
    }
}

/// An `EffectiveTree` wrapping `materialized`; every other field is a placeholder `compare`
/// never reads.
fn query_tree(materialized: Vec<MaterializedNode>) -> EffectiveTree {
    EffectiveTree {
        rules_version: 3,
        template_id: "sig".into(),
        root_street: Street::Flop,
        menus: BTreeMap::new(),
        add_allin_threshold: 0.0,
        force_allin_threshold: 0.0,
        merging_threshold: 0.0,
        wager_cap: 1,
        inserted: vec![],
        materialized,
    }
}

/// Two sorted-by-path nodes: an `oop` root deciding Check/Bet{to: `bet_to`}, and an `ip` node at
/// `[1]` deciding Fold/Call, both terminal (fold -> 100, call -> 200).
fn two_node_tree(bet_to: u32) -> Vec<MaterializedNode> {
    vec![
        MaterializedNode { path: vec![], street: Street::Flop, actor: "oop".into(), actions: vec![Action::Check, Action::Bet { to: bet_to }], terminal_pots: vec![None, None] },
        MaterializedNode { path: vec![1], street: Street::Flop, actor: "ip".into(), actions: vec![Action::Fold, Action::Call], terminal_pots: vec![Some(100), Some(200)] },
    ]
}

// --- action_to -----------------------------------------------------------------------------

#[test]
fn action_to_maps_wager_actions_to_their_to_amount() {
    assert_eq!(action_to(&Action::Bet { to: 50 }), Some(50));
    assert_eq!(action_to(&Action::Raise { to: 150 }), Some(150));
    assert_eq!(action_to(&Action::AllIn { to: 500 }), Some(500));
    assert_eq!(action_to(&Action::Fold), None);
    assert_eq!(action_to(&Action::Check), None);
    assert_eq!(action_to(&Action::Call), None);
}

// --- cap_agrees ------------------------------------------------------------------------------

#[test]
fn cap_agrees_when_both_sides_land_on_the_same_side_of_the_cap() {
    let rate = 0.05_f32 as f64;
    // Both uncapped: rake (5.0) stays under the 55.2-chip cap on both pots.
    assert!(cap_agrees(rate, 55200, 1000, 55200, 1050));
    // Both capped: rake exceeds the same cap on both pots.
    assert!(cap_agrees(rate, 55200, 1200, 55200, 5000));
}

// --- compare: degenerate inputs -----------------------------------------------------------------

#[test]
fn compare_rejects_zero_query_pot() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let t = query_tree(two_node_tree(50));
    assert!(compare(&e, &t, 0, 500, 5000).is_none());
}

#[test]
fn compare_rejects_zero_query_effective_stack() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let t = query_tree(two_node_tree(50));
    assert!(compare(&e, &t, 100, 0, 5000).is_none());
}

// --- compare: happy path -------------------------------------------------------------------------

#[test]
fn compare_matches_identical_topology_with_zero_delta_and_zero_deviation() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let t = query_tree(two_node_tree(50));
    let c = compare(&e, &t, 100, 500, 5000).expect("identical topology and financials must match");
    assert_eq!(c, Comparison { delta: 0.0, max_dev: 0.0, delta_num: 0, delta_den: 500 * 100, menu_num: 0, menu_den: 100 * 100 });
}

/// Integration check against the fully-validated fixture used by task 2's tests, not just this
/// file's minimal builder: an entry queried against its own tree at its own financials matches.
#[test]
fn compare_matches_the_validated_support_fixture_against_itself() {
    let e = support::entry();
    let eff = e.source.stack_oop.min(e.source.stack_ip);
    let c = compare(&e, &e.tree.clone(), e.source.pot, eff, e.source.cap_mchips).expect("fixture must match itself");
    assert_eq!(c.delta, 0.0);
    assert_eq!(c.max_dev, 0.0);
}

// --- compare: SPR delta bucket tolerance (spec 10.4: delta <= 0.02) ------------------------------

#[test]
fn compare_accepts_spr_delta_at_the_exact_two_percent_boundary() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let t = query_tree(two_node_tree(50));
    // eff = 510: |510*100 - 500*100| / (500*100) = 1000/50000 = 0.02 exactly.
    let c = compare(&e, &t, 100, 510, 5000).expect("delta == 0.02 must still match (inclusive threshold)");
    assert_eq!(c.delta, 0.02);
}

#[test]
fn compare_rejects_spr_delta_just_over_the_two_percent_boundary() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let t = query_tree(two_node_tree(50));
    // eff = 511: |511*100 - 500*100| / (500*100) = 1100/50000 = 0.022 > 0.02.
    assert!(compare(&e, &t, 100, 511, 5000).is_none());
}

// --- compare: full-topology equality across the whole tree ---------------------------------------

#[test]
fn compare_rejects_materialized_length_mismatch() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let mut short = two_node_tree(50);
    short.truncate(1);
    let t = query_tree(short);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_unsorted_entry_tree() {
    let mut unsorted = two_node_tree(50);
    unsorted.swap(0, 1);
    let e = minimal_entry(100, 500, 500, 5000, 0.0, unsorted);
    let t = query_tree(two_node_tree(50));
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_unsorted_query_tree() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let mut unsorted = two_node_tree(50);
    unsorted.swap(0, 1);
    let t = query_tree(unsorted);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_path_mismatch() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let mut different_path = two_node_tree(50);
    different_path[1].path = vec![0]; // still sorted ([] < [0]), only the path itself differs
    let t = query_tree(different_path);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_actor_mismatch() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let mut different_actor = two_node_tree(50);
    different_actor[1].actor = "oop".into();
    let t = query_tree(different_actor);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_street_mismatch() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let mut different_street = two_node_tree(50);
    different_street[1].street = Street::Turn;
    let t = query_tree(different_street);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_action_count_mismatch() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let mut extra_action = two_node_tree(50);
    extra_action[1].actions.push(Action::Raise { to: 300 });
    extra_action[1].terminal_pots.push(None); // kept consistent with its own actions length
    let t = query_tree(extra_action);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_entry_terminal_pots_length_disagreeing_with_its_own_actions() {
    let mut malformed = two_node_tree(50);
    malformed[1].terminal_pots.pop(); // now 1 terminal_pots for 2 actions
    let e = minimal_entry(100, 500, 500, 5000, 0.0, malformed);
    let t = query_tree(two_node_tree(50));
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_query_terminal_pots_length_disagreeing_with_its_own_actions() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let mut malformed = two_node_tree(50);
    malformed[1].terminal_pots.pop();
    let t = query_tree(malformed);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_action_kind_discriminant_mismatch() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let mut different_kind = two_node_tree(50);
    // Call -> Bet{to: 200}, keeping the terminal (Some) classification identical so only the
    // action *kind* differs, isolating the discriminant check from the terminal-classification one.
    different_kind[1].actions[1] = Action::Bet { to: 200 };
    let t = query_tree(different_kind);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

#[test]
fn compare_rejects_terminal_classification_flip_with_identical_kinds_paths_actors_streets() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_node_tree(50));
    let mut flipped = two_node_tree(50);
    flipped[1].terminal_pots[1] = None; // Call is no longer terminal on the query side
    let t = query_tree(flipped);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

// --- compare: terminal rake-cap activation (independent of topology) -----------------------------

#[test]
fn compare_rejects_rake_cap_activation_mismatch_at_an_otherwise_identical_terminal() {
    // Same tree shape on both sides; only the entry's stored terminal chip pot (1100, from the
    // fixed 500/500 all-in call) differs from the query's (1108) enough to cross the cap.
    let materialized = vec![
        MaterializedNode { path: vec![], street: Street::Flop, actor: "oop".into(), actions: vec![Action::AllIn { to: 500 }], terminal_pots: vec![None] },
        MaterializedNode { path: vec![0], street: Street::Flop, actor: "ip".into(), actions: vec![Action::Call], terminal_pots: vec![Some(1100)] },
    ];
    let e = minimal_entry(100, 500, 500, 55200, 0.05, materialized.clone());
    let mut query = materialized;
    query[1].terminal_pots[0] = Some(1108);
    let t = query_tree(query);
    assert!(compare(&e, &t, 100, 500, 55200).is_none());
}

#[test]
fn compare_accepts_rake_cap_agreement_when_both_terminals_stay_capped() {
    // At rate 0.05 and cap 55200 mchips (55.2 chips), the activation threshold is 1104 chips
    // (0.05 * 1104 = 55.2). Both 1108 and 1150 sit past it, so both terminals are capped.
    let materialized = vec![
        MaterializedNode { path: vec![], street: Street::Flop, actor: "oop".into(), actions: vec![Action::AllIn { to: 500 }], terminal_pots: vec![None] },
        MaterializedNode { path: vec![0], street: Street::Flop, actor: "ip".into(), actions: vec![Action::Call], terminal_pots: vec![Some(1108)] },
    ];
    let e = minimal_entry(100, 500, 500, 55200, 0.05, materialized.clone());
    let mut query = materialized;
    query[1].terminal_pots[0] = Some(1150);
    let t = query_tree(query);
    assert!(compare(&e, &t, 100, 500, 55200).is_some());
}

// --- compare: maximum menu deviation (spec 10.4: max(dev) <= 0.05) --------------------------------

/// Single-node, single-action tree isolating the wager-deviation check: pot and effective stack
/// are identical on both sides (so the SPR check is trivially satisfied), and the sole action is
/// a continuation (`terminal_pots: [None]`), so the terminal-classification/rake-cap checks never
/// fire either.
fn single_bet_tree(to: u32) -> Vec<MaterializedNode> {
    vec![MaterializedNode { path: vec![], street: Street::Flop, actor: "oop".into(), actions: vec![Action::Bet { to }], terminal_pots: vec![None] }]
}

#[test]
fn compare_accepts_wager_deviation_at_the_exact_five_percent_boundary() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, single_bet_tree(500));
    let t = query_tree(single_bet_tree(505));
    // dev = |505/100 - 500/100| = 0.05 exactly.
    let c = compare(&e, &t, 100, 500, 5000).expect("dev == 0.05 must still match (inclusive threshold)");
    assert_eq!(c.max_dev, 0.05);
}

#[test]
fn compare_rejects_wager_deviation_just_over_the_five_percent_boundary() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, single_bet_tree(500));
    let t = query_tree(single_bet_tree(506));
    // dev = |506/100 - 500/100| = 0.06 > 0.05.
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

// --- Comparison::rank ------------------------------------------------------------------------

#[test]
fn rank_orders_by_delta_first_using_exact_cross_products() {
    let closer = Comparison { delta: 1.0 / 3.0, max_dev: 0.0, delta_num: 1, delta_den: 3, menu_num: 0, menu_den: 1 };
    let farther = Comparison { delta: 2.0 / 7.0, max_dev: 0.0, delta_num: 2, delta_den: 7, menu_num: 0, menu_den: 1 };
    // 1/3 ~= 0.333, 2/7 ~= 0.286: farther has the larger delta even though its raw numerator is
    // bigger, so a naive numerator-only comparison would get this backwards.
    assert_eq!(closer.rank(&farther), std::cmp::Ordering::Greater);
    assert_eq!(farther.rank(&closer), std::cmp::Ordering::Less);
}

#[test]
fn rank_breaks_equal_delta_ties_by_max_dev_using_exact_cross_products() {
    // delta 1/2 == 2/4 (equal value, different num/den representation): rank must treat them as
    // tied on delta (via cross-multiplication, not a direct num/den comparison) and fall through
    // to the menu (max_dev) tie-break.
    let a = Comparison { delta: 0.5, max_dev: 0.1, delta_num: 1, delta_den: 2, menu_num: 1, menu_den: 10 };
    let b = Comparison { delta: 0.5, max_dev: 0.2, delta_num: 2, delta_den: 4, menu_num: 1, menu_den: 5 };
    assert_eq!(a.rank(&b), std::cmp::Ordering::Less);
    assert_eq!(b.rank(&a), std::cmp::Ordering::Greater);
}

#[test]
fn rank_is_equal_for_identical_comparisons() {
    let a = Comparison { delta: 0.1, max_dev: 0.02, delta_num: 1, delta_den: 10, menu_num: 1, menu_den: 50 };
    assert_eq!(a.rank(&a), std::cmp::Ordering::Equal);
}
