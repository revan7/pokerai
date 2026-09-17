use proto::worker::*;
use proto::*;

fn tree() -> Vec<MaterializedNode> {
    serde_json::from_str(r#"[{"path":[],"street":"river","actor":"oop","actions":[{"kind":"check"}],"terminal_pots":[null]},{"path":[0],"street":"river","actor":"ip","actions":[{"kind":"check"},{"kind":"allin","to":100}],"terminal_pots":[100,null]},{"path":[0,1],"street":"river","actor":"oop","actions":[{"kind":"fold"},{"kind":"call"}],"terminal_pots":[100,300]}]"#).unwrap()
}

fn node(path: Vec<Action>, actor: &str, actions: Vec<Action>) -> NodeStrategy {
    let a = actions.len();
    let mut probs = vec![vec![1.0 / a as f32; a]; 1326];
    let mut ev = vec![vec![1.5; a]; 1326];
    let mut available = vec![true; 1326];
    for c in (0..1326).step_by(5) { available[c] = false; probs[c] = vec![0.0; a]; ev[c] = vec![0.0; a]; }
    // Spec section 2 / T8-R1: fold EV is exactly 0.0 (chips already in the pot are sunk), never
    // the fixture's generic 1.5 filler, on every row (available or not).
    for (i, act) in actions.iter().enumerate() {
        if *act == Action::Fold {
            for row in ev.iter_mut() { row[i] = 0.0; }
        }
    }
    NodeStrategy { path, actor: actor.into(), actions, probs, ev_chips: ev, available }
}

fn solution() -> StreetSolution {
    StreetSolution { nodes: vec![node(vec![Action::Check], "ip", vec![Action::Check, Action::AllIn { to: 100 }]), node(vec![Action::Check, Action::AllIn { to: 100 }], "oop", vec![Action::Fold, Action::Call])], requested: 0, exploitability_chips: 0.09, iterations: 50, memory_bytes: 1, mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths: vec![vec![Action::Check], vec![Action::Check, Action::AllIn { to: 100 }]] }
}

#[test]
fn valid_solution_resolves_ordinal_paths() {
    assert_eq!(validate_solution(&solution(), &tree()).unwrap(), vec![vec![0u8], vec![0, 1]]);
}

#[test]
fn validate_solution_rejects_negative_and_above_one() {
    let mut s = solution();
    s.nodes[0].probs[7] = vec![-0.1, 1.1]; // sums to 1 but leaves [0, 1]
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("outside [0, 1]"), "{err}");
    let mut s = solution();
    s.nodes[1].probs[8] = vec![1.1, -0.1];
    assert!(validate_solution(&s, &tree()).is_err());
}

#[test]
fn validate_solution_row_rules() {
    let mut s = solution();
    s.nodes[0].probs[3] = vec![0.2, 0.2];
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("sums to"));
    let mut s = solution();
    s.nodes[0].probs[0] = vec![0.5, 0.5]; // combo 0 is unavailable
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("unavailable"));
    let mut s = solution();
    s.nodes[0].ev_chips[0] = vec![0.0, 1.0];
    assert!(validate_solution(&s, &tree()).is_err());
    let mut s = solution();
    s.nodes[0].ev_chips[9][1] = f32::NAN;
    assert!(validate_solution(&s, &tree()).is_err());
    let mut s = solution();
    s.nodes[0].probs.pop();
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("1326"));
    let mut s = solution();
    s.nodes[0].probs[4] = vec![1.0];
    assert!(validate_solution(&s, &tree()).is_err());
}

#[test]
fn validate_solution_structure_rules() {
    let mut s = solution();
    s.requested = 2;
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("requested"));
    let mut s = solution();
    s.covered_paths[1] = vec![Action::Check];
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("covered_paths"));
    let mut s = solution();
    s.nodes[1].path = vec![Action::Check, Action::Bet { to: 50 }];
    s.covered_paths[1] = s.nodes[1].path.clone();
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("resolve"));
    let mut s = solution();
    s.nodes[0].actor = "oop".into();
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("actor"));
    let mut s = solution();
    s.nodes[1].actions = vec![Action::Fold, Action::Check];
    assert!(validate_solution(&s, &tree()).is_err());
    let mut s = solution();
    s.mode = "f64".into();
    assert!(validate_solution(&s, &tree()).is_err());
}

#[test]
fn validate_locks_rules() {
    let mut probs = vec![vec![0.0, 0.0]; 1326];
    probs[10] = vec![0.3, 0.7];
    let lock = NodeLock { path: vec![Action::Check, Action::AllIn { to: 100 }], actor: "oop".into(), probs };
    assert_eq!(validate_locks(&[lock.clone()], &tree()).unwrap(), vec![vec![0u8, 1]]);
    let mut bad = lock.clone(); bad.probs[11] = vec![-0.1, 1.1];
    assert!(validate_locks(&[bad], &tree()).unwrap_err().contains("outside [0, 1]"));
    let mut bad = lock.clone(); bad.probs[11] = vec![0.2, 0.2];
    assert!(validate_locks(&[bad], &tree()).is_err());
    let mut bad = lock.clone(); bad.actor = "ip".into();
    assert!(validate_locks(&[bad], &tree()).is_err());
    let mut bad = lock; bad.probs.truncate(1325);
    assert!(validate_locks(&[bad], &tree()).is_err());
}

// --- T8-R1 fix round: fold EV must be exactly 0.0 (bit-exact) on available rows ---
// `solution().nodes[1]` is the "oop" node with actions [Fold, Call] (fold at action index 0).
// Combo 11 is available (`11 % 5 != 0`; see `node()`'s every-5th-combo unavailable pattern).

#[test]
fn validate_solution_rejects_positive_nonzero_fold_ev() {
    let mut s = solution();
    s.nodes[1].ev_chips[11][0] = 0.5;
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("fold"), "{err}");
}

#[test]
fn validate_solution_rejects_negative_nonzero_fold_ev() {
    let mut s = solution();
    s.nodes[1].ev_chips[11][0] = -0.5;
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("fold"), "{err}");
}

#[test]
fn validate_solution_rejects_negative_zero_fold_ev() {
    let mut s = solution();
    s.nodes[1].ev_chips[11][0] = -0.0f32; // same numeric value as 0.0 but a different bit pattern
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("fold"), "{err}");
}

#[test]
fn validate_solution_accepts_signed_finite_ev_for_non_fold_actions() {
    let mut s = solution();
    s.nodes[1].ev_chips[11][1] = -37.25; // action 1 of node 1 is Call, not Fold: any signed finite value is fine
    assert!(validate_solution(&s, &tree()).is_ok());
}

// --- T8-R2 fix round: one focused case per rule, asserting the specific error text so a
// different guard cannot mask a missing one. Combo 11 is used throughout (available; see above).

#[test]
fn validate_solution_rejects_upper_bound_alone() {
    let mut s = solution();
    s.nodes[0].probs[11] = vec![1.0001, 0.0]; // sum 1.0001 is within row-sum tolerance; only the entry itself is out of [0, 1]
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("outside [0, 1]"), "{err}");
}

#[test]
fn validate_locks_rejects_upper_bound_alone() {
    let mut probs = vec![vec![0.0, 0.0]; 1326];
    probs[11] = vec![1.0001, 0.0];
    let lock = NodeLock { path: vec![Action::Check, Action::AllIn { to: 100 }], actor: "oop".into(), probs };
    let err = validate_locks(&[lock], &tree()).unwrap_err();
    assert!(err.contains("outside [0, 1]"), "{err}");
}

#[test]
fn validate_solution_rejects_lower_bound_alone() {
    let mut s = solution();
    s.nodes[0].probs[11] = vec![-0.0001, 0.0]; // only the low bound is violated
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("outside [0, 1]"), "{err}");
}

#[test]
fn validate_locks_rejects_lower_bound_alone() {
    let mut probs = vec![vec![0.0, 0.0]; 1326];
    probs[11] = vec![-0.0001, 0.0];
    let lock = NodeLock { path: vec![Action::Check, Action::AllIn { to: 100 }], actor: "oop".into(), probs };
    let err = validate_locks(&[lock], &tree()).unwrap_err();
    assert!(err.contains("outside [0, 1]"), "{err}");
}

#[test]
fn validate_solution_rejects_non_finite_probabilities() {
    let mut s = solution();
    s.nodes[0].probs[11] = vec![f32::NAN, 0.0];
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("non-finite"));
    let mut s = solution();
    s.nodes[0].probs[11] = vec![f32::INFINITY, 0.0];
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("non-finite"));
    let mut s = solution();
    s.nodes[0].probs[11] = vec![f32::NEG_INFINITY, 0.0];
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("non-finite"));
}

#[test]
fn validate_solution_rejects_unknown_mode() {
    let mut s = solution();
    s.mode = "f64".into();
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("mode"), "{err}");
}

#[test]
fn validate_solution_rejects_unknown_export() {
    let mut s = solution();
    s.export = "partial".into();
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("export"), "{err}");
}

#[test]
fn validate_solution_rejects_negative_exploitability() {
    let mut s = solution();
    s.exploitability_chips = -1.0;
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("exploitability_chips"), "{err}");
}

#[test]
fn validate_solution_rejects_nan_exploitability() {
    let mut s = solution();
    s.exploitability_chips = f32::NAN;
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("exploitability_chips"), "{err}");
}

#[test]
fn validate_solution_rejects_node_count_over_the_cap() {
    // Cheap fixture: the node-count guard fires before any per-node field is inspected, so every
    // element can be a minimal, allocation-free `NodeStrategy` rather than a full 1326-row node.
    let cheap = NodeStrategy { path: vec![], actor: String::new(), actions: vec![], probs: vec![], ev_chips: vec![], available: vec![] };
    let s = StreetSolution {
        nodes: vec![cheap; MAX_EXPORTED_NODES + 1],
        requested: 0, exploitability_chips: 0.0, iterations: 0, memory_bytes: 0,
        mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths: vec![],
    };
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains(&MAX_EXPORTED_NODES.to_string()), "{err}");
}

#[test]
fn validate_solution_row_sum_tolerance_boundary() {
    let mut s = solution();
    s.nodes[0].probs[11] = vec![1.0, 0.001]; // sum = 1 + 1e-3: exactly at tolerance, accepted
    assert!(validate_solution(&s, &tree()).is_ok());
    let mut s = solution();
    s.nodes[0].probs[11] = vec![1.0, 0.0015]; // sum = 1 + 1.5e-3: over tolerance, rejected
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("sums to"), "{err}");
}

#[test]
fn validate_solution_rejects_ev_shape_mismatch_independent_of_probs() {
    let mut s = solution();
    s.nodes[0].ev_chips[11] = vec![1.5]; // wrong width; probs[11] is untouched and still valid
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("ev row"), "{err}");
}

#[test]
fn validate_locks_rejects_row_that_is_neither_zero_nor_summing_to_one() {
    let mut probs = vec![vec![0.0, 0.0]; 1326];
    probs[11] = vec![0.2, 0.2]; // neither the free-combo all-zero row nor summing to 1
    let lock = NodeLock { path: vec![Action::Check, Action::AllIn { to: 100 }], actor: "oop".into(), probs };
    let err = validate_locks(&[lock], &tree()).unwrap_err();
    assert!(err.contains("sums to"), "{err}");
}

#[test]
fn validate_solution_rejects_actions_mismatch_with_content() {
    let mut s = solution();
    s.nodes[1].actions = vec![Action::Fold, Action::Check]; // materialized node wants [Fold, Call]
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("actions"), "{err}");
}

// --- S2 (final review): validation must be linear in the materialized tree, not quadratic ---
//
// `resolve_chip_path` rebuilds an index of the whole materialized tree on every call, and
// `validate_solution` / `validate_locks` called it once per node and then scanned the tree again
// linearly, so validation cost Theta(nodes x materialized) against a declared cap of
// `MAX_EXPORTED_NODES = 100_000`. Both entry points now build one index per call.

use std::time::{Duration, Instant};

/// A wide, shallow materialized tree of `1 + roots * (1 + kids)` nodes: a root with `roots`
/// continuation actions, one child per action, and `kids` grandchildren under each child. Ordinal
/// paths are at most two bytes, so every resolution is two edges and what the timings below measure
/// is index construction, not path length.
fn wide_tree(roots: u8, kids: u8) -> Vec<MaterializedNode> {
    let act = |i: u8| Action::Bet { to: 1 + i as u32 };
    let mut out = vec![MaterializedNode {
        path: vec![],
        street: Street::Flop,
        actor: "oop".into(),
        actions: (0..roots).map(act).collect(),
        terminal_pots: vec![None; roots as usize],
    }];
    for i in 0..roots {
        out.push(MaterializedNode {
            path: vec![i],
            street: Street::Flop,
            actor: "ip".into(),
            actions: (0..kids).map(act).collect(),
            terminal_pots: vec![None; kids as usize],
        });
        for j in 0..kids {
            out.push(MaterializedNode { path: vec![i, j], street: Street::Flop, actor: "oop".into(), actions: vec![], terminal_pots: vec![] });
        }
    }
    out
}

/// Every grandchild's chip path, in materialized order.
fn grandchild_paths(roots: u8, kids: u8) -> Vec<Vec<Action>> {
    let act = |i: u8| Action::Bet { to: 1 + i as u32 };
    (0..roots).flat_map(|i| (0..kids).map(move |j| vec![act(i), act(j)])).collect()
}

/// S2: 20,000 resolutions against a 20,101-node materialized tree share one index and finish in
/// well under a second. Before the fix there was no way to share it -- every call rebuilt the whole
/// index -- and the review measured 3.9987096 s for this shape in a **release** build.
#[test]
fn twenty_thousand_paths_resolve_against_one_shared_index() {
    let materialized = wide_tree(100, 200);
    assert_eq!(materialized.len(), 20_101);
    let paths = grandchild_paths(100, 200);
    assert_eq!(paths.len(), 20_000);
    let index = index_materialized(&materialized);
    let start = Instant::now();
    let mut resolved = 0usize;
    for (k, path) in paths.iter().enumerate() {
        let ordinal = resolve_chip_path_indexed(&index, path).expect("every grandchild path resolves");
        assert_eq!(ordinal, vec![(k / 200) as u8, (k % 200) as u8]);
        resolved += 1;
    }
    let elapsed = start.elapsed();
    assert_eq!(resolved, 20_000);
    assert!(elapsed < Duration::from_secs(2), "20,000 resolutions took {elapsed:?}; the shared index must make this linear");
    // The wrapper keeps its old signature and its old answers, index or no index.
    for path in paths.iter().take(8).chain(paths.iter().rev().take(8)) {
        assert_eq!(resolve_chip_path(&materialized, path), resolve_chip_path_indexed(&index, path));
    }
    assert_eq!(resolve_chip_path_indexed(&index, &[Action::Check]), None, "an action outside the menu still does not resolve");
}

/// S2, at the public entry point plan 2 and plan 4 actually call: `validate_locks`' cost must be
/// driven by the locks it is given, not by the size of the tree they are resolved against. The same
/// 600 locks are validated against a 2,011-node tree and against a 20,101-node tree -- ten times
/// larger. One index per call makes the two times essentially equal (the extra 18,090 index entries
/// are dwarfed by the 795,600 probability rows both calls check); one index per lock made the
/// second call ten times the first.
#[test]
fn validate_locks_cost_does_not_grow_with_the_materialized_tree() {
    let small = wide_tree(10, 200);
    let big = wide_tree(100, 200);
    assert_eq!((small.len(), big.len()), (2_011, 20_101));
    // Each grandchild is a leaf with an empty menu, so every lock row is the empty (all-zero) row.
    let locks: Vec<NodeLock> = grandchild_paths(10, 200).iter().take(600)
        .map(|p| NodeLock { path: p.clone(), actor: "oop".into(), probs: vec![Vec::new(); 1326] })
        .collect();

    let start = Instant::now();
    let small_ordinals = validate_locks(&locks, &small).expect("every lock resolves against the small tree");
    let small_elapsed = start.elapsed();

    let start = Instant::now();
    let big_ordinals = validate_locks(&locks, &big).expect("every lock resolves against the large tree");
    let big_elapsed = start.elapsed();

    assert_eq!(small_ordinals, big_ordinals, "the same locks resolve to the same ordinals in both trees");
    assert_eq!(small_ordinals.len(), 600);
    assert_eq!((small_ordinals[0].clone(), small_ordinals[599].clone()), (vec![0u8, 0], vec![2u8, 199]));
    assert!(big_elapsed < Duration::from_secs(2), "validating 600 locks against 20,101 nodes took {big_elapsed:?}");
    // 3x, not 10x: the slack absorbs timer noise on a small absolute measurement while still
    // failing a per-lock index rebuild, which scales with the tenfold tree.
    assert!(
        big_elapsed < small_elapsed * 3 + Duration::from_millis(50),
        "a tenfold materialized tree changed validate_locks from {small_elapsed:?} to {big_elapsed:?}: the index is being rebuilt per lock"
    );
}
