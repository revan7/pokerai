use proto::worker::*;
use proto::*;

/// The two-combo river fixture of spec 4.5: OOP = the six AA combos at 1.0; IP = the three legal QQ combos on Qs Jd 7h 3c 2d at 1.0 and the twelve 54o combos at 0.25.
///
/// This function is the **definition** of that data (cross-plan §2 D7). Python cannot import it,
/// so plan 2's `tools/gen_worker_fixtures.py` re-states it and plan 2 owes a cross-check test that
/// `fixtures/worker/river_two_combo.jsonl` agrees with this function combo by combo.
pub fn river_two_combo_ranges() -> (Range1326, Range1326) {
    let card = |s: &str| s.parse::<Card>().unwrap();
    let mut oop = Range1326::zero();
    for c in class_combos(0) { oop.set(c, 1.0); } // AA
    let mut ip = Range1326::zero();
    let queens = ["Qc", "Qd", "Qh"];
    for a in 0..3 { for b in (a + 1)..3 { ip.set(combo_index(card(queens[a]), card(queens[b])), 1.0); } }
    for i in 0..1326u16 {
        let [lo, hi] = combo_cards(i);
        let ranks = (lo.rank(), hi.rank());
        if (ranks == (2, 3) || ranks == (3, 2)) && lo.suit() != hi.suit() { ip.set(i, 0.25); }
    }
    (oop, ip)
}

const READY: &str = r#"{"type":"ready","proto_version":3,"solver_commit":"9d1509fe5077d019825f833eed04b16d342dfda1","adapter_version":1,"threads":16,"build_features":["avx2"],"cpu_features":["avx2","fma"],"capabilities":["solve","lock","cancel","street_export","i16"]}"#;

#[test]
fn worker_identity_constants_match_the_ready_line() {
    let m: WorkerMessage = serde_json::from_str(READY).unwrap();
    let WorkerMessage::Ready(r) = m else { panic!("ready") };
    assert_eq!(r.proto_version, PROTO_VERSION);
    assert_eq!(r.solver_commit, SOLVER_COMMIT);
    assert_eq!(r.adapter_version, ADAPTER_VERSION);
    assert_eq!(SOLVER_COMMIT.len(), 40, "spec 3.7 pins a full sha1");
}

#[test]
fn ready_ack_cancel_shutdown_roundtrip_exactly() {
    let m: WorkerMessage = serde_json::from_str(READY).unwrap();
    assert_eq!(serde_json::to_string(&m).unwrap(), READY);
    for line in [r#"{"type":"ack","id":"41","status":"accepted"}"#, r#"{"type":"ack","id":"42","status":"already_finished"}"#, r#"{"type":"ack","id":"47","status":"staged","replaced":false}"#, r#"{"type":"ack","id":"9","status":"rejected","reason":"busy"}"#] {
        let m: WorkerMessage = serde_json::from_str(line).unwrap();
        assert_eq!(serde_json::to_string(&m).unwrap(), line);
    }
    for line in [r#"{"type":"cancel","id":"42","target":"41"}"#, r#"{"type":"shutdown","id":"48"}"#] {
        let m: EngineMessage = serde_json::from_str(line).unwrap();
        assert_eq!(serde_json::to_string(&m).unwrap(), line);
    }
    let p: WorkerMessage = serde_json::from_str(r#"{"type":"progress","id":"41","stage":"building","iterations":0,"exploitability_chips":null,"elapsed_ms":3,"memory_bytes":331776}"#).unwrap();
    assert!(matches!(p, WorkerMessage::Progress { exploitability_chips: None, stage: Stage::Building, .. }));
    assert!(serde_json::to_string(&p).unwrap().contains(r#""exploitability_chips":null"#));
    let e: WorkerMessage = serde_json::from_str(r#"{"type":"result","id":"49","status":"error","elapsed_ms":2,"error":{"code":"tree_too_large","message":"f32 estimate above limit","retryable":false,"estimate_bytes":9126805504}}"#).unwrap();
    match e { WorkerMessage::Result { status: ResultStatus::Error, error: Some(err), solution: None, .. } => assert_eq!(err.estimate_bytes, Some(9126805504)), other => panic!("{other:?}") }
}

#[test]
fn solve_and_result_roundtrip_with_full_vectors() {
    let (oop, ip) = river_two_combo_ranges();
    let tree: EffectiveTree = serde_json::from_str(r#"{"rules_version":3,"template_id":"river_oracle_v1","root_street":"river","menus":{"river":{"oop":{"bet":[],"raise":[]},"ip":{"bet":[1.0],"raise":[]}}},"add_allin_threshold":0.0,"force_allin_threshold":0.0,"merging_threshold":0.0,"wager_cap":1,"inserted":[],"materialized":[{"path":[],"street":"river","actor":"oop","actions":[{"kind":"check"}],"terminal_pots":[null]},{"path":[0],"street":"river","actor":"ip","actions":[{"kind":"check"},{"kind":"allin","to":100}],"terminal_pots":[100,null]},{"path":[0,1],"street":"river","actor":"oop","actions":[{"kind":"fold"},{"kind":"call"}],"terminal_pots":[100,300]}]}"#).unwrap();
    let board: Vec<Card> = ["Qs", "Jd", "7h", "3c", "2d"].iter().map(|s| s.parse().unwrap()).collect();
    let solve = EngineMessage::Solve(SolveRequest { id: "41".into(), spot: "3f9c".into(), board, oop_range: oop, ip_range: ip, pot: 100, stack_oop: 100, stack_ip: 100, rake_rate: 0.0, rake_cap_mchips: 0, tree: tree.clone(), history: vec![Action::Check], target_bp: 10, deadline_ms: 1500, extraction_margin_ms: 200, memory_limit_bytes: 10737418240, background: false });
    let line = serde_json::to_string(&solve).unwrap();
    assert!(line.starts_with(r#"{"type":"solve","id":"41","spot":"3f9c","board":["Qs","Jd","7h","3c","2d"],"oop_range":[1"#) || line.starts_with(r#"{"type":"solve","id":"41","spot":"3f9c","board":["Qs","Jd","7h","3c","2d"],"oop_range":[0"#));
    assert!(line.len() < REQUEST_LINE_MAX);
    let back: EngineMessage = serde_json::from_str(&line).unwrap();
    assert_eq!(back, solve);
    let node = |path: Vec<Action>, actor: &str, actions: Vec<Action>| NodeStrategy { path, actor: actor.into(), actions: actions.clone(), probs: vec![vec![1.0 / actions.len() as f32; actions.len()]; 1326], ev_chips: vec![vec![0.0; actions.len()]; 1326], available: vec![true; 1326] };
    let sol = StreetSolution { nodes: vec![node(vec![Action::Check], "ip", vec![Action::Check, Action::AllIn { to: 100 }]), node(vec![Action::Check, Action::AllIn { to: 100 }], "oop", vec![Action::Fold, Action::Call])], requested: 0, exploitability_chips: 0.09, iterations: 50, memory_bytes: 6914048, mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths: vec![vec![Action::Check], vec![Action::Check, Action::AllIn { to: 100 }]] };
    let result = WorkerMessage::Result { id: "41".into(), status: ResultStatus::Ok, elapsed_ms: 12, solution: Some(sol), error: None };
    let line = serde_json::to_string(&result).unwrap();
    assert!(line.starts_with(r#"{"type":"result","id":"41","status":"ok","elapsed_ms":12,"solution":{"nodes":[{"path":[{"kind":"check"}],"actor":"ip""#));
    assert!(!line.contains(r#""error""#));
    let back: WorkerMessage = serde_json::from_str(&line).unwrap();
    assert_eq!(back, result);
}

#[test]
fn structurally_invalid_lines_are_rejected() {
    assert!(serde_json::from_str::<EngineMessage>(r#"{"type":"nope","id":"1"}"#).is_err());
    assert!(serde_json::from_str::<EngineMessage>(r#"{"type":"cancel","id":"1","target":"2","extra":true}"#).is_err());
    assert!(serde_json::from_str::<WorkerMessage>(r#"{"type":"ack","id":"1","status":"maybe"}"#).is_err());
    assert!(serde_json::from_str::<WorkerMessage>(&READY.replace(r#""threads":16"#, r#""threads":16,"bunching":true"#)).is_err());
    let short = format!(r#"{{"type":"lock","id":"1","spot":"x","locks":[{{"path":[],"actor":"oop","probs":{}}}]}}"#, serde_json::to_string(&vec![vec![1.0f32]; 1325]).unwrap());
    let parsed: EngineMessage = serde_json::from_str(&short).unwrap();
    assert!(matches!(parsed, EngineMessage::Lock { .. }), "shape errors of lock matrices are validate_locks' job, not serde's");
    assert!(serde_json::from_str::<WorkerMessage>(r#"{"type":"progress","id":"1","stage":"solving","iterations":1,"exploitability_chips":1e999,"elapsed_ms":1,"memory_bytes":1}"#).is_err(), "non-finite numbers are rejected");
}

/// T7-R1: every f32 wire field must be validated in its wide f64 form before narrowing (a
/// value that merely rounds into a domain on narrowing, e.g. a probability `1.00000001`, or
/// that overflows to infinity on narrowing, e.g. `1e39`, must be rejected), and again on
/// serialize (a non-finite in-memory value must error, never silently become JSON `null`).
#[test]
fn wire_floats_are_validated_before_narrowing_and_on_serialize() {
    // Probabilities (NodeLock.probs / NodeStrategy.probs share the same [0,1] domain codec):
    // reject anything that would only round into range on narrowing, and overflow, accept the
    // closed endpoints and an ordinary in-domain value.
    let node_lock = |probs: &str| format!(r#"{{"path":[],"actor":"oop","probs":[[{probs}]]}}"#);
    for bad in ["1.00000001", "-1e-50", "1e39", "-1e39"] {
        assert!(serde_json::from_str::<NodeLock>(&node_lock(bad)).is_err(), "probability {bad} must be rejected");
    }
    for ok in ["0.0", "1.0", "0.5"] {
        assert!(serde_json::from_str::<NodeLock>(&node_lock(ok)).is_ok(), "probability {ok} must be accepted");
    }
    let bad_lock = NodeLock { path: vec![], actor: "oop".into(), probs: vec![vec![1.5]] };
    assert!(serde_json::to_string(&bad_lock).is_err(), "an out-of-domain in-memory probability must not serialize");

    // ev_chips: finite only, no [0,1] bound, but overflow-on-narrow must still be rejected.
    let node_strategy = |ev: &str| format!(r#"{{"path":[],"actor":"oop","actions":[],"probs":[],"ev_chips":[[{ev}]],"available":[]}}"#);
    assert!(serde_json::from_str::<NodeStrategy>(&node_strategy("1e39")).is_err(), "ev_chips overflow-on-narrow must be rejected");
    assert!(serde_json::from_str::<NodeStrategy>(&node_strategy("-123.5")).is_ok());

    // StreetSolution.exploitability_chips: scalar, finite only, always present.
    let street = |v: &str| format!(r#"{{"nodes":[],"requested":0,"exploitability_chips":{v},"iterations":0,"memory_bytes":0,"mode":"f32","locks_applied":0,"export":"street","covered_paths":[]}}"#);
    assert!(serde_json::from_str::<StreetSolution>(&street("1e39")).is_err());
    assert!(serde_json::from_str::<StreetSolution>(&street("12.5")).is_ok());
    let bad_street = StreetSolution { nodes: vec![], requested: 0, exploitability_chips: f32::NAN, iterations: 0, memory_bytes: 0, mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths: vec![] };
    assert!(serde_json::to_string(&bad_street).is_err(), "NaN exploitability_chips must not serialize");

    // rake_rate: half-open [0,1); build one valid SolveRequest, then vary rake_rate by text
    // substitution (deserialize direction) and by direct field mutation (serialize direction).
    let (oop, ip) = river_two_combo_ranges();
    let tree: EffectiveTree = serde_json::from_str(r#"{"rules_version":3,"template_id":"river_oracle_v1","root_street":"river","menus":{"river":{"oop":{"bet":[],"raise":[]},"ip":{"bet":[1.0],"raise":[]}}},"add_allin_threshold":0.0,"force_allin_threshold":0.0,"merging_threshold":0.0,"wager_cap":1,"inserted":[],"materialized":[{"path":[],"street":"river","actor":"oop","actions":[{"kind":"check"}],"terminal_pots":[null]}]}"#).unwrap();
    let board: Vec<Card> = ["Qs", "Jd", "7h", "3c", "2d"].iter().map(|s| s.parse().unwrap()).collect();
    let base = SolveRequest { id: "1".into(), spot: "x".into(), board, oop_range: oop, ip_range: ip, pot: 100, stack_oop: 100, stack_ip: 100, rake_rate: 0.0, rake_cap_mchips: 0, tree, history: vec![], target_bp: 10, deadline_ms: 1500, extraction_margin_ms: 200, memory_limit_bytes: 1, background: false };
    let line = serde_json::to_string(&EngineMessage::Solve(base.clone())).unwrap();
    assert!(line.contains(r#""rake_rate":0.0"#), "{line}");
    for bad in ["1.0", "1.5", "-0.1", "1e39"] {
        let replaced = line.replacen(r#""rake_rate":0.0"#, &format!(r#""rake_rate":{bad}"#), 1);
        assert!(serde_json::from_str::<EngineMessage>(&replaced).is_err(), "rake_rate {bad} must be rejected");
    }
    let ok_line = line.replacen(r#""rake_rate":0.0"#, r#""rake_rate":0.999"#, 1);
    assert!(serde_json::from_str::<EngineMessage>(&ok_line).is_ok());
    let mut nan_request = base;
    nan_request.rake_rate = f32::NAN;
    assert!(serde_json::to_string(&EngineMessage::Solve(nan_request)).is_err(), "NaN rake_rate must not serialize");

    // Progress.exploitability_chips: reject overflow-on-narrow, accept explicit null and a
    // finite measurement.
    let progress = |v: &str| format!(r#"{{"type":"progress","id":"1","stage":"solving","iterations":1,"exploitability_chips":{v},"elapsed_ms":1,"memory_bytes":1}}"#);
    assert!(serde_json::from_str::<WorkerMessage>(&progress("1e39")).is_err());
    assert!(serde_json::from_str::<WorkerMessage>(&progress("-1e39")).is_err());
    assert!(serde_json::from_str::<WorkerMessage>(&progress("null")).is_ok());
    assert!(serde_json::from_str::<WorkerMessage>(&progress("0.5")).is_ok());

    // Serialize-side rejection: an in-memory NaN/Infinity must error, never become `null`
    // (serde_json's default rendering of a non-finite float, indistinguishable from `None`).
    let nan_progress = WorkerMessage::Progress { id: "1".into(), stage: Stage::Solving, iterations: 1, exploitability_chips: Some(f32::NAN), elapsed_ms: 1, memory_bytes: 1 };
    assert!(serde_json::to_string(&nan_progress).is_err(), "NaN must not silently serialize as null");
    let inf_progress = WorkerMessage::Progress { id: "1".into(), stage: Stage::Solving, iterations: 1, exploitability_chips: Some(f32::INFINITY), elapsed_ms: 1, memory_bytes: 1 };
    assert!(serde_json::to_string(&inf_progress).is_err(), "infinity must not silently serialize as null");
}

/// T7-R2: `Progress.exploitability_chips` is required (always present) but nullable (spec 4.5:
/// `null` until measured). A plain `Option<f32>` would accept a missing key as `None`; the key
/// must be required, while its value may legitimately be `null`.
#[test]
fn progress_measurement_is_required_but_nullable() {
    let missing = r#"{"type":"progress","id":"1","stage":"solving","iterations":1,"elapsed_ms":1,"memory_bytes":1}"#;
    assert!(serde_json::from_str::<WorkerMessage>(missing).is_err(), "an omitted exploitability_chips key must be rejected");

    let explicit_null = r#"{"type":"progress","id":"1","stage":"solving","iterations":1,"exploitability_chips":null,"elapsed_ms":1,"memory_bytes":1}"#;
    let m: WorkerMessage = serde_json::from_str(explicit_null).unwrap();
    assert!(matches!(m, WorkerMessage::Progress { exploitability_chips: None, .. }));
    assert_eq!(serde_json::to_string(&m).unwrap(), explicit_null);

    let measured = r#"{"type":"progress","id":"1","stage":"solving","iterations":1,"exploitability_chips":0.5,"elapsed_ms":1,"memory_bytes":1}"#;
    let m: WorkerMessage = serde_json::from_str(measured).unwrap();
    assert!(matches!(m, WorkerMessage::Progress { exploitability_chips: Some(v), .. } if v == 0.5));
    assert_eq!(serde_json::to_string(&m).unwrap(), measured);
}
