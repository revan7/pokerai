//! Canonical structural cache keys (spec section 10.4) and the exact-rational SPR identity
//! they bucket. A `KeyFields` never carries seat ids, hand id, `bb_chips`, raw chip pot or
//! stacks, the wager quantum, hero's cards, `target_bp`, or the requested path -- only the
//! normalized fields a solved node's *shape* depends on. Canonical bytes are produced by
//! serializing `KeyFields` with a fixed struct field order, plain integers, strings and enum
//! tags (`serde_json`, never an unordered map), so the digest is deterministic across runs.

use crate::CacheError;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// An exact rational in lowest terms: `num / den`. Two `Rational`s that denote the same
/// value compare equal regardless of the chip scale they were built from (spec section 10.4
/// "exact SPR rational"), because `new` always reduces by the gcd before storing, and the
/// fields are private so every value -- constructed or decoded -- is forced through that
/// reduction. `Deserialize` rejects a zero denominator and a noncanonical (unreduced)
/// encoding rather than silently renormalizing it, so a persisted `500/100` can never
/// coexist with a constructed `5/1` under different byte representations and different
/// digests (review R3).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Rational {
    num: u64,
    den: u64,
}

impl Rational {
    /// Reduces `num / den` to lowest terms.
    ///
    /// # Errors
    /// `CacheError::Invalid` if `den == 0`. A zero numerator is accepted here -- a zero rake
    /// cap is a legitimate rational -- but SPR-specific call sites reject a zero SPR
    /// themselves (see `spr_bucket`, `KeyFields::scenario_identity`; review R1).
    pub fn new(num: u64, den: u64) -> Result<Self, CacheError> {
        if den == 0 {
            return Err(CacheError::Invalid("zero denominator"));
        }
        let (mut a, mut b) = (num, den);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Self { num: num / a, den: den / a })
    }

    pub fn num(self) -> u64 {
        self.num
    }

    pub fn den(self) -> u64 {
        self.den
    }

    pub fn value(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

#[derive(Deserialize)]
struct RawRational {
    num: u64,
    den: u64,
}

impl<'de> Deserialize<'de> for Rational {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Rational, D::Error> {
        let raw = RawRational::deserialize(d)?;
        let reduced = Rational::new(raw.num, raw.den)
            .map_err(|_| de::Error::custom(format!("Rational: zero denominator ({}/{})", raw.num, raw.den)))?;
        if reduced.num != raw.num || reduced.den != raw.den {
            return Err(de::Error::custom(format!(
                "Rational: {}/{} is not in canonical reduced form; expected {}/{}",
                raw.num, raw.den, reduced.num, reduced.den
            )));
        }
        Ok(reduced)
    }
}

/// `round(ln(SPR) / ln(1.02))` (spec section 10.4): the geometric SPR bucket a lookup
/// searches at `b - 1, b, b + 1`.
///
/// # Panics
/// Panics (in every build profile, not just debug) if `s.num() == 0`. The brief requires a
/// positive SPR (`P`, `eff` both positive); without this guard `ln(0) = -inf` would silently
/// saturate the `as i32` cast to `i32::MIN`, manufacturing a normal-looking bucket from an
/// invalid SPR (review R1).
pub fn spr_bucket(s: Rational) -> i32 {
    assert!(
        s.num() > 0,
        "spr_bucket: SPR must be positive (numerator > 0), got numerator 0 with denominator {}",
        s.den()
    );
    (s.value().ln() / 1.02_f64.ln()).round() as i32
}

/// The solver-strategy identity a cache entry was produced under. `Baseline` is model
/// revision 0; `Locked` pins a specific model fingerprint (spec section 10.5).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Model {
    Baseline,
    Locked { fingerprint: [u8; 32] },
}

/// The rake shape a cache entry was solved under, normalized away from raw chip amounts:
/// `rate_bits` stores the rake rate's validated bit pattern, `cap_over_p` is the rake cap
/// expressed as an exact fraction of `P` rather than raw `cap_mchips`, and
/// `collection_rule_version` pins the rule (e.g. no-flop-no-drop) that applied. `rate_bits`
/// is private: the only ways to build a `RakeKey` are `RakeKey::new` and a validating
/// `Deserialize`, both of which reject a rate that is not finite and in `[0, 1)` rather than
/// accepting an arbitrary bit pattern such as a negative rate, `+inf`, or `NaN` (review R2).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RakeKey {
    rate_bits: u32,
    pub cap_over_p: Rational,
    pub collection_rule_version: u16,
}

impl RakeKey {
    /// # Errors
    /// `CacheError::Invalid` if `rate` -- checked widened to `f64`, never clamped -- is not
    /// finite and in `[0, 1)`.
    pub fn new(rate: f32, cap_over_p: Rational, collection_rule_version: u16) -> Result<Self, CacheError> {
        let wide = rate as f64;
        if !wide.is_finite() || !(0.0..1.0).contains(&wide) {
            return Err(CacheError::Invalid("rake rate must be finite and in [0, 1)"));
        }
        Ok(Self { rate_bits: rate.to_bits(), cap_over_p, collection_rule_version })
    }

    pub fn rate(self) -> f32 {
        f32::from_bits(self.rate_bits)
    }
}

#[derive(Deserialize)]
struct RawRakeKey {
    rate_bits: u32,
    cap_over_p: Rational,
    collection_rule_version: u16,
}

impl<'de> Deserialize<'de> for RakeKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<RakeKey, D::Error> {
        let raw = RawRakeKey::deserialize(d)?;
        let rate = f32::from_bits(raw.rate_bits);
        let wide = rate as f64;
        if !wide.is_finite() || !(0.0..1.0).contains(&wide) {
            return Err(de::Error::custom(format!(
                "RakeKey: rate bit pattern {:#010x} decodes to {rate}, which is not finite and in [0, 1)",
                raw.rate_bits
            )));
        }
        Ok(RakeKey {
            rate_bits: raw.rate_bits,
            cap_over_p: raw.cap_over_p,
            collection_rule_version: raw.collection_rule_version,
        })
    }
}

/// The canonical structural cache key (spec section 10.4). Every field is normalized: no
/// seat ids, hand id, `bb_chips`, raw chip pot/stacks, wager quantum, hero's cards,
/// `target_bp`, or requested path ever enters it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyFields {
    pub schema_version: u16,
    pub solver_commit: String,
    pub adapter_version: u16,
    pub rules_version: u16,
    pub canonical_board: Vec<proto::Card>,
    pub root_street: proto::Street,
    pub spr_bucket: i32,
    pub tree_signature: String,
    pub rake: RakeKey,
    pub range_hash_oop: [u8; 32],
    pub range_hash_ip: [u8; 32],
    pub model: Model,
}

impl KeyFields {
    /// The on-disk/lookup digest of these fields, over their fixed-order canonical JSON bytes.
    pub fn digest(&self) -> [u8; 32] {
        Sha256::digest(serde_json::to_vec(self).expect("finite key")).into()
    }

    /// This key with `spr_bucket` replaced by `b`, used to probe the `b - 1, b, b + 1` search.
    pub fn at_bucket(&self, b: i32) -> Self {
        let mut k = self.clone();
        k.spr_bucket = b;
        k
    }

    /// The identity of the underlying scenario independent of bucketing: this key at bucket 0
    /// paired with the exact SPR rational, digested together. Two entries with the same
    /// `scenario_identity` are the same scenario even if a bucket boundary put them in
    /// different `spr_bucket`s.
    ///
    /// # Panics
    /// Panics (in every build profile) if `spr.num() == 0`, for the same reason `spr_bucket`
    /// does (review R1).
    pub fn scenario_identity(&self, spr: Rational) -> [u8; 32] {
        assert!(
            spr.num() > 0,
            "scenario_identity: SPR must be positive (numerator > 0), got numerator 0 with denominator {}",
            spr.den()
        );
        Sha256::digest(serde_json::to_vec(&(self.at_bucket(0), spr)).expect("finite key")).into()
    }
}
