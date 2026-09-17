//! Metadata numeric codec tests for `BundleInfo::source_blinds` and
//! `RakeProfile::rate`/`cap_bb` (fix round 1, R2): these fields kept plain derived float
//! deserialization even after the P3.T1 pre-review fix covered `EnvelopeNode`'s matrices,
//! so a wire blind pair `[0.50000000001, 1.00000001]` decoded to exactly `[0.5, 1.0]` and a
//! rake `{"rate":-1e-50,"cap_bb":1e39,...}` decoded to `rate = -0.0`, `cap_bb = inf` and then
//! serialized `cap_bb` back out as JSON `null`. Same wide-`f64`-before-narrow discipline as
//! `tests/numeric_codec.rs`, applied to these two metadata types directly.

use core_preflop::{BundleInfo, EvReference, RakeProfile, SourceKind};

fn base_bundle_info() -> BundleInfo {
    BundleInfo {
        bundle_id: "b".into(),
        source: SourceKind::PokerDataJson,
        depth_bb: 100,
        depths: vec![100],
        source_blinds: [0.5, 1.0],
        rake_profile: "5% cap 0.5bb".into(),
        rake: Some(RakeProfile { rate: 0.05, cap_bb: 0.5, no_flop_no_drop: true }),
        straddle: false,
        version: 2,
        game: "nl".into(),
        ev_unit: "source_sb".into(),
        ev_reference: EvReference::Unverified,
        license_note: "note".into(),
        accuracy: "unverified".into(),
        sha256: "deadbeef".into(),
    }
}

fn bundle_info_json_with_blinds(blinds: &str) -> String {
    format!(
        r#"{{"bundle_id":"b","source":"PokerDataJson","depth_bb":100,"depths":[100],"source_blinds":{blinds},"rake_profile":"r","rake":null,"straddle":false,"version":2,"game":"nl","ev_unit":"source_sb","ev_reference":"unverified","license_note":"n","accuracy":"unverified","sha256":"h"}}"#
    )
}

// --- source_blinds ---

#[test]
fn source_blinds_rejects_rounding_boundary_values() {
    // The domain (finite, strictly positive, second >= first) has no upper bound, so
    // unlike `weights`' `[0, 1]` domain, a value merely close to a round number (e.g.
    // 1.00000001) is not itself a boundary violation -- it stays a legitimate positive
    // blind either way. The domain's one real boundary is "> 0": 1e-50 is positive at the
    // wide f64 value (passes the pre-narrow check) but underflows to exactly 0.0 when
    // narrowed to f32, which fails the post-narrow re-check in `narrow_checked` (0.0 is
    // not > 0) -- this is the meaningful wide-then-narrow case for this domain.
    let json = bundle_info_json_with_blinds("[1e-50,1.0]");
    assert!(serde_json::from_str::<BundleInfo>(&json).is_err());
}

#[test]
fn source_blinds_accepts_prescribed_values() {
    let json = bundle_info_json_with_blinds("[0.5,1.0]");
    let decoded: BundleInfo = serde_json::from_str(&json).expect("valid blinds must decode");
    assert_eq!(decoded.source_blinds, [0.5, 1.0]);
}

#[test]
fn source_blinds_rejects_non_positive_and_out_of_order_on_encode() {
    let mut b = base_bundle_info();
    b.source_blinds = [0.0, 1.0]; // not strictly positive
    assert!(serde_json::to_string(&b).is_err());

    let mut b2 = base_bundle_info();
    b2.source_blinds = [0.5, 0.4]; // second < first
    assert!(serde_json::to_string(&b2).is_err());

    let mut b3 = base_bundle_info();
    b3.source_blinds = [f32::NAN, 1.0];
    assert!(serde_json::to_string(&b3).is_err());
}

#[test]
fn bundle_info_round_trips_a_valid_value() {
    let b = base_bundle_info();
    let text = serde_json::to_string(&b).expect("valid BundleInfo must encode");
    let decoded: BundleInfo = serde_json::from_str(&text).expect("valid BundleInfo must decode");
    assert_eq!(decoded, b);
}

// --- RakeProfile::rate / cap_bb ---

#[test]
fn rake_rejects_the_demonstrated_invalid_fields() {
    // rate -1e-50 (out of [0, 1) at the wide value) and cap_bb 1e39 (finite at f64,
    // overflows to inf when narrowed to f32) -- the exact R2 probe.
    let json = r#"{"rate":-1e-50,"cap_bb":1e39,"no_flop_no_drop":true}"#;
    assert!(serde_json::from_str::<RakeProfile>(json).is_err());
}

#[test]
fn rake_rate_domain_is_half_open_at_one() {
    let mut r = RakeProfile { rate: 0.0, cap_bb: 0.5, no_flop_no_drop: false };
    assert!(serde_json::to_string(&r).is_ok(), "0.0 is included");
    r.rate = 1.0;
    assert!(serde_json::to_string(&r).is_err(), "1.0 is excluded (half-open)");
}

#[test]
fn cap_bb_rejects_negative() {
    let r = RakeProfile { rate: 0.05, cap_bb: -0.1, no_flop_no_drop: false };
    assert!(serde_json::to_string(&r).is_err());
}

#[test]
fn rake_round_trips_an_ordinary_value() {
    let r = RakeProfile { rate: 0.05, cap_bb: 0.5, no_flop_no_drop: true };
    let text = serde_json::to_string(&r).expect("valid rake must encode");
    let back: RakeProfile = serde_json::from_str(&text).expect("valid rake must decode");
    assert_eq!(back, r);
}
