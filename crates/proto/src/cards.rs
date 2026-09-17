use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// id = rank_index*4 + suit_index; ranks 2..A = 0..12; suits c,d,h,s = 0..3 (spec 4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Card(#[cfg_attr(feature = "typescript", ts(type = "string"))] pub u8);

pub const RANK_CHARS: &[u8; 13] = b"23456789TJQKA";
pub const SUIT_CHARS: &[u8; 4] = b"cdhs";
pub const COMBOS: usize = 1326;
pub const CLASSES: usize = 169;
pub type ComboIndex = u16;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum CardParseError {
    #[error("card text {0:?} must be exactly two characters")]
    Length(String),
    #[error("unknown rank character {0:?}")]
    Rank(char),
    #[error("unknown suit character {0:?}")]
    Suit(char),
    #[error("card id {0} is outside 0..52")]
    Id(u8),
    #[error("duplicate card {0}")]
    Duplicate(Card),
}

impl Card {
    /// Builds a card from a rank index (0..13, 2..A) and suit index (0..4, c/d/h/s).
    ///
    /// # Panics
    /// Panics (in every build profile, not just debug) if `rank >= 13` or `suit >= 4`.
    /// Callers ingesting untrusted numeric input must validate first, or use
    /// `Card::checked`/`Card::parse`, which return a `Result` instead of panicking.
    pub fn new(rank: u8, suit: u8) -> Card {
        assert!(
            rank < 13 && suit < 4,
            "Card::new: rank {rank} or suit {suit} out of range"
        );
        Card(rank * 4 + suit)
    }
    pub fn rank(self) -> u8 { self.0 / 4 }
    pub fn suit(self) -> u8 { self.0 % 4 }
    pub fn all() -> impl Iterator<Item = Card> { (0..52u8).map(Card) }
    pub fn checked(id: u8) -> Result<Card, CardParseError> {
        if id < 52 { Ok(Card(id)) } else { Err(CardParseError::Id(id)) }
    }
    /// Inherent parse form used across the workspace; identical to `s.parse::<Card>()`.
    pub fn parse(s: &str) -> Result<Card, CardParseError> { s.parse() }
}

impl FromStr for Card {
    type Err = CardParseError;
    fn from_str(s: &str) -> Result<Card, CardParseError> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() != 2 { return Err(CardParseError::Length(s.to_string())); }
        let r = chars[0].to_ascii_uppercase();
        let rank = RANK_CHARS.iter().position(|c| *c as char == r).ok_or(CardParseError::Rank(chars[0]))?;
        let su = chars[1].to_ascii_lowercase();
        let suit = SUIT_CHARS.iter().position(|c| *c as char == su).ok_or(CardParseError::Suit(chars[1]))?;
        Ok(Card::new(rank as u8, suit as u8))
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", RANK_CHARS[self.rank() as usize] as char, SUIT_CHARS[self.suit() as usize] as char)
    }
}

impl Serialize for Card {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(&self.to_string()) }
}

impl<'de> Deserialize<'de> for Card {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Card, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// `idx = hi*(hi-1)/2 + lo` for card ids `lo < hi` (spec 4.1).
///
/// # Panics
/// Panics (in every build profile) if `a == b`; a combo needs two distinct cards.
pub fn combo_index(a: Card, b: Card) -> ComboIndex {
    assert!(a != b, "combo_index: cards {a:?} and {b:?} must be distinct");
    let (lo, hi) = if a.0 < b.0 { (a.0 as u16, b.0 as u16) } else { (b.0 as u16, a.0 as u16) };
    hi * (hi - 1) / 2 + lo
}

/// Inverse of `combo_index`: returns `[lo, hi]`.
///
/// # Panics
/// Panics (in every build profile) if `i >= COMBOS`.
pub fn combo_cards(i: ComboIndex) -> [Card; 2] {
    assert!(
        (i as usize) < COMBOS,
        "combo_cards: index {i} out of range 0..{COMBOS}"
    );
    let mut hi: u16 = 1;
    while (hi + 1) * hi / 2 <= i { hi += 1; }
    let lo = i - hi * (hi - 1) / 2;
    [Card(lo as u8), Card(hi as u8)]
}

/// 169-class of a combo: row-major grid from A down to 2, `i == j` pair, `i < j` suited, `i > j` offsuit.
pub fn class_of(i: ComboIndex) -> u8 {
    let [lo, hi] = combo_cards(i);
    let (r_hi, r_lo) = (hi.rank().max(lo.rank()), hi.rank().min(lo.rank()));
    let (row_hi, row_lo) = (12 - r_hi, 12 - r_lo); // row_hi <= row_lo
    if r_hi == r_lo { row_hi * 13 + row_hi }
    else if hi.suit() == lo.suit() { row_hi * 13 + row_lo }
    else { row_lo * 13 + row_hi }
}

/// Every combo of a class, ascending by combo index.
pub fn class_combos(class: u8) -> Vec<ComboIndex> {
    (0..COMBOS as u16).filter(|i| class_of(*i) == class).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_ids_match_spec() {
        assert_eq!("2c".parse::<Card>().unwrap(), Card(0));
        assert_eq!("As".parse::<Card>().unwrap(), Card(51));
        assert_eq!("Td".parse::<Card>().unwrap(), Card(8 * 4 + 1));
        assert_eq!(Card::parse("Kh").unwrap(), Card(11 * 4 + 2));
        assert_eq!(Card::parse("As").unwrap(), "As".parse::<Card>().unwrap());
        assert!(Card::parse("Zz").is_err());
        assert_eq!(Card(51).to_string(), "As");
        assert!("1s".parse::<Card>().is_err());
        assert!("Ax".parse::<Card>().is_err());
        assert!("Ass".parse::<Card>().is_err());
        assert_eq!(serde_json::to_string(&Card(43)).unwrap(), "\"Qs\"");
        assert_eq!(serde_json::from_str::<Card>("\"Qs\"").unwrap(), Card(43));
        assert!(serde_json::from_str::<Card>("\"Zz\"").is_err());
    }

    #[test]
    fn combo_index_bijection() {
        let mut seen = vec![false; COMBOS];
        for hi in 1..52u8 {
            for lo in 0..hi {
                let i = combo_index(Card(hi), Card(lo));
                assert_eq!(i, combo_index(Card(lo), Card(hi)));
                assert_eq!(combo_cards(i), [Card(lo), Card(hi)]);
                assert!(!seen[i as usize]);
                seen[i as usize] = true;
            }
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn class_order_spec() {
        let aa = combo_index("As".parse().unwrap(), "Ah".parse().unwrap());
        let aks = combo_index("As".parse().unwrap(), "Ks".parse().unwrap());
        let ako = combo_index("As".parse().unwrap(), "Kh".parse().unwrap());
        let s22 = combo_index("2c".parse().unwrap(), "2d".parse().unwrap());
        assert_eq!(class_of(aa), 0);
        assert_eq!(class_of(aks), 1);
        assert_eq!(class_of(ako), 13);
        assert_eq!(class_of(s22), 168);
        let mut counts = [0usize; CLASSES];
        for i in 0..COMBOS as u16 { counts[class_of(i) as usize] += 1; }
        for c in 0..CLASSES as u8 {
            let (i, j) = (c / 13, c % 13);
            let expected = if i == j { 6 } else if i < j { 4 } else { 12 };
            assert_eq!(counts[c as usize], expected, "class {c}");
            assert_eq!(class_combos(c).len(), expected);
            assert!(class_combos(c).iter().all(|k| class_of(*k) == c));
        }
    }

    // R1 fix: these three guards must hold in release builds too (debug_assert! alone
    // is compiled out under `cargo test --release`). Each probe reproduces the exact
    // invalid input from the review finding.
    #[test]
    #[should_panic(expected = "Card::new: rank 0 or suit 4 out of range")]
    fn card_new_rejects_out_of_range_suit_in_release() {
        Card::new(0, 4);
    }

    #[test]
    #[should_panic(expected = "combo_index: cards Card(1) and Card(1) must be distinct")]
    fn combo_index_rejects_duplicate_cards_in_release() {
        combo_index(Card(1), Card(1));
    }

    #[test]
    #[should_panic(expected = "combo_cards: index 1326 out of range 0..1326")]
    fn combo_cards_rejects_out_of_range_index_in_release() {
        combo_cards(1326);
    }
}
