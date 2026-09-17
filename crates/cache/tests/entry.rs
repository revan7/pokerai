mod support;

use cache::entry::{validate_entry, CacheEntry};
use cache::CacheError;
use cache::key::{Model, RakeKey, Rational};
use proto::{Action, MaterializedNode, Street};

#[test]
fn actor_owned_paths_are_not_root_aliases() {
    let tree = vec![
        MaterializedNode {
            path: vec![],
            street: Street::Flop,
            actor: "oop".into(),
            actions: vec![Action::Check, Action::Bet { to: 50 }],
            terminal_pots: vec![None, None],
        },
        MaterializedNode {
            path: vec![0],
            street: Street::Flop,
            actor: "ip".into(),
            actions: vec![Action::Check, Action::Bet { to: 50 }],
            terminal_pots: vec![None, None],
        },
        MaterializedNode {
            path: vec![1],
            street: Street::Flop,
            actor: "ip".into(),
            actions: vec![Action::Fold, Action::Call],
            terminal_pots: vec![Some(100), None],
        },
    ];
    assert_eq!(cache::entry::resolve_path(&tree, &[Action::Bet { to: 50 }]), Some(vec![1]));
    assert_eq!(cache::entry::chip_path(&tree, &[0]), Some(vec![Action::Check]));
    assert_eq!(cache::entry::resolve_path(&tree, &[Action::Bet { to: 51 }]), None);
}

// --- normalize ------------------------------------------------------------------------------

/// A minimal two-node flop tree (root `oop` deciding Check/Bet{50}, child `ip` deciding
/// Fold/Call after the bet) plus a `StreetSolution` that already satisfies
/// `proto::worker::validate_solution` against it, built from `support::rows`.
fn simple_tree_and_solution() -> (proto::EffectiveTree, proto::worker::StreetSolution) {
    use proto::{MenuSize, PlayerMenus, SideMenu};
    use std::collections::BTreeMap;

    let materialized = vec![
        MaterializedNode {
            path: vec![],
            street: Street::Flop,
            actor: "oop".into(),
            actions: vec![Action::Check, Action::Bet { to: 50 }],
            terminal_pots: vec![None, None],
        },
        MaterializedNode {
            path: vec![1],
            street: Street::Flop,
            actor: "ip".into(),
            actions: vec![Action::Fold, Action::Call],
            terminal_pots: vec![Some(100), Some(200)],
        },
    ];
    let side = SideMenu { bet: vec![MenuSize::Pot(0.5)], raise: vec![] };
    let mut menus = BTreeMap::new();
    menus.insert(Street::Flop, PlayerMenus { oop: side.clone(), ip: side, donk: None });
    let tree = proto::EffectiveTree {
        rules_version: 3,
        template_id: "normalize_test_v1".into(),
        root_street: Street::Flop,
        menus,
        add_allin_threshold: 0.0,
        force_allin_threshold: 0.0,
        merging_threshold: 0.0,
        wager_cap: 1,
        inserted: vec![],
        materialized: materialized.clone(),
    };

    let (probs0, ev0, avail0) = support::rows(2);
    let (probs1, mut ev1, avail1) = support::rows(2);
    // Action 0 at the `ip` node is Fold: spec section 2 requires EV(fold) == 0 exactly.
    for row in &mut ev1 {
        row[0] = 0.0;
    }
    let nodes = vec![
        proto::worker::NodeStrategy {
            path: vec![],
            actor: "oop".into(),
            actions: materialized[0].actions.clone(),
            probs: probs0,
            ev_chips: ev0,
            available: avail0,
        },
        proto::worker::NodeStrategy {
            path: vec![Action::Bet { to: 50 }],
            actor: "ip".into(),
            actions: materialized[1].actions.clone(),
            probs: probs1,
            ev_chips: ev1,
            available: avail1,
        },
    ];
    let sol = proto::worker::StreetSolution {
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        requested: 0,
        exploitability_chips: 0.1,
        iterations: 10,
        memory_bytes: 10,
        mode: "f32".into(),
        locks_applied: 0,
        export: "street".into(),
    };
    (tree, sol)
}

#[test]
fn normalize_converts_chip_path_to_ordinal_and_scales_ev_by_pot() {
    let (tree, sol) = simple_tree_and_solution();
    let out = cache::entry::normalize(&sol, &tree, 100).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].path, Vec::<u8>::new(), "root chip path [] must resolve to ordinal path []");
    assert_eq!(out[0].actor, "oop");
    assert_eq!(out[1].path, vec![1u8], "Bet{{to:50}} at the root must resolve to ordinal [1]");
    assert_eq!(out[1].actor, "ip");
    // support::rows populates exactly the AhAd combo (index 1274) with ev [0.0, 10.0] chips;
    // normalize must divide by the pot (100) to store the pot-relative fraction.
    assert_eq!(out[0].ev_over_P[1274], vec![0.0, 0.1]);
    assert_eq!(out[0].probs[1274], vec![0.5, 0.5]);
    assert!(out[0].available[1274]);
    assert!(out[0].available.iter().enumerate().all(|(i, a)| *a == (i == 1274)));
}

#[test]
fn normalize_rejects_zero_pot() {
    let (tree, sol) = simple_tree_and_solution();
    let err = cache::entry::normalize(&sol, &tree, 0).unwrap_err();
    assert!(matches!(err, CacheError::Invalid("zero pot")));
}

/// `normalize` must not re-implement `proto::worker::validate_solution`'s structural checks --
/// it delegates and maps any failure to `CacheError::Invalid`. An empty node list is one such
/// failure (`validate_solution` rejects it outright).
#[test]
fn normalize_propagates_upstream_validate_solution_errors() {
    let (tree, mut sol) = simple_tree_and_solution();
    sol.nodes.clear();
    sol.covered_paths.clear();
    let err = cache::entry::normalize(&sol, &tree, 100).unwrap_err();
    assert!(matches!(err, CacheError::Invalid("invalid solution")));
}

// --- validate_entry ---------------------------------------------------------------------------

fn mutate(f: impl FnOnce(&mut CacheEntry)) -> CacheEntry {
    let mut e = support::entry();
    f(&mut e);
    e
}

#[test]
fn support_entry_fixture_is_itself_valid() {
    assert!(validate_entry(&support::entry()).is_ok());
}

#[test]
fn validate_entry_rejects_zero_financial_inputs() {
    assert!(validate_entry(&mutate(|e| e.source.pot = 0)).is_err(), "pot == 0");
    assert!(validate_entry(&mutate(|e| e.source.stack_oop = 0)).is_err(), "stack_oop == 0");
    assert!(validate_entry(&mutate(|e| e.source.stack_ip = 0)).is_err(), "stack_ip == 0");
    assert!(validate_entry(&mutate(|e| e.source.bb_chips = 0)).is_err(), "bb_chips == 0");
}

#[test]
fn validate_entry_rejects_chip_totals_at_or_above_2_31() {
    let e = mutate(|e| e.source.stack_oop = u32::MAX);
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_wrong_schema_rules_adapter_or_solver_versions() {
    assert!(validate_entry(&mutate(|e| e.key.schema_version = 2)).is_err(), "schema_version");
    assert!(validate_entry(&mutate(|e| e.key.rules_version = 2)).is_err(), "key.rules_version");
    assert!(validate_entry(&mutate(|e| e.tree.rules_version = 2)).is_err(), "tree.rules_version");
    assert!(validate_entry(&mutate(|e| e.key.adapter_version = 99)).is_err(), "adapter_version");
    assert!(validate_entry(&mutate(|e| e.key.solver_commit = "wrong".into())).is_err(), "solver_commit");
}

#[test]
fn validate_entry_rejects_non_flop_turn_root_street() {
    let e = mutate(|e| e.key.root_street = Street::River);
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_key_tree_root_street_disagreement() {
    // Turn is itself an allowed root street, so this isolates the key/tree agreement check
    // from the "must be Flop or Turn" membership check.
    let e = mutate(|e| e.key.root_street = Street::Turn);
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_bad_exploitability() {
    assert!(validate_entry(&mutate(|e| e.exploitability_over_P = f64::NAN)).is_err(), "NaN");
    assert!(validate_entry(&mutate(|e| e.exploitability_over_P = -0.001)).is_err(), "negative");
}

#[test]
fn validate_entry_rejects_unknown_mode() {
    let e = mutate(|e| e.mode = "i8".into());
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_empty_node_list() {
    let e = mutate(|e| {
        e.nodes.clear();
        e.covered_paths.clear();
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_wrong_spr_bucket_or_rake_cap() {
    assert!(
        validate_entry(&mutate(|e| e.source.spr = Rational::new(6, 1).unwrap())).is_err(),
        "spr does not match min(stack_oop, stack_ip) / pot"
    );
    assert!(validate_entry(&mutate(|e| e.key.spr_bucket = 999)).is_err(), "spr_bucket does not match spr_bucket(spr)");
    assert!(
        validate_entry(&mutate(|e| e.key.rake = RakeKey::new(0.05, Rational::new(1, 10).unwrap(), 1).unwrap())).is_err(),
        "rake.cap_over_p does not match cap_mchips / (1000 * pot)"
    );
}

#[test]
fn validate_entry_rejects_unsorted_materialized() {
    // review m2 / brief step 5b: swapping two materialized nodes must fail `validate_entry`.
    let e = mutate(|e| e.tree.materialized.swap(0, 1));
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_empty_materialized() {
    let e = mutate(|e| e.tree.materialized.clear());
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_wrong_board_length() {
    let e = mutate(|e| e.key.canonical_board.push(proto::Card(10)));
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_invalid_or_duplicate_board_cards() {
    assert!(validate_entry(&mutate(|e| e.key.canonical_board[0] = proto::Card(52))).is_err(), "card id >= 52");
    assert!(
        validate_entry(&mutate(|e| {
            let dup = e.key.canonical_board[1];
            e.key.canonical_board[0] = dup;
        }))
        .is_err(),
        "duplicate card id"
    );
}

#[test]
fn validate_entry_rejects_zero_quantum_over_p() {
    let e = mutate(|e| e.source.quantum_over_p = Rational::new(0, 100).unwrap());
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_unknown_export() {
    let e = mutate(|e| e.export = "bogus".into());
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_locks_applied_under_baseline_model() {
    let e = mutate(|e| {
        e.key.model = Model::Baseline;
        e.locks_applied = 1;
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_fractions_mismatch() {
    let e = mutate(|e| {
        e.fractions.pop();
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_out_of_domain_range_weight() {
    let e = mutate(|e| {
        let i = e.source.ranges[0].0.iter().position(|x| *x > 0.0).unwrap();
        e.source.ranges[0].0[i] = 2.0;
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_zero_mass_range() {
    // Recompute the stored hash to match the zeroed range so this test isolates the
    // "positive mass" check from the (also true) hash-mismatch check.
    let e = mutate(|e| {
        let zero = proto::Range1326::zero();
        e.key.range_hash_oop = core_ranges::hash_scaled(&zero);
        e.source.ranges[0] = zero;
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_range_hash_mismatch() {
    let e = mutate(|e| e.key.range_hash_oop = [0xFFu8; 32]);
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_range_weight_on_blocked_combo() {
    let e = mutate(|e| {
        let board = e.key.canonical_board.clone();
        let other = (0..52u8).map(proto::Card).find(|c| *c != board[0]).unwrap();
        let idx = proto::combo_index(board[0], other);
        e.source.ranges[0].0[idx as usize] = 1.0;
        e.key.range_hash_oop = core_ranges::hash_scaled(&e.source.ranges[0]);
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_noncanonical_board() {
    // A board that is a valid, distinct, non-duplicate 3-card board but was never actually
    // produced by canonicalizing (board, ranges) together -- `key.canonical_board` no longer
    // agrees with a fresh `core_iso::canonicalize` of itself.
    let e = mutate(|e| {
        e.key.canonical_board = vec![proto::Card(1), proto::Card(2), proto::Card(3)];
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_covered_paths_mismatch() {
    let e = mutate(|e| {
        e.covered_paths[0] = vec![255];
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_duplicate_covered_paths() {
    // Point nodes[1] at nodes[0]'s path *and* actor (both are the root's real owner) so only
    // the uniqueness check -- not the unrelated actor-agreement check -- can catch the
    // duplicate; verified by temporarily removing the uniqueness clause during review (it made
    // this test the only one of the suite to flip green while every other test stayed red).
    let e = mutate(|e| {
        let path = e.nodes[0].path.clone();
        let actor = e.nodes[0].actor.clone();
        e.nodes[1].path = path.clone();
        e.nodes[1].actor = actor;
        e.covered_paths[1] = path;
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_node_path_absent_from_tree() {
    let e = mutate(|e| {
        e.nodes[0].path = vec![255];
        e.covered_paths[0] = vec![255];
    });
    assert!(validate_entry(&e).is_err());
}

/// Note: the early `m.actor != n.actor` check this exercises is also caught, redundantly, by
/// the final reconstructed `validate_solution` pass below (its own `resolve_node` compares the
/// same two actor strings) -- confirmed by temporarily disabling the early check and observing
/// this test stay red. The `m.street != e.key.root_street` half of the same `if`, by contrast,
/// is load-bearing: `NodeStrategy` carries no street field, so nothing else in the function
/// could catch a street mismatch (see the next test).
#[test]
fn validate_entry_rejects_node_actor_disagreeing_with_tree() {
    let e = mutate(|e| e.nodes[0].actor = "not-a-real-actor".into());
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_node_street_disagreeing_with_root_street() {
    let e = mutate(|e| {
        let path = e.nodes[0].path.clone();
        let m = e.tree.materialized.iter_mut().find(|m| m.path == path).unwrap();
        m.street = Street::Turn;
    });
    assert!(validate_entry(&e).is_err());
}

#[test]
fn validate_entry_rejects_reconstructed_solution_failing_validate_solution() {
    // Break the probability row sum for the populated combo: the reconstructed
    // `StreetSolution` must fail `proto::worker::validate_solution`.
    let e = mutate(|e| {
        e.nodes[0].probs[1274] = vec![0.1, 0.1];
    });
    assert!(validate_entry(&e).is_err());
}
