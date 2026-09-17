//! Bincode-compatibility tests for proto's internally tagged enums (Task: P4.T2-followup).
//!
//! Origin: `#[serde(tag = "kind")]` (and `#[serde(tag = "phase")]`) internally-tagged enums
//! decode by buffering their content via `Deserializer::deserialize_any` to find the tag before
//! knowing which variant to build; bincode 1.3.3 -- the pinned cache storage format, spec 10.4 --
//! unconditionally refuses `deserialize_any` (`ErrorKind::DeserializeAnyNotSupported`, regardless
//! of the visitor). `Action` (`crates/proto/src/hand.rs`) and `ApproxReason`
//! (`crates/proto/src/recommendation.rs`) are the two internally-tagged enums reachable
//! (transitively) from `cache::entry::CacheEntry`: `Action` via
//! `EffectiveTree.materialized[].actions` / `EffectiveTree.inserted`, `ApproxReason` via
//! `CacheEntry::reasons`. Both now carry hand-written `Serialize`/`Deserialize` that keep the
//! exact tagged-map JSON form for human-readable formats (via a private mirror type that still
//! carries the original `#[serde(...)]` attributes byte-for-byte) and add a bincode-native,
//! ordinary variant-index encoding for every non-human-readable format (via a second private
//! mirror with no `#[serde(...)]` container attribute at all).
//!
//! `Street` is included below as a control: it is unit-only, was never internally tagged (plain
//! `#[serde(rename_all = "lowercase")]` derive, no `tag` attribute), and was already
//! bincode-compatible before this fix -- listed in the brief as a type to check, not one that
//! needed changing.
//!
//! `Rake`, `LegalAction` and `HandPhase` are also internally tagged (`#[serde(tag = "kind"/"phase")]`)
//! but are **not** reachable from `CacheEntry` (`Rake`/`LegalAction`/`HandPhase` only appear in
//! `GameConfig`/`HandConfig`, `Derived`/`Recommendation`, and `HandState` respectively -- none of
//! which `CacheEntry` embeds), so they are out of this task's scope and unchanged.
//!
//! This same non-self-describing-format incompatibility also affected two non-enum, hand-written
//! codecs reachable from `CacheEntry` -- `MenuSize` (`crates/proto/src/tree.rs`, whose JSON union
//! wire form used `deserialize_any` directly) and `Range1326` (`crates/proto/src/range.rs`,
//! whose `Deserialize` always read the wire as `f64` regardless of format, mismatching its own
//! `Serialize`'s native `f32` write) -- plus a *third*, independent defect in `PlayerMenus`
//! (`crates/proto/src/tree.rs`): `#[serde(skip_serializing_if = "Option::is_none")]` on `donk`
//! omits the field from the wire on every format, not just human-readable ones, desyncing
//! bincode's positional field count. All three were required, in addition to the `Action`/
//! `ApproxReason` tag fix, for `CacheEntry` (which embeds `EffectiveTree`, and so `MenuSize` and
//! `PlayerMenus`, and its own `SourceInputs.ranges: [Range1326; 2]`) to round-trip through
//! bincode at all -- see `crates/cache/tests/entry.rs`'s acceptance test. They are exercised here
//! too, alongside the two enums the brief named.

use proto::{Action, ApproxReason, MenuSize, PlayerMenus, Range1326, Seat, SideMenu, Street};

fn bincode_roundtrip<T>(v: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let bytes = bincode::serialize(v).unwrap_or_else(|e| panic!("bincode::serialize failed for {v:?}: {e}"));
    let back: T = bincode::deserialize(&bytes).unwrap_or_else(|e| panic!("bincode::deserialize failed for {v:?}: {e}"));
    assert_eq!(&back, v, "bincode round trip changed the value");
}

fn all_actions() -> Vec<Action> {
    vec![Action::Fold, Action::Check, Action::Call, Action::Bet { to: 50 }, Action::Raise { to: 250 }, Action::AllIn { to: 100 }]
}

// --- Action: JSON golden per variant (RED before this fix: identical assertions already existed
// in `hand.rs`'s `action_wire_tags` for two variants; every variant is pinned here, and every one
// must still produce the *unmodified* tagged-map form). ---
#[test]
fn action_json_goldens_per_variant() {
    assert_eq!(serde_json::to_string(&Action::Fold).unwrap(), r#"{"kind":"fold"}"#);
    assert_eq!(serde_json::to_string(&Action::Check).unwrap(), r#"{"kind":"check"}"#);
    assert_eq!(serde_json::to_string(&Action::Call).unwrap(), r#"{"kind":"call"}"#);
    assert_eq!(serde_json::to_string(&Action::Bet { to: 50 }).unwrap(), r#"{"kind":"bet","to":50}"#);
    assert_eq!(serde_json::to_string(&Action::Raise { to: 250 }).unwrap(), r#"{"kind":"raise","to":250}"#);
    assert_eq!(serde_json::to_string(&Action::AllIn { to: 100 }).unwrap(), r#"{"kind":"allin","to":100}"#);
    for a in all_actions() {
        assert_eq!(serde_json::from_str::<Action>(&serde_json::to_string(&a).unwrap()).unwrap(), a, "JSON round trip for {a:?}");
    }
}

/// GREEN evidence for the fix: before it, every one of these panicked with
/// `DeserializeAnyNotSupported` (confirmed directly against `Action::Bet { to: 50 }` in the
/// N1 comment this task's brief originates from, and reproduced again in this task's own RED
/// step against every variant).
#[test]
fn action_bincode_roundtrips_every_variant() {
    for a in all_actions() {
        bincode_roundtrip(&a);
    }
}

fn all_approx_reasons() -> Vec<ApproxReason> {
    vec![
        ApproxReason::BetTranslation { street: Street::Flop, seat: Seat(1), observed_pct: 0.5, mapped: vec![(0.5, 1.0)], deviation: 0.25, prominent: true },
        ApproxReason::DepthBucket { seat: Seat(2), actual_bb: 100.0, used_bb: 100, prominent: false },
        ApproxReason::AsymmetricStacks { stacks_bb: vec![100.0, 110.0], prominent: true },
        ApproxReason::RakeProfileMapped { actual: "5pct_cap".into(), used: "no_rake".into() },
        ApproxReason::StraddleMapped { posts: [0.5, 1.0, 1.0] },
        ApproxReason::ShortHandedMapped { dealt: 4 },
        ApproxReason::DeadlineBestSoFar { reached_bp: 40, target_bp: 50 },
        ApproxReason::ChartRounded,
        ApproxReason::EvReferenceUnverified,
        ApproxReason::UnconditionedPriorStreet { street: Street::Turn, seat: Seat(0), cause: "no board".into() },
        ApproxReason::UnconditionedCurrentStreet,
        ApproxReason::MultiwayStreetRoot { folded_this_street: 1, dead_this_street: 50 },
        ApproxReason::SprBucketed { actual: 5.5, used: 5.0 },
        ApproxReason::MenuRounded { max_delta_pct: 0.1 },
        ApproxReason::BranchResidual { seat: Seat(3), residual_mass_pct: 0.02, cause: "locked".into() },
    ]
}

/// JSON golden per variant, pinning the exact tagged-map string. Every domain-checked float
/// field above (`observed_pct`, `mapped`, `deviation`, `actual_bb`, `stacks_bb`, `posts`,
/// `actual`, `used`, `max_delta_pct`, `residual_mass_pct`) still goes through
/// `crate::numeric`'s codecs, unchanged for the human-readable path (Task: P4.T2-followup made
/// only the `deserialize` half of those codecs branch on format; `serialize` was already
/// format-agnostic).
#[test]
fn approx_reason_json_goldens_per_variant() {
    let expected = [
        r#"{"kind":"BetTranslation","street":"flop","seat":1,"observed_pct":0.5,"mapped":[[0.5,1.0]],"deviation":0.25,"prominent":true}"#,
        r#"{"kind":"DepthBucket","seat":2,"actual_bb":100.0,"used_bb":100,"prominent":false}"#,
        r#"{"kind":"AsymmetricStacks","stacks_bb":[100.0,110.0],"prominent":true}"#,
        r#"{"kind":"RakeProfileMapped","actual":"5pct_cap","used":"no_rake"}"#,
        r#"{"kind":"StraddleMapped","posts":[0.5,1.0,1.0]}"#,
        r#"{"kind":"ShortHandedMapped","dealt":4}"#,
        r#"{"kind":"DeadlineBestSoFar","reached_bp":40,"target_bp":50}"#,
        r#"{"kind":"ChartRounded"}"#,
        r#"{"kind":"EvReferenceUnverified"}"#,
        r#"{"kind":"UnconditionedPriorStreet","street":"turn","seat":0,"cause":"no board"}"#,
        r#"{"kind":"UnconditionedCurrentStreet"}"#,
        r#"{"kind":"MultiwayStreetRoot","folded_this_street":1,"dead_this_street":50}"#,
        r#"{"kind":"SprBucketed","actual":5.5,"used":5.0}"#,
        r#"{"kind":"MenuRounded","max_delta_pct":0.1}"#,
        r#"{"kind":"BranchResidual","seat":3,"residual_mass_pct":0.02,"cause":"locked"}"#,
    ];
    let reasons = all_approx_reasons();
    assert_eq!(reasons.len(), expected.len(), "one golden per variant");
    for (r, want) in reasons.iter().zip(expected.iter()) {
        let got = serde_json::to_string(r).unwrap();
        assert_eq!(&got, want, "JSON golden mismatch for {r:?}");
        assert_eq!(&serde_json::from_str::<ApproxReason>(&got).unwrap(), r, "JSON round trip for {r:?}");
    }
}

/// GREEN evidence for the fix: before it, every variant with a `crate::numeric`-codec'd field
/// additionally failed for a *second*, independent reason even once the tag was fixed in
/// isolation -- those codecs read a wide `f64` unconditionally, mismatching their own `f32`
/// write under bincode. Both defects are exercised together here since fixing only one would
/// still leave every such variant failing.
#[test]
fn approx_reason_bincode_roundtrips_every_variant() {
    for r in all_approx_reasons() {
        bincode_roundtrip(&r);
    }
}

// --- Street: unit-only, not internally tagged, already bincode-compatible (control). ---
#[test]
fn street_bincode_roundtrips_every_variant() {
    for s in [Street::Preflop, Street::Flop, Street::Turn, Street::River] {
        bincode_roundtrip(&s);
    }
}

// --- MenuSize: hand-written codec whose JSON union form uses `deserialize_any` directly (not an
// internally-tagged *derive*, so outside the brief's literal enumeration, but the same root cause
// -- bincode's Deserializer refuses `deserialize_any` unconditionally -- and reachable from
// `CacheEntry` via `EffectiveTree.menus`, so it blocked the same acceptance test). ---
#[test]
fn menu_size_bincode_roundtrips_every_variant() {
    bincode_roundtrip(&MenuSize::Pot(0.5));
    bincode_roundtrip(&MenuSize::AllIn);
}

#[test]
fn menu_size_json_wire_form_is_unchanged() {
    assert_eq!(serde_json::to_string(&MenuSize::Pot(2.5)).unwrap(), "2.5");
    assert_eq!(serde_json::to_string(&MenuSize::AllIn).unwrap(), r#""a""#);
}

// --- Range1326: `Serialize` already wrote a native f32 on every format; only `Deserialize`
// always read a wide f64, mismatching bincode's own write. ---
#[test]
fn range1326_bincode_roundtrips_zero_uniform_and_a_mixed_range() {
    bincode_roundtrip(&Range1326::zero());
    bincode_roundtrip(&Range1326::uniform());
    let mixed = Range1326::from_fn(|i| if i % 3 == 0 { 0.5 } else { 0.0 });
    bincode_roundtrip(&mixed);
}

/// Corrupting a bincode-encoded `Range1326`'s bytes to a NaN bit pattern must be rejected on
/// decode, matching `CachedNode`'s identical crafted-NaN probe in `crates/cache/tests/entry.rs`:
/// bincode carries no wider source value to check before narrowing, but the f32 actually read is
/// still finiteness/domain checked.
#[test]
fn range1326_bincode_decode_rejects_a_crafted_nan_bit_pattern() {
    let sentinel = 0.437_500_3_f32;
    let r = Range1326::from_fn(|i| if i == 7 { sentinel } else { 0.0 });
    let mut bytes = bincode::serialize(&r).unwrap();
    let needle = sentinel.to_le_bytes();
    let occurrences = bytes.windows(4).filter(|w| *w == needle).count();
    assert_eq!(occurrences, 1, "sentinel float must appear exactly once in the encoded bytes");
    let pos = bytes.windows(4).position(|w| w == needle).unwrap();
    bytes[pos..pos + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(bincode::deserialize::<Range1326>(&bytes).is_err());
}

// --- PlayerMenus: `#[serde(skip_serializing_if = "Option::is_none")]` on `donk` omitted the
// field from the wire on every format (not just human-readable ones), desyncing bincode's
// positional field count -- observed directly as `InvalidTagEncoding` deep in an unrelated
// `Option` read a few fields later, while fixing only `Action`/`ApproxReason`/`MenuSize`/
// `Range1326`. Both the `None` and `Some` shapes of `donk` must round-trip. ---
#[test]
fn player_menus_bincode_roundtrips_donk_none_and_some() {
    let side = SideMenu { bet: vec![MenuSize::AllIn], raise: vec![MenuSize::Pot(2.5)] };
    bincode_roundtrip(&PlayerMenus { oop: side.clone(), ip: side.clone(), donk: None });
    bincode_roundtrip(&PlayerMenus { oop: side.clone(), ip: side, donk: Some(vec![MenuSize::Pot(0.33), MenuSize::AllIn]) });
}
