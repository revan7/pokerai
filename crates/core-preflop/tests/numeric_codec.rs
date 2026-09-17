//! Wire-numeric codec tests for `EnvelopeNode::weights`/`evs` (pre-review fix, P3.T1):
//! every JSON number is read as `f64` and domain-checked before narrowing to `f32`, and
//! re-checked the same way on serialize, so a value that would otherwise round into range
//! during narrowing (or a non-finite/out-of-domain in-memory value built directly in Rust)
//! can never pass either serde direction silently. `validate()`'s own structural rules
//! (shape, duplicates, sibling sums) are exercised separately in `tests/envelope.rs`; these
//! tests only cover the serde-boundary numeric domain, so they deserialize `Envelope`
//! directly rather than going through `decode`/`validate`.

use core_preflop::{Envelope, EnvelopeAction, EnvelopeNode};

fn base_envelope() -> Envelope {
    Envelope {
        bundle_id: "synthetic".into(),
        depth_bb: 100,
        rake_profile: "5% cap 0.5bb".into(),
        straddle: false,
        class_order: "A-2 row-major, section 4.1".into(),
        nodes: vec![EnvelopeNode {
            history: vec![],
            actor: "UTG".into(),
            actions: vec![EnvelopeAction { step: "fold".into(), to_bb_x1000: None, label: None }],
            weights: vec![vec![0.0; 169]],
            evs: None,
            unreachable_classes: vec![],
        }],
    }
}

/// A 169-entry JSON array of `"0"` with `tok` substituted at `idx`.
fn row_json(idx: usize, tok: &str) -> String {
    let mut parts = vec!["0".to_string(); 169];
    parts[idx] = tok.to_string();
    format!("[{}]", parts.join(","))
}

fn envelope_json_with_weight(tok: &str) -> String {
    format!(
        r#"{{"bundle_id":"s","depth_bb":100,"rake_profile":"r","straddle":false,"class_order":"A-2 row-major, section 4.1","nodes":[{{"history":[],"actor":"UTG","actions":[{{"step":"fold"}}],"weights":[{}],"unreachable_classes":[]}}]}}"#,
        row_json(0, tok)
    )
}

fn envelope_json_with_ev(tok: &str) -> String {
    format!(
        r#"{{"bundle_id":"s","depth_bb":100,"rake_profile":"r","straddle":false,"class_order":"A-2 row-major, section 4.1","nodes":[{{"history":[],"actor":"UTG","actions":[{{"step":"fold"}}],"weights":[{}],"evs":[{}],"unreachable_classes":[]}}]}}"#,
        row_json(0, "0"),
        row_json(0, tok)
    )
}

// --- decode-side: wide (f64) domain check before narrowing to f32 ---

#[test]
fn weight_rejects_rounding_boundary_just_above_one() {
    // 1.00000001 narrows to f32 1.0 if narrowing happens before the domain check.
    assert!(serde_json::from_str::<Envelope>(&envelope_json_with_weight("1.00000001")).is_err());
}

#[test]
fn weight_rejects_rounding_boundary_just_below_zero() {
    // -1e-50 narrows to f32 -0.0 if narrowing happens before the domain check.
    assert!(serde_json::from_str::<Envelope>(&envelope_json_with_weight("-1e-50")).is_err());
}

#[test]
fn weight_accepts_domain_endpoints() {
    assert!(serde_json::from_str::<Envelope>(&envelope_json_with_weight("0")).is_ok());
    assert!(serde_json::from_str::<Envelope>(&envelope_json_with_weight("1")).is_ok());
}

#[test]
fn ev_rejects_overflow_on_narrow_to_f32() {
    // 1e39 is finite as f64 but overflows to infinity when narrowed to f32.
    assert!(serde_json::from_str::<Envelope>(&envelope_json_with_ev("1e39")).is_err());
}

#[test]
fn ev_rejects_value_already_infinite_at_f64() {
    // 1e400 is syntactically a valid JSON number but parses to f64::INFINITY.
    assert!(serde_json::from_str::<Envelope>(&envelope_json_with_ev("1e400")).is_err());
}

#[test]
fn ev_accepts_none_and_finite_values() {
    assert!(serde_json::from_str::<Envelope>(&envelope_json_with_ev("2.5")).is_ok());
    assert!(serde_json::from_str::<Envelope>(&envelope_json_with_ev("null")).is_ok());
}

// --- encode-side: an in-memory value built directly in Rust, not via decode ---

#[test]
fn encode_rejects_nan_weight() {
    let mut e = base_envelope();
    e.nodes[0].weights[0][0] = f32::NAN;
    assert!(serde_json::to_string(&e).is_err());
}

#[test]
fn encode_rejects_out_of_domain_weight() {
    let mut e = base_envelope();
    e.nodes[0].weights[0][0] = 1.5;
    assert!(serde_json::to_string(&e).is_err());
}

#[test]
fn encode_rejects_nan_ev() {
    let mut e = base_envelope();
    let mut row = vec![Some(0.0f32); 169];
    row[0] = Some(f32::NAN);
    e.nodes[0].evs = Some(vec![row]);
    assert!(serde_json::to_string(&e).is_err());
}

#[test]
fn encode_rejects_infinite_ev() {
    let mut e = base_envelope();
    let mut row = vec![Some(0.0f32); 169];
    row[0] = Some(f32::INFINITY);
    e.nodes[0].evs = Some(vec![row]);
    assert!(serde_json::to_string(&e).is_err());
}

// --- round trip ---

#[test]
fn valid_envelope_round_trips_unchanged() {
    let mut e = base_envelope();
    e.nodes[0].weights = vec![vec![1.0; 169]];
    let mut ev_row = vec![Some(0.25f32); 169];
    ev_row[3] = None;
    e.nodes[0].evs = Some(vec![ev_row]);

    let text = serde_json::to_string(&e).expect("valid envelope must encode");
    let decoded: Envelope = serde_json::from_str(&text).expect("valid envelope must decode");

    assert_eq!(decoded.bundle_id, e.bundle_id);
    assert_eq!(decoded.nodes[0].weights, e.nodes[0].weights);
    assert_eq!(decoded.nodes[0].evs, e.nodes[0].evs);
}
