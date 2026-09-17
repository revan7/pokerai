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

// --- task 4: inherited reasons and raw-accuracy filtering (spec 10.4/13.1) -----------------------
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

/// Isolates the finiteness guard from a narrower "NaN-only" check: `+inf` is not NaN, so an
/// implementation that only tested `raw.is_nan()` (rather than `!raw.is_finite()`) would wrongly
/// accept it here.
#[test]
fn accuracy_ok_rejects_positive_infinity() {
    use cache::label::accuracy_ok;
    assert!(!accuracy_ok(f64::INFINITY, 50));
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

/// The brief's boundary case: `support::entry()` has `exploitability_over_P = 0.004` (40bp) and
/// no reasons. A 30bp target misses (`provisional`), but a raw-accuracy miss alone must never
/// synthesize an `Approximate{reasons: []}` -- it stays `Coverage::Exact`, disclosed solely by
/// the `Provisional` phase plus the raw reached exploitability.
#[test]
fn provisional_without_reasons_is_never_an_empty_approximate() {
    let entry = support::entry();
    let comparison = zero_comparison();
    let (coverage, provisional) = cache::label::label(&entry, entry.source.spr, &comparison, 30, &[]);
    assert!(provisional, "0.004 > 30 bp");
    assert!(matches!(coverage, proto::Coverage::Exact));
}

/// Isolates the "inherited reasons survive" guard: query spr equals the entry's own stored spr
/// (no `SprBucketed`), `max_dev` is zero (no `MenuRounded`), and the target is loose enough that
/// `0.004` passes (`provisional` false, so no accuracy-driven disclosure either). Only
/// `e.reasons` can produce the result.
#[test]
fn label_preserves_entrys_own_inherited_reasons_when_no_other_guard_fires() {
    let mut entry = support::entry();
    entry.reasons = vec![proto::ApproxReason::ChartRounded];
    let (coverage, provisional) = cache::label::label(&entry, entry.source.spr, &zero_comparison(), 50, &[]);
    assert_eq!(coverage, proto::Coverage::Approximate { reasons: vec![proto::ApproxReason::ChartRounded] });
    assert!(!provisional, "0.004 <= 50bp target");
}

/// Isolates dedup-on-merge inside `label` itself (not just the standalone `merge_reasons` unit
/// test above): the same reason is both inherited (`e.reasons`) and incurred by the query
/// (`query_reasons`), and must appear exactly once in the disclosed coverage, alongside the
/// entry's other, non-duplicated inherited reason.
#[test]
fn label_merges_query_reasons_with_inherited_reasons_and_dedups() {
    let mut entry = support::entry();
    entry.reasons = vec![proto::ApproxReason::ChartRounded, proto::ApproxReason::EvReferenceUnverified];
    let (coverage, provisional) =
        cache::label::label(&entry, entry.source.spr, &zero_comparison(), 50, &[proto::ApproxReason::ChartRounded]);
    assert_eq!(
        coverage,
        proto::Coverage::Approximate { reasons: vec![proto::ApproxReason::ChartRounded, proto::ApproxReason::EvReferenceUnverified] }
    );
    assert!(!provisional);
}

/// Isolates the `SprBucketed` guard: no inherited/query reasons, `max_dev` is zero (no
/// `MenuRounded`), accuracy passes (no provisional) -- the sole possible addition is
/// `SprBucketed`, triggered only because the query's exact spr differs from the entry's stored
/// spr (`support::entry()`'s `source.spr` is `5/1`).
#[test]
fn label_adds_spr_bucketed_reason_only_when_query_spr_differs_from_entry_spr() {
    let entry = support::entry();
    let query_spr = Rational::new(6, 1).unwrap();
    let (coverage, provisional) = cache::label::label(&entry, query_spr, &zero_comparison(), 50, &[]);
    assert_eq!(coverage, proto::Coverage::Approximate { reasons: vec![proto::ApproxReason::SprBucketed { actual: 6.0, used: 5.0 }] });
    assert!(!provisional);
}

/// The control case for the guard above: when the query's spr equals the entry's own, no
/// `SprBucketed` (or any other) reason fires and coverage is plain `Exact` -- proving the guard
/// above is conditioned on the spr difference, not unconditional.
#[test]
fn label_omits_spr_bucketed_reason_when_query_spr_equals_entry_spr() {
    let entry = support::entry();
    let (coverage, provisional) = cache::label::label(&entry, entry.source.spr, &zero_comparison(), 50, &[]);
    assert_eq!(coverage, proto::Coverage::Exact);
    assert!(!provisional);
}

/// Isolates the `MenuRounded` guard: query spr equals the entry's own (no `SprBucketed`), no
/// inherited/query reasons, accuracy passes (no provisional) -- the sole possible addition is
/// `MenuRounded`, triggered solely by a positive `c.max_dev` (spec 13.1's own worked example:
/// `100/500` vs `20/100` -> `MenuRounded{2.0}`).
#[test]
fn label_adds_menu_rounded_reason_only_when_max_dev_is_positive() {
    let entry = support::entry();
    let (coverage, provisional) = cache::label::label(&entry, entry.source.spr, &comparison_with_max_dev(0.02), 50, &[]);
    assert_eq!(coverage, proto::Coverage::Approximate { reasons: vec![proto::ApproxReason::MenuRounded { max_delta_pct: 2.0 }] });
    assert!(!provisional);
}

/// Standing rule (task brief): reaching a looser target on this query never strips a reason the
/// entry's own solve already incurred. `support::entry()`'s `exploitability_over_P` (0.004, 40bp)
/// fails a 30bp target but passes a looser 100bp one; the stored `DeadlineBestSoFar` from the
/// entry's own (tighter) original solve must still be disclosed even though *this* query's own
/// target now passes -- coverage never upgrades back toward `Exact` on a looser query.
#[test]
fn label_never_removes_a_stored_deadline_reason_when_a_looser_query_target_now_passes() {
    let mut entry = support::entry();
    entry.reasons = vec![proto::ApproxReason::DeadlineBestSoFar { reached_bp: 60, target_bp: 50 }];
    let (coverage, provisional) = cache::label::label(&entry, entry.source.spr, &zero_comparison(), 100, &[]);
    assert_eq!(
        coverage,
        proto::Coverage::Approximate { reasons: vec![proto::ApproxReason::DeadlineBestSoFar { reached_bp: 60, target_bp: 50 }] }
    );
    assert!(!provisional, "0.004 <= 100bp target, so this query itself is not provisional");
}

/// Accuracy miss (`provisional`) fires even while `SprBucketed`, `MenuRounded` and an inherited
/// reason are all simultaneously present -- the two disclosures (the `Coverage` reason list and
/// the `Provisional` phase) are independent axes; neither masks the other.
#[test]
fn label_provisional_flag_is_independent_of_reasons_present() {
    let mut entry = support::entry(); // exploitability_over_P = 0.004 = 40bp
    entry.reasons = vec![proto::ApproxReason::ChartRounded];
    let query_spr = Rational::new(6, 1).unwrap();
    let (coverage, provisional) = cache::label::label(&entry, query_spr, &comparison_with_max_dev(0.02), 30, &[]);
    assert!(provisional, "40bp > 30bp target");
    match coverage {
        proto::Coverage::Approximate { reasons } => {
            assert_eq!(reasons.len(), 3, "expected inherited + SprBucketed + MenuRounded, got {reasons:?}");
            assert!(reasons.contains(&proto::ApproxReason::ChartRounded));
            assert!(reasons.contains(&proto::ApproxReason::SprBucketed { actual: 6.0, used: 5.0 }));
            assert!(reasons.contains(&proto::ApproxReason::MenuRounded { max_delta_pct: 2.0 }));
        }
        other => panic!("expected Approximate with 3 reasons, got {other:?}"),
    }
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
