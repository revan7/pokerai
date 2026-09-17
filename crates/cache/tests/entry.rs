mod support;

use cache::entry::{validate_entry, CacheEntry, CachedNode};
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
    // R3 (fix round 1): the child must be populated at a *different* combo, with *different*
    // probabilities and a *different* non-fold EV than the root -- support::rows alone would
    // give both nodes the identical single-combo payload (AhAd, [0.5,0.5], [0.0,10.0]), which
    // cannot distinguish a correct per-node normalize from a regression that aliases every
    // node's payload to the root's.
    let child_combo = 0usize; // 2c2d (combo_index 0): distinct from the root's AhAd (1274).
    let mut probs1 = vec![vec![0.0; 2]; 1326];
    let mut ev1 = vec![vec![0.0; 2]; 1326];
    let mut avail1 = vec![false; 1326];
    avail1[child_combo] = true;
    probs1[child_combo] = vec![0.3, 0.7];
    // Action 0 at the `ip` node is Fold: spec section 2 requires EV(fold) == 0 exactly.
    ev1[child_combo] = vec![0.0, 25.0];
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

    // Root (`oop`, ordinal path []): populated at AhAd (combo 1274) by support::rows.
    assert_eq!(out[0].path, Vec::<u8>::new(), "root chip path [] must resolve to ordinal path []");
    assert_eq!(out[0].actor, "oop");
    assert_eq!(out[0].probs[1274], vec![0.5, 0.5]);
    // support::rows populates AhAd with ev [0.0, 10.0] chips; normalize must divide by the
    // pot (100) to store the pot-relative fraction.
    assert_eq!(out[0].ev_over_P[1274], vec![0.0, 0.1]);
    assert!(out[0].available[1274]);
    assert_eq!(out[0].probs[0], vec![0.0, 0.0], "root's unpopulated 2c2d combo must be a zero row");
    assert_eq!(out[0].ev_over_P[0], vec![0.0, 0.0]);
    assert!(!out[0].available[0]);
    assert!(out[0].available.iter().enumerate().all(|(i, a)| *a == (i == 1274)));

    // Child (`ip`, ordinal path [1]): populated at 2c2d (combo 0) -- deliberately a *different*
    // combo with *different* values from the root (R3, fix round 1), so this test cannot pass
    // if normalize aliases the child's payload to the root's.
    assert_eq!(out[1].path, vec![1u8], "Bet{{to:50}} at the root must resolve to ordinal [1]");
    assert_eq!(out[1].actor, "ip");
    assert_eq!(out[1].probs[0], vec![0.3, 0.7]);
    // The child's EV was set to [0.0, 25.0] chips; normalize must divide by the pot (100),
    // producing a pot-normalized fraction distinct from the root's.
    assert_eq!(out[1].ev_over_P[0], vec![0.0, 0.25]);
    assert!(out[1].available[0]);
    assert_eq!(out[1].probs[1274], vec![0.0, 0.0], "child's unpopulated AhAd combo must be a zero row");
    assert_eq!(out[1].ev_over_P[1274], vec![0.0, 0.0]);
    assert!(!out[1].available[1274]);
    assert!(out[1].available.iter().enumerate().all(|(i, a)| *a == (i == 0)));
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

// --- R1 (fix round 1): CachedNode's probs/ev_over_P matrices admit wide values -----------------

/// A JSON probability row that narrows to the in-domain `[1.0, -0.0]` but whose wide (f64)
/// values are out of domain -- the exact row the Codex review's probe demonstrated decoding
/// successfully (and then passing `validate_entry`) before the R1 fix.
#[test]
fn cached_node_probs_reject_invalid_wide_values_before_narrowing() {
    let json = r#"{"path":[],"actor":"oop","probs":[[1.00000001,-1e-50]],"ev_over_P":[[0.0,0.0]],"available":[true]}"#;
    assert!(serde_json::from_str::<CachedNode>(json).is_err());
}

#[test]
fn cached_node_ev_over_p_rejects_non_finite_values_on_decode() {
    for bad in ["1e999", "-1e999", "1e39", "-1e39"] {
        let json = format!(r#"{{"path":[],"actor":"oop","probs":[[0.5,0.5]],"ev_over_P":[[{bad},0.0]],"available":[true]}}"#);
        assert!(serde_json::from_str::<CachedNode>(&json).is_err(), "ev_over_P {bad} must be rejected");
    }
}

#[test]
fn cached_node_serialize_rejects_non_finite_or_out_of_domain_in_memory_values() {
    let nan_ev = CachedNode {
        path: vec![],
        actor: "oop".into(),
        probs: vec![vec![0.5, 0.5]],
        ev_over_P: vec![vec![f32::NAN, 0.0]],
        available: vec![true],
    };
    assert!(serde_json::to_string(&nan_ev).is_err(), "NaN ev_over_P must not silently serialize");

    let out_of_domain_prob = CachedNode {
        path: vec![],
        actor: "oop".into(),
        probs: vec![vec![1.5, -0.1]],
        ev_over_P: vec![vec![0.0, 0.0]],
        available: vec![true],
    };
    assert!(serde_json::to_string(&out_of_domain_prob).is_err(), "out-of-[0,1] probs must not silently serialize");
}

/// The Codex review's exact reproduction (R1 evidence): a full, otherwise-valid `CacheEntry`
/// JSON with one populated probability row corrupted to `[1.00000001, -1e-50]` must fail to
/// decode at all -- not decode successfully (narrowing to `[1.0, -0.0]`) and then separately
/// pass or fail `validate_entry`.
#[test]
fn cache_entry_json_decode_rejects_invalid_wide_probabilities_row() {
    let text = serde_json::to_string(&support::entry()).unwrap();
    assert!(text.contains("[0.5,0.5]"), "fixture must contain a populated probability row to corrupt");
    let corrupted = text.replacen("[0.5,0.5]", "[1.00000001,-1e-50]", 1);
    assert!(serde_json::from_str::<CacheEntry>(&corrupted).is_err());
}

/// A valid entry -- matrices included -- round-trips through JSON unchanged and stays valid.
#[test]
fn cache_entry_with_valid_matrices_round_trips_through_json() {
    let e = support::entry();
    let text = serde_json::to_string(&e).unwrap();
    let back: CacheEntry = serde_json::from_str(&text).unwrap();
    assert!(validate_entry(&back).is_ok());
    assert_eq!(back.nodes.len(), e.nodes.len());
    for (a, b) in e.nodes.iter().zip(back.nodes.iter()) {
        assert_eq!(a.path, b.path);
        assert_eq!(a.actor, b.actor);
        assert_eq!(a.probs, b.probs);
        assert_eq!(a.ev_over_P, b.ev_over_P);
        assert_eq!(a.available, b.available);
    }
}

// --- R2 (fix round 1): canonical range identity is bit-exact ---------------------------------

/// Reproduces the Codex review's exact probe. With canonical board `[Card(0), Card(20),
/// Card(44)]` (2c, 7c, Kc -- three same-suit cards, giving the board a nontrivial stabilizer)
/// and OOP range index 41 set to `-0.0`, `canonicalize`'s tie-break selects the non-identity
/// `SuitPerm([0, 3, 1, 2])`. The permuted range is numerically equal to the stored range
/// (ordinary float `==` treats `+0.0 == -0.0`) but has a different `hash_scaled` digest, so
/// `validate_entry` must reject it, not accept it.
#[test]
fn validate_entry_rejects_asymmetric_stabilizer_signed_zero_regression() {
    let e = mutate(|e| {
        e.key.canonical_board = vec![proto::Card(0), proto::Card(20), proto::Card(44)];
        e.source.ranges[0].0[41] = -0.0f32;
        e.key.range_hash_oop = core_ranges::hash_scaled(&e.source.ranges[0]);
    });
    assert!(validate_entry(&e).is_err());
}

// --- N1 (fix round 2): bincode wire uses native f32, JSON keeps the f64 wide-check ------------
//
// This fix round scoped the bincode round-trip tests below to `CachedNode` alone, because
// `CacheEntry` as a whole did not round-trip through bincode at all: `proto::Action` is
// `#[serde(tag = "kind", ...)]` (internally tagged), and serde's internally-tagged enum decoding
// requires `Deserializer::deserialize_any`, which bincode 1.3.3 unconditionally refuses
// (`DeserializeAnyNotSupported`) -- a pre-existing defect in `proto::Action`'s tagging, out of
// that fix round's scope (`crates/cache/src/entry.rs` and its tests only), and flagged for a
// future task.
//
// Task: P4.T2-followup fixed it, in `proto`: `Action` and `ApproxReason` (the two internally
// tagged enums reachable from `CacheEntry`) now carry hand-written `Serialize`/`Deserialize`
// that keep the JSON tagged-map form byte-for-byte and add a bincode-native encoding for every
// non-human-readable format (`crates/proto/src/hand.rs`, `crates/proto/src/recommendation.rs`).
// Two further, independent bincode incompatibilities reachable from `CacheEntry` were found and
// fixed alongside it, using the same `is_human_readable()` split already established here for
// `CachedNode`'s own matrices: `MenuSize`'s JSON union wire form used `deserialize_any` directly
// (`crates/proto/src/tree.rs`), and `Range1326`'s `Deserialize` always read a wide `f64`
// regardless of format, mismatching its own `Serialize`'s native `f32` write
// (`crates/proto/src/range.rs`); a third, `PlayerMenus.donk`'s
// `#[serde(skip_serializing_if = "Option::is_none")]` (`crates/proto/src/tree.rs`), omitted a
// struct field from the wire on every format, not only human-readable ones, desyncing bincode's
// positional field count. See `crates/proto/tests/bincode_wire.rs` for per-type coverage of all
// of these, and `cache_entry_round_trips_through_bincode_with_the_support_fixture` below for the
// acceptance test this whole chain blocked.

/// A `CachedNode` with valid, in-domain matrices round-trips through bincode unchanged.
#[test]
fn cached_node_round_trips_through_bincode_with_bit_identical_matrices() {
    let node = CachedNode {
        path: vec![1, 0, 1],
        actor: "ip".into(),
        probs: vec![vec![0.25, 0.75], vec![0.0, 0.0]],
        ev_over_P: vec![vec![0.0, 12.5], vec![-3.5, 0.0]],
        available: vec![true, false],
    };
    let bytes = bincode::serialize(&node).unwrap();
    let back: CachedNode = bincode::deserialize(&bytes).unwrap();
    assert_eq!(back.path, node.path);
    assert_eq!(back.actor, node.actor);
    assert_eq!(back.probs, node.probs);
    assert_eq!(back.ev_over_P, node.ev_over_P);
    assert_eq!(back.available, node.available);
}

/// Bincode is non-human-readable (spec 10.4: the on-disk cache storage format), so the codec
/// must write/read native `f32` (4 bytes/value), not `f64` (8 bytes/value). Isolate the
/// per-element wire width by comparing two otherwise-identical nodes whose *only* difference is
/// one extra value in a single `probs` row: the encoded-size delta is exactly that one value's
/// wire width, independent of any other framing bincode adds (which is identical between the
/// two encodings since nothing else about the shape changed).
#[test]
fn cached_node_bincode_wire_width_is_native_f32_not_f64() {
    let base = CachedNode {
        path: vec![],
        actor: "oop".into(),
        probs: vec![vec![0.0f32; 5]],
        ev_over_P: vec![vec![0.0f32; 1]],
        available: vec![true],
    };
    let mut grown = base.clone();
    grown.probs[0].push(0.5); // exactly one more f32 value, in the same, only, row
    let base_len = bincode::serialized_size(&base).unwrap();
    let grown_len = bincode::serialized_size(&grown).unwrap();
    assert_eq!(grown_len - base_len, 4, "one extra probability value must cost exactly 4 bytes (native f32) under bincode, not 8 (f64)");
}

/// Corrupting a *bincode-encoded* `CachedNode`'s bytes to a NaN bit pattern must be rejected on
/// decode -- bincode carries no wider source value to check before narrowing (there is no
/// narrowing step at all on this path), but the f32 actually read is still finiteness/domain
/// checked.
#[test]
fn cached_node_bincode_decode_rejects_a_crafted_nan_bit_pattern() {
    let sentinel = 12345.6789f32;
    let node = CachedNode {
        path: vec![],
        actor: "oop".into(),
        probs: vec![vec![0.5]],
        ev_over_P: vec![vec![sentinel]],
        available: vec![true],
    };
    let mut bytes = bincode::serialize(&node).unwrap();
    let needle = sentinel.to_le_bytes();
    let occurrences = bytes.windows(4).filter(|w| *w == needle).count();
    assert_eq!(occurrences, 1, "sentinel float must appear exactly once in the encoded bytes");
    let pos = bytes.windows(4).position(|w| w == needle).unwrap();
    bytes[pos..pos + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(bincode::deserialize::<CachedNode>(&bytes).is_err());
}

/// Task: P4.T2-followup's acceptance test: the exact reproduction the brief's origin (task-2
/// fix round 2's N1 comment above) named as the still-failing case -- a whole `CacheEntry`,
/// built from the same `support::entry()` fixture every other test in this file uses, round
/// tripping through bincode 1.3.3, the pinned cache storage format (spec 10.4). Before this
/// task's fix this panicked with `DeserializeAnyNotSupported` (from `proto::Action`'s internal
/// tag, reached first via `SourceInputs.ranges: [Range1326; 2]`'s own, independent bincode
/// defect surfacing as a bogus domain-check panic before `Action` was ever reached); with only
/// `Action`/`ApproxReason` fixed it instead panicked with `InvalidTagEncoding` (from
/// `PlayerMenus.donk`'s `skip_serializing_if` desyncing bincode's field count). All of it must
/// now round-trip byte-for-byte equal, field by field (the derived `CacheEntry` has no
/// `PartialEq`, so equality is asserted per top-level field rather than as one struct compare).
#[test]
fn cache_entry_round_trips_through_bincode_with_the_support_fixture() {
    let e = support::entry();
    let bytes = bincode::serialize(&e).expect("a valid CacheEntry must serialize through bincode");
    let back: CacheEntry = bincode::deserialize(&bytes).expect("a bincode-encoded CacheEntry must deserialize back");

    assert_eq!(back.key.digest(), e.key.digest(), "key");
    assert_eq!(back.source.pot, e.source.pot);
    assert_eq!(back.source.stack_oop, e.source.stack_oop);
    assert_eq!(back.source.stack_ip, e.source.stack_ip);
    assert_eq!(back.source.bb_chips, e.source.bb_chips);
    assert_eq!(back.source.cap_mchips, e.source.cap_mchips);
    assert!(back.source.ranges[0].0.iter().zip(e.source.ranges[0].0.iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "oop range, bit-exact");
    assert!(back.source.ranges[1].0.iter().zip(e.source.ranges[1].0.iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "ip range, bit-exact");
    assert_eq!(back.tree, e.tree, "tree (EffectiveTree derives PartialEq)");
    assert_eq!(back.fractions, e.fractions);
    assert_eq!(back.covered_paths, e.covered_paths);
    assert_eq!(back.nodes.len(), e.nodes.len());
    for (a, b) in e.nodes.iter().zip(back.nodes.iter()) {
        assert_eq!(a.path, b.path);
        assert_eq!(a.actor, b.actor);
        assert_eq!(a.probs, b.probs);
        assert_eq!(a.ev_over_P, b.ev_over_P);
        assert_eq!(a.available, b.available);
    }
    assert_eq!(back.exploitability_over_P, e.exploitability_over_P);
    assert_eq!(back.target_bp, e.target_bp);
    assert_eq!(back.iterations, e.iterations);
    assert_eq!(back.mode, e.mode);
    assert_eq!(back.export, e.export);
    assert_eq!(back.reasons, e.reasons);
    // The round-tripped entry must still pass every structural/numeric check `validate_entry`
    // runs against a freshly built entry -- not just look equal field by field.
    assert!(validate_entry(&back).is_ok(), "a bincode round trip must not produce an entry that fails validate_entry");
}

/// Review R1 (fix round 1, Task: P4.T2): the binary (non-human-readable) branch of
/// `Range1326`'s decoder must be a lossless persistence codec for an already-validated
/// in-memory value -- it must preserve signed-zero bits exactly, not run the signed-zero
/// normalization that belongs only to the human-readable ingestion boundary (S5's "one
/// ingestion boundary" ruling; `hash_scaled`/`apply_range`/`canonicalize` stay bit-exact per
/// T20/T21). `Serialize` never normalizes -0.0 on write (it only checks finiteness/domain), so
/// a decoder that normalizes on the binary read desyncs a valid entry's stored range hash from
/// its own decoded bits -- exactly the reviewer's reproduction: every previously-zero weight in
/// both ranges is set to `-0.0` (still domain-valid, `-0.0 == 0.0`), the stored hashes are
/// recomputed over those bits so the *unmodified* entry validates, and only the *binary*
/// round-trip must still validate afterward.
#[test]
fn cache_entry_bincode_preserves_signed_zero_range_bits_after_binary_round_trip() {
    let mut e = support::entry();
    for range in &mut e.source.ranges {
        for w in range.0.iter_mut() {
            if *w == 0.0 {
                *w = -0.0;
            }
        }
    }
    e.key.range_hash_oop = core_ranges::hash_scaled(&e.source.ranges[0]);
    e.key.range_hash_ip = core_ranges::hash_scaled(&e.source.ranges[1]);
    assert!(validate_entry(&e).is_ok(), "the modified-but-internally-consistent -0.0 entry must validate before any round trip");

    // Control: at least one signed zero actually exists in each range, or this test would not
    // be exercising the regression at all.
    for range in &e.source.ranges {
        assert!(range.0.iter().any(|w| w.is_sign_negative() && *w == 0.0), "fixture must contain a genuine -0.0 to be a meaningful regression");
    }

    let bytes = bincode::serialize(&e).expect("a -0.0-laden CacheEntry must still serialize through bincode");
    let back: CacheEntry = bincode::deserialize(&bytes).expect("a bincode-encoded CacheEntry must deserialize back");

    // Bit-exact: every weight's sign bit, not just its numeric value, must survive the binary
    // round trip (ordinary float `==` treats `+0.0` and `-0.0` as equal and would not catch this).
    for (range, back_range) in e.source.ranges.iter().zip(back.source.ranges.iter()) {
        assert!(range.0.iter().zip(back_range.0.iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "binary round trip must preserve every weight's bit pattern exactly, including sign");
    }
    // Stored hash agreement: the hash written into the key must still match a fresh hash of the
    // decoded range -- this is what `InvalidTagEncoding`-adjacent identity corruption would break.
    assert_eq!(core_ranges::hash_scaled(&back.source.ranges[0]), back.key.range_hash_oop, "decoded oop range must still hash to the stored key");
    assert_eq!(core_ranges::hash_scaled(&back.source.ranges[1]), back.key.range_hash_ip, "decoded ip range must still hash to the stored key");
    assert!(validate_entry(&back).is_ok(), "a valid -0.0-laden entry must still validate after a lossless binary round trip");
}
