//! Canonical structural cache keys (spec section 10.4) and the exact-rational SPR identity
//! they bucket. A `KeyFields` never carries seat ids, hand id, `bb_chips`, raw chip pot or
//! stacks, the wager quantum, hero's cards, `target_bp`, or the requested path -- only the
//! normalized fields a solved node's *shape* depends on. Canonical bytes are produced by
//! serializing `KeyFields` with a fixed struct field order, plain integers, strings and enum
//! tags (`serde_json`, never an unordered map), so the digest is deterministic across runs.

use crate::CacheError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// An exact rational in lowest terms: `num / den`. Two `Rational`s that denote the same
/// value compare equal regardless of the chip scale they were built from (spec section 10.4
/// "exact SPR rational"), because `new` always reduces by the gcd before storing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Rational {
    pub num: u64,
    pub den: u64,
}

impl Rational {
    /// Reduces `num / den` to lowest terms.
    ///
    /// # Errors
    /// `CacheError::Invalid` if `den == 0`.
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

    pub fn value(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

/// `round(ln(SPR) / ln(1.02))` (spec section 10.4): the geometric SPR bucket a lookup
/// searches at `b - 1, b, b + 1`.
pub fn spr_bucket(s: Rational) -> i32 {
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
/// `rate_bits` is the rake rate's bit pattern, `cap_over_p` is the rake cap expressed as an
/// exact fraction of `P` rather than raw `cap_mchips`, and `collection_rule_version` pins the
/// rule (e.g. no-flop-no-drop) that applied.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RakeKey {
    pub rate_bits: u32,
    pub cap_over_p: Rational,
    pub collection_rule_version: u16,
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
    pub fn scenario_identity(&self, spr: Rational) -> [u8; 32] {
        Sha256::digest(serde_json::to_vec(&(self.at_bucket(0), spr)).expect("finite key")).into()
    }
}
