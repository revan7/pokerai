//! Wire-numeric codecs for `EnvelopeNode::weights`/`evs` (standing ruling: validate a wide
//! `f64` value against its domain *before* narrowing to `f32`, and re-validate on serialize
//! so an in-memory value built any other way -- not just one that came off the wire -- can
//! never reach the wire either). Mirrors `proto::worker`'s
//! `narrow_checked`/`widen_checked`/`deserialize_matrix` machinery; not reused directly
//! (`proto` has no reason to depend on `core-preflop`'s wire shape and vice versa -- numeric
//! wire safety is owned locally by each crate's own types, per the standing ruling, not
//! centralized behind a cross-crate dependency). `validate()` in `validate.rs` is unchanged
//! and remains the single place for structural rules (shape, duplicates, sibling sums); these
//! codecs only guarantee the numeric domain of each element at the serde boundary.
//!
//! JSON only for now: `decode`'s only encoding today is `serde_json::from_slice` (JSON
//! numbers are format-agnostic text, so reading as `f64` and writing the narrowed `f32` value
//! back is safe and lossless for any in-domain value). If `Envelope` is ever bincode-encoded,
//! these codecs would need read/write width parity -- bincode is not self-describing, so
//! reading a fixed 8-byte `f64` where a fixed 4-byte `f32` was written would misparse the
//! rest of the buffer -- that adaptation is deferred until such a need exists.

use serde::de::Error as DeError;
use serde::ser::{Error as SerError, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Probabilities (`weights`): closed unit interval.
fn domain_unit_interval(x: f64) -> bool { (0.0..=1.0).contains(&x) }
/// EV chips in source small-blind units (`evs`): no bound beyond finiteness.
fn domain_finite(_: f64) -> bool { true }
/// `RakeProfile::rate` (spec 2): a fraction, never a full rake (half-open at 1), mirroring
/// `proto::worker`'s identical `domain_rake_rate`.
fn domain_rake_rate(x: f64) -> bool { (0.0..1.0).contains(&x) }
/// `RakeProfile::cap_bb`: a chip cap, never negative.
fn domain_nonneg(x: f64) -> bool { x >= 0.0 }
/// `BundleInfo::source_blinds` entries: a blind size, always strictly positive.
fn domain_positive(x: f64) -> bool { x > 0.0 }

/// Reads a wire number as `f64`, checks it is finite and inside `domain`, and only then
/// narrows to `f32`, re-checking the narrowed value against the same domain. This ordering is
/// load-bearing: an f64 merely close to (but outside) the domain must never be admitted by
/// rounding into it during narrowing (a weight `1.00000001` must not become `1.0`, `-1e-50`
/// must not become `-0.0`), and a finite, in-domain f64 that overflows f32 on narrowing (e.g.
/// `1e39`) must be rejected rather than silently becoming `inf`.
fn narrow_checked(raw: f64, domain: fn(f64) -> bool, what: &str) -> Result<f32, String> {
    if !raw.is_finite() || !domain(raw) {
        return Err(format!("{what} {raw} is outside its valid domain"));
    }
    let narrowed = raw as f32;
    if !narrowed.is_finite() || !domain(narrowed as f64) {
        return Err(format!("{what} {raw} narrows to {narrowed:e}, outside its valid domain"));
    }
    Ok(narrowed)
}

/// The serialize-side counterpart of `narrow_checked`: rejects an in-memory value that is
/// non-finite or out of domain instead of letting it reach the wire -- in particular it can
/// never surface as JSON `null`, which is serde_json's silent rendering of a non-finite float
/// and would be indistinguishable from a legitimately absent value.
fn widen_checked(v: f32, domain: fn(f64) -> bool, what: &str) -> Result<f32, String> {
    if !v.is_finite() || !domain(v as f64) {
        return Err(format!("{what} {v} is outside its valid domain"));
    }
    Ok(v)
}

/// Narrows one already wide-domain-checked weight (`[0, 1]`, which can never overflow `f32`).
/// Used by `decode`'s wide-then-narrow restructuring (R1): the aggregate node/class checks
/// in `validate.rs` run on raw `f64` first, and only after they pass does `decode` call this
/// to produce the `f32` value it stores -- the same `narrow_checked`/`domain_unit_interval`
/// used by `deserialize_weights` below, not a second copy of the domain rule.
pub(crate) fn narrow_weight(raw: f64) -> Result<f32, String> {
    narrow_checked(raw, domain_unit_interval, "weight")
}
/// Narrows one already wide-finite-checked EV cell; unlike a weight, a finite `f64` (e.g.
/// `1e39`) can still overflow `f32` on narrowing, so this can legitimately fail even after
/// the wide check passed.
pub(crate) fn narrow_ev(raw: f64) -> Result<f32, String> {
    narrow_checked(raw, domain_finite, "ev_source_sb")
}

pub(crate) fn deserialize_weights<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<f32>>, D::Error> {
    let raw: Vec<Vec<f64>> = Deserialize::deserialize(d)?;
    raw.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|x| narrow_checked(x, domain_unit_interval, "weight").map_err(DeError::custom))
                .collect()
        })
        .collect()
}

pub(crate) fn serialize_weights<S: Serializer>(v: &[Vec<f32>], s: S) -> Result<S::Ok, S::Error> {
    let mut outer = s.serialize_seq(Some(v.len()))?;
    for row in v {
        let mut checked_row = Vec::with_capacity(row.len());
        for x in row {
            checked_row.push(widen_checked(*x, domain_unit_interval, "weight").map_err(SerError::custom)?);
        }
        outer.serialize_element(&checked_row)?;
    }
    outer.end()
}

pub(crate) fn deserialize_evs<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<Vec<Option<f32>>>>, D::Error> {
    let raw: Option<Vec<Vec<Option<f64>>>> = Deserialize::deserialize(d)?;
    let Some(rows) = raw else { return Ok(None) };
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut out_row = Vec::with_capacity(row.len());
        for cell in row {
            out_row.push(match cell {
                None => None,
                Some(x) => Some(narrow_checked(x, domain_finite, "ev_source_sb").map_err(DeError::custom)?),
            });
        }
        out.push(out_row);
    }
    Ok(Some(out))
}

pub(crate) fn serialize_evs<S: Serializer>(v: &Option<Vec<Vec<Option<f32>>>>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        None => s.serialize_none(),
        Some(rows) => {
            let mut outer = s.serialize_seq(Some(rows.len()))?;
            for row in rows {
                let mut checked_row = Vec::with_capacity(row.len());
                for cell in row {
                    checked_row.push(match cell {
                        None => None,
                        Some(x) => Some(widen_checked(*x, domain_finite, "ev_source_sb").map_err(SerError::custom)?),
                    });
                }
                outer.serialize_element(&checked_row)?;
            }
            outer.end()
        }
    }
}

// --- R2: BundleInfo/RakeProfile metadata codecs (same wide-then-narrow pattern) ---

pub(crate) fn deserialize_rake_rate<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    let raw = f64::deserialize(d)?;
    narrow_checked(raw, domain_rake_rate, "rate").map_err(DeError::custom)
}
pub(crate) fn serialize_rake_rate<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    let checked = widen_checked(*v, domain_rake_rate, "rate").map_err(SerError::custom)?;
    s.serialize_f32(checked)
}

pub(crate) fn deserialize_cap_bb<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    let raw = f64::deserialize(d)?;
    narrow_checked(raw, domain_nonneg, "cap_bb").map_err(DeError::custom)
}
pub(crate) fn serialize_cap_bb<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    let checked = widen_checked(*v, domain_nonneg, "cap_bb").map_err(SerError::custom)?;
    s.serialize_f32(checked)
}

/// `[source_blind_sb, source_blind_bb]`: both strictly positive and finite, and the second
/// at least the first -- checked wide (`f64`) before narrowing, and re-checked (including
/// the ordering) on serialize, matching every other codec in this module.
pub(crate) fn deserialize_source_blinds<'de, D: Deserializer<'de>>(d: D) -> Result<[f32; 2], D::Error> {
    let raw: [f64; 2] = Deserialize::deserialize(d)?;
    let sb = narrow_checked(raw[0], domain_positive, "source_blinds[0]").map_err(DeError::custom)?;
    let bb = narrow_checked(raw[1], domain_positive, "source_blinds[1]").map_err(DeError::custom)?;
    if bb < sb {
        return Err(DeError::custom(format!("source_blinds[1] {bb} must be >= source_blinds[0] {sb}")));
    }
    Ok([sb, bb])
}
pub(crate) fn serialize_source_blinds<S: Serializer>(v: &[f32; 2], s: S) -> Result<S::Ok, S::Error> {
    let sb = widen_checked(v[0], domain_positive, "source_blinds[0]").map_err(SerError::custom)?;
    let bb = widen_checked(v[1], domain_positive, "source_blinds[1]").map_err(SerError::custom)?;
    if bb < sb {
        return Err(SerError::custom(format!("source_blinds[1] {bb} must be >= source_blinds[0] {sb}")));
    }
    Serialize::serialize(&[sb, bb], s)
}
