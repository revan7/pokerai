use core_model::betting::Round;
use proto::*;

/// Six seats, button 5: preflop order UTG(2), HJ(3), CO(4), BTN(5), SB(0), BB(1); blinds 1/2.
fn preflop(stacks: [u32; 6]) -> Round {
    let mut r = Round::open(Street::Preflop, vec![Seat(2), Seat(3), Seat(4), Seat(5), Seat(0), Seat(1)], stacks, [false; 6], [false; 6], 2);
    r.post(Seat(0), 1);
    r.post(Seat(1), 2);
    r
}
fn has_raise(legal: &[LegalAction]) -> Option<(u32, u32)> {
    legal.iter().find_map(|l| match l { LegalAction::Raise { min_to, max_to } | LegalAction::Bet { min_to, max_to } => Some((*min_to, *max_to)), _ => None })
}
fn has_allin(legal: &[LegalAction]) -> bool { legal.iter().any(|l| matches!(l, LegalAction::AllIn { .. })) }

/// The street's share of the conservation invariant (spec 4.3): chips only move between a seat's
/// stack and its commitment, so this total never changes while the street runs.
fn chips_in_play(r: &Round) -> u64 { (0..6).map(|i| u64::from(r.stacks[i]) + u64::from(r.committed[i])).sum() }

#[test]
fn min_raise_and_short_allin_no_reopen() {
    let mut r = preflop([200, 200, 200, 14, 200, 200]);
    assert_eq!(r.to_act(), Some(Seat(2)));
    assert_eq!(has_raise(&r.legal()), Some((4, 200)));
    r.apply(Seat(2), Action::Raise { to: 10 }).unwrap();
    assert_eq!(r.last_full_raise, 8);
    assert_eq!(r.to_act(), Some(Seat(3)));
    let legal = r.legal();
    assert_eq!(legal, vec![LegalAction::Fold, LegalAction::Call { cost: 10 }, LegalAction::AllIn { to: 14 }], "14 < min raise-to 18: all-in only");
    assert_eq!(r.apply(Seat(3), Action::AllIn { to: 14 }).unwrap(), (Action::AllIn { to: 14 }, 14));
    assert_eq!(r.last_full_raise, 8, "a short all-in never becomes the full raise");
    assert_eq!(has_raise(&r.legal()), Some((22, 200)), "CO faces 14 with a full raise of 8");
    for seat in [4, 5, 0] { r.apply(Seat(seat), Action::Fold).unwrap(); }
    r.apply(Seat(1), Action::Call).unwrap();
    assert_eq!(r.to_act(), Some(Seat(2)), "the raiser owes a response to the short all-in");
    let legal = r.legal();
    assert_eq!(legal, vec![LegalAction::Fold, LegalAction::Call { cost: 4 }], "a single short all-in does not reopen");
    assert!(matches!(r.apply(Seat(2), Action::Raise { to: 30 }), Err(core_model::RulesError::IllegalAction { .. })));
    r.apply(Seat(2), Action::Call).unwrap();
    assert!(r.closed());
}

#[test]
fn cumulative_short_allins_reopen() {
    for (shorts, reopens) in [((14, 17), false), ((15, 19), true)] {
        let mut r = preflop([200, 200, 200, shorts.0, shorts.1, 200]);
        r.apply(Seat(2), Action::Raise { to: 10 }).unwrap();
        r.apply(Seat(3), Action::AllIn { to: shorts.0 }).unwrap();
        r.apply(Seat(4), Action::AllIn { to: shorts.1 }).unwrap();
        assert_eq!(has_raise(&r.legal()), Some((shorts.1 + 8, 200)), "BTN: facing + last full raise");
        r.apply(Seat(5), Action::Call).unwrap();
        r.apply(Seat(0), Action::Fold).unwrap();
        r.apply(Seat(1), Action::Fold).unwrap();
        assert_eq!(r.to_act(), Some(Seat(2)));
        let legal = r.legal();
        assert_eq!(has_raise(&legal).is_some(), reopens, "cumulative {} vs full raise 8", shorts.1 - 10);
        assert_eq!(has_allin(&legal), reopens);
        if reopens { assert_eq!(has_raise(&legal), Some((27, 200))); }
    }
}

#[test]
fn straddle_min_raise_and_normalization() {
    let mut r = Round::open(Street::Preflop, vec![Seat(3), Seat(4), Seat(5), Seat(0), Seat(1), Seat(2)], [200; 6], [false; 6], [false; 6], 4);
    r.post(Seat(0), 1); r.post(Seat(1), 2); r.post(Seat(2), 4);
    assert_eq!(r.to_act(), Some(Seat(3)));
    assert_eq!(has_raise(&r.legal()), Some((8, 200)), "minimum open over a straddle is 2S");
    r.apply(Seat(3), Action::Raise { to: 8 }).unwrap();
    assert_eq!(has_raise(&r.legal()), Some((12, 200)));
    assert_eq!(r.apply(Seat(4), Action::Raise { to: 200 }).unwrap(), (Action::AllIn { to: 200 }, 200), "a raise to the stack is recorded as all-in");
    assert_eq!(r.legal(), vec![LegalAction::Fold, LegalAction::Call { cost: 200 }], "covered seats cannot raise");
    assert_eq!(r.apply(Seat(5), Action::AllIn { to: 200 }).unwrap(), (Action::Call, 200), "an all-in that only matches is a call");
    assert!(matches!(r.apply(Seat(0), Action::Check), Err(_)));
    assert!(matches!(r.apply(Seat(1), Action::Fold), Err(_)), "seat 0 is to act");
    let mut flop = Round::open(Street::Flop, vec![Seat(0), Seat(1)], [100, 50, 0, 0, 0, 0], [false, false, true, true, true, true], [false; 6], 2);
    assert_eq!(flop.legal(), vec![LegalAction::Check, LegalAction::Bet { min_to: 2, max_to: 100 }, LegalAction::AllIn { to: 100 }]);
    assert!(flop.apply(Seat(0), Action::Raise { to: 10 }).is_err(), "no wager pending: bet, not raise");
    flop.apply(Seat(0), Action::Bet { to: 10 }).unwrap();
    assert!(flop.apply(Seat(1), Action::Bet { to: 30 }).is_err(), "wager pending: raise, not bet");
    flop.apply(Seat(1), Action::Raise { to: 30 }).unwrap();
    assert_eq!(flop.last_full_raise, 20);
    assert_eq!(has_raise(&flop.legal()), Some((50, 100)));
}

/// Standing ruling: chip arithmetic that can exceed `u32` on valid inputs is widened to `u64`
/// before comparing. `facing + last_full_raise` overflows at deep stacks, and a wrapped minimum
/// would advertise a "raise" far below the wager already facing the actor. Also checks the
/// street's share of the conservation invariant (spec 4.3): chips only move stack <-> commitment.
#[test]
fn deep_stacks_never_wrap_the_minimum_raise() {
    const MAX: u32 = u32::MAX;
    let mut r = Round::open(Street::Flop, vec![Seat(0), Seat(1)], [MAX, MAX, 0, 0, 0, 0], [false, false, true, true, true, true], [false; 6], 2_000_000_000);
    let start = 2 * u64::from(MAX);
    assert_eq!(chips_in_play(&r), start);
    assert_eq!(r.legal(), vec![LegalAction::Check, LegalAction::Bet { min_to: 2_000_000_000, max_to: MAX }, LegalAction::AllIn { to: MAX }]);
    r.apply(Seat(0), Action::Bet { to: 3_000_000_000 }).unwrap();
    assert_eq!(r.last_full_raise, 3_000_000_000);
    assert_eq!(r.min_raise_to(), MAX, "6e9 saturates; wrapping would give 1_705_032_704");
    assert_eq!(r.legal(), vec![LegalAction::Fold, LegalAction::Call { cost: 3_000_000_000 }, LegalAction::AllIn { to: MAX }], "the minimum raise is unreachable: all-in only");
    assert!(matches!(r.apply(Seat(1), Action::Raise { to: 4_000_000_000 }), Err(core_model::RulesError::IllegalAction { .. })), "4e9 is below the true minimum raise-to of 6e9");
    assert_eq!(r.apply(Seat(1), Action::AllIn { to: MAX }).unwrap(), (Action::AllIn { to: MAX }, MAX));
    assert_eq!(chips_in_play(&r), start);
    r.apply(Seat(0), Action::Call).unwrap();
    assert!(r.closed());
    assert_eq!(chips_in_play(&r), start, "a closed street has moved no chips in or out");
}

/// Review R1: the planned lifecycle caller (`Sim::open_street`, plan 1 task 12) passes the full
/// dealt-seat order with the current folded flags, so a seat folded on an earlier street is an
/// ordinary member of `order` and must simply never be given a turn.
#[test]
fn folded_dealt_seat_can_remain_in_street_order() {
    let order: Vec<Seat> = (0..6u8).map(Seat).collect();
    let mut r = Round::open(Street::Flop, order, [100; 6], [false, false, true, false, false, false], [false; 6], 2);
    assert_eq!(r.eligible_count(), 5);
    assert_eq!(r.to_act(), Some(Seat(0)));
    r.apply(Seat(0), Action::Check).unwrap();
    r.apply(Seat(1), Action::Check).unwrap();
    assert_eq!(r.to_act(), Some(Seat(3)), "the folded seat never gets a turn");
    r.apply(Seat(3), Action::Bet { to: 10 }).unwrap();
    let queued: Vec<Seat> = r.pending.iter().copied().collect();
    assert_eq!(queued, vec![Seat(4), Seat(5), Seat(0), Seat(1)], "the wager reopens every live seat and skips the folded one");
}

/// Review R2: a short big blind does not lower the bring-in (BCLC no-limit rules 33.0 r1.2).
/// Blinds 1/2 with a one-chip BB: the BB is all-in for the chip it has, while everyone else
/// still enters for 2 and raises to at least 4. The nominal post is the wager faced; the
/// poster's commitment stays capped at its stack, so no chips are manufactured.
#[test]
fn short_big_blind_preserves_the_full_bring_in() {
    const TOTAL: u64 = 200 + 1 + 4 * 200;
    let mut r = preflop([200, 1, 200, 200, 200, 200]);
    assert_eq!(chips_in_play(&r), TOTAL);
    assert_eq!((r.committed[1], r.stacks[1], r.all_in[1]), (1, 0, true), "the big blind commits only what it has");
    assert_eq!(r.facing, 2, "the nominal big blind is the wager to face");
    assert_eq!(r.to_act(), Some(Seat(2)));
    assert_eq!(r.legal(), vec![LegalAction::Fold, LegalAction::Call { cost: 2 }, LegalAction::Raise { min_to: 4, max_to: 200 }, LegalAction::AllIn { to: 200 }]);
    assert!(matches!(r.apply(Seat(2), Action::Raise { to: 3 }), Err(core_model::RulesError::IllegalAction { .. })), "3 is below the minimum raise-to of 4");
    assert_eq!(r.apply(Seat(2), Action::Call).unwrap(), (Action::Call, 2), "the full blind is called, not the chip the BB could cover");
    assert_eq!(chips_in_play(&r), TOTAL, "a short blind manufactures no chips");
    assert_eq!(r.to_act(), Some(Seat(3)), "the all-in big blind owes no response");
    assert!(!r.pending.contains(&Seat(1)));
}

/// Standing ruling: the infallible constructor enforces its documented contract with always-on
/// assertions. A live seat left out of the action order would never be given a turn, yet would
/// still count as a responder in the reopening test.
#[test]
#[should_panic(expected = "action order")]
fn open_rejects_a_live_seat_missing_from_the_order() {
    let _ = Round::open(Street::Flop, vec![Seat(0)], [100, 100, 0, 0, 0, 0], [false, false, true, true, true, true], [false; 6], 2);
}

/// T1 (final review): a seat that posts twice keeps `committed[i] <= facing` -- the guard is the
/// commitment, not the nominal amount (`Round::post`'s `.max(committed[i])`).
///
/// That `.max` term is the only branch in `Round` with no coverage, and what it protects is an
/// always-on assert: `owed` asserts `facing >= committed[i]`, so a `facing` left behind a
/// double-poster's commitment turns `legal()` into a process abort.
#[test]
fn a_double_post_raises_the_bring_in_to_the_commitment() {
    let mut r = Round::open(
        Street::Preflop,
        vec![Seat(0), Seat(1)],
        [100, 100, 0, 0, 0, 0],
        [false, false, true, true, true, true],
        [false; 6],
        2,
    );
    r.post(Seat(0), 1);
    r.post(Seat(0), 2);
    assert_eq!((r.committed[0], r.facing), (3, 3), "facing follows the commitment, not the 2-chip nominal");
    assert_eq!(
        r.legal(),
        vec![LegalAction::Check, LegalAction::Raise { min_to: 5, max_to: 100 }, LegalAction::AllIn { to: 100 }],
        "seat 1 is not the double-poster and must still get a legal set, not an assert"
    );
    assert_eq!(chips_in_play(&r), 200, "two posts by one seat manufacture no chips");
}
