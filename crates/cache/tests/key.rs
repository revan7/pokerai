#[test]
fn normalized_rationals_do_not_embed_chip_scale() {
    use cache::key::{Rational,spr_bucket};
    assert_eq!(Rational::new(500,100).unwrap(),Rational::new(1000,200).unwrap());
    assert_eq!(Rational::new(5000,100_000).unwrap(),Rational::new(10000,200_000).unwrap());
    assert_eq!(spr_bucket(Rational::new(500,100).unwrap()),81);
    assert!(Rational::new(1,0).is_err());
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
/// `SCENARIO_DIGEST` below.
#[test]
fn key_fields_digest_matches_a_frozen_independently_computed_value() {
    use cache::key::{KeyFields, Model, RakeKey, Rational};
    let key = KeyFields {
        schema_version: 3,
        solver_commit: "9d1509fe5077d019825f833eed04b16d342dfda1".to_string(),
        adapter_version: 1,
        rules_version: 3,
        canonical_board: vec!["2c".parse().unwrap(), "7d".parse().unwrap(), "Ks".parse().unwrap()],
        root_street: proto::Street::Flop,
        spr_bucket: 81,
        tree_signature: "flop_fast_v1".to_string(),
        rake: RakeKey { rate_bits: 0.05f32.to_bits(), cap_over_p: Rational::new(1, 20).unwrap(), collection_rule_version: 1 },
        range_hash_oop: [0u8; 32],
        range_hash_ip: [1u8; 32],
        model: Model::Baseline,
    };

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
