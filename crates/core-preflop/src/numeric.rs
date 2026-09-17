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
use serde::{Deserialize, Deserializer, Serializer};

/// Probabilities (`weights`): closed unit interval.
fn domain_unit_interval(x: f64) -> bool { (0.0..=1.0).contains(&x) }
/// EV chips in source small-blind units (`evs`): no bound beyond finiteness.
fn domain_finite(_: f64) -> bool { true }

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
