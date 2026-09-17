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
