//! Checked float codecs shared by every wire `proto` defines (spec 4.5, 4.4, 4.6).
//!
//! Standing ruling: numeric input from text or wire is validated in its **wide** form (f64) BEFORE
//! narrowing to f32, and is never clamped. The same domain is enforced on serialize, so an invalid
//! in-memory value — however it was constructed — can never reach the wire; in particular it can
//! never surface as JSON `null`, which is serde_json's silent rendering of a non-finite float and
//! would be indistinguishable from a legitimate absent value. For an `Option<f32>` field that
//! degradation is worse than a hard error: `Some(NaN)` would serialize as `null` and read back as
//! `None`, silently deleting the value. `None` stays the one and only nullable value here.
//!
//! This module started as the worker-wire codec of `worker.rs` (review S1) and is now the single
//! definition for the worker wire, the config wire (`game.rs`), the tree wire (`tree.rs`) and the
//! UI/IPC wire (`recommendation.rs`).

use serde::de::Error as DeError;
use serde::ser::{Error as SerError, SerializeSeq};
use serde::{Deserialize, Deserializer, Serializer};

use crate::hand::{Action, Seat};

/// A domain predicate over the **wide** value. Finiteness is checked separately, by
/// `narrow_checked` / `widen_checked`, so a domain never has to repeat it.
pub(crate) type Domain = fn(f64) -> bool;

/// No bound beyond finiteness: EV chips, exploitability, observed pot fractions, bucket deltas.
pub(crate) fn domain_finite(_: f64) -> bool { true }
/// Probabilities, frequencies, posteriors and masses in `[0, 1]` (spec 4.1, 4.4, 4.5).
pub(crate) fn domain_unit_interval(x: f64) -> bool { (0.0..=1.0).contains(&x) }
/// `rake_rate` (spec 2): a fraction, never a full rake (half-open at 1).
pub(crate) fn domain_rake_rate(x: f64) -> bool { (0.0..1.0).contains(&x) }
/// Magnitudes that are never negative but are not bounded above: tree thresholds (spec 4.6),
/// range mass (up to 1,326 combos), exploitability percentages and bet-translation deviations
/// (spec 8.4's `d`, a distance between pot fractions).
pub(crate) fn domain_non_negative(x: f64) -> bool { x >= 0.0 }

/// Reads a wire number as `f64`, checks it is finite and inside `domain`, and only then narrows to
/// `f32`, re-checking the narrowed value against the same domain. This ordering is load-bearing: an
/// f64 merely close to (but outside) the domain must never be admitted by rounding into it during
/// narrowing (e.g. a probability `1.00000001` must not become `1.0`, `-1e-50` must not become
/// `-0.0`), and a finite, in-domain f64 that overflows f32 on narrowing (e.g. `1e39`) must be
/// rejected rather than silently becoming `inf`.
pub(crate) fn narrow_checked(raw: f64, domain: Domain, what: &str) -> Result<f32, String> {
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
/// non-finite or out of domain instead of letting it reach the wire.
pub(crate) fn widen_checked(v: f32, domain: Domain, what: &str) -> Result<f32, String> {
    if !v.is_finite() || !domain(v as f64) {
        return Err(format!("{what} {v} is outside its valid domain"));
    }
    Ok(v)
}

/// Checks a value already at its native (non-widened) width -- the bincode read path, where the
/// wire value already is an `f32` (there is no wider source representation to validate before
/// narrowing, and no narrowing step at all). Review R2 (fix round 1): this is exactly
/// `widen_checked`'s check (finiteness and `domain` on an `f32`), so it delegates rather than
/// duplicating the body; the two keep distinct names because each documents which *direction* --
/// write (`widen_checked`, called from `serialize`) or read (`native_checked`, called from
/// `deserialize`) -- the caller is on, matching the read/write split every codec below makes
/// explicit. `narrow_checked` stays a genuinely separate helper: it is the only one of the three
/// that starts from a wider `f64` and narrows.
pub(crate) fn native_checked(v: f32, domain: Domain, what: &str) -> Result<f32, String> {
    widen_checked(v, domain, what)
}

/// Generates the `#[serde(with = "...")]` module for a required `f32` field of one domain.
///
/// Bincode 1.3.3 (the pinned cache storage format, spec 10.4) is not self-describing: whatever
/// primitive type a `Deserializer` call asks for is read at that exact wire width, with no type
/// tag to check against. `serialize` here already writes a native `f32` on every format (JSON's
/// `serialize_f32` and bincode's both encode the value at `f32` width), so only `deserialize`
/// needs to branch: a human-readable format (JSON) still reads the wider `f64` first and checks
/// the *wide* value's domain before narrowing (a JSON number can arrive with more precision than
/// f32, e.g. `1.00000001`, and narrowing before checking would silently admit it -- unchanged
/// from before this fix); a non-self-describing format (bincode) reads the `f32` actually on the
/// wire directly, since there is no wider source value to check and no narrowing step that could
/// lose precision it did not already have -- it still rejects a corrupted (e.g. hand-crafted NaN)
/// bit pattern via `native_checked`.
macro_rules! scalar_codec {
    ($name:ident, $domain:path, $what:literal) => {
        pub(crate) mod $name {
            use super::*;
            pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
                if d.is_human_readable() {
                    narrow_checked(f64::deserialize(d)?, $domain, $what).map_err(DeError::custom)
                } else {
                    native_checked(f32::deserialize(d)?, $domain, $what).map_err(DeError::custom)
                }
            }
            pub(crate) fn serialize<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_f32(widen_checked(*v, $domain, $what).map_err(SerError::custom)?)
            }
        }
    };
}

/// Generates the `#[serde(with = "...")]` module for an `Option<f32>` field. `None` is the one
/// nullable value; `Some(x)` out of domain is an error on both directions, never a silent `null`.
/// Same human-readable/native split as `scalar_codec!` (see its doc comment); `serialize` is
/// already format-agnostic (native `f32` on the wire either way), only `deserialize` branches.
macro_rules! option_codec {
    ($name:ident, $domain:path, $what:literal) => {
        pub(crate) mod $name {
            use super::*;
            pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
                if d.is_human_readable() {
                    match Option::<f64>::deserialize(d)? {
                        None => Ok(None),
                        Some(x) => narrow_checked(x, $domain, $what).map(Some).map_err(DeError::custom),
                    }
                } else {
                    match Option::<f32>::deserialize(d)? {
                        None => Ok(None),
                        Some(x) => native_checked(x, $domain, $what).map(Some).map_err(DeError::custom),
                    }
                }
            }
            pub(crate) fn serialize<S: Serializer>(v: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
                match v {
                    None => s.serialize_none(),
                    Some(x) => {
                        let checked = widen_checked(*x, $domain, $what).map_err(SerError::custom)?;
                        s.serialize_some(&checked)
                    }
                }
            }
        }
    };
}

scalar_codec!(rake_rate, domain_rake_rate, "rake_rate");
scalar_codec!(probability, domain_unit_interval, "a probability");
scalar_codec!(finite, domain_finite, "a finite value");
scalar_codec!(non_negative, domain_non_negative, "a non-negative value");

option_codec!(probability_opt, domain_unit_interval, "a probability");
option_codec!(finite_opt, domain_finite, "a finite value");
option_codec!(non_negative_opt, domain_non_negative, "a non-negative value");

/// `Vec<f32>` of finite values (`AsymmetricStacks::stacks_bb`).
pub(crate) mod finite_vec {
    use super::*;
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
        if d.is_human_readable() {
            Vec::<f64>::deserialize(d)?
                .into_iter()
                .map(|x| narrow_checked(x, domain_finite, "a finite value").map_err(DeError::custom))
                .collect()
        } else {
            Vec::<f32>::deserialize(d)?
                .into_iter()
                .map(|x| native_checked(x, domain_finite, "a finite value").map_err(DeError::custom))
                .collect()
        }
    }
    pub(crate) fn serialize<S: Serializer>(v: &[f32], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for x in v {
            seq.serialize_element(&widen_checked(*x, domain_finite, "a finite value").map_err(SerError::custom)?)?;
        }
        seq.end()
    }
}

/// `[f32; 3]` of finite values (`StraddleMapped::posts`: `sb/S`, `bb/S`, `1`).
pub(crate) mod finite_array3 {
    use super::*;
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[f32; 3], D::Error> {
        let mut out = [0f32; 3];
        if d.is_human_readable() {
            let raw = <[f64; 3]>::deserialize(d)?;
            for (slot, x) in out.iter_mut().zip(raw) {
                *slot = narrow_checked(x, domain_finite, "a straddle post").map_err(DeError::custom)?;
            }
        } else {
            let raw = <[f32; 3]>::deserialize(d)?;
            for (slot, x) in out.iter_mut().zip(raw) {
                *slot = native_checked(x, domain_finite, "a straddle post").map_err(DeError::custom)?;
            }
        }
        Ok(out)
    }
    pub(crate) fn serialize<S: Serializer>(v: &[f32; 3], s: S) -> Result<S::Ok, S::Error> {
        // A fixed-size array's `Deserialize` (both branches above, `<[f64; 3]>`/`<[f32; 3]>`)
        // goes through serde's tuple deserialization (a statically known arity, no length
        // prefix), not its seq deserialization (a dynamic length, length-prefixed under
        // bincode) -- `serialize_seq` here would write a length prefix `deserialize_tuple` never
        // reads back, desyncing bincode's byte stream by 8 bytes (observed directly: decoding
        // `StraddleMapped { posts: [0.5, 1.0, 1.0] }` through bincode silently produced `[4e-45,
        // 0.0, 0.5]`, the length-prefix bytes misread as data). `serialize_tuple` matches the
        // read side on every format; serde_json's `SerializeTuple` and `SerializeSeq` both write
        // a plain JSON array, so this is not a wire-format change for the human-readable path.
        use serde::ser::SerializeTuple;
        let mut tup = s.serialize_tuple(3)?;
        for x in v {
            tup.serialize_element(&widen_checked(*x, domain_finite, "a straddle post").map_err(SerError::custom)?)?;
        }
        tup.end()
    }
}

/// `Vec<(f32, f32)>` of `(menu size, weight)` pairs (`BetTranslation::mapped`): the size is a pot
/// fraction, which is finite but unbounded above (a 3x-pot bet is a legitimate 3.0); the weight is
/// an interpolation share in `[0, 1]`.
pub(crate) mod mapped_sizes {
    use super::*;
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(f32, f32)>, D::Error> {
        if d.is_human_readable() {
            Vec::<(f64, f64)>::deserialize(d)?
                .into_iter()
                .map(|(size, weight)| {
                    let size = narrow_checked(size, domain_finite, "a mapped menu size").map_err(DeError::custom)?;
                    let weight = narrow_checked(weight, domain_unit_interval, "a mapped weight").map_err(DeError::custom)?;
                    Ok((size, weight))
                })
                .collect()
        } else {
            Vec::<(f32, f32)>::deserialize(d)?
                .into_iter()
                .map(|(size, weight)| {
                    let size = native_checked(size, domain_finite, "a mapped menu size").map_err(DeError::custom)?;
                    let weight = native_checked(weight, domain_unit_interval, "a mapped weight").map_err(DeError::custom)?;
                    Ok((size, weight))
                })
                .collect()
        }
    }
    pub(crate) fn serialize<S: Serializer>(v: &[(f32, f32)], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for (size, weight) in v {
            let size = widen_checked(*size, domain_finite, "a mapped menu size").map_err(SerError::custom)?;
            let weight = widen_checked(*weight, domain_unit_interval, "a mapped weight").map_err(SerError::custom)?;
            seq.serialize_element(&(size, weight))?;
        }
        seq.end()
    }
}

/// `Option<Vec<(Action, f32)>>` of action weights (`Recommendation::range_mix`).
pub(crate) mod action_weights_opt {
    use super::*;
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<(Action, f32)>>, D::Error> {
        if d.is_human_readable() {
            let Some(raw) = Option::<Vec<(Action, f64)>>::deserialize(d)? else { return Ok(None) };
            raw.into_iter()
                .map(|(a, w)| narrow_checked(w, domain_unit_interval, "a range-mix weight").map(|w| (a, w)).map_err(DeError::custom))
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        } else {
            let Some(raw) = Option::<Vec<(Action, f32)>>::deserialize(d)? else { return Ok(None) };
            raw.into_iter()
                .map(|(a, w)| native_checked(w, domain_unit_interval, "a range-mix weight").map(|w| (a, w)).map_err(DeError::custom))
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        }
    }
    pub(crate) fn serialize<S: Serializer>(v: &Option<Vec<(Action, f32)>>, s: S) -> Result<S::Ok, S::Error> {
        let Some(v) = v else { return s.serialize_none() };
        let mut checked = Vec::with_capacity(v.len());
        for (a, w) in v {
            checked.push((a, widen_checked(*w, domain_unit_interval, "a range-mix weight").map_err(SerError::custom)?));
        }
        s.serialize_some(&checked)
    }
}

/// One `(Seat, name, mass)` triple: the mass is the total weight of a 1,326-combo range, so it is
/// non-negative but not bounded by 1.
fn narrow_mass(m: f64) -> Result<f32, String> { narrow_checked(m, domain_non_negative, "a range mass") }
fn widen_mass(m: f32) -> Result<f32, String> { widen_checked(m, domain_non_negative, "a range mass") }
fn native_mass(m: f32) -> Result<f32, String> { native_checked(m, domain_non_negative, "a range mass") }

/// `Vec<(Seat, String, f32)>` (`Assumptions::ranges_used`).
pub(crate) mod mass_triples {
    use super::*;
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(Seat, String, f32)>, D::Error> {
        if d.is_human_readable() {
            Vec::<(Seat, String, f64)>::deserialize(d)?
                .into_iter()
                .map(|(seat, name, mass)| narrow_mass(mass).map(|mass| (seat, name, mass)).map_err(DeError::custom))
                .collect()
        } else {
            Vec::<(Seat, String, f32)>::deserialize(d)?
                .into_iter()
                .map(|(seat, name, mass)| native_mass(mass).map(|mass| (seat, name, mass)).map_err(DeError::custom))
                .collect()
        }
    }
    pub(crate) fn serialize<S: Serializer>(v: &[(Seat, String, f32)], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for (seat, name, mass) in v {
            seq.serialize_element(&(seat, name, widen_mass(*mass).map_err(SerError::custom)?))?;
        }
        seq.end()
    }
}

/// `[(Seat, String, f32); 2]` (`ExperimentalHu::ranges_used`: hero and the one opponent).
pub(crate) mod mass_pair {
    use super::*;
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[(Seat, String, f32); 2], D::Error> {
        if d.is_human_readable() {
            let [a, b] = <[(Seat, String, f64); 2]>::deserialize(d)?;
            let narrow = |(seat, name, mass): (Seat, String, f64)| narrow_mass(mass).map(|mass| (seat, name, mass)).map_err(DeError::custom);
            Ok([narrow(a)?, narrow(b)?])
        } else {
            let [a, b] = <[(Seat, String, f32); 2]>::deserialize(d)?;
            let native = |(seat, name, mass): (Seat, String, f32)| native_mass(mass).map(|mass| (seat, name, mass)).map_err(DeError::custom);
            Ok([native(a)?, native(b)?])
        }
    }
    pub(crate) fn serialize<S: Serializer>(v: &[(Seat, String, f32); 2], s: S) -> Result<S::Ok, S::Error> {
        // Same fixed-arity tuple-vs-seq framing mismatch as `finite_array3` (see its comment):
        // a fixed-size array deserializes as a tuple (no length prefix), so this must serialize
        // as one too, or bincode's length prefix would desync the reader. Identical JSON output
        // either way (`serde_json` renders both as a plain array).
        use serde::ser::SerializeTuple;
        let mut tup = s.serialize_tuple(2)?;
        for (seat, name, mass) in v {
            tup.serialize_element(&(seat, name, widen_mass(*mass).map_err(SerError::custom)?))?;
        }
        tup.end()
    }
}

/// Validates every element of a `Vec<Vec<f32>>` wire matrix against `domain` before narrowing (see
/// `narrow_checked`). Shared by `NodeLock::probs`, `NodeStrategy::probs` (unit interval) and
/// `NodeStrategy::ev_chips` (finite only). Matrix shape/row-length/path validation stays
/// `validate_solution`'s job; this only guards the numeric domain of each element.
pub(crate) fn deserialize_matrix<'de, D: Deserializer<'de>>(d: D, domain: Domain, what: &str) -> Result<Vec<Vec<f32>>, D::Error> {
    if d.is_human_readable() {
        let raw: Vec<Vec<f64>> = Deserialize::deserialize(d)?;
        raw.into_iter()
            .map(|row| row.into_iter().map(|x| narrow_checked(x, domain, what).map_err(DeError::custom)).collect())
            .collect()
    } else {
        let raw: Vec<Vec<f32>> = Deserialize::deserialize(d)?;
        raw.into_iter()
            .map(|row| row.into_iter().map(|x| native_checked(x, domain, what).map_err(DeError::custom)).collect())
            .collect()
    }
}

pub(crate) fn serialize_matrix<S: Serializer>(v: &[Vec<f32>], s: S, domain: Domain, what: &str) -> Result<S::Ok, S::Error> {
    let mut outer = s.serialize_seq(Some(v.len()))?;
    for row in v {
        let mut checked_row = Vec::with_capacity(row.len());
        for x in row {
            checked_row.push(widen_checked(*x, domain, what).map_err(SerError::custom)?);
        }
        outer.serialize_element(&checked_row)?;
    }
    outer.end()
}
