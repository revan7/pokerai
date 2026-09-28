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

// Fix round 1 (review R1): swapping only one side also puts that side's path out of step with
// the other side's positionally-zipped path, so these two tests are also (and, on their own,
// ambiguously) caught by the ordinary per-node `a.path != b.path` check, not demonstrably by the
// `sorted_by_path` guard itself. Kept as ordinary "some reordering is rejected" coverage, but
// `compare_rejects_matching_unsorted_order_on_both_sides` and
// `compare_rejects_duplicate_path_present_on_both_sides` below are the tests that isolate the
// sortedness guard: both sides carry the identical (unsorted) path sequence, so the per-node
// checks all pass pairwise and only `sorted_by_path` can reject.
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

/// Isolates the `sorted_by_path` guard from the per-node path-equality check: both sides carry
/// the *same* (reversed, unsorted) path sequence, so every positional pair is pairwise identical
/// (`a.path == b.path`, `a.actor == b.actor`, etc. at every index) and only `sorted_by_path`
/// rejects. Mutation-tested in fix round 1: removing both `sorted_by_path` checks from `compare`
/// flips this test's result from `None` to `Some` (see task-3-report.md "Fix round 1").
#[test]
fn compare_rejects_matching_unsorted_order_on_both_sides() {
    let mut reordered = two_node_tree(50);
    reordered.reverse(); // [child(path=[1]), root(path=[])] on both sides identically
    let e = minimal_entry(100, 500, 500, 5000, 0.0, reordered.clone());
    let t = query_tree(reordered);
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

/// The matching duplicate-path case from the same isolation family: both sides list the same
/// node twice at the same path. `sorted_by_path` uses a strict `<`, so an equal-path adjacent
/// pair is itself unsorted; every positional pair is still pairwise identical, so only
/// `sorted_by_path` rejects. Mutation-tested alongside the test above.
#[test]
fn compare_rejects_duplicate_path_present_on_both_sides() {
    let node = MaterializedNode { path: vec![], street: Street::Flop, actor: "oop".into(), actions: vec![Action::Check], terminal_pots: vec![None] };
    let duplicated = vec![node.clone(), node];
    let e = minimal_entry(100, 500, 500, 5000, 0.0, duplicated.clone());
    let t = query_tree(duplicated);
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

/// Root (`Flop`, `Check`, no wager) and a `Turn` child with a single `Bet`.
fn root_then_turn_wager_tree(turn_to: u32) -> Vec<MaterializedNode> {
    vec![
        MaterializedNode { path: vec![], street: Street::Flop, actor: "oop".into(), actions: vec![Action::Check], terminal_pots: vec![None] },
        MaterializedNode { path: vec![0], street: Street::Turn, actor: "oop".into(), actions: vec![Action::Bet { to: turn_to }], terminal_pots: vec![None] },
    ]
}

/// Fix round 1 (review R1): the previous two deviation tests both used a single-node,
/// single-wager tree, so they could not show that `compare` walks *every* street rather than
/// stopping at the root. Here the root is unchanged (`Check`, no wager) on both sides, and the
/// only wager -- and the only source of deviation -- is on the `Turn` child.
#[test]
fn compare_rejects_wager_deviation_exceeding_threshold_on_a_later_street_with_root_unchanged() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, root_then_turn_wager_tree(500));
    let t = query_tree(root_then_turn_wager_tree(506));
    // dev = |506/100 - 500/100| = 0.06 > 0.05, entirely from the Turn node.
    assert!(compare(&e, &t, 100, 500, 5000).is_none());
}

/// Two wager nodes, `Flop` root then `Turn` child.
fn two_street_wager_tree(flop_to: u32, turn_to: u32) -> Vec<MaterializedNode> {
    vec![
        MaterializedNode { path: vec![], street: Street::Flop, actor: "oop".into(), actions: vec![Action::Bet { to: flop_to }], terminal_pots: vec![None] },
        MaterializedNode { path: vec![0], street: Street::Turn, actor: "oop".into(), actions: vec![Action::Bet { to: turn_to }], terminal_pots: vec![None] },
    ]
}

/// Fix round 1 (review R1): proves `maximum` is accumulated with `.max(...)` across every wager
/// in the tree, not overwritten by the latest one. The larger deviation (Flop, diff 3 -> 0.03) is
/// encountered *before* a smaller one (Turn, diff 1 -> 0.01); if the accumulation were replaced
/// by plain assignment of the latest deviation, the result would be the smaller, later 0.01, not
/// the correct 0.03. Asserts the exact `menu_num`/`menu_den` alongside `max_dev`, not just the
/// `f64` display value. Mutation-tested in fix round 1 (see task-3-report.md "Fix round 1").
#[test]
fn compare_accepts_when_the_largest_deviation_occurs_before_a_smaller_final_deviation() {
    let e = minimal_entry(100, 500, 500, 5000, 0.0, two_street_wager_tree(500, 500));
    let t = query_tree(two_street_wager_tree(503, 501));
    let c = compare(&e, &t, 100, 500, 5000).expect("both deviations are within the 5% threshold");
    assert_eq!(c, Comparison { delta: 0.0, max_dev: 0.03, delta_num: 0, delta_den: 500 * 100, menu_num: 300, menu_den: 100 * 100 });
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

// --- task 4: inherited reasons and raw-accuracy filtering (spec 10.4/13.1/3.5) --------------------
//
// `zero_comparison`/`comparison_with_max_dev` build a `Comparison` for `label` tests: `label`
// only ever reads `c.max_dev`, never `c.delta`/`_num`/`_den` (the query's own exact `spr` is
// compared against `e.source.spr` directly instead), so the other fields are harmless
// placeholders here.
fn zero_comparison() -> Comparison {
    Comparison { delta: 0.0, max_dev: 0.0, delta_num: 0, delta_den: 1, menu_num: 0, menu_den: 1 }
}

fn comparison_with_max_dev(max_dev: f64) -> Comparison {
    Comparison { delta: 0.0, max_dev, delta_num: 0, delta_den: 1, menu_num: 0, menu_den: 1 }
}

#[test]
fn cache_inherited_reasons_survive() {
    use cache::label::{accuracy_ok, merge_reasons};
    use proto::ApproxReason;
    assert!(!accuracy_ok(0.005049, 50));
    assert!(accuracy_ok(0.005049, 51));
    assert!(accuracy_ok(0.005, 50));
    assert!(!accuracy_ok(f64::NAN, 50));
    let reasons = vec![
        ApproxReason::ChartRounded,
        ApproxReason::DeadlineBestSoFar { reached_bp: 190, target_bp: 50 },
        ApproxReason::UnconditionedPriorStreet { street: proto::Street::Flop, seat: proto::Seat(2), cause: "uncovered path []".into() },
    ];
    assert_eq!(merge_reasons(&reasons, &[ApproxReason::ChartRounded]).len(), 3);
}

/// Isolates the nonnegativity guard from the finiteness guard: `-0.001` is finite (passes
/// `is_finite()`) but negative, so only the explicit `raw >= 0.0` check can reject it.
#[test]
fn accuracy_ok_rejects_finite_negative_raw() {
    use cache::label::accuracy_ok;
    assert!(!accuracy_ok(-0.001, 50));
}

/// Fix round 1 (review R3): the previous comment here claimed this test isolates `is_finite()`
/// from a narrower `is_nan()`-only check. That claim does not hold: `raw <= target as f64 /
/// 10000.0` already evaluates to `false` for `+inf` on its own (infinity exceeds every finite
/// target), so a mutant that dropped `is_finite()` entirely would still reject `+inf` here via
/// the upper-bound comparison alone. This is kept as a plain input-domain regression (`+inf` is
/// never accurate), not a guard-isolation claim.
#[test]
fn accuracy_ok_rejects_positive_infinity() {
    use cache::label::accuracy_ok;
    assert!(!accuracy_ok(f64::INFINITY, 50));
}

/// Fix round 1 (review R3): added per the concrete edit to test `NaN` explicitly (previously only
/// exercised inline inside `cache_inherited_reasons_survive`). Note this also does not cleanly
/// isolate `is_finite()`: IEEE-754 makes every ordered comparison against `NaN` false, so
/// `raw >= 0.0` alone already rejects `NaN` too, independent of `is_finite()`. `is_finite()` is
/// kept in `accuracy_ok` for explicitness/defense-in-depth (matching this crate's numeric-domain
/// style elsewhere, e.g. `entry.rs`'s `narrow_checked`/`widen_checked`), not because any single
/// guard in this expression is uniquely responsible for rejecting a non-finite `raw`.
#[test]
fn accuracy_ok_rejects_nan() {
    use cache::label::accuracy_ok;
    assert!(!accuracy_ok(f64::NAN, 50));
}

/// Isolates ordering and true (not just count-based) dedup: two of `b`'s three reasons duplicate
/// `a`'s entries and must be dropped, the third (unique) reason from `b` must survive, and every
/// surviving reason must keep the position of its *first* occurrence.
#[test]
fn merge_reasons_preserves_first_occurrence_order_and_drops_only_the_duplicate_from_b() {
    use cache::label::merge_reasons;
    let a = vec![proto::ApproxReason::ChartRounded, proto::ApproxReason::EvReferenceUnverified];
    let b = vec![
        proto::ApproxReason::EvReferenceUnverified, // duplicate of a[1]
        proto::ApproxReason::ChartRounded,          // duplicate of a[0]
        proto::ApproxReason::DeadlineBestSoFar { reached_bp: 190, target_bp: 50 },
    ];
    let merged = merge_reasons(&a, &b);
    assert_eq!(
        merged,
        vec![
            proto::ApproxReason::ChartRounded,
            proto::ApproxReason::EvReferenceUnverified,
            proto::ApproxReason::DeadlineBestSoFar { reached_bp: 190, target_bp: 50 },
        ]
    );
}

/// Fix round 1 (review R1), frozen regression per the orchestrator's ruling: `support::entry()`
/// has `exploitability_over_P = 0.004` (40bp) and no reasons. A 30bp target misses raw accuracy,
/// and spec section 3.5/`constraints.md:11` require raw accuracy to pass for `Exact` -- so this
/// is `Label::Provisional { reasons: vec![] }`, **never** `Exact` (the previous round's `Exact`
/// result for this exact input was the R1 defect). The accuracy shortfall is disclosed solely by
/// being the `Provisional` variant, not by inventing a reason.
#[test]
fn provisional_with_no_reasons_is_a_provisional_label_not_exact() {
    let entry = support::entry();
    let comparison = zero_comparison();
    let label = cache::label::label(&entry, entry.source.spr, &comparison, 30, &[]);
    assert_eq!(label, cache::label::Label::Provisional { reasons: vec![] }, "0.004 > 30bp: must be Provisional, never Exact");
    assert!(label.is_provisional());
    assert!(label.reasons().is_empty());
}

/// Isolates the "inherited reasons survive" guard: query spr equals the entry's own stored spr
/// (no `SprBucketed`), `max_dev` is zero (no `MenuRounded`), and the target is loose enough that
/// `0.004` passes (not `Provisional`). Only `e.reasons` can produce the result.
#[test]
fn label_preserves_entrys_own_inherited_reasons_when_no_other_guard_fires() {
    let mut entry = support::entry();
    entry.reasons = vec![proto::ApproxReason::ChartRounded];
    let label = cache::label::label(&entry, entry.source.spr, &zero_comparison(), 50, &[]);
    assert_eq!(label, cache::label::Label::Approximate { reasons: vec![proto::ApproxReason::ChartRounded] });
    assert!(!label.is_provisional(), "0.004 <= 50bp target");
}

/// Fix round 1 (review R2): the guard this isolates is dedup-on-merge inside `label` itself (not
/// just the standalone `merge_reasons` unit test above), using a query reason that duplicates an
/// inherited one. On its own this does **not** discriminate `label` dropping `query_reasons`
/// entirely (`merge_reasons(&[], &e.reasons)` would produce the identical result here, since the
/// only query reason is a duplicate) -- see `label_propagates_a_reason_unique_to_the_query` below
/// for the test that does.
#[test]
fn label_query_reason_duplicating_an_inherited_reason_does_not_double_count() {
    let mut entry = support::entry();
    entry.reasons = vec![proto::ApproxReason::ChartRounded, proto::ApproxReason::EvReferenceUnverified];
    let label = cache::label::label(&entry, entry.source.spr, &zero_comparison(), 50, &[proto::ApproxReason::ChartRounded]);
    assert_eq!(
        label,
        cache::label::Label::Approximate { reasons: vec![proto::ApproxReason::ChartRounded, proto::ApproxReason::EvReferenceUnverified] }
    );
    assert!(!label.is_provisional());
}

/// Fix round 1 (review R2 primary fix): the entry carries **no** inherited reasons, so the sole
/// possible source of a reason is `query_reasons` itself. Dropping `label`'s `query_reasons`
/// argument (e.g. calling `merge_reasons(&[], &e.reasons)` instead of
/// `merge_reasons(query_reasons, &e.reasons)`) would produce `Label::Exact` here instead of
/// `Approximate` -- this is the test the review's static counterexample said was missing.
/// Mutation-demonstrated in this fix round (see "Task 4 fix round 1" in the report): temporarily
/// reverting `label` to drop `query_reasons` flips this test's result to `Exact`, failing the
/// assertion below.
#[test]
fn label_propagates_a_reason_unique_to_the_query() {
    let entry = support::entry(); // reasons: vec![]
    let label =
        cache::label::label(&entry, entry.source.spr, &zero_comparison(), 50, &[proto::ApproxReason::EvReferenceUnverified]);
    assert_eq!(label, cache::label::Label::Approximate { reasons: vec![proto::ApproxReason::EvReferenceUnverified] });
    assert!(!label.is_provisional());
}

/// Fix round 1 (review R2 second part): distinct query and stored reasons, plus one duplicate
/// between them, must produce the complete ordered union (query reasons first, then each
/// not-already-seen stored reason, in `e.reasons` order) -- not just the query side or just the
/// stored side.
#[test]
fn label_combines_distinct_query_and_stored_reasons_with_one_duplicate_into_the_complete_ordered_union() {
    let mut entry = support::entry();
    entry.reasons = vec![proto::ApproxReason::EvReferenceUnverified, proto::ApproxReason::DeadlineBestSoFar { reached_bp: 60, target_bp: 50 }];
    let query_reasons = [proto::ApproxReason::ChartRounded, proto::ApproxReason::EvReferenceUnverified]; // 2nd duplicates entry.reasons[0]
    let label = cache::label::label(&entry, entry.source.spr, &zero_comparison(), 50, &query_reasons);
    assert_eq!(
        label,
        cache::label::Label::Approximate {
            reasons: vec![
                proto::ApproxReason::ChartRounded,
                proto::ApproxReason::EvReferenceUnverified,
                proto::ApproxReason::DeadlineBestSoFar { reached_bp: 60, target_bp: 50 },
            ]
        }
    );
    assert!(!label.is_provisional());
}

/// Isolates the `SprBucketed` guard: no inherited/query reasons, `max_dev` is zero (no
/// `MenuRounded`), accuracy passes (not `Provisional`) -- the sole possible addition is
/// `SprBucketed`, triggered only because the query's exact spr differs from the entry's stored
/// spr (`support::entry()`'s `source.spr` is `5/1`).
#[test]
fn label_adds_spr_bucketed_reason_only_when_query_spr_differs_from_entry_spr() {
    let entry = support::entry();
    let query_spr = Rational::new(6, 1).unwrap();
    let label = cache::label::label(&entry, query_spr, &zero_comparison(), 50, &[]);
    assert_eq!(label, cache::label::Label::Approximate { reasons: vec![proto::ApproxReason::SprBucketed { actual: 6.0, used: 5.0 }] });
    assert!(!label.is_provisional());
}

/// The control case for the guard above: when the query's spr equals the entry's own, no
/// `SprBucketed` (or any other) reason fires and the label is plain `Exact` -- proving the guard
/// above is conditioned on the spr difference, not unconditional.
#[test]
fn label_omits_spr_bucketed_reason_when_query_spr_equals_entry_spr() {
    let entry = support::entry();
    let label = cache::label::label(&entry, entry.source.spr, &zero_comparison(), 50, &[]);
    assert_eq!(label, cache::label::Label::Exact);
    assert!(!label.is_provisional());
}

/// Isolates the `MenuRounded` guard: query spr equals the entry's own (no `SprBucketed`), no
/// inherited/query reasons, accuracy passes (not `Provisional`) -- the sole possible addition is
/// `MenuRounded`, triggered solely by a positive `c.max_dev` (spec 13.1's own worked example:
/// `100/500` vs `20/100` -> `MenuRounded{2.0}`).
#[test]
fn label_adds_menu_rounded_reason_only_when_max_dev_is_positive() {
    let entry = support::entry();
    let label = cache::label::label(&entry, entry.source.spr, &comparison_with_max_dev(0.02), 50, &[]);
    assert_eq!(label, cache::label::Label::Approximate { reasons: vec![proto::ApproxReason::MenuRounded { max_delta_pct: 2.0 }] });
    assert!(!label.is_provisional());
}

/// Standing rule (task brief): reaching a looser target on this query never strips a reason the
/// entry's own solve already incurred. `support::entry()`'s `exploitability_over_P` (0.004, 40bp)
/// fails a 30bp target but passes a looser 100bp one; the stored `DeadlineBestSoFar` from the
/// entry's own (tighter) original solve must still be disclosed even though *this* query's own
/// target now passes -- the label never upgrades back toward `Exact` on a looser query.
#[test]
fn label_never_removes_a_stored_deadline_reason_when_a_looser_query_target_now_passes() {
    let mut entry = support::entry();
    entry.reasons = vec![proto::ApproxReason::DeadlineBestSoFar { reached_bp: 60, target_bp: 50 }];
    let label = cache::label::label(&entry, entry.source.spr, &zero_comparison(), 100, &[]);
    assert_eq!(
        label,
        cache::label::Label::Approximate { reasons: vec![proto::ApproxReason::DeadlineBestSoFar { reached_bp: 60, target_bp: 50 }] }
    );
    assert!(!label.is_provisional(), "0.004 <= 100bp target, so this query itself is not provisional");
}

/// A raw-accuracy miss still carries every reason that would otherwise have been disclosed (an
/// inherited reason, `SprBucketed`, and `MenuRounded` all fire simultaneously): `Provisional`
/// does not suppress or replace the reason list, it is the same merged list `Approximate` would
/// have carried, just under the variant that also discloses the accuracy shortfall.
#[test]
fn label_provisional_still_carries_every_incurred_reason() {
    let mut entry = support::entry(); // exploitability_over_P = 0.004 = 40bp
    entry.reasons = vec![proto::ApproxReason::ChartRounded];
    let query_spr = Rational::new(6, 1).unwrap();
    let label = cache::label::label(&entry, query_spr, &comparison_with_max_dev(0.02), 30, &[]);
    assert!(label.is_provisional(), "40bp > 30bp target");
    let reasons = label.reasons();
    assert_eq!(reasons.len(), 3, "expected inherited + SprBucketed + MenuRounded, got {reasons:?}");
    assert!(reasons.contains(&proto::ApproxReason::ChartRounded));
    assert!(reasons.contains(&proto::ApproxReason::SprBucketed { actual: 6.0, used: 5.0 }));
    assert!(reasons.contains(&proto::ApproxReason::MenuRounded { max_delta_pct: 2.0 }));
    assert_eq!(
        label,
        cache::label::Label::Provisional {
            reasons: vec![
                proto::ApproxReason::ChartRounded,
                proto::ApproxReason::SprBucketed { actual: 6.0, used: 5.0 },
                proto::ApproxReason::MenuRounded { max_delta_pct: 2.0 },
            ]
        }
    );
}

/// "mode is disclosed and does not change raw accuracy or labels" (task brief): two entries
/// identical except `mode` ("f32" vs "i16") must label identically under the same query.
#[test]
fn label_output_is_unaffected_by_entry_mode_f32_or_i16() {
    let mut f32_entry = support::entry();
    f32_entry.mode = "f32".into();
    let mut i16_entry = f32_entry.clone();
    i16_entry.mode = "i16".into();
    let query_spr = Rational::new(6, 1).unwrap();
    let c = comparison_with_max_dev(0.02);
    let f32_result = cache::label::label(&f32_entry, query_spr, &c, 30, &[]);
    let i16_result = cache::label::label(&i16_entry, query_spr, &c, 30, &[]);
    assert_eq!(f32_result, i16_result);
}

// --- task 7: strict menu legality, bounded lookup, node reconstruction (spec 10.4) -------------

/// Fix round 1 (review P4T7-I4): only an accuracy-passing label claims coverage. `Provisional`
/// maps to none at all, with or without reasons, so no caller can read it as `Exact`.
#[test]
fn only_an_accuracy_passing_label_claims_coverage() {
    use cache::label::Label;
    let reasons = vec![proto::ApproxReason::ChartRounded];
    assert_eq!(Label::Exact.coverage(), Some(proto::Coverage::Exact));
    assert_eq!(Label::Approximate { reasons: reasons.clone() }.coverage(), Some(proto::Coverage::Approximate { reasons: reasons.clone() }));
    assert_eq!(Label::Provisional { reasons: vec![] }.coverage(), None);
    assert_eq!(Label::Provisional { reasons }.coverage(), None);
}

#[test]
fn query_menu_must_be_legal_without_probability_moves() {
    use cache::lookup::legal_menu;
    use proto::{Action, LegalAction};
    let legal = vec![LegalAction::Fold, LegalAction::Call { cost: 50 }, LegalAction::Raise { min_to: 100, max_to: 500 }, LegalAction::AllIn { to: 500 }];
    assert!(legal_menu(&[Action::Fold, Action::Call, Action::Raise { to: 100 }], &legal));
    assert!(!legal_menu(&[Action::Raise { to: 99 }], &legal));
    assert!(!legal_menu(&[Action::AllIn { to: 501 }], &legal));
}

#[test]
fn legal_menu_matches_kinds_strictly_and_wager_sizes_inside_their_bounds() {
    use cache::lookup::legal_menu;
    use proto::LegalAction;
    let legal = [LegalAction::Check, LegalAction::Bet { min_to: 20, max_to: 300 }, LegalAction::AllIn { to: 300 }];
    assert!(legal_menu(&[Action::Check, Action::Bet { to: 20 }, Action::Bet { to: 300 }, Action::AllIn { to: 300 }], &legal));
    assert!(!legal_menu(&[Action::Bet { to: 19 }], &legal), "below min_to");
    assert!(!legal_menu(&[Action::Bet { to: 301 }], &legal), "above max_to");
    assert!(!legal_menu(&[Action::Raise { to: 100 }], &legal), "a raise is not a bet");
    assert!(!legal_menu(&[Action::Call], &legal), "nothing to call");
    assert!(!legal_menu(&[Action::Fold], &legal), "no fold offered when checking is free");
    assert!(!legal_menu(&[Action::Check, Action::AllIn { to: 299 }], &legal), "one illegal entry makes the whole menu illegal");
}

#[test]
fn map_rows_and_map_flags_move_every_combo_to_its_image_under_the_inverse_permutation() {
    use cache::lookup::{map_flags, map_rows};
    use core_iso::{apply, inverse, SuitPerm};
    let perm = SuitPerm([2, 0, 3, 1]);
    let inv = inverse(&perm);
    let rows: Vec<Vec<f32>> = (0..1326).map(|i| vec![i as f32, -(i as f32)]).collect();
    let flags: Vec<bool> = (0..1326).map(|i| i % 3 == 0).collect();
    let mapped = map_rows(&rows, &inv);
    let mapped_flags = map_flags(&flags, &inv);
    for i in 0..1326_u16 {
        let [a, b] = proto::combo_cards(i);
        let j = proto::combo_index(apply(&inv, a), apply(&inv, b)) as usize;
        assert_eq!(mapped[j], rows[i as usize], "combo {i}");
        assert_eq!(mapped_flags[j], flags[i as usize], "combo {i}");
    }
    assert_eq!(map_rows(&mapped, &perm), rows, "mapping back under the permutation itself is the identity");
    assert_eq!(map_flags(&mapped_flags, &perm), flags);
    assert_eq!(map_rows(&rows, &SuitPerm::IDENTITY), rows);
}

// --- task 7: the bounded lookup against a real store ------------------------------------------------
//
// Every store goes through the real writer and is awaited through its `StoreReceipt`, and every
// lookup through the real `cache-reader` thread. `support::entry()` is a canonical-suit check/jam
// flop entry at pot 100, stacks 500/500 (SPR 5, bucket 81), raw accuracy 0.004 (40bp).

#[path = "support/temp_dir.rs"]
mod temp_dir;

use cache::lookup::{CacheQuery, Lookup, MissReason};
use cache::Cache;
use std::time::Duration;
use temp_dir::TempDir;

/// A failure budget for the writer thread, never a synchronization delay.
const WRITER_BUDGET: Duration = Duration::from_secs(30);

/// `support::entry()` retuned to root pot `pot` and equal stacks `stack`: every field derived from
/// the pot or the SPR (bucket, SPR rational, quantum, rake-cap fraction, menu fractions) is
/// recomputed, so the entry stays valid in the SPR bucket its own SPR names.
fn entry_at_spr(pot: u32, stack: u32) -> CacheEntry {
    let mut e = support::entry();
    e.source.pot = pot;
    e.source.stack_oop = stack;
    e.source.stack_ip = stack;
    e.source.spr = Rational::new(stack as u64, pot as u64).unwrap();
    e.source.quantum_over_p = Rational::new(1, pot as u64).unwrap();
    e.key.spr_bucket = cache::key::spr_bucket(e.source.spr);
    e.key.rake = RakeKey::new(0.05, Rational::new(e.source.cap_mchips as u64, 1000 * pot as u64).unwrap(), 1).unwrap();
    e.fractions = e.tree.materialized.iter().map(|n| n.actions.iter().map(|a| action_to(a).map(|to| Rational::new(to as u64, pot as u64).unwrap())).collect()).collect();
    cache::entry::validate_entry(&e).expect("the retuned fixture must stay a valid entry");
    e
}

/// The query a live request at root pot `pot` and equal stacks `stack` makes against entry `e`'s
/// scenario: the same canonical key at the query's own SPR bucket, the entry's own tree and suits,
/// the root node for `oop`, whose legal menu is exactly the tree's `Check` / `AllIn{500}`.
fn query_for(e: &CacheEntry, pot: u32, stack: u32) -> CacheQuery {
    let spr = Rational::new(stack as u64, pot as u64).unwrap();
    CacheQuery {
        key: e.key.at_bucket(cache::key::spr_bucket(spr)),
        source: SourceInputs {
            pot,
            stack_oop: stack,
            stack_ip: stack,
            spr,
            bb_chips: 2,
            quantum_over_p: Rational::new(1, pot as u64).unwrap(),
            cap_mchips: e.source.cap_mchips,
            ranges: e.source.ranges.clone(),
        },
        tree: e.tree.clone(),
        requested: vec![],
        actor: "oop".into(),
        legal: vec![proto::LegalAction::Check, proto::LegalAction::AllIn { to: 500 }],
        target_bp: 50,
        reasons: vec![],
        inverse_perm: core_iso::SuitPerm::IDENTITY,
        budget: Duration::from_secs(5),
    }
}

/// Opens a real cache at `dir` and stores `entries` through the writer, each one confirmed durable.
fn open_with(dir: &TempDir, entries: &[&CacheEntry]) -> Cache {
    let cache = Cache::open(dir.path().to_path_buf(), cache::CACHE_QUOTA_BYTES);
    for e in entries {
        assert!(cache.store_tracked(e).wait(WRITER_BUDGET), "the writer must store the fixture");
    }
    cache
}

/// The cell file is still where it was and still decodes: an ordinary query mismatch is a miss,
/// never a deletion.
fn assert_cell_intact(dir: &TempDir, e: &CacheEntry) {
    let path = cache::storage::entry_path(dir.path(), e.key.digest());
    assert!(cache::storage::read_cell(&path).is_some(), "a query mismatch must never delete or damage the cell at {path:?}");
}

#[test]
fn lookup_serves_an_exact_hit_rebuilt_in_the_querys_chips() {
    let e = support::entry();
    let dir = TempDir::new("lookup-exact");
    let cache = open_with(&dir, &[&e]);
    let q = query_for(&e, 100, 500);
    let hit = match cache.lookup(&q) {
        Lookup::Exact { hit } => hit,
        other => panic!("an identical query is an exact hit, got {other:?}"),
    };
    assert_eq!(hit.coverage, Some(proto::Coverage::Exact));
    proto::worker::validate_solution(&hit.solution, &q.tree.materialized).expect("the served solution validates against the query tree");
    assert_eq!(hit.solution.requested, 0, "the root is the first covered node");
    assert_eq!(hit.solution.nodes.len(), e.nodes.len());
    for (node, cached) in hit.solution.nodes.iter().zip(&e.nodes) {
        assert_eq!(node.probs, cached.probs);
        assert_eq!(node.available, cached.available);
        let ev: Vec<Vec<f32>> = cached.ev_over_P.iter().map(|r| r.iter().map(|v| v * 100.0).collect()).collect();
        assert_eq!(node.ev_chips, ev, "EV is ev_over_P times the query's pot");
    }
    assert_eq!(hit.covered_paths, e.covered_paths);
    assert_eq!(hit.tree, q.tree);
    assert_eq!(hit.source_mode, "f32");
    assert_eq!(hit.tree_signature, e.key.tree_signature);
    assert_eq!(hit.raw_exploitability_over_p, e.exploitability_over_P);
    assert_eq!(hit.solution.exploitability_chips, (e.exploitability_over_P * 100.0) as f32);
    assert_eq!(hit.notes, vec!["cache realized menu at the requested node: [500]".to_string(), "cache source storage mode: f32".to_string()]);
    drop(cache);
}

/// The query is at twice the entry's chips (same SPR, same rake-cap fraction): every exported node
/// takes the query's own materialized actions and chip path, and EV is `ev_over_P * P_query`.
#[test]
fn lookup_rebuilds_nodes_with_the_query_trees_actions_chip_paths_and_pot() {
    let e = support::entry();
    let dir = TempDir::new("lookup-scaled");
    let cache = open_with(&dir, &[&e]);
    let mut q = query_for(&e, 200, 1000);
    q.source.cap_mchips = 2 * e.source.cap_mchips;
    q.key = e.key.clone();
    for node in &mut q.tree.materialized {
        for a in &mut node.actions {
            if let Action::AllIn { to } = a {
                *to *= 2;
            }
        }
        for t in node.terminal_pots.iter_mut().flatten() {
            *t *= 2;
        }
    }
    q.legal = vec![proto::LegalAction::Check, proto::LegalAction::AllIn { to: 1000 }];
    let hit = match cache.lookup(&q) {
        Lookup::Exact { hit } => hit,
        other => panic!("a same-SPR query at twice the chips is an exact hit, got {other:?}"),
    };
    for (node, cached) in hit.solution.nodes.iter().zip(&e.nodes) {
        let m = q.tree.materialized.iter().find(|m| m.path == cached.path).unwrap();
        assert_eq!(node.actions, m.actions, "the query's materialized actions");
        assert_eq!(Some(node.path.clone()), cache::entry::chip_path(&q.tree.materialized, &cached.path), "the query's chip path");
        let ev: Vec<Vec<f32>> = cached.ev_over_P.iter().map(|r| r.iter().map(|v| v * 200.0).collect()).collect();
        assert_eq!(node.ev_chips, ev);
    }
    assert!(hit.solution.nodes.iter().any(|n| n.path == vec![Action::AllIn { to: 1000 }]), "a chip path in the query's own chips");
    assert_eq!(hit.notes[0], "cache realized menu at the requested node: [1000]");
    drop(cache);
}

/// A query in non-canonical suits carries the inverse of its canonicalizing permutation: every
/// combo row, EV row and availability flag lands on its image. The query's board and ranges are
/// the entry's canonical ones with their suits relabelled, and the permutation that maps them back
/// is the one `core_iso::canonicalize` itself returns for them. The fixture's six AA combos are a
/// suit-symmetric set, so each gets its own probability and EV row here: a row left in canonical
/// suits then lands on the wrong combo and is seen.
#[test]
fn lookup_maps_every_row_back_to_the_querys_suits() {
    let mut e = support::entry();
    for node in &mut e.nodes {
        let live = (0..1326).filter(|&i| node.available[i]).collect::<Vec<_>>();
        for (n, &i) in live.iter().enumerate() {
            let p = 0.1 * (n + 1) as f32;
            node.probs[i] = vec![p, 1.0 - p];
            node.ev_over_P[i][1] = 0.01 * (n + 1) as f32;
        }
    }
    cache::entry::validate_entry(&e).expect("per-combo rows keep the entry valid");
    let relabel = core_iso::SuitPerm([1, 3, 0, 2]);
    let board = e.key.canonical_board.iter().map(|c| core_iso::apply(&relabel, *c)).collect::<Vec<_>>();
    let aa = core_iso::apply_range(&relabel, &e.source.ranges[0]);
    let (canonical, perm) = core_iso::canonicalize(&board, &[&aa, &aa]);
    assert_ne!(perm, core_iso::SuitPerm::IDENTITY, "the query's suits must need a real permutation");
    assert_eq!(canonical.cards(), e.key.canonical_board.as_slice(), "the query canonicalizes onto the entry's board");
    assert_eq!(core_iso::apply_range(&perm, &aa), e.source.ranges[0], "and onto the entry's ranges");
    let inverse = core_iso::inverse(&perm);
    let dir = TempDir::new("lookup-suits");
    let cache = open_with(&dir, &[&e]);
    let mut q = query_for(&e, 100, 500);
    q.inverse_perm = inverse;
    let hit = match cache.lookup(&q) {
        Lookup::Exact { hit } => hit,
        other => panic!("{other:?}"),
    };
    for (node, cached) in hit.solution.nodes.iter().zip(&e.nodes) {
        for c in 0..1326_u16 {
            let [a, b] = proto::combo_cards(c);
            let j = proto::combo_index(core_iso::apply(&inverse, a), core_iso::apply(&inverse, b)) as usize;
            let i = c as usize;
            assert_eq!(node.available[j], cached.available[i]);
            assert_eq!(node.probs[j], cached.probs[i]);
            assert_eq!(node.ev_chips[j], cached.ev_over_P[i].iter().map(|v| v * 100.0).collect::<Vec<_>>());
        }
        let live: Vec<usize> = (0..1326).filter(|&i| node.available[i]).collect();
        assert_eq!(live.len(), 6, "the six AA combos, back in the query's own suits");
        assert!(live.iter().all(|&i| aa.0[i] > 0.0), "every served combo is an AA combo of the original suits");
    }
    drop(cache);
}

/// Spec 10.4: a query at bucket `b` reads `b - 1, b, b + 1`. An entry one bucket up is found and
/// disclosed as `SprBucketed`; an entry two buckets up is never read, even though `compare` alone
/// would accept it (`delta` = 1005/51229 < 0.02 across the bucket edges).
#[test]
fn lookup_reads_the_neighbouring_buckets_and_never_further() {
    let (pot, query_stack) = (10_000, 50_224);
    let near = entry_at_spr(pot, 51_000);
    let far = entry_at_spr(pot, 51_229);
    let q = query_for(&near, pot, query_stack);
    assert_eq!((q.key.spr_bucket, near.key.spr_bucket, far.key.spr_bucket), (81, 82, 83));
    assert!(compare(&far, &q.tree, pot, query_stack, q.source.cap_mchips).is_some(), "only the bucket bound may reject the far entry");

    let dir = TempDir::new("lookup-far");
    let cache = open_with(&dir, &[&far]);
    assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::NoMatch }, "an entry two buckets away is never read");
    assert_cell_intact(&dir, &far);
    drop(cache);

    let dir = TempDir::new("lookup-near");
    let cache = open_with(&dir, &[&near]);
    match cache.lookup(&q) {
        Lookup::Approximate { hit, reasons } => {
            let bucketed = vec![proto::ApproxReason::SprBucketed { actual: q.source.spr.value() as f32, used: near.source.spr.value() as f32 }];
            assert_eq!(reasons, bucketed);
            assert_eq!(hit.coverage, Some(proto::Coverage::Approximate { reasons: bucketed }));
        }
        other => panic!("the neighbouring bucket's entry is an approximate hit, got {other:?}"),
    }
    drop(cache);
}

/// The probe's other side: a query at bucket 82 finds an entry one bucket below it.
#[test]
fn lookup_reads_the_bucket_below_too() {
    let pot = 10_000;
    let below = entry_at_spr(pot, 50_224);
    let q = query_for(&below, pot, 51_000);
    assert_eq!((q.key.spr_bucket, below.key.spr_bucket), (82, 81));
    let dir = TempDir::new("lookup-below");
    let cache = open_with(&dir, &[&below]);
    assert!(
        matches!(cache.lookup(&q), Lookup::Approximate { ref reasons, .. } if matches!(reasons[..], [proto::ApproxReason::SprBucketed { .. }])),
        "the entry one bucket below is read and disclosed as SprBucketed"
    );
    drop(cache);
}

/// Candidates rank by SPR delta first: an above-target entry at the query's exact SPR beats an
/// at-target entry one bucket away and is served `Provisional` -- never reordered for accuracy.
#[test]
fn lookup_ranks_by_closeness_before_accuracy_and_serves_the_closer_one_provisional() {
    let pot = 10_000;
    let mut close = entry_at_spr(pot, 50_224);
    close.exploitability_over_P = 0.006;
    let mut accurate = entry_at_spr(pot, 51_000);
    accurate.exploitability_over_P = 0.001;
    let dir = TempDir::new("lookup-rank");
    let cache = open_with(&dir, &[&close, &accurate]);
    match cache.lookup(&query_for(&close, pot, 50_224)) {
        Lookup::Provisional { hit, reasons } => {
            assert!(reasons.is_empty(), "the closer entry is at the query's exact SPR: {reasons:?}");
            assert_eq!(hit.raw_exploitability_over_p, 0.006);
            assert_eq!(hit.coverage, None, "an above-target hit claims no coverage (spec 2: Exact requires the raw accuracy)");
        }
        other => panic!("the closer, above-target entry must be served Provisional, got {other:?}"),
    }
    drop(cache);
}

/// Fix round 1 (review P4T7-I4): an above-target, reason-free entry is `Provisional` with no
/// reasons and NO coverage -- never `Coverage::Exact`, which spec 2 line 51 reserves for inputs
/// whose requested accuracy was reached on the raw exploitability (spec 10.4: a validated entry
/// above the target is served as Provisional). No `DeadlineBestSoFar` is synthesized: this query
/// never reached a live deadline. The raw accuracy stays on the hit.
#[test]
fn an_above_target_reason_free_entry_is_provisional_with_no_coverage() {
    let e = support::entry();
    let dir = TempDir::new("lookup-provisional");
    let cache = open_with(&dir, &[&e]);
    let mut q = query_for(&e, 100, 500);
    q.target_bp = 30;
    match cache.lookup(&q) {
        Lookup::Provisional { hit, reasons } => {
            assert!(reasons.is_empty(), "no reason applies and none is invented: {reasons:?}");
            assert_eq!(hit.coverage, None, "0.004 > 30bp: never Coverage::Exact");
            assert_eq!(hit.raw_exploitability_over_p, 0.004);
        }
        other => panic!("0.004 > 30bp must be Provisional, got {other:?}"),
    }
    drop(cache);
}

#[test]
fn lookup_carries_the_querys_own_reasons() {
    let e = support::entry();
    let dir = TempDir::new("lookup-reasons");
    let cache = open_with(&dir, &[&e]);
    let mut q = query_for(&e, 100, 500);
    q.reasons = vec![proto::ApproxReason::EvReferenceUnverified];
    match cache.lookup(&q) {
        Lookup::Approximate { hit, reasons } => {
            assert_eq!(reasons, vec![proto::ApproxReason::EvReferenceUnverified]);
            assert_eq!(hit.coverage, Some(proto::Coverage::Approximate { reasons }));
        }
        other => panic!("{other:?}"),
    }
    drop(cache);
}

/// Strict legality: a requested node whose menu is not exactly legal at the live state is a miss,
/// and the cell stays on disk.
#[test]
fn lookup_misses_when_the_requested_menu_is_not_legal_and_keeps_the_cell() {
    let e = support::entry();
    let dir = TempDir::new("lookup-illegal");
    let cache = open_with(&dir, &[&e]);
    let mut q = query_for(&e, 100, 500);
    q.legal = vec![proto::LegalAction::Check, proto::LegalAction::AllIn { to: 499 }];
    assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::Rejected }, "an all-in to 500 is not the legal all-in to 499");
    q.legal = vec![proto::LegalAction::AllIn { to: 500 }];
    assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::Rejected }, "a check the live state does not offer");
    assert_cell_intact(&dir, &e);
    q.legal = vec![proto::LegalAction::Check, proto::LegalAction::AllIn { to: 500 }];
    assert!(matches!(cache.lookup(&q), Lookup::Exact { .. }), "the same cell still serves a legal query");
    drop(cache);
}

/// A requested node that is not covered, or whose actor is not the query's, is a node-only miss:
/// no deletion. A covered non-root node is served at its own index.
#[test]
fn lookup_misses_on_an_uncovered_path_or_the_wrong_actor_and_serves_a_covered_node() {
    let e = support::entry();
    let dir = TempDir::new("lookup-node");
    let cache = open_with(&dir, &[&e]);
    let mut q = query_for(&e, 100, 500);
    q.actor = "ip".into();
    assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::NoMatch }, "the root is oop's");
    let mut q = query_for(&e, 100, 500);
    q.requested = vec![0, 0];
    assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::NoMatch }, "the turn root is not in a flop street export");
    q.requested = vec![7];
    assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::NoMatch }, "not a node of the tree at all");
    assert_cell_intact(&dir, &e);

    let mut q = query_for(&e, 100, 500);
    q.requested = vec![1];
    q.actor = "ip".into();
    q.legal = vec![proto::LegalAction::Fold, proto::LegalAction::Call { cost: 400 }];
    let hit = match cache.lookup(&q) {
        Lookup::Exact { hit } => hit,
        other => panic!("ip facing the jam is covered, got {other:?}"),
    };
    let index = e.covered_paths.iter().position(|p| *p == vec![1]).unwrap();
    assert_eq!(hit.solution.requested as usize, index);
    assert_eq!(hit.solution.nodes[index].actor, "ip");
    assert_eq!(hit.notes[0], "cache realized menu at the requested node: []");
    drop(cache);
}

/// A zero remaining budget is an immediate `BudgetExhausted` miss (review P4T7-I2: decided before
/// anything is posted; the private-channel unit test in `src/lib.rs` pins that nothing is), and the
/// one reader thread keeps serving the requests after it.
#[test]
fn a_spent_budget_is_an_immediate_miss_and_the_reader_keeps_serving() {
    let e = support::entry();
    let dir = TempDir::new("lookup-budget");
    let cache = open_with(&dir, &[&e]);
    let mut q = query_for(&e, 100, 500);
    q.budget = Duration::ZERO;
    assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::BudgetExhausted });
    q.budget = Duration::from_secs(5);
    for _ in 0..3 {
        assert!(matches!(cache.lookup(&q), Lookup::Exact { .. }), "the reader still serves after a spent request");
    }
    drop(cache);
}

#[test]
fn a_disabled_or_shut_down_cache_always_misses() {
    let e = support::entry();
    let q = query_for(&e, 100, 500);
    assert_eq!(Cache::disabled().lookup(&q), Lookup::Miss { reason: MissReason::ReaderUnavailable });
    let dir = TempDir::new("lookup-shutdown");
    let cache = open_with(&dir, &[&e]);
    assert!(matches!(cache.lookup(&q), Lookup::Exact { .. }));
    cache.shutdown();
    assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::ReaderUnavailable }, "a stopped reader is a miss, never a hang");
    drop(cache);
}

/// Review P4T7-I6: a key with nothing stored under any of its three buckets is `NoMatch`.
#[test]
fn an_absent_board_class_is_a_no_match_miss() {
    let e = support::entry();
    let dir = TempDir::new("lookup-empty");
    let cache = open_with(&dir, &[]);
    assert_eq!(cache.lookup(&query_for(&e, 100, 500)), Lookup::Miss { reason: MissReason::NoMatch });
    drop(cache);
}

/// A served hit re-dates its entry through the writer (`Cache::touch` with the entry's payload
/// digest), so eviction sees it as fresh.
#[test]
fn a_served_hit_touches_its_entry() {
    let e = support::entry();
    let dir = TempDir::new("lookup-touch");
    let cache = open_with(&dir, &[&e]);
    let path = cache::storage::entry_path(dir.path(), e.key.digest());
    let before = cache::storage::read_cell(&path).unwrap().entries[0].last_hit;
    assert!(matches!(cache.lookup(&query_for(&e, 100, 500)), Lookup::Exact { .. }));
    // A writer barrier: a refused store is answered only after every command queued before it.
    let mut barrier = support::entry();
    barrier.key.schema_version = 2;
    assert!(!cache.store_tracked(&barrier).wait(WRITER_BUDGET));
    let after = cache::storage::read_cell(&path).unwrap().entries[0].last_hit;
    assert!(after > before, "the hit must re-date its entry ({before} -> {after})");
    drop(cache);
}

// --- task 7: candidate selection over cells already read ---------------------------------------------

#[test]
fn select_prefers_better_raw_accuracy_at_equal_rank_and_breaks_full_ties_by_payload_digest() {
    use cache::lookup::select;
    use cache::storage::Cell;
    let q = query_for(&support::entry(), 100, 500);
    let mut worse = support::entry();
    worse.exploitability_over_P = 0.004;
    let mut better = support::entry();
    better.exploitability_over_P = 0.002;
    let chosen = select(vec![Cell { entries: vec![worse.clone(), better.clone()] }], &q).unwrap().0;
    assert_eq!(chosen.exploitability_over_P, 0.002);
    let chosen = select(vec![Cell { entries: vec![better.clone(), worse] }], &q).unwrap().0;
    assert_eq!(chosen.exploitability_over_P, 0.002, "arrival order never decides");

    let mut twin = better.clone();
    twin.iterations += 1;
    let a = select(vec![Cell { entries: vec![better.clone(), twin.clone()] }], &q).unwrap().0;
    let b = select(vec![Cell { entries: vec![twin, better] }], &q).unwrap().0;
    assert_eq!(a.iterations, b.iterations, "a full tie is broken by the payload digest, never by order");
}

#[test]
fn select_skips_invalid_foreign_uncovered_and_wrong_actor_candidates() {
    use cache::lookup::select;
    use cache::storage::Cell;
    let q = query_for(&support::entry(), 100, 500);
    let mut invalid = support::entry();
    invalid.key.schema_version = 2;
    let mut foreign = support::entry();
    foreign.key.tree_signature = "another_tree_v1".into();
    assert!(select(vec![Cell { entries: vec![invalid, foreign] }], &q).is_none());
    let mut uncovered = q.clone();
    uncovered.requested = vec![0, 0];
    assert!(select(vec![Cell { entries: vec![support::entry()] }], &uncovered).is_none());
    let mut wrong_actor = q.clone();
    wrong_actor.actor = "ip".into();
    assert!(select(vec![Cell { entries: vec![support::entry()] }], &wrong_actor).is_none());
    assert!(select(vec![Cell { entries: vec![support::entry()] }], &q).is_some());
}
