use cache::key::{KeyFields, Model, RakeKey, Rational};

#[test]
fn normalized_rationals_do_not_embed_chip_scale() {
    use cache::key::{Rational, spr_bucket};
    assert_eq!(Rational::new(500,100).unwrap(),Rational::new(1000,200).unwrap());
    assert_eq!(Rational::new(5000,100_000).unwrap(),Rational::new(10000,200_000).unwrap());
    assert_eq!(spr_bucket(Rational::new(500,100).unwrap()),81);
    assert!(Rational::new(1,0).is_err());
}

/// A concrete, fully valid `KeyFields`, shared by the digest and boundary-assertion tests
/// below so each test only varies the one thing it's checking.
fn sample_key() -> KeyFields {
    KeyFields {
        schema_version: 3,
        solver_commit: "9d1509fe5077d019825f833eed04b16d342dfda1".to_string(),
        adapter_version: 1,
        rules_version: 3,
        canonical_board: vec!["2c".parse().unwrap(), "7d".parse().unwrap(), "Ks".parse().unwrap()],
        root_street: proto::Street::Flop,
        spr_bucket: 81,
        tree_signature: "flop_fast_v1".to_string(),
        rake: RakeKey::new(0.05, Rational::new(1, 20).unwrap(), 1).unwrap(),
        range_hash_oop: [0u8; 32],
        range_hash_ip: [1u8; 32],
        model: Model::Baseline,
    }
}

/// Frozen expected digests pin `KeyFields::digest`/`scenario_identity` to their exact byte
/// layout so a future change to field order, JSON shape, or hash input is caught as a
/// regression rather than silently reshuffling every on-disk cache entry.
///
/// Independently verified (not just asserted against Rust's own output) with:
/// ```text
/// python3 -c "
/// import hashlib
/// json1 = '{\"schema_version\":3,\"solver_commit\":\"9d1509fe5077d019825f833eed04b16d342dfda1\",\"adapter_version\":1,\"rules_version\":3,\"canonical_board\":[\"2c\",\"7d\",\"Ks\"],\"root_street\":\"flop\",\"spr_bucket\":81,\"tree_signature\":\"flop_fast_v1\",\"rake\":{\"rate_bits\":1028443341,\"cap_over_p\":{\"num\":1,\"den\":20},\"collection_rule_version\":1},\"range_hash_oop\":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],\"range_hash_ip\":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],\"model\":\"Baseline\"}'
/// print(hashlib.sha256(json1.encode('utf-8')).hexdigest())
/// "
/// # -> b11f944a8f3a985c62d07e740cf06d418b47d56838a2931b3de28ad171612d8c
/// ```
/// and the analogous computation over the `(at_bucket(0), spr)` tuple JSON for
/// `SCENARIO_DIGEST` below. Fix round 1 (review R2/R3) changed `RakeKey`/`Rational`
/// construction to go through checked constructors instead of struct literals, but the
/// serialized JSON *shape* (field names, value bytes) is unchanged for this same valid
/// input, and both frozen digests below are unchanged from before the fix -- reconfirmed
/// with the same Python computation after the fix (see task-1-report.md "Fix round 1").
#[test]
fn key_fields_digest_matches_a_frozen_independently_computed_value() {
    let key = sample_key();

    const DIGEST: [u8; 32] = [
        0xb1, 0x1f, 0x94, 0x4a, 0x8f, 0x3a, 0x98, 0x5c, 0x62, 0xd0, 0x7e, 0x74, 0x0c, 0xf0, 0x6d,
        0x41, 0x8b, 0x47, 0xd5, 0x68, 0x38, 0xa2, 0x93, 0x1b, 0x3d, 0xe2, 0x8a, 0xd1, 0x71, 0x61,
        0x2d, 0x8c,
    ];
    assert_eq!(key.digest(), DIGEST);

    let spr = Rational::new(500, 100).unwrap();
    const SCENARIO_DIGEST: [u8; 32] = [
        0x4c, 0xc7, 0x35, 0x28, 0xd4, 0x80, 0x95, 0x9c, 0x7c, 0x39, 0xb2, 0x20, 0xdf, 0x9f, 0xce,
        0x7a, 0xb2, 0x07, 0xfb, 0x15, 0x11, 0x1e, 0x33, 0x89, 0x03, 0xee, 0xac, 0xa7, 0x8b, 0xfe,
        0x12, 0xa1,
    ];
    assert_eq!(key.scenario_identity(spr), SCENARIO_DIGEST);

    // `scenario_identity` normalizes away the bucket: a key that differs only in
    // `spr_bucket` still has the same scenario identity for the same exact SPR.
    assert_eq!(key.at_bucket(7).scenario_identity(spr), SCENARIO_DIGEST);
}

// --- R1: positive-SPR boundaries (Rational itself keeps accepting a zero numerator) ---

#[test]
fn rational_new_still_accepts_a_zero_numerator() {
    // A zero rake cap is a legitimate rational; only the SPR-specific boundaries below
    // reject zero.
    let zero = Rational::new(0, 100).unwrap();
    assert_eq!(zero, Rational::new(0, 1).unwrap());
}

#[test]
#[should_panic(expected = "spr_bucket: SPR must be positive")]
fn spr_bucket_rejects_zero_spr() {
    use cache::key::spr_bucket;
    spr_bucket(Rational::new(0, 100).unwrap());
}

#[test]
fn spr_bucket_sub_unit_spr_gives_a_negative_bucket() {
    use cache::key::spr_bucket;
    // Independently verified: python3 -c "import math; print(round(math.log(1/2)/math.log(1.02)))" -> -35
    assert_eq!(spr_bucket(Rational::new(1, 2).unwrap()), -35);
}

#[test]
#[should_panic(expected = "scenario_identity: SPR must be positive")]
fn scenario_identity_rejects_zero_spr() {
    let key = sample_key();
    let _ = key.scenario_identity(Rational::new(0, 50).unwrap());
}

// --- R2: RakeKey rejects a non-finite/out-of-range rate on construction and on decode ---

#[test]
fn rake_key_new_rejects_negative_nan_and_infinite_rates() {
    let cap = Rational::new(1, 20).unwrap();
    assert!(RakeKey::new(-0.05, cap, 1).is_err());
    assert!(RakeKey::new(f32::NAN, cap, 1).is_err());
    assert!(RakeKey::new(f32::INFINITY, cap, 1).is_err());
    assert!(RakeKey::new(f32::NEG_INFINITY, cap, 1).is_err());
    // Half-open upper bound: 1.0 itself is out of [0, 1) and must be rejected, not clamped.
    assert!(RakeKey::new(1.0, cap, 1).is_err());
}

#[test]
fn rake_key_new_accepts_zero_rate_and_zero_cap() {
    let zero_cap = Rational::new(0, 1).unwrap();
    assert!(RakeKey::new(0.0, zero_cap, 1).is_ok());
}

#[test]
fn rake_key_decode_rejects_negative_nan_and_infinite_rate_bit_patterns() {
    for bits in [(-0.05f32).to_bits(), f32::NAN.to_bits(), f32::INFINITY.to_bits(), f32::NEG_INFINITY.to_bits()] {
        let json = format!(
            r#"{{"rate_bits":{bits},"cap_over_p":{{"num":1,"den":20}},"collection_rule_version":1}}"#
        );
        assert!(
            serde_json::from_str::<RakeKey>(&json).is_err(),
            "rate bit pattern {bits:#010x} should be rejected on decode"
        );
    }
}

#[test]
fn rake_key_decode_accepts_zero_rate_and_zero_cap() {
    let bits = 0.0f32.to_bits();
    let json = format!(r#"{{"rate_bits":{bits},"cap_over_p":{{"num":0,"den":1}},"collection_rule_version":1}}"#);
    assert!(serde_json::from_str::<RakeKey>(&json).is_ok());
}

#[test]
fn rake_key_decode_rejects_a_non_canonical_nested_cap_over_p() {
    let bits = 0.05f32.to_bits();
    let json = format!(
        r#"{{"rate_bits":{bits},"cap_over_p":{{"num":5000,"den":100000}},"collection_rule_version":1}}"#
    );
    assert!(serde_json::from_str::<RakeKey>(&json).is_err());
}

// --- R3: Rational decode rejects a zero denominator and a non-reduced encoding ---

#[test]
fn rational_decode_rejects_zero_denominator() {
    assert!(serde_json::from_str::<Rational>(r#"{"num":1,"den":0}"#).is_err());
}

#[test]
fn rational_decode_rejects_non_reduced_encoding_while_canonical_form_round_trips() {
    assert!(serde_json::from_str::<Rational>(r#"{"num":500,"den":100}"#).is_err());
    let decoded: Rational = serde_json::from_str(r#"{"num":5,"den":1}"#).unwrap();
    assert_eq!(decoded, Rational::new(5, 1).unwrap());
}

#[test]
fn equivalent_inputs_built_through_new_compare_equal_and_digest_identically() {
    // Two keys differing only in how an equal rational (rake cap) was constructed must
    // still compare equal and produce the same digest -- the identity lives in the reduced
    // value, not the construction path.
    let mut a = sample_key();
    let mut b = sample_key();
    a.rake = RakeKey::new(0.05, Rational::new(1, 20).unwrap(), 1).unwrap();
    b.rake = RakeKey::new(0.05, Rational::new(5000, 100_000).unwrap(), 1).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.digest(), b.digest());
}
