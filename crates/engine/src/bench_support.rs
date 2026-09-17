//! The façade `bench` uses instead of depending on `core-*` directly (spec §3.2 dependency direction).
//! Plan 4 extends this module; it never grows a second entry point.
use proto::{Card, Range1326};

/// Parses a Pio-style range string and applies board blocking, exactly as the engine does at a street root.
pub fn prepared_range(text: &str, board: &[Card]) -> Result<Range1326, String> {
    let mut r = core_ranges::parse_range(text).map_err(|e| format!("range {text:?}: {e}"))?;
    core_ranges::block_public(&mut r, board);
    Ok(r)
}
/// Total weight of a range, for report and sanity checks.
pub fn range_mass(r: &Range1326) -> f32 { core_ranges::mass(r) }

#[cfg(test)]
mod tests {
    use super::*;
    use proto::Card;
    #[test]
    fn prepared_range_parses_and_blocks_the_board() {
        let board: Vec<Card> = ["Kh", "7d", "2c"].iter().map(|s| Card::parse(s).unwrap()).collect();
        let r = prepared_range("AA,KK", &board).unwrap();
        assert_eq!((range_mass(&r) * 1000.0).round() as u32, 9000);   // 6 aces + 3 kings (Kh is on the board)
        assert!(prepared_range("not a range", &board).is_err());
    }
}
