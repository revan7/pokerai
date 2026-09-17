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
