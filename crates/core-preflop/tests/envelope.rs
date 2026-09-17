//! Envelope validation matrix (spec section 8.2). One test per validation rule in
//! `core_preflop::validate`/`core_preflop::decode`, with both boundaries exercised for
//! each numeric domain (probability bound `[0,1]`, sibling-sum tolerance `1e-3`, the
//! unreachable-class index `<169`, the raise-amount `>0` rule, and `MAX_BUNDLE_BYTES`).

use core_preflop::{decode, valid_step, validate, BundleError, Envelope, EnvelopeAction, PreflopSource, MAX_BUNDLE_BYTES};

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

use std::path::Path;

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

/// A single-node envelope as raw JSON text (not a typed `Envelope`, so a case can inject a
/// value -- e.g. `1e400`, an out-of-domain weight, or an unknown history step token -- that
/// the checked codecs in `numeric.rs`/the structural rules in `validate.rs` would never let a
/// typed value hold in the first place). `history_json`/`actor` let R2's cases place a node
/// whose declared actor does not match the next eligible actor for that history.
fn nodes_json_text(
    bundle_id: &str,
    history_json: &str,
    actor: &str,
    actions_json: &str,
    weights_json: &str,
    evs_json: Option<&str>,
    unreachable: &str,
) -> String {
    let evs_part = match evs_json {
        Some(e) => format!(r#","evs":{e}"#),
        None => String::new(),
    };
    format!(
        r#"{{"bundle_id":"{bundle_id}","depth_bb":100,"rake_profile":"5% cap 0.5bb","straddle":false,"class_order":"A-2 row-major, section 4.1","nodes":[{{"history":{history_json},"actor":"{actor}","actions":{actions_json},"weights":{weights_json}{evs_part},"unreachable_classes":{unreachable}}}]}}"#
    )
}

/// The two-action ("fold","call"), empty-history, UTG-actor baseline used by most cases
/// below: valid unless a case deliberately perturbs one field.
fn baseline_actions() -> &'static str {
    r#"[{"step":"fold"},{"step":"call"}]"#
}
fn baseline_weights() -> String {
    format!("[{},{}]", json_row("1", &[]), json_row("0", &[]))
}
fn baseline_nodes_json_text(bundle_id: &str) -> String {
    nodes_json_text(bundle_id, "[]", "UTG", baseline_actions(), &baseline_weights(), None, "[]")
}

fn minimal_manifest_json(bundle_id: &str, source: &str, blinds_json: &str, sha256: &str) -> String {
    format!(
        r#"{{"bundle_id":"{bundle_id}","source":"{source}","depth_bb":100,"depths":[100],"source_blinds":{blinds_json},"rake_profile":"5% cap 0.5bb","rake":null,"straddle":false,"version":2,"game":"nl","ev_unit":"source_sb","ev_reference":"unverified","license_note":"n","accuracy":"unverified","sha256":"{sha256}"}}"#
    )
}

/// A single-node, two-action envelope with a large filler `label` on the "call" action (a
/// free-form descriptive field `check_envelope` never inspects -- see `EnvelopeAction::label`
/// in `envelope.rs`) so the JSON's total byte length can be tuned exactly, independent of its
/// validity. Used by R5's oversized/boundary cases: content stays valid regardless of
/// `label_len`, only the size changes.
fn padded_valid_nodes_json_text(bundle_id: &str, label_len: usize) -> String {
    let label = "x".repeat(label_len);
    let actions = format!(r#"[{{"step":"fold"}},{{"step":"call","label":"{label}"}}]"#);
    nodes_json_text(bundle_id, "[]", "UTG", &actions, &baseline_weights(), None, "[]")
}

/// Pads `padded_valid_nodes_json_text` to land at exactly `target_len` bytes.
fn padded_valid_nodes_json_at_exactly(bundle_id: &str, target_len: usize) -> Vec<u8> {
    let base = padded_valid_nodes_json_text(bundle_id, 0);
    assert!(base.len() <= target_len, "target too small for the unpadded baseline");
    let pad_len = target_len - base.len();
    let text = padded_valid_nodes_json_text(bundle_id, pad_len);
    assert_eq!(text.len(), target_len, "padding must land exactly on the target length");
    text.into_bytes()
}

/// Writes a `good` sibling (the real committed fixture bytes, unmodified) and a `bad` sibling
/// (`manifest_text` + `nodes_bytes`) into a fresh temporary directory, runs
/// `PreflopStore::open`, and asserts: exactly one banner, `bad.bad` quarantined, and the good
/// sibling still answers a direct key lookup. Removes the directory afterward.
fn run_quarantine_case_with_manifest(case_id: &str, manifest_text: &str, nodes_bytes: &[u8]) {
    let dir = std::env::temp_dir().join(format!("core_preflop_p3t2_{}_{case_id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("good")).unwrap();
    std::fs::create_dir_all(dir.join("bad")).unwrap();

    std::fs::write(dir.join("good").join("manifest.json"), MANIFEST_JSON).unwrap();
    std::fs::write(dir.join("good").join("nodes.json"), NODES_JSON).unwrap();

    std::fs::write(dir.join("bad").join("manifest.json"), manifest_text.as_bytes()).unwrap();
    std::fs::write(dir.join("bad").join("nodes.json"), nodes_bytes).unwrap();

    let (store, banners) = core_preflop::PreflopStore::open(&dir);
    assert_eq!(banners.len(), 1, "case {case_id}: expected exactly one banner, got {banners:?}");
    assert!(dir.join("bad.bad").exists(), "case {case_id}: expected a bad.bad quarantine directory");

    let good_id = good_bundle_id();
    let good = store.bundle_of(&good_id).unwrap_or_else(|| panic!("case {case_id}: good sibling must still load, banners: {banners:?}"));
    let key = core_preflop::PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history: vec![] };
    assert!(good.lookup(&key).is_some(), "case {case_id}: good source must answer a direct key lookup");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The common case of `run_quarantine_case_with_manifest`: only the node content varies,
/// with `PokerDataJson`/`[0.5,1.0]` blinds and a hash recomputed to match `nodes_bytes`
/// (unless `corrupt_hash` deliberately mismatches it instead).
fn run_quarantine_case(case_id: &str, nodes_bytes: &[u8], corrupt_hash: bool) {
    let hash = if corrupt_hash { "0".repeat(64) } else { sha256_hex(nodes_bytes) };
    let manifest_text = minimal_manifest_json("bad", "PokerDataJson", "[0.5,1.0]", &hash);
    run_quarantine_case_with_manifest(case_id, &manifest_text, nodes_bytes);
}

#[test]
fn bundle_validation_quarantine() {
    // The exact starter matrix (spec section 13.1's `bundle_validation_quarantine` row):
    // decode-level rejections for a malformed weight, a truncated document, and an unknown
    // history step token. R5: each assertion below uses a *fresh* copy of the base value, so
    // the malformed-weight mutation from the first assertion can never be the actual reason
    // a later assertion (the unknown history token) fails -- the original starter code
    // reused one `v` across all three, so the third assertion would still fail even with
    // token validation removed, entirely on the strength of the still-present first defect.
    let base: serde_json::Value = serde_json::from_slice(NODES_JSON).unwrap();

    let mut v1 = base.clone();
    v1["nodes"][0]["weights"][0][0] = serde_json::json!(0.4);
    assert!(core_preflop::decode(&serde_json::to_vec(&v1).unwrap()).is_err());

    assert!(core_preflop::decode(br#"{"nodes": ["#).is_err());

    let mut v3 = base.clone();
    v3["nodes"][0]["history"] = serde_json::json!([["UTG", "mystery", 0]]);
    assert!(core_preflop::decode(&serde_json::to_vec(&v3).unwrap()).is_err());

    // A valid hash alone must never pass: every content case below carries a hash recomputed
    // to match its (invalid) content, so only content validation is what rejects it.

    // Wrong action count: 3 declared actions, only 2 weight rows.
    let actions = r#"[{"step":"fold"},{"step":"call"},{"step":"raise","to_bb_x1000":2500}]"#;
    let text = nodes_json_text("bad", "[]", "UTG", actions, &baseline_weights(), None, "[]");
    run_quarantine_case("wrong_action_count", text.as_bytes(), false);

    // 168 entries: both weight rows one short of the required 169.
    let row168a = format!("[{}]", vec!["1".to_string(); 168].join(","));
    let row168b = format!("[{}]", vec!["0".to_string(); 168].join(","));
    let weights = format!("[{row168a},{row168b}]");
    let text = nodes_json_text("bad", "[]", "UTG", baseline_actions(), &weights, None, "[]");
    run_quarantine_case("168_entries", text.as_bytes(), false);

    // 1e400: syntactically a valid JSON number, parses to f64::INFINITY -- rejected by the
    // EV finite check.
    let evs = format!("[{},{}]", json_row("null", &[]), json_row("null", &[(0, "1e400")]));
    let text = nodes_json_text("bad", "[]", "UTG", baseline_actions(), &baseline_weights(), Some(&evs), "[]");
    run_quarantine_case("ev_1e400", text.as_bytes(), false);

    // [-0.1, 1.1]: sibling sum stays exactly 1.0 (isolating the per-element bound rule from
    // the sibling-sum rule), but each value is individually outside [0, 1].
    let weights = format!("[{},{}]", json_row("1", &[(0, "-0.1")]), json_row("0", &[(0, "1.1")]));
    let text = nodes_json_text("bad", "[]", "UTG", baseline_actions(), &weights, None, "[]");
    run_quarantine_case("out_of_domain_weights", text.as_bytes(), false);

    // Undeclared all-zero class: class 0 sums to exactly 0 across both actions but is not
    // listed in `unreachable_classes`.
    let weights = format!("[{},{}]", json_row("1", &[(0, "0")]), json_row("0", &[]));
    let text = nodes_json_text("bad", "[]", "UTG", baseline_actions(), &weights, None, "[]");
    run_quarantine_case("undeclared_zero_class", text.as_bytes(), false);

    // The exact "0.4" starter-matrix defect (R5), now isolated through the store with
    // another bundle active, not just through a bare `decode` call: class 0's first action
    // weight becomes 0.4 while every other class still sums to 1.
    let weights = format!("[{},{}]", json_row("1", &[(0, "0.4")]), json_row("0", &[]));
    let text = nodes_json_text("bad", "[]", "UTG", baseline_actions(), &weights, None, "[]");
    run_quarantine_case("malformed_weight_0_4", text.as_bytes(), false);

    // Malformed/truncated JSON (R5), through the store: hashed correctly (a hash is well
    // defined over any bytes), but `decode` fails to parse it at all.
    run_quarantine_case("malformed_truncated_json", br#"{"nodes": ["#, false);

    // Unknown history step token "mystery" (R5), through the store with a fresh valid
    // control -- not the starter test's carried-over 0.4 sibling-sum defect.
    let text = nodes_json_text("bad", r#"[["UTG","mystery",0]]"#, "HJ", baseline_actions(), &baseline_weights(), None, "[]");
    run_quarantine_case("unknown_history_token", text.as_bytes(), false);

    // Hash mismatch: otherwise fully valid content, but the manifest's sha256 is wrong.
    let text = baseline_nodes_json_text("bad");
    run_quarantine_case("hash_mismatch", text.as_bytes(), true);

    // 64 MiB + 1 (R5): padded via a free-form action `label`, not via invalidity, so
    // removing the size gate would make this exact content decode successfully -- proving
    // the rejection is genuinely about size, not an incidental JSON defect (the old version
    // padded with `vec![b' '; N]`, which is *also* invalid JSON independent of its length).
    let oversized = padded_valid_nodes_json_at_exactly("bad", (MAX_BUNDLE_BYTES + 1) as usize);
    run_quarantine_case("oversized_otherwise_valid", &oversized, false);

    // R2: an empty-history node whose declared actor is BB, not the next eligible actor
    // (UTG) -- otherwise fully valid content.
    let text = nodes_json_text("bad", "[]", "BB", baseline_actions(), &baseline_weights(), None, "[]");
    run_quarantine_case("r2_empty_history_wrong_actor", text.as_bytes(), false);

    // R2: history [(UTG, fold)] with actor UTG -- the same seat that just folded, so it is
    // no longer eligible to act, let alone act again immediately.
    let text = nodes_json_text("bad", r#"[["UTG","fold",0]]"#, "UTG", baseline_actions(), &baseline_weights(), None, "[]");
    run_quarantine_case("r2_folded_actor", text.as_bytes(), false);

    // R3: an otherwise byte-identical bundle, labelled ChartTranscription, whose envelope
    // still carries EV data -- must be rejected at load time, not just filtered at lookup.
    let hash = sha256_hex(NODES_JSON);
    let manifest_text = minimal_manifest_json(&good_bundle_id(), "ChartTranscription", "[0.5,1.0]", &hash);
    run_quarantine_case_with_manifest("r3_chart_with_ev", &manifest_text, NODES_JSON);

    // R4: each source blind independently off by a value that rounds to the accepted `f32`
    // on narrowing -- the wide `f64` check must catch this before `BundleInfo` narrows it.
    let text = baseline_nodes_json_text("bad");
    let hash = sha256_hex(text.as_bytes());
    let manifest_text = minimal_manifest_json("bad", "PokerDataJson", "[0.500000001,1.0]", &hash);
    run_quarantine_case_with_manifest("r4_blind_sb_near_miss", &manifest_text, text.as_bytes());

    let manifest_text = minimal_manifest_json("bad", "PokerDataJson", "[0.5,1.000000001]", &hash);
    run_quarantine_case_with_manifest("r4_blind_bb_near_miss", &manifest_text, text.as_bytes());
}

/// R5's positive boundary control for the 64 MiB size gate: otherwise-valid content at
/// exactly `MAX_BUNDLE_BYTES` must load successfully (the gate is strictly greater-than).
#[test]
fn oversized_boundary_control_at_exactly_max_bytes_loads_successfully() {
    let bundle_id = "boundary_ok";
    let nodes_bytes = padded_valid_nodes_json_at_exactly(bundle_id, MAX_BUNDLE_BYTES as usize);
    let manifest_text = minimal_manifest_json(bundle_id, "PokerDataJson", "[0.5,1.0]", &sha256_hex(&nodes_bytes));

    let dir = std::env::temp_dir().join(format!("core_preflop_p3t2_boundary_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(bundle_id)).unwrap();
    std::fs::write(dir.join(bundle_id).join("manifest.json"), manifest_text.as_bytes()).unwrap();
    std::fs::write(dir.join(bundle_id).join("nodes.json"), &nodes_bytes).unwrap();

    let (store, banners) = core_preflop::PreflopStore::open(&dir);
    assert_eq!(banners.len(), 0, "an exactly-MAX_BUNDLE_BYTES, otherwise valid bundle must load: {banners:?}");
    assert!(store.bundle_of(bundle_id).is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

/// R3 positive control: an EV-free chart bundle loads successfully, and its lookup result
/// carries `ev_source_sb = None` (trivially true here since none was ever present -- see
/// `chart_lookup_clears_ev_even_when_constructed_directly` below for the case where EV data
/// *is* present in the underlying map and must still be cleared).
#[test]
fn chart_bundle_without_ev_loads_with_none() {
    let bundle_id = "chart_ok";
    let nodes_text = baseline_nodes_json_text(bundle_id);
    let hash = sha256_hex(nodes_text.as_bytes());
    let manifest_text = minimal_manifest_json(bundle_id, "ChartTranscription", "[0.5,1.0]", &hash);

    let manifest: core_preflop::BundleInfo = serde_json::from_str(&manifest_text).unwrap();
    let envelope = core_preflop::checked_envelope(&manifest, nodes_text.as_bytes()).expect("EV-free chart bundle must validate");
    let map = core_preflop::build_node_map(&manifest, &envelope).unwrap();
    let chart = core_preflop::ChartTranscription { info: manifest, nodes: map };
    let key = core_preflop::PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history: vec![] };
    let node = chart.lookup(&key).expect("EV-free chart bundle must load");
    assert_eq!(node.ev_source_sb, None);
}

/// R3: bypasses `load_bundle`/`checked_envelope` entirely, proving `ChartTranscription::
/// lookup`'s EV-clearing is a property of the adapter itself, not just something load-time
/// validation happens to guarantee.
#[test]
fn chart_lookup_clears_ev_even_when_constructed_directly() {
    let info: core_preflop::BundleInfo = serde_json::from_slice(MANIFEST_JSON).unwrap();
    let envelope = core_preflop::checked_envelope(&info, NODES_JSON).unwrap();
    let map = core_preflop::build_node_map(&info, &envelope).unwrap();
    let key = core_preflop::PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history: vec![] };
    // Sanity: this map really does carry a live EV before being wrapped as a chart adapter.
    assert!(map[&core_preflop::node_key(&key)].ev_source_sb.is_some());

    let chart = core_preflop::ChartTranscription { info, nodes: map };
    let node = chart.lookup(&key).unwrap();
    assert_eq!(node.ev_source_sb, None, "ChartTranscription::lookup must clear EV regardless of what is stored");
}

/// R6 regression: a malformed node late in a multi-node bundle must be identified by its own
/// history, not the first node's (which stays valid and empty here).
#[test]
fn node_validation_error_names_the_offending_nodes_own_history() {
    let mut e = base_envelope(); // node 0: empty history, actor UTG, valid.
    let mut second = e.nodes[0].clone();
    second.history = vec![("UTG".into(), "raise".into(), 2500)];
    second.actor = "HJ".into();
    e.nodes.push(second);
    e.nodes[1].weights[0][3] = 0.0; // corrupt only the second node: an undeclared zero class.

    let err = validate(&e).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("raise") && msg.contains("2500"), "message must name the second node's own history: {msg}");
    assert!(!msg.contains("node 0"), "message must identify the second (index 1) node, not the first: {msg}");
}

// --- R1: filesystem containment must be link-safe, not merely lexical ---

/// Creates a Windows directory junction at `link` -> `target` via `mklink /J` (unlike a true
/// symbolic link, this needs no elevated privilege). Returns `false` and prints an explicit
/// reason if creation is denied in this environment, so a caller can skip the link-dependent
/// assertions explicitly -- never silently pass.
fn try_make_junction(link: &Path, target: &Path) -> bool {
    let out = std::process::Command::new("cmd.exe").args(["/C", "mklink", "/J", &link.display().to_string(), &target.display().to_string()]).output();
    match out {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            eprintln!(
                "skipping junction-dependent assertions: mklink /J denied (exit {:?}): {}{}",
                o.status.code(),
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            false
        }
        Err(e) => {
            eprintln!("skipping junction-dependent assertions: could not run mklink: {e}");
            false
        }
    }
}

/// Creates a true Windows file symlink (needs `SeCreateSymbolicLinkPrivilege`, e.g.
/// Developer Mode or an elevated process -- unlike a junction, there is no unprivileged
/// equivalent for a single file). Returns `false` and prints an explicit reason if denied.
fn try_make_file_symlink(link: &Path, target: &Path) -> bool {
    match std::os::windows::fs::symlink_file(target, link) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("skipping file-symlink-dependent assertions: symlink_file denied: {e}");
            false
        }
    }
}

#[test]
fn open_rejects_external_directory_link_without_touching_its_target() {
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("core_preflop_p3t2_r1_link_{pid}"));
    let external = std::env::temp_dir().join(format!("core_preflop_p3t2_r1_external_{pid}"));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&external).unwrap();

    // A fully valid bundle, planted OUTSIDE the store's directory.
    std::fs::write(external.join("manifest.json"), MANIFEST_JSON).unwrap();
    std::fs::write(external.join("nodes.json"), NODES_JSON).unwrap();

    let link = root.join("evil");
    if !try_make_junction(&link, &external) {
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&external);
        return;
    }

    let (store, banners) = core_preflop::PreflopStore::open(&root);
    assert!(store.bundle_of(&good_bundle_id()).is_none(), "a bundle reached through a directory link must never load: {banners:?}");
    assert!(!banners.is_empty(), "the link entry must be rejected with a banner");
    // External content must remain completely untouched: still present at its own path,
    // still byte-identical, never renamed or read-and-rewritten.
    assert_eq!(std::fs::read(external.join("nodes.json")).unwrap(), NODES_JSON);
    assert_eq!(std::fs::read(external.join("manifest.json")).unwrap(), MANIFEST_JSON);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&external);
}

#[test]
fn open_rejects_bundle_whose_input_file_is_a_symlink() {
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("core_preflop_p3t2_r1_filelink_{pid}"));
    let external = std::env::temp_dir().join(format!("core_preflop_p3t2_r1_filelink_ext_{pid}"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(dir.join("sneaky")).unwrap();
    std::fs::create_dir_all(&external).unwrap();

    // A real manifest.json inside a genuine child directory, but nodes.json is a symlink to
    // external content.
    std::fs::write(dir.join("sneaky").join("manifest.json"), MANIFEST_JSON).unwrap();
    std::fs::write(external.join("nodes.json"), NODES_JSON).unwrap();
    let link = dir.join("sneaky").join("nodes.json");

    if !try_make_file_symlink(&link, &external.join("nodes.json")) {
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&external);
        return;
    }

    let (store, banners) = core_preflop::PreflopStore::open(&dir);
    assert!(store.bundle_of(&good_bundle_id()).is_none(), "a bundle read through a linked input file must never load: {banners:?}");
    assert!(!banners.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&external);
}

#[test]
fn quarantine_name_skips_a_name_already_occupied_by_a_plain_entry() {
    let dir = std::env::temp_dir().join(format!("core_preflop_p3t2_r1_occupied_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // A bad bundle that will need quarantining.
    let text = nodes_json_text("bad", "[]", "BB", baseline_actions(), &baseline_weights(), None, "[]"); // R2 defect: wrong actor
    let hash = sha256_hex(text.as_bytes());
    let manifest_text = minimal_manifest_json("bad", "PokerDataJson", "[0.5,1.0]", &hash);
    std::fs::create_dir_all(dir.join("bad")).unwrap();
    std::fs::write(dir.join("bad").join("manifest.json"), manifest_text.as_bytes()).unwrap();
    std::fs::write(dir.join("bad").join("nodes.json"), text.as_bytes()).unwrap();

    // Pre-occupy the first quarantine name with an ordinary file.
    std::fs::write(dir.join("bad.bad"), b"already here").unwrap();

    let (_, banners) = core_preflop::PreflopStore::open(&dir);
    assert_eq!(banners.len(), 1, "{banners:?}");
    assert!(dir.join("bad.1.bad").exists(), "the smallest free name must be bad.1.bad, since bad.bad is occupied");
    assert_eq!(std::fs::read(dir.join("bad.bad")).unwrap(), b"already here", "the pre-existing bad.bad entry must be untouched");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quarantine_name_treats_a_dangling_junction_as_occupied() {
    let dir = std::env::temp_dir().join(format!("core_preflop_p3t2_r1_dangling_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let text = nodes_json_text("bad", "[]", "BB", baseline_actions(), &baseline_weights(), None, "[]"); // R2 defect: wrong actor
    let hash = sha256_hex(text.as_bytes());
    let manifest_text = minimal_manifest_json("bad", "PokerDataJson", "[0.5,1.0]", &hash);
    std::fs::create_dir_all(dir.join("bad")).unwrap();
    std::fs::write(dir.join("bad").join("manifest.json"), manifest_text.as_bytes()).unwrap();
    std::fs::write(dir.join("bad").join("nodes.json"), text.as_bytes()).unwrap();

    // A dangling junction at the first quarantine name: `Path::exists` reports `false` for
    // this (it tries to resolve the target and fails), which is exactly the bug R1 fixes.
    let target = dir.join("dangling_target");
    std::fs::create_dir_all(&target).unwrap();
    let dangling = dir.join("bad.bad");
    let have_junction = try_make_junction(&dangling, &target);
    std::fs::remove_dir_all(&target).unwrap();
    if !have_junction {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert!(!dangling.exists(), "sanity: Path::exists must report false for this dangling link");
    assert!(std::fs::symlink_metadata(&dangling).is_ok(), "sanity: symlink_metadata must still see the link itself");

    let (_, banners) = core_preflop::PreflopStore::open(&dir);
    assert_eq!(banners.len(), 1, "{banners:?}");
    assert!(dir.join("bad.1.bad").exists(), "bad.bad is occupied by a dangling link, so the free name must be bad.1.bad");

    let _ = std::fs::remove_dir_all(&dir);
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
