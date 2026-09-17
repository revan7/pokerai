mod parse;

pub use parse::{class_index, class_name, expand_169, parse_range, range_to_string};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RangeError {
    #[error("range syntax: {0}")]
    Syntax(String),
    #[error("range weight: {0}")]
    Weight(String),
}

/// Total weight (accumulated in f64).
pub fn mass(r: &proto::Range1326) -> f32 { r.0.iter().map(|w| *w as f64).sum::<f64>() as f32 }

use proto::{combo_cards, Card, Range1326, COMBOS};
use sha2::{Digest, Sha256};

/// Public blocking: combos containing a board card get weight 0 (never hero's cards; spec section 2).
pub fn block_public(r: &mut Range1326, board: &[Card]) {
    for i in 0..COMBOS {
        let [a, b] = combo_cards(i as u16);
        if board.contains(&a) || board.contains(&b) { r.0[i] = 0.0; }
    }
}

/// Hero-conditioned copy for equity and terminal calculations only.
pub fn hero_conditioned(r: &Range1326, hero: [Card; 2]) -> Range1326 {
    let mut out = r.clone();
    block_public(&mut out, &hero);
    out
}

/// Spec section 2 range hash: weights divided by the maximum (one f32 division each), bit patterns hashed with sha256.
pub fn hash_scaled(r: &Range1326) -> [u8; 32] {
    let max = r.0.iter().copied().fold(0.0f32, f32::max);
    let mut h = Sha256::new();
    for w in r.0.iter() {
        let v: f32 = if max > 0.0 { *w / max } else { 0.0 };
        h.update(v.to_le_bytes());
    }
    h.finalize().into()
}
