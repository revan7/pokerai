use serde::de::{self, SeqAccess, Visitor};
use serde::ser::{self, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use crate::cards::{ComboIndex, COMBOS};

/// Weights in `[0, 1]` indexed by combo index (spec 4.1).
#[derive(Clone, PartialEq)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Range1326(#[cfg_attr(feature = "typescript", ts(type = "Array<number>"))] pub [f32; COMBOS]);

impl Range1326 {
    pub fn zero() -> Range1326 { Range1326([0.0; COMBOS]) }
    pub fn uniform() -> Range1326 { Range1326([1.0; COMBOS]) }
    pub fn from_fn(mut f: impl FnMut(ComboIndex) -> f32) -> Range1326 {
        let mut r = Range1326::zero();
        for i in 0..COMBOS { r.0[i] = f(i as ComboIndex); }
        r
    }
    pub fn get(&self, i: ComboIndex) -> f32 { self.0[i as usize] }
    pub fn set(&mut self, i: ComboIndex, w: f32) { self.0[i as usize] = w; }
}

impl fmt::Debug for Range1326 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let support = self.0.iter().filter(|w| **w > 0.0).count();
        let mass: f64 = self.0.iter().map(|w| *w as f64).sum();
        write!(f, "Range1326(support={support}, mass={mass})")
    }
}

impl Serialize for Range1326 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Validate every weight before opening the sequence so a malformed
        // range never reaches the wire as valid-looking (or `null`) JSON.
        for (i, w) in self.0.iter().enumerate() {
            if !w.is_finite() || !(0.0..=1.0).contains(w) {
                return Err(ser::Error::custom(format!(
                    "weight {w} at combo {i} is outside [0, 1]"
                )));
            }
        }
        let mut seq = s.serialize_seq(Some(COMBOS))?;
        for w in self.0.iter() { seq.serialize_element(w)?; }
        seq.end()
    }
}

struct RangeVisitor;

impl<'de> Visitor<'de> for RangeVisitor {
    type Value = Range1326;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("an array of exactly 1326 finite numbers in [0, 1]") }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Range1326, A::Error> {
        let mut out = [0f32; COMBOS];
        for (i, slot) in out.iter_mut().enumerate() {
            // Read as f64 first and validate the *wide* value: narrowing to f32
            // before checking would let e.g. 1.00000001 round to 1.0 and
            // -1e-50 round to -0.0, silently admitting out-of-domain input.
            let raw: f64 = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(i, &self))?;
            if !raw.is_finite() || !(0.0..=1.0).contains(&raw) {
                return Err(de::Error::custom(format!("weight {raw} at combo {i} is outside [0, 1]")));
            }
            // `+ 0.0` maps `-0.0` to `+0.0` and is the identity on every other in-domain value.
            // `-0.0` passes the `[0, 1]` domain check above (`-0.0 == 0.0`) and would otherwise
            // survive into the bit-exact layers, where `hash_scaled` hashes `0x80000000`
            // differently from `0x00000000`: a range and its own `range_to_string` round trip are
            // numerically identical yet produce different cache keys (review S5). Normalizing here,
            // at the one ingestion boundary, leaves `hash_scaled`, `apply_range` and `canonicalize`
            // bit-exact as ruled by T20/T21 -- including `iso_tiebreak_negative_zero_regression`,
            // which builds its `-0.0` in memory and never crosses this boundary.
            let w = (raw as f32) + 0.0;
            if !w.is_finite() || !(0.0..=1.0).contains(&w) {
                return Err(de::Error::custom(format!("weight {w} at combo {i} is outside [0, 1]")));
            }
            *slot = w;
        }
        if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom("range has more than 1326 entries"));
        }
        Ok(Range1326(out))
    }
}

impl<'de> Deserialize<'de> for Range1326 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Range1326, D::Error> { d.deserialize_seq(RangeVisitor) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_serde_validates_shape_and_domain() {
        let r = Range1326::from_fn(|i| if i % 7 == 0 { 0.5 } else { 0.0 });
        let text = serde_json::to_string(&r).unwrap();
        let back: Range1326 = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r);
        let short = serde_json::to_string(&vec![0.0f32; 1325]).unwrap();
        assert!(serde_json::from_str::<Range1326>(&short).is_err());
        let long = serde_json::to_string(&vec![0.0f32; 1327]).unwrap();
        assert!(serde_json::from_str::<Range1326>(&long).is_err());
        let mut v = vec![0.0f32; 1326]; v[3] = 1.5;
        assert!(serde_json::from_str::<Range1326>(&serde_json::to_string(&v).unwrap()).is_err());
        v[3] = -0.1;
        assert!(serde_json::from_str::<Range1326>(&serde_json::to_string(&v).unwrap()).is_err());
        assert!(serde_json::from_str::<Range1326>("[null]").is_err());
        assert_eq!(Range1326::uniform().0.iter().sum::<f32>(), 1326.0);
        assert_eq!(format!("{:?}", Range1326::zero()), "Range1326(support=0, mass=0)");
    }

    #[test]
    fn range_serde_round_trips_zero_and_uniform() {
        let zero = Range1326::zero();
        let text = serde_json::to_string(&zero).unwrap();
        let back: Range1326 = serde_json::from_str(&text).unwrap();
        assert_eq!(back, zero);

        let uniform = Range1326::uniform();
        let text = serde_json::to_string(&uniform).unwrap();
        let back: Range1326 = serde_json::from_str(&text).unwrap();
        assert_eq!(back, uniform);
    }

    #[test]
    fn range_serialize_rejects_non_finite_and_out_of_domain_weights() {
        let mut r = Range1326::zero();
        r.0[5] = f32::NAN;
        assert!(serde_json::to_string(&r).is_err());

        let mut r = Range1326::zero();
        r.0[5] = f32::INFINITY;
        assert!(serde_json::to_string(&r).is_err());

        let mut r = Range1326::zero();
        r.0[5] = f32::NEG_INFINITY;
        assert!(serde_json::to_string(&r).is_err());

        let mut r = Range1326::zero();
        r.0[5] = -0.1;
        assert!(serde_json::to_string(&r).is_err());

        let mut r = Range1326::zero();
        r.0[5] = 1.5;
        assert!(serde_json::to_string(&r).is_err());
    }

    fn array_json_with_first_token(tok: &str) -> String {
        let mut s = String::from("[");
        s.push_str(tok);
        for _ in 1..COMBOS {
            s.push_str(",0");
        }
        s.push(']');
        s
    }

    #[test]
    fn range_deserialize_rejects_rounding_boundary_values() {
        // 1.00000001 narrows to f32 1.0 and -1e-50 narrows to f32 -0.0 if the
        // decoder validates only after narrowing; both must be rejected because
        // the original f64 tokens are outside [0, 1].
        assert!(serde_json::from_str::<Range1326>(&array_json_with_first_token("1.00000001")).is_err());
        assert!(serde_json::from_str::<Range1326>(&array_json_with_first_token("-1e-50")).is_err());
    }

    /// S5: `-0.0` is in `[0, 1]` and so is admitted, but it must be *stored* as `+0.0`, because
    /// everything downstream of this boundary is bit-exact by design. Only the wire boundary
    /// normalizes: an in-memory `-0.0` is preserved, which is what T21's isomorphism tie-break
    /// regression depends on.
    #[test]
    fn range_deserialize_normalizes_negative_zero_to_positive_zero() {
        let r: Range1326 = serde_json::from_str(&array_json_with_first_token("-0.0")).unwrap();
        assert_eq!(r.get(0).to_bits(), 0.0f32.to_bits(), "-0.0 must be ingested as +0.0, not 0x80000000");
        let ordinary: Range1326 = serde_json::from_str(&array_json_with_first_token("0.5")).unwrap();
        assert_eq!(ordinary.get(0).to_bits(), 0.5f32.to_bits(), "every other weight is unchanged, bit for bit");
        let mut in_memory = Range1326::zero();
        in_memory.set(19, -0.0);
        assert_eq!(in_memory.get(19).to_bits(), (-0.0f32).to_bits(), "`set` is not an ingestion boundary and must not normalize");
    }

    #[test]
    fn range_deserialize_accepts_domain_controls() {
        assert!(serde_json::from_str::<Range1326>(&array_json_with_first_token("0")).is_ok());
        assert!(serde_json::from_str::<Range1326>(&array_json_with_first_token("1")).is_ok());
        assert!(serde_json::from_str::<Range1326>(&array_json_with_first_token("0.5")).is_ok());
    }
}
