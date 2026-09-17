//! Card and range mapping by named combos (§4.1: the adapter never assumes the library's ordering).
use postflop_solver::{Card as LibCard, Range as LibRange, NOT_DEALT};
use proto::{combo_cards, combo_index, Card, ComboIndex, Range1326};

/// Library encoding (card.rs): `4 * rank + suit`, ranks 2..A = 0..12, suits c, d, h, s = 0..3.
///
/// # Panics
/// Panics (in every build profile) if `c.0 >= 52`: infallible function, so the valid-id
/// precondition is enforced with an always-on assert. Fallible callers ingesting untrusted card
/// ids (e.g. `board_to_lib`) validate first and return a typed `Err` instead of reaching this.
pub fn to_lib(c: Card) -> LibCard {
    assert!(c.0 < 52, "to_lib: card id {} is outside 0..52 (spec 4.1)", c.0);
    (c.rank() << 2) | c.suit()
}
pub fn from_lib(c: LibCard) -> Card { Card::new(c >> 2, c & 3) }

pub fn range_to_lib(r: &Range1326) -> Result<LibRange, String> {
    let mut out = LibRange::new();
    for (i, &w) in r.0.iter().enumerate() {
        if w == 0.0 { continue; }
        if !w.is_finite() || !(0.0..=1.0).contains(&w) { return Err(format!("weight {w} at combo {i} outside [0, 1]")); }
        let [lo, hi] = combo_cards(i as ComboIndex);
        out.set_weight_by_cards(to_lib(lo), to_lib(hi), w);
    }
    Ok(out)
}

pub fn lib_hand_to_combo(h: (LibCard, LibCard)) -> ComboIndex { combo_index(from_lib(h.0), from_lib(h.1)) }

/// Sorted flop (as `flop_from_str` does) plus turn and river or `NOT_DEALT`.
pub fn board_to_lib(board: &[Card]) -> Result<([LibCard; 3], LibCard, LibCard), String> {
    if !(3..=5).contains(&board.len()) { return Err(format!("board has {} cards", board.len())); }
    // Reject every id >= 52 before it reaches the infallible `to_lib` (spec 4.1 domain): a raw
    // `Card(255)` supplied by a caller must never silently become the backend's own `NOT_DEALT`
    // sentinel (255) by passing straight through as an ordinary card id.
    for (idx, c) in board.iter().enumerate() {
        if c.0 >= 52 { return Err(format!("board card {idx} has id {} outside 0..52 (spec 4.1)", c.0)); }
    }
    let mut ids: Vec<LibCard> = board.iter().map(|c| to_lib(*c)).collect();
    let mut sorted = ids.clone(); sorted.sort_unstable(); sorted.dedup();
    if sorted.len() != ids.len() { return Err("duplicate board card".into()); }
    let mut flop = [ids[0], ids[1], ids[2]]; flop.sort_unstable();
    let turn = if ids.len() > 3 { ids[3] } else { NOT_DEALT };
    let river = if ids.len() > 4 { ids[4] } else { NOT_DEALT };
    ids.clear();
    Ok((flop, turn, river))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_combo_roundtrip() {
        let as_ = Card::parse("As").unwrap(); let ks = Card::parse("Ks").unwrap();
        assert_eq!(to_lib(as_), 51); assert_eq!(to_lib(Card::parse("2c").unwrap()), 0);
        let mut r = Range1326([0.0; 1326]); r.0[combo_index(as_, ks) as usize] = 0.5;
        let lib = range_to_lib(&r).unwrap();
        assert_eq!(lib.get_weight_by_cards(to_lib(as_), to_lib(ks)), 0.5);
        let (hands, weights) = lib.get_hands_weights(0);
        assert_eq!((hands.len(), weights[0]), (1, 0.5));
        assert_eq!(lib_hand_to_combo(hands[0]), combo_index(as_, ks));
        assert!(range_to_lib(&Range1326([1.5; 1326])).is_err());
    }

    /// Proves `to_lib`/`from_lib` round-trip for all 52 cards: the seam the engine and worker
    /// depend on never silently drifts from the vendored library's own encoding.
    #[test]
    fn all_52_cards_roundtrip() {
        for id in 0..52u8 {
            let c = Card::checked(id).unwrap();
            assert_eq!(from_lib(to_lib(c)), c, "card id {id} did not round-trip");
            assert_eq!(to_lib(c), id, "to_lib({c}) != {id}; adapter must match the vendored 4*rank+suit encoding");
        }
    }

    /// Proves the combo-index mapping round-trips for all 1326 combos through the library's own
    /// hand representation, not just the id space.
    #[test]
    fn all_1326_combos_roundtrip() {
        for i in 0..1326u16 {
            let [lo, hi] = combo_cards(i);
            assert_eq!(combo_index(lo, hi), i);
            let lib_hand = (to_lib(lo), to_lib(hi));
            assert_eq!(lib_hand_to_combo(lib_hand), i, "combo {i} ({lo}{hi}) did not round-trip through the library encoding");
        }
    }

    /// T7-R2: `Card` is a public tuple struct (`pub struct Card(pub u8)`), so a caller can build
    /// `Card(255)` directly, bypassing `Card::checked`/`Card::new`. `255` is also the backend's own
    /// `NOT_DEALT` sentinel, so an unvalidated invalid id silently becomes "no card dealt" instead
    /// of a rejected board. `board_to_lib` must reject every id >= 52 before it reaches `to_lib`.
    #[test]
    fn board_to_lib_rejects_invalid_card_ids() {
        // The review's own repro: an invalid "turn" id that collides with the backend's NOT_DEALT sentinel.
        let err = board_to_lib(&[Card(0), Card(1), Card(2), Card(255)]).unwrap_err();
        assert!(err.contains("255"), "error should name the invalid id: {err}");
        assert!(err.contains('3'), "error should name the offending index (3): {err}");

        assert!(board_to_lib(&[Card(52), Card(1), Card(2)]).is_err(), "invalid flop id must be rejected");
        assert!(board_to_lib(&[Card(0), Card(1), Card(2), Card(52)]).is_err(), "invalid turn id must be rejected");
        assert!(board_to_lib(&[Card(0), Card(1), Card(2), Card(3), Card(255)]).is_err(), "invalid river id must be rejected");
    }

    #[test]
    #[should_panic]
    fn to_lib_rejects_invalid_card_id() {
        let _ = to_lib(Card(255));
    }

    #[test]
    fn board_to_lib_valid_boards_order_flop_and_preserve_turn_river() {
        let flop_cards = [Card::parse("2c").unwrap(), Card::parse("Kd").unwrap(), Card::parse("7h").unwrap()];
        let mut expected_flop = [to_lib(flop_cards[0]), to_lib(flop_cards[1]), to_lib(flop_cards[2])];
        expected_flop.sort_unstable();

        let (flop3, turn3, river3) = board_to_lib(&flop_cards).unwrap();
        assert_eq!(flop3, expected_flop);
        assert_eq!(turn3, NOT_DEALT);
        assert_eq!(river3, NOT_DEALT);

        let turn_card = Card::parse("As").unwrap();
        let board4 = [flop_cards[0], flop_cards[1], flop_cards[2], turn_card];
        let (flop4, turn4, river4) = board_to_lib(&board4).unwrap();
        assert_eq!(flop4, expected_flop);
        assert_eq!(turn4, to_lib(turn_card));
        assert_eq!(river4, NOT_DEALT);

        let river_card = Card::parse("9s").unwrap();
        let board5 = [flop_cards[0], flop_cards[1], flop_cards[2], turn_card, river_card];
        let (flop5, turn5, river5) = board_to_lib(&board5).unwrap();
        assert_eq!(flop5, expected_flop);
        assert_eq!(turn5, to_lib(turn_card));
        assert_eq!(river5, to_lib(river_card));
    }
}
