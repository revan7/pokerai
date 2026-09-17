//! Envelope validation matrix (spec section 8.2). One test per validation rule in
//! `core_preflop::validate`/`core_preflop::decode`, with both boundaries exercised for
//! each numeric domain (probability bound `[0,1]`, sibling-sum tolerance `1e-3`, the
//! unreachable-class index `<169`, the raise-amount `>0` rule, and `MAX_BUNDLE_BYTES`).

use core_preflop::{decode, valid_step, validate, BundleError, Envelope, EnvelopeAction, MAX_BUNDLE_BYTES};

fn base_envelope() -> Envelope {
    Envelope {
        bundle_id: "synthetic".into(),
        depth_bb: 100,
        rake_profile: "5% cap 0.5bb".into(),
        straddle: false,
        class_order: "A-2 row-major, section 4.1".into(),
        nodes: vec![core_preflop::EnvelopeNode {
            history: vec![],
            actor: "UTG".into(),
            actions: vec![EnvelopeAction { step: "fold".into(), to_bb_x1000: None, label: None }],
            weights: vec![vec![1.0; 169]],
            evs: None,
            unreachable_classes: vec![],
        }],
    }
}

// The exact matrix test from the task brief.
#[test]
fn pokerdata_schema_mapping() {
    let mut e = core_preflop::Envelope {
        bundle_id: "synthetic".into(), depth_bb: 100,
        rake_profile: "5% cap 0.5bb".into(), straddle: false,
        class_order: "A-2 row-major, section 4.1".into(),
        nodes: vec![core_preflop::EnvelopeNode {
            history: vec![], actor: "UTG".into(),
            actions: vec![core_preflop::EnvelopeAction {
                step: "fold".into(), to_bb_x1000: None, label: None,
            }], weights: vec![vec![1.0; 169]], evs: None,
            unreachable_classes: vec![],
        }],
    };
    assert!(core_preflop::validate(&e).is_ok());
    e.nodes[0].weights[0][17] = 0.4;
    assert!(core_preflop::validate(&e).is_err());
    e.nodes[0].weights[0][17] = 0.0;
    assert!(core_preflop::validate(&e).is_err());
    e.nodes[0].unreachable_classes.push(17);
    assert!(core_preflop::validate(&e).is_ok());
}

#[test]
fn rejects_wrong_class_order() {
    let mut e = base_envelope();
    e.class_order = "wrong".into();
    assert!(validate(&e).is_err());
}

#[test]
fn depth_bb_boundary_zero_rejected_one_accepted() {
    let mut e = base_envelope();
    e.depth_bb = 0;
    assert!(validate(&e).is_err());
    e.depth_bb = 1;
    assert!(validate(&e).is_ok());
}

#[test]
fn rejects_unknown_actor_position() {
    // Uppercase provider aliases (e.g. "MP") are converted at ingestion, never accepted
    // by this strict envelope reader.
    let mut e = base_envelope();
    e.nodes[0].actor = "MP".into();
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_empty_action_menu() {
    let mut e = base_envelope();
    e.nodes[0].actions.clear();
    e.nodes[0].weights.clear();
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_duplicate_history_across_nodes() {
    let mut e = base_envelope();
    let dup = e.nodes[0].clone();
    e.nodes.push(dup);
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_invalid_history_position() {
    let mut e = base_envelope();
    e.nodes[0].history.push(("ZZ".into(), "fold".into(), 0));
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_history_with_unknown_step_token() {
    let mut e = base_envelope();
    e.nodes[0].history.push(("UTG".into(), "bet".into(), 0));
    assert!(validate(&e).is_err());
}

#[test]
fn history_raise_amount_zero_is_malformed_not_omitted() {
    // amount 0 maps to "no amount" for history tuples, and `raise` with no amount is invalid.
    let mut e = base_envelope();
    e.nodes[0].history.push(("UTG".into(), "raise".into(), 0));
    assert!(validate(&e).is_err());
    // Boundary: a positive resolved size on the same token is valid.
    let mut e2 = base_envelope();
    e2.nodes[0].actor = "HJ".into();
    e2.nodes[0].history.push(("UTG".into(), "raise".into(), 2500));
    assert!(validate(&e2).is_ok());
}

#[test]
fn rejects_non_raise_action_carrying_any_amount_including_zero() {
    let mut e = base_envelope();
    e.nodes[0].actions[0].to_bb_x1000 = Some(0);
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_raise_action_missing_amount() {
    let mut e = base_envelope();
    e.nodes[0].actions[0].step = "raise".into();
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_duplicate_action_kind_and_amount() {
    let mut e = base_envelope();
    e.nodes[0].actions.push(EnvelopeAction { step: "fold".into(), to_bb_x1000: None, label: None });
    e.nodes[0].weights.push(vec![0.0; 169]);
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_duplicate_unreachable_class() {
    // R3: zero class 5's only action weight first, so `[5]` alone is a valid unreachable
    // declaration (exact-zero rule satisfied) -- otherwise `[5, 5]` would be rejected by the
    // exact-zero rule regardless of the duplicate-index guard, and this test would not
    // actually exercise that guard.
    let mut e = base_envelope();
    e.nodes[0].weights[0][5] = 0.0;
    e.nodes[0].unreachable_classes = vec![5];
    assert!(validate(&e).is_ok());
    e.nodes[0].unreachable_classes = vec![5, 5];
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_weights_action_count_mismatch() {
    let mut e = base_envelope();
    e.nodes[0].weights.push(vec![0.0; 169]);
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_weights_row_wrong_length() {
    let mut e = base_envelope();
    e.nodes[0].weights[0] = vec![1.0; 168];
    assert!(validate(&e).is_err());
}

#[test]
fn unreachable_class_index_boundary_168_ok_169_rejected() {
    let mut e = base_envelope();
    e.nodes[0].weights[0][168] = 0.0;
    e.nodes[0].unreachable_classes = vec![168];
    assert!(validate(&e).is_ok());

    let mut e2 = base_envelope();
    e2.nodes[0].unreachable_classes = vec![169];
    assert!(validate(&e2).is_err());
}

#[test]
fn probability_bound_boundaries() {
    let mut e = base_envelope();
    e.nodes[0].actions.push(EnvelopeAction { step: "call".into(), to_bb_x1000: None, label: None });
    e.nodes[0].weights = vec![vec![1.0; 169], vec![0.0; 169]];
    assert!(validate(&e).is_ok());

    // R3: a compensating in-domain sibling (1.0) keeps the sum within the 1e-3 tolerance
    // despite the negative value, isolating the per-element domain-bound rule from the
    // sibling-sum rule -- with the original sibling 0.0, sum -0.0001 independently violates
    // the sum tolerance too, so that assertion could not tell which rule actually fired.
    e.nodes[0].weights[1][0] = 1.0;
    e.nodes[0].weights[0][0] = -0.0001;
    assert!(validate(&e).is_err());
    e.nodes[0].weights[1][0] = 0.0; // restore for the remaining sub-cases below

    e.nodes[0].weights[0][0] = 1.0001;
    assert!(validate(&e).is_err());
    e.nodes[0].weights[0][0] = f32::NAN;
    assert!(validate(&e).is_err());
    e.nodes[0].weights[0][0] = f32::INFINITY;
    assert!(validate(&e).is_err());
    e.nodes[0].weights[0][0] = f32::NEG_INFINITY;
    assert!(validate(&e).is_err());
}

#[test]
fn sibling_sum_tolerance_boundary() {
    let mut e = base_envelope();
    e.nodes[0].actions.push(EnvelopeAction { step: "call".into(), to_bb_x1000: None, label: None });
    e.nodes[0].weights = vec![vec![0.5; 169], vec![0.5; 169]];

    e.nodes[0].weights[0][0] = 0.5 + 9e-4; // sum 1.0009, within 1e-3 tolerance
    assert!(validate(&e).is_ok());

    e.nodes[0].weights[0][0] = 0.5 + 2e-3; // sum 1.002, outside 1e-3 tolerance
    assert!(validate(&e).is_err());
}

#[test]
fn unreachable_class_sum_must_be_exactly_zero() {
    let mut e = base_envelope();
    e.nodes[0].weights[0][17] = 1e-6;
    e.nodes[0].unreachable_classes.push(17);
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_ev_action_count_mismatch() {
    let mut e = base_envelope();
    e.nodes[0].evs = Some(vec![]);
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_ev_row_wrong_length() {
    let mut e = base_envelope();
    e.nodes[0].evs = Some(vec![vec![Some(0.0); 168]]);
    assert!(validate(&e).is_err());
}

#[test]
fn rejects_ev_non_finite_value() {
    let mut e = base_envelope();
    e.nodes[0].evs = Some(vec![vec![Some(f32::NAN); 169]]);
    assert!(validate(&e).is_err());

    let mut e2 = base_envelope();
    e2.nodes[0].evs = Some(vec![vec![Some(f32::INFINITY); 169]]);
    assert!(validate(&e2).is_err());
}

#[test]
fn accepts_ev_with_none_entries_and_finite_values() {
    let mut e = base_envelope();
    let mut row = vec![Some(0.0f32); 169];
    row[3] = None;
    e.nodes[0].evs = Some(vec![row]);
    assert!(validate(&e).is_ok());
}

#[test]
fn valid_step_matrix() {
    assert!(valid_step("raise", Some(2500)));
    assert!(!valid_step("raise", Some(0)));
    assert!(!valid_step("raise", None));
    for tok in ["fold", "check", "call", "allin"] {
        assert!(valid_step(tok, None), "{tok} with no amount must be valid");
        assert!(!valid_step(tok, Some(1)), "{tok} with a positive amount must be invalid");
        assert!(!valid_step(tok, Some(0)), "{tok} with an explicit zero amount must be invalid");
    }
    assert!(!valid_step("bet", None));
}

#[test]
fn decode_rejects_oversized_bundle() {
    let oversized = vec![b'a'; (MAX_BUNDLE_BYTES + 1) as usize];
    assert!(matches!(decode(&oversized), Err(BundleError::TooLarge)));
}

#[test]
fn decode_boundary_at_exactly_max_bytes_reaches_the_json_parser() {
    // Proves the size gate is strictly-greater-than, not off-by-one: a MAX_BUNDLE_BYTES
    // payload that fails to parse must fail as `Json`, not `TooLarge`.
    let bytes = vec![b' '; MAX_BUNDLE_BYTES as usize];
    assert!(matches!(decode(&bytes), Err(BundleError::Json(_))));
}

#[test]
fn decode_rejects_unknown_field() {
    let json = r#"{"bundle_id":"s","depth_bb":100,"rake_profile":"r","straddle":false,
        "class_order":"A-2 row-major, section 4.1","nodes":[],"extra":1}"#;
    assert!(matches!(decode(json.as_bytes()), Err(BundleError::Json(_))));
}

#[test]
fn decode_round_trips_a_valid_bundle() {
    let e = base_envelope();
    let bytes = serde_json::to_vec(&e).unwrap();
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.bundle_id, e.bundle_id);
    assert_eq!(decoded.nodes.len(), e.nodes.len());
}

// --- R1 (fix round 1): decode rejects wide-form violations that per-element narrowing
// would otherwise hide, with no clamping or renormalization -- both are the review's exact
// probe inputs, each paired with a nearby valid control. ---

/// A two-action ("fold", "call") envelope, 169-wide, valid at every class except `idx`,
/// where the two given tokens are substituted (`row0[idx] = tok0`, `row1[idx] = tok1`).
/// `unreachable` optionally declares `idx` unreachable.
fn decode_two_action_envelope(idx: usize, tok0: &str, tok1: &str, unreachable: bool) -> Result<Envelope, BundleError> {
    let mut row0 = vec!["1".to_string(); 169];
    let mut row1 = vec!["0".to_string(); 169];
    row0[idx] = tok0.to_string();
    row1[idx] = tok1.to_string();
    let unreachable_classes = if unreachable { format!("[{idx}]") } else { "[]".to_string() };
    let json = format!(
        r#"{{"bundle_id":"s","depth_bb":100,"rake_profile":"r","straddle":false,"class_order":"A-2 row-major, section 4.1","nodes":[{{"history":[],"actor":"UTG","actions":[{{"step":"fold"}},{{"step":"call"}}],"weights":[[{}],[{}]],"unreachable_classes":{unreachable_classes}}}]}}"#,
        row0.join(","),
        row1.join(",")
    );
    decode(json.as_bytes())
}

#[test]
fn decode_rejects_sibling_sum_that_narrowing_would_have_hidden() {
    // The review's exact probe: raw sum 1.00100000001 (outside the 1e-3 tolerance), but the
    // narrowed-f32-then-summed value 1.000999987125 was inside it under the pre-fix code.
    assert!(decode_two_action_envelope(0, "0.5", "0.50100000001", false).is_err());
}

#[test]
fn decode_accepts_nearby_valid_sibling_sum_control() {
    assert!(decode_two_action_envelope(0, "0.5", "0.5", false).is_ok());
}

#[test]
fn decode_rejects_unreachable_class_with_tiny_nonzero_weight_that_narrowing_would_have_hidden() {
    // The review's exact probe: a declared-unreachable class holding 1e-50, which narrows to
    // exactly 0.0 (satisfying the exact-zero rule) under the pre-fix code, hiding the
    // nonzero wire content.
    assert!(decode_two_action_envelope(0, "1e-50", "0", true).is_err());
}

#[test]
fn decode_accepts_unreachable_class_with_exact_zero_control() {
    assert!(decode_two_action_envelope(0, "0", "0", true).is_ok());
}

// --- P3.T2: `store` -- independent-bundle loading and per-bundle quarantine ---

const MANIFEST_JSON: &[u8] = include_bytes!("../../../fixtures/preflop/synthetic_v2/manifest.json");
const NODES_JSON: &[u8] = include_bytes!("../../../fixtures/preflop/synthetic_v2/nodes.json");

fn good_bundle_id() -> String {
    let v: serde_json::Value = serde_json::from_slice(MANIFEST_JSON).unwrap();
    v["bundle_id"].as_str().unwrap().to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// A 169-length JSON array of `default`, with `overrides` substituted at their indices.
fn json_row(default: &str, overrides: &[(usize, &str)]) -> String {
    let mut parts = vec![default.to_string(); 169];
    for &(i, tok) in overrides {
        parts[i] = tok.to_string();
    }
    format!("[{}]", parts.join(","))
}

/// A single-node, empty-history, UTG-actor envelope as raw JSON text (not a typed `Envelope`,
/// so a case can inject a value -- e.g. `1e400` -- that the checked codecs in `numeric.rs`
/// would never let a typed value hold in the first place).
fn nodes_json_text(bundle_id: &str, actions_json: &str, weights_json: &str, evs_json: Option<&str>, unreachable: &str) -> String {
    let evs_part = match evs_json {
        Some(e) => format!(r#","evs":{e}"#),
        None => String::new(),
    };
    format!(
        r#"{{"bundle_id":"{bundle_id}","depth_bb":100,"rake_profile":"5% cap 0.5bb","straddle":false,"class_order":"A-2 row-major, section 4.1","nodes":[{{"history":[],"actor":"UTG","actions":{actions_json},"weights":{weights_json}{evs_part},"unreachable_classes":{unreachable}}}]}}"#
    )
}

fn minimal_manifest_json(bundle_id: &str, sha256: &str) -> String {
    format!(
        r#"{{"bundle_id":"{bundle_id}","source":"PokerDataJson","depth_bb":100,"depths":[100],"source_blinds":[0.5,1.0],"rake_profile":"5% cap 0.5bb","rake":null,"straddle":false,"version":2,"game":"nl","ev_unit":"source_sb","ev_reference":"unverified","license_note":"n","accuracy":"unverified","sha256":"{sha256}"}}"#
    )
}

/// Writes a `good` sibling (the real committed fixture bytes, unmodified) and a `bad` sibling
/// (`nodes_bytes`, with its manifest's `sha256` recomputed from those exact bytes unless
/// `corrupt_hash` deliberately mismatches it) into a fresh temporary directory, runs
/// `PreflopStore::open`, and asserts: exactly one banner, `bad.bad` quarantined, and the good
/// sibling still answers a direct key lookup. Removes the directory afterward.
fn run_quarantine_case(case_idx: usize, nodes_bytes: &[u8], corrupt_hash: bool) {
    let dir = std::env::temp_dir().join(format!("core_preflop_p3t2_{}_{case_idx}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("good")).unwrap();
    std::fs::create_dir_all(dir.join("bad")).unwrap();

    std::fs::write(dir.join("good").join("manifest.json"), MANIFEST_JSON).unwrap();
    std::fs::write(dir.join("good").join("nodes.json"), NODES_JSON).unwrap();

    let hash = if corrupt_hash { "0".repeat(64) } else { sha256_hex(nodes_bytes) };
    let manifest_text = minimal_manifest_json("bad", &hash);
    std::fs::write(dir.join("bad").join("manifest.json"), manifest_text.as_bytes()).unwrap();
    std::fs::write(dir.join("bad").join("nodes.json"), nodes_bytes).unwrap();

    let (store, banners) = core_preflop::PreflopStore::open(&dir);
    assert_eq!(banners.len(), 1, "case {case_idx}: expected exactly one banner, got {banners:?}");
    assert!(dir.join("bad.bad").exists(), "case {case_idx}: expected a bad.bad quarantine directory");

    let good_id = good_bundle_id();
    let good = store.bundle_of(&good_id).unwrap_or_else(|| panic!("case {case_idx}: good sibling must still load, banners: {banners:?}"));
    let key = core_preflop::PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history: vec![] };
    assert!(good.lookup(&key).is_some(), "case {case_idx}: good source must answer a direct key lookup");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bundle_validation_quarantine() {
    // The exact starter matrix (spec section 13.1's `bundle_validation_quarantine` row):
    // decode-level rejections for a malformed weight, a truncated document, and an unknown
    // history step token.
    let raw = NODES_JSON;
    let mut v: serde_json::Value = serde_json::from_slice(raw).unwrap();
    v["nodes"][0]["weights"][0][0] = serde_json::json!(0.4);
    let malformed = serde_json::to_vec(&v).unwrap();
    assert!(core_preflop::decode(&malformed).is_err());
    assert!(core_preflop::decode(br#"{"nodes": ["#).is_err());
    v["nodes"][0]["history"] = serde_json::json!([["UTG", "mystery", 0]]);
    assert!(core_preflop::decode(&serde_json::to_vec(&v).unwrap()).is_err());

    // A valid hash alone must never pass: cases 1-5 below all carry a hash recomputed to
    // match their (invalid) content, so only content validation is what rejects them.

    // Wrong action count: 3 declared actions, only 2 weight rows.
    let actions = r#"[{"step":"fold"},{"step":"call"},{"step":"raise","to_bb_x1000":2500}]"#;
    let weights = format!("[{},{}]", json_row("1", &[]), json_row("0", &[]));
    let text = nodes_json_text("bad", actions, &weights, None, "[]");
    run_quarantine_case(1, text.as_bytes(), false);

    // 168 entries: both weight rows one short of the required 169.
    let row168a = format!("[{}]", vec!["1".to_string(); 168].join(","));
    let row168b = format!("[{}]", vec!["0".to_string(); 168].join(","));
    let weights = format!("[{row168a},{row168b}]");
    let actions = r#"[{"step":"fold"},{"step":"call"}]"#;
    let text = nodes_json_text("bad", actions, &weights, None, "[]");
    run_quarantine_case(2, text.as_bytes(), false);

    // 1e400: syntactically a valid JSON number, parses to f64::INFINITY -- rejected by the
    // EV finite check.
    let weights = format!("[{},{}]", json_row("1", &[]), json_row("0", &[]));
    let evs = format!("[{},{}]", json_row("null", &[]), json_row("null", &[(0, "1e400")]));
    let text = nodes_json_text("bad", actions, &weights, Some(&evs), "[]");
    run_quarantine_case(3, text.as_bytes(), false);

    // [-0.1, 1.1]: sibling sum stays exactly 1.0 (isolating the per-element bound rule from
    // the sibling-sum rule), but each value is individually outside [0, 1].
    let weights = format!("[{},{}]", json_row("1", &[(0, "-0.1")]), json_row("0", &[(0, "1.1")]));
    let text = nodes_json_text("bad", actions, &weights, None, "[]");
    run_quarantine_case(4, text.as_bytes(), false);

    // Undeclared all-zero class: class 0 sums to exactly 0 across both actions but is not
    // listed in `unreachable_classes`.
    let weights = format!("[{},{}]", json_row("1", &[(0, "0")]), json_row("0", &[]));
    let text = nodes_json_text("bad", actions, &weights, None, "[]");
    run_quarantine_case(5, text.as_bytes(), false);

    // Hash mismatch: otherwise fully valid content, but the manifest's sha256 is wrong.
    let weights = format!("[{},{}]", json_row("1", &[]), json_row("0", &[]));
    let text = nodes_json_text("bad", actions, &weights, None, "[]");
    run_quarantine_case(6, text.as_bytes(), true);

    // 64 MiB + 1: rejected on size before any JSON parsing or hashing is attempted.
    let oversized = vec![b' '; (MAX_BUNDLE_BYTES + 1) as usize];
    run_quarantine_case(7, &oversized, false);
}

#[test]
fn synthetic_v2_node_map_matches_expected_shape() {
    let info: core_preflop::BundleInfo = serde_json::from_slice(MANIFEST_JSON).expect("fixture manifest must parse");
    let envelope = core_preflop::checked_envelope(&info, NODES_JSON).expect("fixture must hash-and-content-validate");
    let map = core_preflop::build_node_map(&info, &envelope).expect("fixture must build a node map");
    assert_eq!(map.len(), 7, "seven fixture nodes (the eighth table row is absences-only)");

    let key = |history: Vec<(proto::Position, core_preflop::PreflopStep)>| {
        core_preflop::node_key(&core_preflop::PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history })
    };

    // UTG RFI: empty history, actor UTG, no prior commitment, action-major -> class-major
    // transpose, explicit fold action, unreachable class 168, sparse zero-weight EV retention.
    let utg_rfi = &map[&key(vec![])];
    assert_eq!(utg_rfi.actor, proto::Position::Utg);
    assert_eq!(utg_rfi.actions, vec![core_preflop::PreflopStep::Fold, core_preflop::PreflopStep::Raise { to_bb_x1000: 2500 }]);
    assert_eq!(utg_rfi.probs.len(), 169, "class-major: one row per class");
    assert_eq!(utg_rfi.probs[0].len(), 2, "class-major: one column per action");
    assert_eq!(utg_rfi.probs[0], vec![1.0, 0.0], "class 0 (AA): all weight on the fold action");
    assert_eq!(utg_rfi.probs[1], vec![0.65, 0.35], "class 1 (AKs): split across fold/raise");
    assert!(utg_rfi.unreachable[168], "class 168 declared unreachable");
    assert!(!utg_rfi.unreachable[0]);
    assert_eq!(utg_rfi.committed_by_actor_sb, 0.0, "UTG has posted nothing and has not acted yet");
    let evs = utg_rfi.ev_source_sb.as_ref().expect("evs present");
    assert_eq!(evs[14][1], Some(2.31), "class 14 (KK): EV retained despite zero weight on the raise action");
    assert_eq!(utg_rfi.probs[14][1], 0.0, "class 14 (KK): zero weight on the raise action");
    assert_eq!(evs[5][1], None, "an unset EV cell stays absent (None), never inferred");

    // HJ vs UTG RFI: history carries an explicit UTG raise; HJ has not acted yet.
    let hj_vs_utg_rfi_history = vec![(proto::Position::Utg, core_preflop::PreflopStep::Raise { to_bb_x1000: 2500 })];
    let hj_vs_utg_rfi = &map[&key(hj_vs_utg_rfi_history)];
    assert_eq!(hj_vs_utg_rfi.actor, proto::Position::Hj);
    assert_eq!(hj_vs_utg_rfi.committed_by_actor_sb, 0.0, "HJ has not acted yet");

    // UTG vs HJ 3bet: history carries four explicit folds (CO, BTN, SB, BB); UTG's own prior
    // raise (2500 -> 5.0 source SB) is what `committed_by_actor_sb` must recover.
    let utg_vs_hj_3bet_history = vec![
        (proto::Position::Utg, core_preflop::PreflopStep::Raise { to_bb_x1000: 2500 }),
        (proto::Position::Hj, core_preflop::PreflopStep::Raise { to_bb_x1000: 8750 }),
        (proto::Position::Co, core_preflop::PreflopStep::Fold),
        (proto::Position::Btn, core_preflop::PreflopStep::Fold),
        (proto::Position::Sb, core_preflop::PreflopStep::Fold),
        (proto::Position::Bb, core_preflop::PreflopStep::Fold),
    ];
    let utg_vs_hj_3bet = &map[&key(utg_vs_hj_3bet_history.clone())];
    assert_eq!(utg_vs_hj_3bet.actor, proto::Position::Utg);
    assert_eq!(utg_vs_hj_3bet.committed_by_actor_sb, 5.0, "UTG's own 2500 (bb x1000) raise -> 5.0 source SB");

    // HJ vs UTG 4bet: HJ's own prior 8750 raise -> 17.5 source SB; menu carries an explicit
    // all-in with no amount.
    let mut hj_vs_utg_4bet_history = utg_vs_hj_3bet_history.clone();
    hj_vs_utg_4bet_history.push((proto::Position::Utg, core_preflop::PreflopStep::Raise { to_bb_x1000: 22000 }));
    let hj_vs_utg_4bet = &map[&key(hj_vs_utg_4bet_history)];
    assert_eq!(hj_vs_utg_4bet.actor, proto::Position::Hj);
    assert_eq!(
        hj_vs_utg_4bet.actions,
        vec![core_preflop::PreflopStep::Fold, core_preflop::PreflopStep::Call, core_preflop::PreflopStep::AllIn]
    );
    assert_eq!(hj_vs_utg_4bet.committed_by_actor_sb, 17.5);

    // BB squeeze: BB's own post (2.0 source SB) is unaffected by the other seats' actions.
    let bb_squeeze_history = vec![
        (proto::Position::Utg, core_preflop::PreflopStep::Raise { to_bb_x1000: 2500 }),
        (proto::Position::Hj, core_preflop::PreflopStep::Call),
        (proto::Position::Co, core_preflop::PreflopStep::Fold),
        (proto::Position::Btn, core_preflop::PreflopStep::Fold),
        (proto::Position::Sb, core_preflop::PreflopStep::Fold),
    ];
    let bb_squeeze = &map[&key(bb_squeeze_history)];
    assert_eq!(bb_squeeze.actor, proto::Position::Bb);
    assert_eq!(bb_squeeze.committed_by_actor_sb, 2.0);

    // SB limp: all four early positions explicitly fold; SB's own post (1.0 source SB).
    let sb_limp_history = vec![
        (proto::Position::Utg, core_preflop::PreflopStep::Fold),
        (proto::Position::Hj, core_preflop::PreflopStep::Fold),
        (proto::Position::Co, core_preflop::PreflopStep::Fold),
        (proto::Position::Btn, core_preflop::PreflopStep::Fold),
    ];
    let sb_limp = &map[&key(sb_limp_history.clone())];
    assert_eq!(sb_limp.actor, proto::Position::Sb);
    assert_eq!(sb_limp.actions[1], core_preflop::PreflopStep::Call, "the limp itself is the call option");
    assert_eq!(sb_limp.committed_by_actor_sb, 1.0);

    // BB vs SB limp: SB completes (Call) in the history; BB's menu carries an explicit check.
    let mut bb_vs_sb_limp_history = sb_limp_history;
    bb_vs_sb_limp_history.push((proto::Position::Sb, core_preflop::PreflopStep::Call));
    let bb_vs_sb_limp = &map[&key(bb_vs_sb_limp_history)];
    assert_eq!(bb_vs_sb_limp.actor, proto::Position::Bb);
    assert_eq!(
        bb_vs_sb_limp.actions,
        vec![core_preflop::PreflopStep::Check, core_preflop::PreflopStep::Raise { to_bb_x1000: 3500 }]
    );
    assert_eq!(bb_vs_sb_limp.committed_by_actor_sb, 2.0, "BB has not acted yet, still its own post");
}

#[test]
fn store_from_sources_and_bundles_accessors() {
    let info: core_preflop::BundleInfo = serde_json::from_slice(MANIFEST_JSON).unwrap();
    let envelope = core_preflop::checked_envelope(&info, NODES_JSON).unwrap();
    let map = core_preflop::build_node_map(&info, &envelope).unwrap();
    let bundle_id = info.bundle_id.clone();
    let source: Box<dyn core_preflop::PreflopSource> = Box::new(core_preflop::PokerDataJson { info, nodes: map });
    let store = core_preflop::PreflopStore::from_sources(vec![source]);
    assert_eq!(store.bundles().len(), 1);
    assert!(store.bundle_of(&bundle_id).is_some());
    assert!(store.bundle_of("does-not-exist").is_none());
}

#[test]
fn open_reports_missing_directory_without_panicking() {
    let dir = std::env::temp_dir().join(format!("core_preflop_p3t2_missing_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (store, banners) = core_preflop::PreflopStore::open(&dir);
    assert_eq!(store.bundles().len(), 0);
    assert_eq!(banners.len(), 1);
}
