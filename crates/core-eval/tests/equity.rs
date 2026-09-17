use core_eval::*;
use core_ranges::{block_public, parse_range};
use proto::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn cards(text: &str) -> Vec<Card> { text.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap().parse().unwrap()).collect() }
fn combo(text: &str) -> ComboIndex { let c = cards(text); combo_index(c[0], c[1]) }
fn player(seat: u8, text: &str, board: &[Card]) -> PlayerRange {
    let mut range = parse_range(text).unwrap();
    block_public(&mut range, board);
    PlayerRange { seat: Seat(seat), range }
}
fn no_cancel() -> AtomicBool { AtomicBool::new(false) }
fn share(res: &EquityResult, seat: u8) -> f32 { res.shares.iter().find(|s| s.seat == Seat(seat) && s.pot_index == 0).unwrap().value }

/// A one-combo player range: the "fixed hero combo" form of `PlayerRange` (spec section 3.5).
fn fixed(seat: u8, text: &str) -> PlayerRange {
    PlayerRange { seat: Seat(seat), range: parse_range(text).unwrap() }
}

fn share_in(res: &EquityResult, pot: u8, seat: u8) -> f32 {
    res.shares.iter().find(|s| s.seat == Seat(seat) && s.pot_index == pot).unwrap().value
}

#[test]
fn terminal_payoff_equity_times_pot() {
    let board = cards("QsJd7h3c2d");
    let oop = player(0, "AA,KK,QQ,JJ,TT,AKs,AQs,54s", &board).range;
    let ip = player(1, "QQ,JJ,77,AKo,54o,T9s", &board).range;
    let eq_oop = per_combo_equity(&oop, &ip, &board);
    let eq_ip = per_combo_equity(&ip, &oop, &board);
    let unraked = Rake::TimeCharge;
    let raked = Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false };
    for (range, eq) in [(&oop, &eq_oop), (&ip, &eq_ip)] {
        for i in 0..1326u16 {
            if range.get(i) == 0.0 { assert_eq!(eq[i as usize], 0.0); continue; }
            let e = eq[i as usize];
            assert!((0.0..=1.0).contains(&e));
            assert!((terminal_payoff(e, 100, &unraked) - e * 100.0).abs() < 1e-4);
            assert!((terminal_payoff(e, 100, &raked) - e * 95.0).abs() < 1e-4, "5% of 100 capped at 5 chips");
        }
    }
    let aces = eq_oop[combo("AcAd") as usize];
    assert!(aces > 0.0 && aces < 1.0, "AcAd loses to the sets and beats the rest: {aces}");
    let aa = per_combo_equity(&parse_range("AA").unwrap(), &parse_range("54o").unwrap(), &board);
    assert_eq!(aa[combo("AcAd") as usize], 1.0, "aces beat 54o on Q J 7 3 2");
    let set = per_combo_equity(&parse_range("QQ").unwrap(), &parse_range("AA").unwrap(), &board);
    assert_eq!(set[combo("QcQd") as usize], 1.0);
    // swapping seats leaves every value unchanged
    let a = EquityRequest::single_pot(board.clone(), vec![player(0, "AA,KK,QQ,JJ,TT,AKs,AQs,54s", &board), player(1, "QQ,JJ,77,AKo,54o,T9s", &board)], EquityMode::Exact);
    let b = EquityRequest::single_pot(board.clone(), vec![player(1, "QQ,JJ,77,AKo,54o,T9s", &board), player(0, "AA,KK,QQ,JJ,TT,AKs,AQs,54s", &board)], EquityMode::Exact);
    let ra = equity(&a, Duration::from_secs(10), &no_cancel());
    let rb = equity(&b, Duration::from_secs(10), &no_cancel());
    assert_eq!(ra.status, EquityStatus::Ready);
    assert_eq!(ra.method, Some(EquityMethod::Exact));
    assert!((share(&ra, 0) - share(&rb, 0)).abs() < 1e-6);
    assert!((share(&ra, 0) + share(&ra, 1) - 1.0).abs() < 1e-6);
}

#[test]
fn equity_budget_respected() {
    let board = cards("Kh7d2c");
    let req = EquityRequest::single_pot(board.clone(), vec![player(0, "random", &board), player(1, "random", &board)], EquityMode::Exact);
    assert!(exact_cost(&req) > 20_000_000);
    let budget = Duration::from_millis(100);
    let start = Instant::now();
    let res = equity(&req, budget, &no_cancel());
    let took = start.elapsed();
    assert_eq!(res.status, EquityStatus::BudgetExceeded);
    assert!(took < budget + Duration::from_millis(50), "stopped {took:?} after a {budget:?} budget");
    assert!(res.shares.is_empty());
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let setter = std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(30)); flag.store(true, Ordering::Relaxed); });
    let start = Instant::now();
    let res = equity(&req, Duration::from_secs(10), &cancel);
    let took = start.elapsed();
    setter.join().unwrap();
    assert_eq!(res.status, EquityStatus::Cancelled);
    assert!(took < Duration::from_millis(80), "cancelled {took:?} after a 30 ms flag");
}

/// Hand-checkable exact case: AcAd against KcKd on the turn Qs Jd 7h 3c.
///
/// 52 - 4 board - 4 hole = 44 single-card runouts, each equally likely. The board is rainbow
/// (one card of each suit), so no runout can complete a flush; Q J 7 3 holds no two adjacent
/// ranks, so no runout can complete a straight for either holding. Aces therefore lose exactly
/// when a king arrives (Kh or Ks: two cards, giving trip kings against one pair of aces) and
/// win every other runout; no runout can tie, because one player always holds the higher pair
/// or better. Hero equity = 42/44 = 0.954545..., villain = 2/44 = 0.045454...
#[test]
fn turn_aces_versus_kings_has_exactly_two_outs() {
    let board = cards("QsJd7h3c");
    let req = EquityRequest::single_pot(board, vec![fixed(0, "AcAd"), fixed(1, "KcKd")], EquityMode::Exact);
    assert_eq!(exact_cost(&req), 44, "one runout card from a 44-card deck");
    let res = equity(&req, Duration::from_secs(10), &no_cancel());
    assert_eq!(res.status, EquityStatus::Ready);
    assert_eq!(res.method, Some(EquityMethod::Exact));
    // 44 runouts x 2 players = 88 rank evaluations, the unit `Deadline::tick` counts.
    assert_eq!(res.samples, 88);
    assert!((share(&res, 0) - 42.0 / 44.0).abs() < 1e-6, "aces: {}", share(&res, 0));
    assert!((share(&res, 1) - 2.0 / 44.0).abs() < 1e-6, "kings: {}", share(&res, 1));
    assert!((share(&res, 0) + share(&res, 1) - 1.0).abs() < 1e-6);
}

/// Side-pot eligibility and the equal tie split, on a river where every rank is hand-checkable.
///
/// Board 2c 7d 9h Js 4s: no two adjacent ranks (no straight) and only two spades (no flush).
/// Seat 0 (AsAd) plays A A J 9 7, seat 1 (KhKd) and seat 2 (KsKc) both play K K J 9 7, the
/// identical five-card hand. The main pot (all three eligible) goes entirely to seat 0; the
/// side pot (seats 1 and 2 only) is a tie and splits equally.
#[test]
fn side_pot_eligibility_and_tie_split() {
    let board = cards("2c7d9hJs4s");
    let req = EquityRequest {
        board,
        players: vec![fixed(0, "AsAd"), fixed(1, "KhKd"), fixed(2, "KsKc")],
        mode: EquityMode::Exact,
        pots: vec![
            PotEligibility { pot_index: 0, eligible: vec![Seat(0), Seat(1), Seat(2)] },
            PotEligibility { pot_index: 1, eligible: vec![Seat(1), Seat(2)] },
        ],
    };
    let res = equity(&req, Duration::from_secs(10), &no_cancel());
    assert_eq!(res.status, EquityStatus::Ready);
    assert_eq!(res.shares.len(), 5, "three eligible seats in the main pot, two in the side pot");
    assert_eq!(share_in(&res, 0, 0), 1.0);
    assert_eq!(share_in(&res, 0, 1), 0.0);
    assert_eq!(share_in(&res, 0, 2), 0.0);
    assert_eq!(share_in(&res, 1, 1), 0.5);
    assert_eq!(share_in(&res, 1, 2), 0.5);
    // Every pot's shares sum to 1 (spec section 12, per-pot shares).
    for pot in 0..2u8 {
        let total: f32 = res.shares.iter().filter(|s| s.pot_index == pot).map(|s| s.value).sum();
        assert!((total - 1.0).abs() < 1e-6, "pot {pot} sums to {total}");
    }
}

/// `exact_cost` counts runouts as unordered combinations of the remaining deck.
#[test]
fn exact_cost_counts_unordered_runouts() {
    let flop = cards("Kh7d2c");
    let req = EquityRequest::single_pot(flop, vec![fixed(0, "AcAd"), fixed(1, "KcKd")], EquityMode::Exact);
    // 52 - 3 board - 4 hole = 45 cards, two to come: C(45, 2) = 990.
    assert_eq!(exact_cost(&req), 990);

    let req = EquityRequest::single_pot(vec![], vec![fixed(0, "AcAd"), fixed(1, "KcKd")], EquityMode::Exact);
    // 48 cards, five to come: C(48, 5) = 1_712_304.
    assert_eq!(exact_cost(&req), 1_712_304);

    let river = cards("2c7d9hJs4s");
    let req = EquityRequest::single_pot(river, vec![fixed(0, "AsAd"), fixed(1, "KhKd")], EquityMode::Exact);
    // One tuple, no cards to come: C(41, 0) = 1.
    assert_eq!(exact_cost(&req), 1);
}

/// `InvalidRanges` is the status for well-formed ranges that cannot produce a showdown.
#[test]
fn invalid_ranges_when_no_showdown_is_possible() {
    // Every combo of the range is blocked by the board: empty support.
    let board = cards("AcKcQcJcTc");
    let req = EquityRequest::single_pot(board.clone(), vec![player(0, "AcKc", &board), player(1, "AA", &board)], EquityMode::Exact);
    let res = equity(&req, Duration::from_secs(10), &no_cancel());
    assert_eq!(res.status, EquityStatus::InvalidRanges);
    assert!(res.shares.is_empty());
    assert_eq!(res.method, None);

    // Non-empty supports, but the only combo of each collides with the other: no disjoint tuple.
    let board = cards("2c7d9h");
    let req = EquityRequest::single_pot(board, vec![fixed(0, "AsAd"), fixed(1, "AsAd")], EquityMode::Exact);
    let res = equity(&req, Duration::from_secs(10), &no_cancel());
    assert_eq!(res.status, EquityStatus::InvalidRanges);
    assert!(res.shares.is_empty());
}

// Always-on preconditions (never `debug_assert!`): each probe is a caller bug that would
// otherwise surface as a panic several frames deeper, as a wrapped subtraction, or as a
// silently wrong share. They must hold under `--release` too.

#[test]
#[should_panic(expected = "exceeds the 5-card maximum")]
fn equity_rejects_an_oversized_board() {
    let board = cards("2c3c4c5c6c7c");
    let req = EquityRequest::single_pot(board, vec![fixed(0, "AsAd"), fixed(1, "KhKd")], EquityMode::Exact);
    equity(&req, Duration::from_secs(1), &no_cancel());
}

#[test]
#[should_panic(expected = "duplicate board card")]
fn equity_rejects_a_duplicated_board_card() {
    let board = cards("2c3c2c");
    let req = EquityRequest::single_pot(board, vec![fixed(0, "AsAd"), fixed(1, "KhKd")], EquityMode::Exact);
    equity(&req, Duration::from_secs(1), &no_cancel());
}

#[test]
#[should_panic(expected = "seat 0 appears twice in players")]
fn equity_rejects_a_repeated_seat() {
    let board = cards("2c7d9h");
    let req = EquityRequest::single_pot(board, vec![fixed(0, "AsAd"), fixed(0, "KhKd")], EquityMode::Exact);
    equity(&req, Duration::from_secs(1), &no_cancel());
}

#[test]
#[should_panic(expected = "which is not a player")]
fn equity_rejects_an_eligible_seat_that_is_not_a_player() {
    let req = EquityRequest {
        board: cards("2c7d9hJs4s"),
        players: vec![fixed(0, "AsAd"), fixed(1, "KhKd")],
        mode: EquityMode::Exact,
        pots: vec![PotEligibility { pot_index: 0, eligible: vec![Seat(0), Seat(5)] }],
    };
    equity(&req, Duration::from_secs(1), &no_cancel());
}

#[test]
#[should_panic(expected = "is not finite in [0, 1]")]
fn equity_rejects_a_nan_weight() {
    let board = cards("2c7d9hJs4s");
    let nan = PlayerRange { seat: Seat(0), range: Range1326::from_fn(|i| if i == 0 { f32::NAN } else { 0.0 }) };
    let req = EquityRequest::single_pot(board, vec![nan, fixed(1, "KhKd")], EquityMode::Exact);
    equity(&req, Duration::from_secs(1), &no_cancel());
}

#[test]
#[should_panic(expected = "exceeds the 5-card maximum")]
fn per_combo_equity_checks_the_board_even_with_no_supported_hero_combo() {
    // Hero has no supported combo, so the per-combo loop never runs: the board must still be
    // rejected instead of returning 1326 zeros for an impossible request.
    per_combo_equity(&Range1326::zero(), &parse_range("AA").unwrap(), &cards("2c3c4c5c6c7c"));
}

/// `terminal_payoff` = `equity * (pot - min(rate * pot, cap))`; a time charge takes no rake.
#[test]
fn terminal_payoff_applies_the_cap_and_skips_a_time_charge() {
    let raked = Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false };
    // 5% of 1000 is 50 chips, above the 5.000-chip cap, so the rake is the cap.
    assert!((terminal_payoff(1.0, 1000, &raked) - 995.0).abs() < 1e-3);
    // 5% of 60 is 3 chips, below the cap, so the rake is the percentage.
    assert!((terminal_payoff(1.0, 60, &raked) - 57.0).abs() < 1e-4);
    assert_eq!(terminal_payoff(0.0, 1000, &raked), 0.0);
    // A time charge is collected away from the pot: the whole pot is paid out.
    assert_eq!(terminal_payoff(1.0, 1000, &Rake::TimeCharge), 1000.0);
    assert_eq!(terminal_payoff(0.5, 200, &Rake::TimeCharge), 100.0);
    // `no_flop_no_drop` selects whether the caller rakes at all; it never changes the amount.
    let dropped = Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: true };
    assert_eq!(terminal_payoff(1.0, 60, &dropped), terminal_payoff(1.0, 60, &raked));
}
