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

/// The `EffectiveTree` of spec 4.5 with all three thresholds at `0.0`, used by the S1 tests below
/// to vary one threshold at a time by text substitution.
fn threshold_tree_json() -> &'static str {
    r#"{"rules_version":3,"template_id":"river_oracle_v1","root_street":"river","menus":{"river":{"oop":{"bet":[],"raise":[]},"ip":{"bet":[1.0],"raise":[]}}},"add_allin_threshold":0.0,"force_allin_threshold":0.0,"merging_threshold":0.0,"wager_cap":1,"inserted":[],"materialized":[{"path":[],"street":"river","actor":"oop","actions":[{"kind":"check"}],"terminal_pots":[null]}]}"#
}

/// S1 (final review): `Rake::PotRake.rate` is the same physical quantity as the worker wire's
/// `rake_rate` and must carry the same half-open `[0, 1)` codec on both serde directions -- wide
/// (f64) validation before narrowing, and a serialize-side rejection instead of JSON `null`.
#[test]
fn config_rake_rate_is_validated_before_narrowing_and_on_serialize() {
    let line = |v: &str| format!(r#"{{"kind":"pot_rake","rate":{v},"cap_mchips":5000,"no_flop_no_drop":false}}"#);
    // Out of domain in the wide form, or only rounding into it on narrowing, or overflowing f32.
    for bad in ["1e39", "-1e39", "1.0", "1.00000001", "-1e-50", "-0.5", "2.0"] {
        assert!(serde_json::from_str::<Rake>(&line(bad)).is_err(), "rake rate {bad} must be rejected");
    }
    // Valid endpoint and ordinary values.
    for ok in ["0.0", "0.05", "0.999"] {
        assert!(serde_json::from_str::<Rake>(&line(ok)).is_ok(), "rake rate {ok} must be accepted");
    }
    // Serialize side: an in-memory value built by struct literal must error, never become `null`.
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.5, 1.0, 2.0] {
        let r = Rake::PotRake { rate: bad, cap_mchips: 5000, no_flop_no_drop: false };
        assert!(serde_json::to_string(&r).is_err(), "in-memory rake rate {bad} must not serialize");
    }
    let good = Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false };
    assert_eq!(serde_json::from_str::<Rake>(&serde_json::to_string(&good).unwrap()).unwrap(), good);
    assert_eq!(serde_json::from_str::<Rake>(&serde_json::to_string(&Rake::TimeCharge).unwrap()).unwrap(), Rake::TimeCharge);
}

/// S1: the three `EffectiveTree` thresholds are read by the tree materializer at every
/// opening/facing boundary (spec 4.6); each is non-negative and finite on both directions.
#[test]
fn effective_tree_thresholds_are_validated_before_narrowing_and_on_serialize() {
    for field in ["add_allin_threshold", "force_allin_threshold", "merging_threshold"] {
        let line = |v: &str| threshold_tree_json().replacen(&format!(r#""{field}":0.0"#), &format!(r#""{field}":{v}"#), 1);
        for bad in ["1e39", "-1e39", "-1", "-1e-50"] {
            assert!(serde_json::from_str::<EffectiveTree>(&line(bad)).is_err(), "{field} {bad} must be rejected");
        }
        for ok in ["0.0", "1.5", "0.15"] {
            assert!(serde_json::from_str::<EffectiveTree>(&line(ok)).is_ok(), "{field} {ok} must be accepted");
        }
    }
    let mut tree: EffectiveTree = serde_json::from_str(threshold_tree_json()).unwrap();
    tree.add_allin_threshold = f32::NAN;
    assert!(serde_json::to_string(&tree).is_err(), "NaN add_allin_threshold must not serialize as null");
    tree.add_allin_threshold = 1.5;
    tree.force_allin_threshold = f32::INFINITY;
    assert!(serde_json::to_string(&tree).is_err(), "infinite force_allin_threshold must not serialize as null");
    tree.force_allin_threshold = 0.15;
    tree.merging_threshold = -1.0;
    assert!(serde_json::to_string(&tree).is_err(), "negative merging_threshold must not serialize");
    tree.merging_threshold = 0.0;
    assert_eq!(serde_json::from_str::<EffectiveTree>(&serde_json::to_string(&tree).unwrap()).unwrap(), tree);
}

/// S1: the `Recommendation` tree crosses the Tauri IPC boundary to the UI. Every `f32` and
/// `Option<f32>` on it carries a checked codec: `Some(NaN)` must be a serialize **error**, never
/// the silent `Some -> null -> None` degradation the review measured, and a required `f32` must
/// never be emitted as `null` (which would make the whole event undeserializable).
#[test]
fn recommendation_probability_fields_are_validated_both_directions() {
    // ActionAdvice.frequency: unit interval, nullable.
    let advice = |f: &str| format!(r#"{{"action":{{"kind":"fold"}},"frequency":{f},"ev_bb":null,"unavailable":null,"headline":false}}"#);
    for bad in ["1.00000001", "-1e-50", "1e39", "-1e39", "1.5"] {
        assert!(serde_json::from_str::<ActionAdvice>(&advice(bad)).is_err(), "frequency {bad} must be rejected");
    }
    for ok in ["0.0", "1.0", "0.25", "null"] {
        assert!(serde_json::from_str::<ActionAdvice>(&advice(ok)).is_ok(), "frequency {ok} must be accepted");
    }
    let nan_freq = ActionAdvice { action: Action::Fold, frequency: Some(f32::NAN), ev_bb: None, unavailable: None, headline: false };
    assert!(serde_json::to_string(&nan_freq).is_err(), "Some(NaN) frequency must error, not degrade to None");
    // ev_bb: finite, nullable; overflow-on-narrow rejected, ordinary negatives accepted.
    let ev = |v: &str| format!(r#"{{"action":{{"kind":"fold"}},"frequency":null,"ev_bb":{v},"unavailable":null,"headline":false}}"#);
    assert!(serde_json::from_str::<ActionAdvice>(&ev("1e39")).is_err());
    assert!(serde_json::from_str::<ActionAdvice>(&ev("-2.3")).is_ok());
    assert!(serde_json::from_str::<ActionAdvice>(&ev("null")).is_ok());
    let inf_ev = ActionAdvice { action: Action::Fold, frequency: None, ev_bb: Some(f32::INFINITY), unavailable: None, headline: false };
    assert!(serde_json::to_string(&inf_ev).is_err(), "Some(inf) ev_bb must error, not degrade to None");
    // None is still the one nullable value and still round-trips.
    let none = ActionAdvice { action: Action::Fold, frequency: None, ev_bb: None, unavailable: None, headline: false };
    let text = serde_json::to_string(&none).unwrap();
    assert!(text.contains(r#""frequency":null"#) && text.contains(r#""ev_bb":null"#), "{text}");
    assert_eq!(serde_json::from_str::<ActionAdvice>(&text).unwrap(), none);

    // EquityEstimate.value and EquityMethod::MonteCarlo.std_err: unit interval.
    let est = |v: &str| format!(r#"{{"value":{v},"availability":{{"kind":"Ready"}},"method":null}}"#);
    assert!(serde_json::from_str::<EquityEstimate>(&est("1.00000001")).is_err());
    assert!(serde_json::from_str::<EquityEstimate>(&est("1.0")).is_ok());
    assert!(serde_json::from_str::<EquityEstimate>(&est("null")).is_ok());
    let nan_est = EquityEstimate { value: Some(f32::NAN), availability: Availability::Ready, method: None };
    assert!(serde_json::to_string(&nan_est).is_err(), "Some(NaN) equity value must error");
    let nan_method = EquityMethod::MonteCarlo { samples: 1, std_err: f32::NAN };
    assert!(serde_json::to_string(&nan_method).is_err(), "NaN std_err must error");
    assert!(serde_json::from_str::<EquityMethod>(r#"{"kind":"MonteCarlo","samples":1,"std_err":1e39}"#).is_err());

    // Unavailable::BranchSupportIncomplete.covered_posterior and Recommendation.unresolved_mass.
    assert!(serde_json::from_str::<Unavailable>(r#"{"kind":"BranchSupportIncomplete","covered_posterior":1.5}"#).is_err());
    assert!(serde_json::from_str::<Unavailable>(r#"{"kind":"BranchSupportIncomplete","covered_posterior":0.2}"#).is_ok());
    let bad_cov = Unavailable::BranchSupportIncomplete { covered_posterior: f32::NAN };
    assert!(serde_json::to_string(&bad_cov).is_err());
    // ExploitAdvice.alpha (unit) and ev_delta_bb (finite, nullable).
    let exploit = |alpha: f32, delta: Option<f32>| ExploitAdvice { alpha, model_revision: 1, model_summary: "m".into(), gto_action: Action::Fold, exploit_action: Action::Check, ev_delta_bb: delta, locked_nodes: 0 };
    assert!(serde_json::to_string(&exploit(f32::NAN, None)).is_err(), "NaN alpha must error");
    assert!(serde_json::to_string(&exploit(1.5, None)).is_err(), "alpha above 1 must error");
    assert!(serde_json::to_string(&exploit(0.25, Some(f32::INFINITY))).is_err(), "Some(inf) ev_delta_bb must error");
    let ok_exploit = exploit(0.25, Some(-1.5));
    assert_eq!(serde_json::from_str::<ExploitAdvice>(&serde_json::to_string(&ok_exploit).unwrap()).unwrap(), ok_exploit);
}

/// S1: the `ApproxReason` float families -- scalars, the `stacks_bb` vector, the `posts` array and
/// the `mapped` size/weight pairs -- are all checked, on both directions.
#[test]
fn approx_reason_floats_are_validated_both_directions() {
    // DepthBucket.actual_bb: finite scalar.
    assert!(serde_json::from_str::<ApproxReason>(r#"{"kind":"DepthBucket","seat":0,"actual_bb":1e39,"used_bb":100,"prominent":false}"#).is_err());
    assert!(serde_json::from_str::<ApproxReason>(r#"{"kind":"DepthBucket","seat":0,"actual_bb":104.5,"used_bb":100,"prominent":false}"#).is_ok());
    let nan_depth = ApproxReason::DepthBucket { seat: Seat(0), actual_bb: f32::NAN, used_bb: 100, prominent: false };
    assert!(serde_json::to_string(&nan_depth).is_err(), "NaN actual_bb must error, never serialize as null");
    // AsymmetricStacks.stacks_bb: every element checked.
    assert!(serde_json::from_str::<ApproxReason>(r#"{"kind":"AsymmetricStacks","stacks_bb":[100.0,1e39],"prominent":true}"#).is_err());
    let nan_stacks = ApproxReason::AsymmetricStacks { stacks_bb: vec![100.0, f32::NAN], prominent: true };
    assert!(serde_json::to_string(&nan_stacks).is_err());
    let ok_stacks = ApproxReason::AsymmetricStacks { stacks_bb: vec![100.0, 104.0], prominent: false };
    assert_eq!(serde_json::from_str::<ApproxReason>(&serde_json::to_string(&ok_stacks).unwrap()).unwrap(), ok_stacks);
    // StraddleMapped.posts: the three-element array, the S8 failure mode on the wire.
    assert!(serde_json::from_str::<ApproxReason>(r#"{"kind":"StraddleMapped","posts":[1e39,0.5,1.0]}"#).is_err());
    let inf_posts = ApproxReason::StraddleMapped { posts: [f32::INFINITY, f32::INFINITY, 1.0] };
    assert!(serde_json::to_string(&inf_posts).is_err(), "infinite straddle posts must not serialize as null");
    let ok_posts = ApproxReason::StraddleMapped { posts: [0.25, 0.5, 1.0] };
    assert_eq!(serde_json::from_str::<ApproxReason>(&serde_json::to_string(&ok_posts).unwrap()).unwrap(), ok_posts);
    // BetTranslation: observed_pct finite, deviation in [0,1], mapped = (finite size, unit weight).
    let bt = |observed: &str, mapped: &str, deviation: &str| format!(r#"{{"kind":"BetTranslation","street":"flop","seat":2,"observed_pct":{observed},"mapped":{mapped},"deviation":{deviation},"prominent":true}}"#);
    assert!(serde_json::from_str::<ApproxReason>(&bt("0.73", "[[0.5,0.468],[1.0,0.532]]", "0.23")).is_ok());
    assert!(serde_json::from_str::<ApproxReason>(&bt("1e39", "[[0.5,0.468],[1.0,0.532]]", "0.23")).is_err());
    assert!(serde_json::from_str::<ApproxReason>(&bt("0.73", "[[0.5,1.00000001]]", "0.23")).is_err(), "a mapped weight above 1 must be rejected");
    assert!(serde_json::from_str::<ApproxReason>(&bt("0.73", "[[1e39,0.5]]", "0.23")).is_err(), "a mapped size that overflows f32 must be rejected");
    assert!(serde_json::from_str::<ApproxReason>(&bt("0.73", "[[0.5,0.468]]", "1.5")).is_err(), "a deviation above 1 must be rejected");
    let nan_mapped = ApproxReason::BetTranslation { street: Street::Flop, seat: Seat(2), observed_pct: 0.73, mapped: vec![(0.5, f32::NAN)], deviation: 0.23, prominent: true };
    assert!(serde_json::to_string(&nan_mapped).is_err());
    // SprBucketed / MenuRounded / BranchResidual: finite scalars.
    assert!(serde_json::from_str::<ApproxReason>(r#"{"kind":"SprBucketed","actual":1e39,"used":5.0}"#).is_err());
    assert!(serde_json::to_string(&ApproxReason::MenuRounded { max_delta_pct: f32::NAN }).is_err());
    assert!(serde_json::to_string(&ApproxReason::BranchResidual { seat: Seat(1), residual_mass_pct: f32::INFINITY, cause: "cap".into() }).is_err());
    let ok_residual = ApproxReason::BranchResidual { seat: Seat(1), residual_mass_pct: 35.2, cause: "cap".into() };
    assert_eq!(serde_json::from_str::<ApproxReason>(&serde_json::to_string(&ok_residual).unwrap()).unwrap(), ok_residual);
}

/// S1: `Assumptions.ranges_used`, `ExperimentalHu.ranges_used`, `Recommendation.range_mix` and
/// `RecommendationEvent::Progress.exploitability_pct` are the remaining float carriers on the IPC
/// wire; each is checked on both directions.
#[test]
fn assumption_and_event_floats_are_validated_both_directions() {
    let assumptions = |mass: f32| Assumptions {
        ranges_used: vec![(Seat(0), "AA".into(), mass)], tree_signature: "t".into(), template_id: "x".into(),
        source: "s".into(), source_accuracy: "a".into(), source_granularity: "g".into(),
        target_bp: 50, reached_bp: None, elapsed_ms: 1, cache: "miss".into(),
        translations: vec![], mappings: vec![], notes: vec![],
    };
    assert!(serde_json::to_string(&assumptions(f32::NAN)).is_err(), "NaN range mass must not serialize as null");
    assert!(serde_json::to_string(&assumptions(-1.0)).is_err(), "a negative range mass must not serialize");
    let ok = assumptions(6.0);
    assert_eq!(serde_json::from_str::<Assumptions>(&serde_json::to_string(&ok).unwrap()).unwrap(), ok);
    let bad_text = serde_json::to_string(&ok).unwrap().replacen("6.0", "1e39", 1);
    assert!(serde_json::from_str::<Assumptions>(&bad_text).is_err(), "a range mass that overflows f32 must be rejected");

    let hu = |mass: f32| ExperimentalHu {
        opponent: Seat(3), hero_role: "oop".into(), pot: 100, stack: 200, template_id: "x".into(),
        ranges_used: [(Seat(0), "AA".into(), 6.0), (Seat(3), "KK".into(), mass)],
        actions: vec![], reached_bp: None, elapsed_ms: 1, note: EXPERIMENTAL_NOTE.into(),
    };
    assert!(serde_json::to_string(&hu(f32::NAN)).is_err());
    let ok_hu = hu(6.0);
    assert_eq!(serde_json::from_str::<ExperimentalHu>(&serde_json::to_string(&ok_hu).unwrap()).unwrap(), ok_hu);

    let rec = |mix: Option<Vec<(Action, f32)>>, unresolved: f32| Recommendation {
        identity: DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 },
        phase: Phase::Fast, coverage: Coverage::Exact, legal: vec![], actions: vec![],
        unresolved_mass: unresolved, range_mix: mix, equity: EquitySummary::default(),
        assumptions: assumptions(6.0), experimental: None, exploit: None,
    };
    assert!(serde_json::to_string(&rec(None, f32::NAN)).is_err(), "NaN unresolved_mass must not serialize as null");
    assert!(serde_json::to_string(&rec(None, 1.5)).is_err(), "unresolved_mass above 1 must not serialize");
    assert!(serde_json::to_string(&rec(Some(vec![(Action::Fold, f32::NAN)]), 0.0)).is_err(), "a NaN range_mix weight must not serialize");
    assert!(serde_json::to_string(&rec(Some(vec![(Action::Fold, 1.5)]), 0.0)).is_err(), "a range_mix weight above 1 must not serialize");
    let ok_rec = rec(Some(vec![(Action::Fold, 0.4), (Action::Check, 0.6)]), 0.0);
    assert_eq!(serde_json::from_str::<Recommendation>(&serde_json::to_string(&ok_rec).unwrap()).unwrap(), ok_rec);

    let id = DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 };
    let progress = |v: Option<f32>| RecommendationEvent::Progress { identity: id.clone(), stage: "solving".into(), iterations: 1, exploitability_pct: v, elapsed_ms: 1 };
    assert!(serde_json::to_string(&progress(Some(f32::NAN))).is_err(), "Some(NaN) exploitability_pct must error, not degrade to None");
    assert!(serde_json::to_string(&progress(Some(-1.0))).is_err(), "a negative exploitability_pct must not serialize");
    let ok_progress = progress(Some(1.9));
    assert_eq!(serde_json::from_str::<RecommendationEvent>(&serde_json::to_string(&ok_progress).unwrap()).unwrap(), ok_progress);
    let none_progress = progress(None);
    let text = serde_json::to_string(&none_progress).unwrap();
    assert!(text.contains(r#""exploitability_pct":null"#), "None is still the one nullable value: {text}");
    assert_eq!(serde_json::from_str::<RecommendationEvent>(&text).unwrap(), none_progress);
}
