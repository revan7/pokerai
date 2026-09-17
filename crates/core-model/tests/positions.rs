use core_model::*;
use proto::*;

fn cfg(sb: u32, bb: u32, straddle: Option<u32>) -> HandConfig {
    HandConfig { config_revision: 1, sb_chips: sb, bb_chips: bb, straddle: straddle.map(|a| UtgStraddle { amount_chips: a }), rake: Rake::TimeCharge, chip_label: "$1".into() }
}
fn seats(ids: &[u8]) -> Vec<Seat> { ids.iter().map(|i| Seat(*i)).collect() }

#[test]
fn card_parser_roundtrip() {
    let hand = parse_hand("AsKd").unwrap();
    assert_eq!(cards_to_string(&hand), "AsKd");
    let board = parse_cards("Qs Jd 7h 3c 2d").unwrap();
    assert_eq!(board.len(), 5);
    assert_eq!(cards_to_string(&board), "QsJd7h3c2d");
    assert_eq!(parse_cards("QsJd7h3c2d").unwrap(), board);
    assert_eq!(parse_card(" Td ").unwrap(), Card(33));
    assert!(matches!(parse_hand("AsAs"), Err(RulesError::Card(CardParseError::Duplicate(_)))));
    assert!(parse_hand("As").is_err());
    assert!(parse_hand("AsKdQc").is_err());
    assert!(parse_cards("1s").is_err());
    assert!(parse_cards("AsK").is_err());
}

#[test]
fn straddle_action_order_utg() {
    let dealt = seats(&[0, 1, 2, 3, 4, 5]);
    let btn = Seat(5);
    assert_eq!(positions(btn, &dealt), vec![(Seat(0), Position::Sb), (Seat(1), Position::Bb), (Seat(2), Position::Utg), (Seat(3), Position::Hj), (Seat(4), Position::Co), (Seat(5), Position::Btn)]);
    assert_eq!(preflop_order(btn, &dealt, false), seats(&[2, 3, 4, 5, 0, 1]));
    assert_eq!(preflop_order(btn, &dealt, true), seats(&[3, 4, 5, 0, 1, 2]), "HJ, CO, BTN, SB, BB, UTG");
    assert_eq!(postflop_order(btn, &dealt), seats(&[0, 1, 2, 3, 4, 5]), "SB, BB, UTG, HJ, CO, BTN");
    assert_eq!(straddle_posts(&cfg(1, 2, Some(4))), Some([0.25, 0.5, 1.0]));
    assert_eq!(straddle_posts(&cfg(2, 5, Some(10))), Some([0.2, 0.5, 1.0]));
    assert_eq!(straddle_posts(&cfg(1, 2, None)), None);
    assert_eq!(initial_full_raise(&cfg(1, 2, Some(4))), 4);
    assert_eq!(initial_full_raise(&cfg(2, 5, None)), 5);
    assert_eq!(posts(&cfg(1, 2, Some(4))), vec![(0, 1), (1, 2), (2, 4)]);
    // button elsewhere: seat 2 is the button, so seat 3 posts the SB
    assert_eq!(preflop_order(Seat(2), &dealt, true), seats(&[0, 1, 2, 3, 4, 5]));
}

#[test]
fn dealt_seats_3_to_6() {
    let c = cfg(1, 2, None);
    let three = seats(&[5, 0, 1]);
    assert_eq!(validate_table(&c, Seat(5), &three).unwrap(), seats(&[0, 1, 5]));
    assert_eq!(positions(Seat(5), &three), vec![(Seat(0), Position::Sb), (Seat(1), Position::Bb), (Seat(5), Position::Btn)]);
    let pre = preflop_order(Seat(5), &three, false);
    assert_eq!(pre, seats(&[5, 0, 1]));
    assert_eq!(postflop_order(Seat(5), &three), seats(&[0, 1, 5]));
    let four = seats(&[5, 0, 1, 2]);
    assert_eq!(position_of(Seat(5), &four, Seat(2)), Some(Position::Co));
    assert_eq!(preflop_order(Seat(5), &four, false), seats(&[2, 5, 0, 1]));
    assert_eq!(postflop_order(Seat(5), &four), seats(&[0, 1, 2, 5]));
    let five = seats(&[5, 0, 1, 2, 3]);
    assert_eq!(position_of(Seat(5), &five, Seat(2)), Some(Position::Hj));
    assert_eq!(position_of(Seat(5), &five, Seat(3)), Some(Position::Co));
    assert_eq!(preflop_order(Seat(5), &five, false), seats(&[2, 3, 5, 0, 1]));
    let six = seats(&[0, 1, 2, 3, 4, 5]);
    assert_eq!(preflop_order(Seat(5), &six, false), seats(&[2, 3, 4, 5, 0, 1]));
    assert!(matches!(validate_table(&c, Seat(5), &seats(&[5, 0])), Err(RulesError::FormatUnsupported { detail }) if detail == "two dealt seats"));
    assert!(matches!(validate_table(&cfg(1, 2, Some(4)), Seat(5), &five), Err(RulesError::FormatUnsupported { .. })));
    assert!(matches!(validate_table(&cfg(1, 2, Some(3)), Seat(5), &six), Err(RulesError::FormatUnsupported { detail }) if detail == "short straddle post"));
    assert!(validate_table(&c, Seat(4), &three).is_err(), "button must be dealt");
    assert!(validate_table(&c, Seat(5), &seats(&[5, 0, 0])).is_err(), "duplicate seat");
}

#[test]
fn straddle_validation_does_not_overflow() {
    let six = seats(&[0, 1, 2, 3, 4, 5]);
    // Mathematical minimum straddle for bb_chips = 2_147_483_648 is 4_294_967_296, which is
    // outside u32::MAX (4_294_967_295). This must be rejected via `FormatUnsupported`, not by
    // panicking (overflow checks on) or silently wrapping `2 * bb_chips` to a small/zero value
    // that would make the short-straddle comparison falsely pass (overflow checks off).
    let overflowing = cfg(1, 2_147_483_648, Some(2_147_483_648));
    assert!(matches!(
        validate_table(&overflowing, Seat(5), &six),
        Err(RulesError::FormatUnsupported { detail }) if detail == "short straddle post"
    ));

    // Valid boundary: straddle exactly equals `2 * bb_chips`, with both operands large enough
    // that the doubling itself would overflow u32 (2_147_483_647 * 2 = 4_294_967_294, which
    // fits, but the naive `u32` multiplication path is exercised right at the edge of the
    // representable range). Must be accepted, not rejected.
    let boundary = cfg(1, 2_147_483_647, Some(4_294_967_294));
    assert!(validate_table(&boundary, Seat(5), &six).is_ok());
}
