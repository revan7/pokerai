use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use crate::cards::{ComboIndex, COMBOS};

/// Weights in `[0, 1]` indexed by combo index (spec 4.1).
#[derive(Clone, PartialEq)]
pub struct Range1326(pub [f32; COMBOS]);

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
            let w: f32 = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(i, &self))?;
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
}
