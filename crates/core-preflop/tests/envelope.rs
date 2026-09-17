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
    let mut e = base_envelope();
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

    e.nodes[0].weights[0][0] = -0.0001;
    assert!(validate(&e).is_err());
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
