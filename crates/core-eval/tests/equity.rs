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
    let (st_oop, eq_oop) = per_combo_equity(&oop, &ip, &board, Duration::from_secs(10), &no_cancel());
    assert_eq!(st_oop, EquityStatus::Ready);
    let (st_ip, eq_ip) = per_combo_equity(&ip, &oop, &board, Duration::from_secs(10), &no_cancel());
    assert_eq!(st_ip, EquityStatus::Ready);
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
    let (st, aa) = per_combo_equity(&parse_range("AA").unwrap(), &parse_range("54o").unwrap(), &board, Duration::from_secs(10), &no_cancel());
    assert_eq!(st, EquityStatus::Ready);
    assert_eq!(aa[combo("AcAd") as usize], 1.0, "aces beat 54o on Q J 7 3 2");
    let (st, set) = per_combo_equity(&parse_range("QQ").unwrap(), &parse_range("AA").unwrap(), &board, Duration::from_secs(10), &no_cancel());
    assert_eq!(st, EquityStatus::Ready);
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
    assert_within_bound_eventually("equity_budget_respected", budget + Duration::from_millis(50), || {
        let start = Instant::now();
        let res = equity(&req, budget, &no_cancel());
        (start.elapsed(), res)
    }, |res| {
        assert_eq!(res.status, EquityStatus::BudgetExceeded);
        assert!(res.shares.is_empty());
    });
    // The flag is raised through `cancel_after`, not a raw `std::thread::sleep`: this enumeration
    // is hundreds of milliseconds of work, so a 1 ms spun delay is unambiguously *inside* it on
    // either build profile, whereas a 30 ms `sleep` lands within a Windows timer tick (15.6 ms) of
    // its own overrun bound and made this assertion fail intermittently in release.
    assert_within_bound_eventually("equity_budget_respected cancel path", Duration::from_millis(200), || {
        let cancel = Arc::new(AtomicBool::new(false));
        let setter = cancel_after(&cancel, Duration::from_millis(1));
        let start = Instant::now();
        let res = equity(&req, Duration::from_secs(10), &cancel);
        let took = start.elapsed();
        setter.join().unwrap();
        (took, res)
    }, |res| {
        assert_eq!(res.status, EquityStatus::Cancelled);
    });
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

/// Every combo whose two cards both lie in the card-id block `lo..hi`.
fn block_range(lo: u8, hi: u8) -> Range1326 {
    Range1326::from_fn(|i| { let [a, b] = combo_cards(i); if a.0 >= lo && b.0 < hi { 1.0 } else { 0.0 } })
}

/// The review's R1 probe: four seats with broad disjoint supports, then two seats that both hold
/// the only remaining combo, so every assignment collides at the last seat and no runout — and so
/// no rank evaluation — ever happens. `exact_cost` is 55^4 = 9 150 625, below the section-7
/// threshold of 2 * 10^7, so the engine would legitimately choose exact enumeration here.
fn colliding_six_player() -> EquityRequest {
    let last = Range1326::from_fn(|i| if i == combo_index(Card(49), Card(50)) { 1.0 } else { 0.0 });
    EquityRequest::single_pot(
        vec![Card(44), Card(45), Card(46), Card(47), Card(48)],
        vec![
            PlayerRange { seat: Seat(0), range: block_range(0, 11) },
            PlayerRange { seat: Seat(1), range: block_range(11, 22) },
            PlayerRange { seat: Seat(2), range: block_range(22, 33) },
            PlayerRange { seat: Seat(3), range: block_range(33, 44) },
            PlayerRange { seat: Seat(4), range: last.clone() },
            PlayerRange { seat: Seat(5), range: last },
        ],
        EquityMode::Exact,
    )
}

/// R1: an already-cancelled request must not compute, and must not report `Ready`.
#[test]
fn already_cancelled_request_reports_cancelled_before_any_work() {
    let board = cards("2c7d9hJs4s");
    let req = EquityRequest::single_pot(board, vec![fixed(0, "AsAd"), fixed(1, "KhKd")], EquityMode::Exact);
    let cancel = AtomicBool::new(true);
    let res = equity(&req, Duration::from_secs(10), &cancel);
    assert_eq!(res.status, EquityStatus::Cancelled);
    assert!(res.shares.is_empty());
    assert_eq!(res.samples, 0, "no evaluation may run after the flag is already set");
    assert_eq!(res.method, None);
}

/// R1: a zero budget is an exhausted budget, whatever the clock's resolution.
#[test]
fn zero_budget_request_reports_budget_exceeded_before_any_work() {
    let board = cards("2c7d9hJs4s");
    let req = EquityRequest::single_pot(board, vec![fixed(0, "AsAd"), fixed(1, "KhKd")], EquityMode::Exact);
    let res = equity(&req, Duration::ZERO, &no_cancel());
    assert_eq!(res.status, EquityStatus::BudgetExceeded);
    assert!(res.shares.is_empty());
    assert_eq!(res.samples, 0);
    assert_eq!(res.method, None);
}

/// R1: the assignment search must observe the budget even when it completes no runout, so it
/// never reports `InvalidRanges` for a search it simply did not have time to finish.
#[test]
fn assignment_search_observes_the_budget_with_no_evaluations() {
    let req = colliding_six_player();
    assert_eq!(exact_cost(&req), 9_150_625);
    assert_within_bound_eventually("assignment_search_observes_the_budget_with_no_evaluations", Duration::from_millis(50), || {
        let start = Instant::now();
        let res = equity(&req, Duration::from_millis(1), &no_cancel());
        (start.elapsed(), res)
    }, |res| {
        assert_eq!(res.status, EquityStatus::BudgetExceeded, "not InvalidRanges: the search never finished");
        assert!(res.shares.is_empty());
        assert_eq!(res.samples, 0, "no runout completes, so no rank is evaluated");
    });
}

/// R1: the same search must observe a cancellation raised while it is running, which only a poll
/// inside the assignment traversal can catch.
///
/// The flag is raised through `cancel_after`, whose delay is spun, and after 1 ms rather than 20.
/// A raw `std::thread::sleep(20 ms)` was a genuine race: this search is 9,150,625 candidate visits,
/// about 18 ms in a release build, so the sleep and the search were the same order of magnitude and
/// the search often finished first — reporting `InvalidRanges`, which is the correct answer for a
/// completed proof and a wrong one for this test's intent. 1 ms is a hundred `tick_work` polls into
/// the traversal (one every 4,096 candidates, about 8 us) and a twentieth of the shortest run, so
/// the poll under test is the only thing that can end it, on either profile.
#[test]
fn assignment_search_observes_cancellation_with_no_evaluations() {
    let req = colliding_six_player();
    assert_within_bound_eventually("assignment_search_observes_cancellation_with_no_evaluations", Duration::from_millis(200), || {
        let cancel = Arc::new(AtomicBool::new(false));
        let setter = cancel_after(&cancel, Duration::from_millis(1));
        let start = Instant::now();
        let res = equity(&req, Duration::from_secs(10), &cancel);
        let took = start.elapsed();
        setter.join().unwrap();
        (took, res)
    }, |res| {
        assert_eq!(res.status, EquityStatus::Cancelled);
        assert!(res.shares.is_empty());
        assert_eq!(res.samples, 0, "no runout completes in this request, cancelled early or late");
    });
}

/// R2: non-uniform joint weights, hand-derived, with one partially colliding pair.
///
/// Board 2c 7d 9h Js 4s (no straight, no flush, as in `side_pot_eligibility_and_tie_split`).
/// A holds AsAd at 1.0 and KhKd at 0.5; B holds AsAc at 0.25 and KsKc at 1.0. `As` collides, so
/// three of the four tuples are compatible and each carries the product of its two weights:
///
/// | tuple | weight | showdown |
/// |---|---|---|
/// | AsAd x KsKc | 1.0 x 1.0 = 1.000 | A A J 9 7 beats K K J 9 7: A wins |
/// | KhKd x AsAc | 0.5 x 0.25 = 0.125 | K K J 9 7 loses to A A J 9 7: B wins |
/// | KhKd x KsKc | 0.5 x 1.0 = 0.500 | K K J 9 7 both ways: tie, 0.25 each |
///
/// Total weight 1.625 = 13/8. A's mass 1.0 + 0.25 = 1.25 = 10/8, so A = 10/13 and B = 3/13.
/// Per-combo: AsAd meets only KsKc (AsAc is blocked by `As`) and always wins, so 1.0 exactly;
/// KhKd meets both, winning 0.5 of 1.25 total weight, so 0.4 exactly. Unweighted enumeration
/// would give 0.5, 0.5 and 0.25 instead.
#[test]
fn nonuniform_joint_weights_drive_the_shares() {
    let board = cards("2c7d9hJs4s");
    let a = parse_range("AsAd:1.0,KhKd:0.5").unwrap();
    let b = parse_range("AsAc:0.25,KsKc:1.0").unwrap();
    let pa = PlayerRange { seat: Seat(0), range: a.clone() };
    let pb = PlayerRange { seat: Seat(1), range: b.clone() };

    let req = EquityRequest::single_pot(board.clone(), vec![pa.clone(), pb.clone()], EquityMode::Exact);
    let res = equity(&req, Duration::from_secs(10), &no_cancel());
    assert_eq!(res.status, EquityStatus::Ready);
    assert!((share(&res, 0) - 10.0 / 13.0).abs() < 1e-6, "A: {} expected 10/13", share(&res, 0));
    assert!((share(&res, 1) - 3.0 / 13.0).abs() < 1e-6, "B: {} expected 3/13", share(&res, 1));

    let (st_a, eq_a) = per_combo_equity(&a, &b, &board, Duration::from_secs(10), &no_cancel());
    assert_eq!(st_a, EquityStatus::Ready);
    assert_eq!(eq_a[combo("AsAd") as usize], 1.0, "AsAd blocks AsAc and beats KsKc");
    assert_eq!(eq_a[combo("KhKd") as usize], 0.4, "KhKd ties 1.0 of 1.25 total villain weight");

    // Player order is an implementation detail, not part of the population.
    let swapped = EquityRequest::single_pot(board.clone(), vec![pb, pa], EquityMode::Exact);
    let res_swapped = equity(&swapped, Duration::from_secs(10), &no_cancel());
    assert_eq!(res_swapped.status, EquityStatus::Ready);
    assert!((share(&res_swapped, 0) - share(&res, 0)).abs() < 1e-6);
    assert!((share(&res_swapped, 1) - share(&res, 1)).abs() < 1e-6);

    // Scaling one player's whole range by a common factor scales every tuple weight by that
    // factor, so the normalised shares are unchanged.
    let half = PlayerRange { seat: Seat(0), range: Range1326::from_fn(|i| a.get(i) * 0.5) };
    let rescaled = EquityRequest::single_pot(board, vec![half, PlayerRange { seat: Seat(1), range: b }], EquityMode::Exact);
    let res_rescaled = equity(&rescaled, Duration::from_secs(10), &no_cancel());
    assert_eq!(res_rescaled.status, EquityStatus::Ready);
    assert!((share(&res_rescaled, 0) - share(&res, 0)).abs() < 1e-6, "rescaled A: {}", share(&res_rescaled, 0));
    assert!((share(&res_rescaled, 1) - share(&res, 1)).abs() < 1e-6);
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
    per_combo_equity(&Range1326::zero(), &parse_range("AA").unwrap(), &cards("2c3c4c5c6c7c"), Duration::from_secs(1), &no_cancel());
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

/// Spec 13.1 `equity_mc_within_standard_error`: the Monte Carlo estimate of 20 spots lands inside
/// four of its own reported standard errors of the exact answer, and the reported standard error
/// is exactly the binomial `sqrt(v * (1 - v) / samples)`.
#[test]
fn equity_mc_within_standard_error() {
    // Spec 13.1 requires 20 spots. The board changes every four spots while the hero cycles with
    // period 5 and the villain with period 4, so the 20 (hero, board, villain) triples are distinct
    // and no hero shares a card with its board (7h7s, not 7h7d, keeps Kd7d2c clean).
    let heroes = ["AsKs", "7h7s", "QdJd", "9c8c", "AhQh"];
    let villains = ["QQ+,AKs", "22+,A2s+,KTs+", "JJ-77,AQo,KJs", "random"];
    let boards = ["Jh9h6c", "Kd7d2c", "8s8d3c", "AcQs5h", "Ts6s2d"];
    let mut checked = 0;
    for k in 0..20 {
        let name = boards[(k / 4) % 5];
        let board = cards(name);
        let hero = heroes[k % 5];
        let villain = villains[k % 4];
        assert!(!board.iter().any(|c| cards(hero).contains(c)), "spot {k}: {hero} must not share a card with {name}");
        let players = || vec![player(0, hero, &board), player(1, villain, &board)];
        let exact = equity(&EquityRequest::single_pot(board.clone(), players(), EquityMode::Exact), Duration::from_secs(30), &no_cancel());
        assert_eq!(exact.status, EquityStatus::Ready, "spot {k}");
        let mc_mode = EquityMode::MonteCarlo { seed: 7 + k as u64, max_samples: 100_000 };
        let mc = equity(&EquityRequest::single_pot(board.clone(), players(), mc_mode), Duration::from_secs(30), &no_cancel());
        assert_eq!(mc.status, EquityStatus::Ready, "spot {k}");
        assert_eq!(mc.samples, 100_000);
        let (e, m) = (share(&exact, 0), mc.shares.iter().find(|s| s.seat == Seat(0)).unwrap());
        let bound = ((m.value as f64) * (1.0 - m.value as f64) / 100_000.0).sqrt() as f32;
        assert!((m.std_err - bound).abs() < 1e-9, "spot {k}: reported std_err {} vs binomial bound {bound}", m.std_err);
        assert!((m.value - e).abs() <= 4.0 * m.std_err, "spot {k} ({hero} vs {villain} on {name}): mc {} exact {e} std_err {}", m.value, m.std_err);
        match mc.method { Some(EquityMethod::MonteCarlo { samples: 100_000, std_err }) => assert!(std_err >= m.std_err), other => panic!("{other:?}") }
        checked += 1;
    }
    assert_eq!(checked, 20, "spec 13.1 measures 20 spots");
}

/// Spec 13.1 `equity_joint_disjoint_sampling`: a sampled tuple is pairwise disjoint and disjoint
/// from the board, ranges that admit no disjoint tuple are `InvalidRanges` on both paths, and each
/// pot's shares sum to 1 under both modes.
#[test]
fn equity_joint_disjoint_sampling() {
    let board = cards("Kh7d2c");
    let req = EquityRequest::single_pot(board.clone(), vec![player(0, "AKs,AQs,KQs", &board), player(1, "AKs,AQs,KQs", &board), player(2, "AKs,AQs,KQs,AA", &board)], EquityMode::MonteCarlo { seed: 1, max_samples: 1000 });
    let draws = sample_joint_holes(&req, 1, 20_000);
    assert_eq!(draws.len(), 20_000);
    for holes in &draws {
        let mut all: Vec<Card> = holes.iter().flatten().copied().collect();
        all.extend(board.iter().copied());
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "samples never share a card");
    }
    let mut conflicting = EquityRequest::single_pot(board.clone(), vec![player(0, "AsKs", &board), player(1, "AsKs", &board)], EquityMode::Exact);
    assert_eq!(equity(&conflicting, Duration::from_secs(1), &no_cancel()).status, EquityStatus::InvalidRanges);
    conflicting.mode = EquityMode::MonteCarlo { seed: 1, max_samples: 1000 };
    assert_eq!(equity(&conflicting, Duration::from_secs(1), &no_cancel()).status, EquityStatus::InvalidRanges);
    let hero_blocks = EquityRequest::single_pot(board.clone(), vec![player(0, "AsKs", &board), player(1, "AsQd,AsJd,KsTc", &board)], EquityMode::MonteCarlo { seed: 1, max_samples: 1000 });
    assert_eq!(equity(&hero_blocks, Duration::from_secs(1), &no_cancel()).status, EquityStatus::InvalidRanges);
    // per-pot shares sum to 1 per pot: main pot with three players, side pot between seats 1 and 2
    let river = cards("Kh7d2c9s4h");
    let mut multi = EquityRequest::single_pot(river.clone(), vec![player(0, "QQ+,AK", &river), player(1, "22+,A2s+", &river), player(2, "random", &river)], EquityMode::Exact);
    multi.pots = vec![PotEligibility { pot_index: 0, eligible: vec![Seat(0), Seat(1), Seat(2)] }, PotEligibility { pot_index: 1, eligible: vec![Seat(1), Seat(2)] }];
    for mode in [EquityMode::Exact, EquityMode::MonteCarlo { seed: 3, max_samples: 50_000 }] {
        multi.mode = mode;
        let res = equity(&multi, Duration::from_secs(60), &no_cancel());
        assert_eq!(res.status, EquityStatus::Ready, "{mode:?}");
        for pot in 0..2u8 {
            let sum: f32 = res.shares.iter().filter(|s| s.pot_index == pot).map(|s| s.value).sum();
            assert!((sum - 1.0).abs() < 1e-5, "{mode:?} pot {pot} sums to {sum}");
            assert_eq!(res.shares.iter().filter(|s| s.pot_index == pot).count(), if pot == 0 { 3 } else { 2 });
        }
    }
}

/// One seed defines the whole run: the same seed reproduces every share bit for bit, a different
/// seed walks a different draw sequence. Determinism is what makes a Monte Carlo recommendation
/// reproducible from the hand history alone (spec section 7).
#[test]
fn equity_mc_is_deterministic_per_seed() {
    let board = cards("Kh7d2c");
    let players = || vec![player(0, "AhKd", &board), player(1, "QQ+,AKs,76s", &board)];
    let run = |seed: u64| {
        let req = EquityRequest::single_pot(board.clone(), players(), EquityMode::MonteCarlo { seed, max_samples: 20_000 });
        equity(&req, Duration::from_secs(30), &no_cancel())
    };
    let a = run(11);
    let b = run(11);
    assert_eq!(a.status, EquityStatus::Ready);
    assert_eq!(a.samples, 20_000);
    assert_eq!(a.shares, b.shares, "the same seed reproduces every share exactly");
    assert_eq!(a.method, b.method);
    assert_eq!(a.samples, b.samples);
    let c = run(12);
    assert_eq!(c.status, EquityStatus::Ready);
    assert_ne!(a.shares, c.shares, "a different seed walks a different sample path");
    // The draw sequence itself is what the shares inherit.
    let req = EquityRequest::single_pot(board.clone(), players(), EquityMode::MonteCarlo { seed: 11, max_samples: 20_000 });
    assert_eq!(sample_joint_holes(&req, 11, 200), sample_joint_holes(&req, 11, 200));
    assert_ne!(sample_joint_holes(&req, 11, 200), sample_joint_holes(&req, 12, 200));
}

/// Convergence: on a case small enough to enumerate, the estimate lands within four of its own
/// reported standard errors of the exact answer, on both seats.
#[test]
fn equity_mc_converges_to_the_exact_answer() {
    let board = cards("QsJd7h3c");
    let players = || vec![player(0, "AcAd", &board), player(1, "KK,QQ,JJ,AKs,T9s", &board)];
    let exact = equity(&EquityRequest::single_pot(board.clone(), players(), EquityMode::Exact), Duration::from_secs(30), &no_cancel());
    assert_eq!(exact.status, EquityStatus::Ready);
    assert_eq!(exact.method, Some(EquityMethod::Exact));
    let mc_req = EquityRequest::single_pot(board.clone(), players(), EquityMode::MonteCarlo { seed: 99, max_samples: 100_000 });
    let mc = equity(&mc_req, Duration::from_secs(30), &no_cancel());
    assert_eq!(mc.status, EquityStatus::Ready);
    assert_eq!(mc.samples, 100_000);
    for seat in 0..2u8 {
        let e = share(&exact, seat);
        let m = mc.shares.iter().find(|s| s.seat == Seat(seat)).unwrap();
        assert!(m.std_err > 0.0 && m.std_err < 0.01, "seat {seat}: std_err {}", m.std_err);
        assert!((m.value - e).abs() <= 4.0 * m.std_err, "seat {seat}: mc {} exact {e} std_err {}", m.value, m.std_err);
    }
    // The exact path reports no standard error at all; only the sampled one does.
    for s in &exact.shares { assert_eq!(s.std_err, 0.0); }
}

/// The joint draw is proportional to each combo's weight: 1.0 against 0.25 is a 4:1 population.
/// The villain here shares no card with either hero combo, so no draw is ever rejected and the
/// accepted marginal is the weight law itself.
#[test]
fn joint_draw_follows_the_combo_weights() {
    let board = cards("5h6s8c");
    let weighted = PlayerRange { seat: Seat(0), range: parse_range("AsAd:1.0,KhKd:0.25").unwrap() };
    let req = EquityRequest::single_pot(board, vec![weighted, fixed(1, "3c3d")], EquityMode::MonteCarlo { seed: 5, max_samples: 1000 });
    let draws = sample_joint_holes(&req, 5, 20_000);
    assert_eq!(draws.len(), 20_000);
    let kh: Card = "Kh".parse().unwrap();
    let three: Card = "3c".parse().unwrap();
    let kings = draws.iter().filter(|h| h[0].contains(&kh)).count();
    assert!(draws.iter().all(|h| h[1].contains(&three)), "the villain holds its only combo every time");
    // p = 0.25 / 1.25 = 0.2, so 20 000 draws give 4 000 +- 56.6 (one sigma); this is a five-sigma
    // window, and the seed is fixed, so the bound is deterministic.
    assert!((3700..=4300).contains(&kings), "KhKd drawn {kings} times in 20 000; expected ~4 000 (4:1)");
}

/// A request whose supports admit no disjoint assignment is rejected by the bounded compatibility
/// search, not by sampling until the 10 000 000-rejection cap: it returns `InvalidRanges` promptly
/// and reports no shares.
#[test]
fn colliding_supports_report_invalid_ranges_promptly() {
    let board = cards("2c7d9h");
    let cases: Vec<Vec<PlayerRange>> = vec![
        // every combo of both players holds As
        vec![player(0, "AsKs,AsQs", &board), player(1, "AsKs,AsQs", &board)],
        // a three-way chain of single combos that pairwise share a card
        vec![fixed(0, "AsAd"), fixed(1, "AsAc"), fixed(2, "AhAd")],
        // three players cannot hold three disjoint pairs of aces out of four aces
        vec![player(0, "AA", &board), player(1, "AA", &board), player(2, "AA", &board)],
    ];
    for (i, players) in cases.into_iter().enumerate() {
        let req = EquityRequest::single_pot(board.clone(), players, EquityMode::MonteCarlo { seed: 1, max_samples: 100_000 });
        assert_within_bound_eventually(&format!("colliding_supports_report_invalid_ranges_promptly case {i}"), Duration::from_millis(100), || {
            let start = Instant::now();
            let res = equity(&req, Duration::from_secs(5), &no_cancel());
            (start.elapsed(), res)
        }, |res| {
            assert_eq!(res.status, EquityStatus::InvalidRanges, "case {i}");
            assert!(res.shares.is_empty(), "case {i}");
            assert_eq!(res.samples, 0, "case {i}");
            assert_eq!(res.method, None, "case {i}");
        });
    }
}

// Fix round 1 (review T25-R1, T25-R2): the compatibility search is part of the request's time
// budget, and a rejection count is never evidence about the ranges.

/// The T25-R1 review fixture: six seats on a five-card board whose supports admit no disjoint
/// assignment, and whose proof costs 3,967,092 candidate visits (36 + 36^2 + 36^3 + 36^3 * 28 +
/// two single-combo levels), just under the 5,000,000-step cap - so the compatibility search runs
/// the proof to completion, about 6 ms, instead of falling through to sampling. Seats 0-3 hold
/// every pair inside four disjoint card-id blocks; seats 4 and 5 both hold only the combo (40, 41).
fn incompatible_six_player_mc(seed: u64) -> EquityRequest {
    let last = Range1326::from_fn(|i| if i == combo_index(Card(40), Card(41)) { 1.0 } else { 0.0 });
    EquityRequest::single_pot(
        vec![Card(44), Card(45), Card(46), Card(47), Card(48)],
        vec![
            PlayerRange { seat: Seat(0), range: block_range(0, 9) },
            PlayerRange { seat: Seat(1), range: block_range(9, 18) },
            PlayerRange { seat: Seat(2), range: block_range(18, 27) },
            PlayerRange { seat: Seat(3), range: block_range(27, 35) },
            PlayerRange { seat: Seat(4), range: last.clone() },
            PlayerRange { seat: Seat(5), range: last },
        ],
        EquityMode::MonteCarlo { seed, max_samples: 100_000 },
    )
}

/// Spawns a thread that raises `cancel` `after` a spin-measured delay, and returns once that thread
/// is running. Windows' sleep resolution (15.6 ms by default) is coarser than the searches these
/// tests cancel, so the delay is spun, and the handshake keeps thread-start latency out of it: the
/// caller enters `equity` within microseconds of the setter starting its own clock.
fn cancel_after(cancel: &Arc<AtomicBool>, after: Duration) -> std::thread::JoinHandle<()> {
    let ready = Arc::new(AtomicBool::new(false));
    let (flag, go) = (cancel.clone(), ready.clone());
    let handle = std::thread::spawn(move || {
        go.store(true, Ordering::Relaxed);
        let start = Instant::now();
        while start.elapsed() < after { std::hint::spin_loop(); }
        flag.store(true, Ordering::Relaxed);
    });
    while !ready.load(Ordering::Relaxed) { std::hint::spin_loop(); }
    handle
}

/// Orchestrator ruling (post-close follow-up, plan 1, timing follow-up 4): the retry idea behind
/// `assert_within_bound_eventually` generalizes to *any* scheduling-dependent condition, not only a
/// wall-clock overrun. `mc_partial_runs_keep_their_shares`'s cancel-flag block asserted
/// `res.samples > 0` -- "the samples taken before the flag are not thrown away" -- which is exactly
/// as race-dependent as an elapsed-time bound: under heavy build load the main thread can be
/// descheduled before completing even its first sample, the 1 ms flag wins, and `samples == 0` with
/// status `Cancelled` is a legitimate outcome of the scheduler, not a defect.
///
/// `eventually` is the general shape. `run` produces one fresh attempt; `accept` asserts every hard
/// invariant with a plain `assert!`/`assert_eq!` (these panic immediately, on any attempt, and are
/// never retried away) and returns `Err(reason)` only for a condition that depends on the scheduler
/// -- an elapsed time against a bound, or a sample/evaluation count that depends on how far a
/// preempted thread got before a flag was raised. A caller whose attempt races a helper thread
/// (`cancel_after`) creates and joins a fresh flag and thread inside `run` on every attempt, so no
/// two attempts ever share one. A scheduling stall lengthens at most a few consecutive attempts, not
/// all ten; a genuine regression rejects every attempt and still panics, listing every reason.
fn eventually<R>(label: &str, mut run: impl FnMut() -> R, mut accept: impl FnMut(&R) -> Result<(), String>) -> R {
    let mut reasons = Vec::with_capacity(10);
    for _ in 0..10 {
        let result = run();
        match accept(&result) {
            Ok(()) => return result,
            Err(reason) => reasons.push(reason),
        }
    }
    panic!("{label}: all 10 attempts were rejected: {reasons:?}");
}

/// Thin wrapper over `eventually` for the common case: the only scheduling-dependent condition is a
/// wall-clock *overrun* against the spec 13.1 bound, and every other assertion on the result is a
/// hard invariant. `check` runs on every attempt, not only the one that meets the bound, so a
/// correctness regression cannot hide behind a slow first attempt.
fn assert_within_bound_eventually(
    label: &str,
    bound: Duration,
    run: impl FnMut() -> (Duration, EquityResult),
    mut check: impl FnMut(&EquityResult),
) -> (Duration, EquityResult) {
    eventually(label, run, |(took, res)| {
        check(res);
        if *took < bound { Ok(()) } else { Err(format!("{took:?} overran the {bound:?} bound")) }
    })
}

/// T25-R1: the compatibility search runs inside the request's budget like every other phase. This
/// proof takes about 6 ms, so a 1 ms budget must stop it mid-proof and report `BudgetExceeded` -
/// never `InvalidRanges`, which a caller reads as a statement about its ranges, not about the clock.
#[test]
fn mc_compatibility_search_observes_the_budget() {
    let req = incompatible_six_player_mc(1);
    assert_within_bound_eventually("mc_compatibility_search_observes_the_budget", Duration::from_millis(50), || {
        let start = Instant::now();
        let res = equity(&req, Duration::from_millis(1), &no_cancel());
        (start.elapsed(), res)
    }, |res| {
        assert_eq!(res.status, EquityStatus::BudgetExceeded, "must stop on the budget, not report InvalidRanges");
        assert_eq!(res.samples, 0, "no tuple is sampled during the compatibility search");
        assert!(res.shares.is_empty());
        assert_eq!(res.method, None);
    });
}

/// T25-R1: the same search must observe a cancellation raised while it is running, which only a
/// poll inside the traversal can catch.
#[test]
fn mc_compatibility_search_observes_cancellation() {
    let req = incompatible_six_player_mc(1);
    assert_within_bound_eventually("mc_compatibility_search_observes_cancellation", Duration::from_millis(200), || {
        let cancel = Arc::new(AtomicBool::new(false));
        let setter = cancel_after(&cancel, Duration::from_millis(1));
        let start = Instant::now();
        let res = equity(&req, Duration::from_secs(5), &cancel);
        let took = start.elapsed();
        setter.join().unwrap();
        (took, res)
    }, |res| {
        assert_eq!(res.status, EquityStatus::Cancelled);
        assert_eq!(res.samples, 0, "no tuple is sampled during the compatibility search");
        assert!(res.shares.is_empty());
        assert_eq!(res.method, None);
    });
}

/// T25-R2: rejections are a property of the sampler, never evidence about the ranges.
///
/// `AcAd` (weight 1) collides with the villain's only combo, but `KcKd` (weight 1e-12) does not, so
/// a disjoint tuple exists and exact enumeration answers it. A sampler needs about 10^12 draws to
/// find it, so this request is exactly the case where a rejection count must not be mistaken for a
/// proof: the run may only end on the budget or the cancel flag, reporting `BudgetExceeded` with no
/// shares. The contrast below is a genuinely impossible pair, which the compatibility proof still
/// reports as `InvalidRanges`.
#[test]
fn rejection_exhaustion_is_never_invalid_ranges() {
    let board = cards("2c7d9hJs4s");
    let rare = Range1326::from_fn(|i| if i == combo("AcAd") { 1.0 } else if i == combo("KcKd") { 1e-12 } else { 0.0 });
    let players = || vec![PlayerRange { seat: Seat(0), range: rare.clone() }, fixed(1, "AcAh")];

    let exact = equity(&EquityRequest::single_pot(board.clone(), players(), EquityMode::Exact), Duration::from_secs(10), &no_cancel());
    assert_eq!(exact.status, EquityStatus::Ready, "KcKd against AcAh is a disjoint tuple");
    assert_eq!(share(&exact, 0), 0.0, "K K J 9 7 never beats A A J 9 7");
    assert_eq!(share(&exact, 1), 1.0);

    // The rejection cap used to be reached in about 60 ms, so this budget is long enough for the
    // old behaviour to surface and short enough to keep the test cheap.
    let budget = Duration::from_millis(300);
    let mc = EquityRequest::single_pot(board.clone(), players(), EquityMode::MonteCarlo { seed: 1, max_samples: 1 });
    let (took, _) = assert_within_bound_eventually("rejection_exhaustion_is_never_invalid_ranges budget path", budget + Duration::from_millis(100), || {
        let start = Instant::now();
        let res = equity(&mc, budget, &no_cancel());
        (start.elapsed(), res)
    }, |res| {
        assert_ne!(res.status, EquityStatus::InvalidRanges, "a compatible tuple exists; rejections prove nothing");
        assert_eq!(res.status, EquityStatus::BudgetExceeded, "must stop on the budget");
        assert_eq!(res.samples, 0);
        assert!(res.shares.is_empty());
        assert_eq!(res.method, None);
    });
    assert!(took >= budget, "the loop must sample until the budget, not stop at a rejection count: {took:?}");

    // Contrast: no disjoint tuple at all, which the completed proof does report.
    let impossible = EquityRequest::single_pot(board, vec![fixed(0, "AcAd"), fixed(1, "AcAd")], EquityMode::MonteCarlo { seed: 1, max_samples: 1 });
    assert_within_bound_eventually("rejection_exhaustion_is_never_invalid_ranges impossible path", Duration::from_millis(100), || {
        let start = Instant::now();
        let res = equity(&impossible, Duration::from_secs(5), &no_cancel());
        (start.elapsed(), res)
    }, |res| {
        assert_eq!(res.status, EquityStatus::InvalidRanges);
        assert_eq!(res.samples, 0);
    });
}

/// A Monte Carlo request that is already cancelled, or has no budget left, reports that before it
/// builds a support or visits a single candidate (the exact path's rule, spec 13.1).
#[test]
fn mc_entry_stop_reports_before_any_work() {
    let board = cards("Kh7d2c");
    let req = || EquityRequest::single_pot(board.clone(), vec![player(0, "random", &board), player(1, "random", &board)], EquityMode::MonteCarlo { seed: 2, max_samples: 100_000 });
    let res = equity(&req(), Duration::from_secs(10), &AtomicBool::new(true));
    assert_eq!(res.status, EquityStatus::Cancelled);
    assert_eq!(res.samples, 0);
    assert!(res.shares.is_empty());
    assert_eq!(res.method, None);
    let res = equity(&req(), Duration::ZERO, &no_cancel());
    assert_eq!(res.status, EquityStatus::BudgetExceeded);
    assert_eq!(res.samples, 0);
    assert!(res.shares.is_empty());
    assert_eq!(res.method, None);
}

/// Partial results: a stop that arrives after some samples keeps the estimate. The budget stopping
/// the loop is still `Ready` (the estimate is what was asked for, only shorter); the cancel flag is
/// `Cancelled` but still carries the shares, because the caller may yet want them.
#[test]
fn mc_partial_runs_keep_their_shares() {
    let board = cards("Kh7d2c");
    let req = || EquityRequest::single_pot(board.clone(), vec![player(0, "random", &board), player(1, "random", &board)], EquityMode::MonteCarlo { seed: 8, max_samples: u32::MAX });

    let budget = Duration::from_millis(50);
    assert_within_bound_eventually("mc_partial_runs_keep_their_shares", budget + Duration::from_millis(50), || {
        let start = Instant::now();
        let res = equity(&req(), budget, &no_cancel());
        (start.elapsed(), res)
    }, |res| {
        assert_eq!(res.status, EquityStatus::Ready, "a budget stop with samples in hand is still an answer");
        assert!(res.samples > 0 && res.samples < u32::MAX as u64, "partial sample count {}", res.samples);
        let sum: f32 = res.shares.iter().map(|s| s.value).sum();
        assert!((sum - 1.0).abs() < 1e-5, "shares sum to {sum}");
        match &res.method {
            Some(EquityMethod::MonteCarlo { samples, std_err }) => {
                assert_eq!(*samples as u64, res.samples);
                assert!(*std_err > 0.0);
            }
            other => panic!("{other:?}"),
        }
    });

    // 1 ms, not 20: `max_samples` is `u32::MAX`, so the run cannot end on its own and the only
    // question is whether a sample completes before the flag -- one takes microseconds. Spinning
    // for 20 ms burned a core for the whole delay and left the 200 ms bound below within reach of
    // scheduling noise when the rest of this binary runs in parallel.
    //
    // Whether a sample completes before the 1 ms flag is itself scheduling-dependent (timing
    // follow-up 4): under heavy load the main thread can be descheduled before its first sample
    // finishes, so `samples == 0` with status `Cancelled` is a legitimate scheduler outcome, not a
    // defect. `eventually` retries that rejection exactly like the elapsed-time bound, while status,
    // the share sum and the method shape stay hard invariants on every attempt.
    eventually("mc_partial_runs_keep_their_shares cancel path", || {
        let cancel = Arc::new(AtomicBool::new(false));
        let setter = cancel_after(&cancel, Duration::from_millis(1));
        let start = Instant::now();
        let res = equity(&req(), Duration::from_secs(10), &cancel);
        let took = start.elapsed();
        setter.join().unwrap();
        (took, res)
    }, |(took, res)| {
        assert_eq!(res.status, EquityStatus::Cancelled, "stopped after {took:?}");
        if res.samples == 0 {
            return Err(format!("0 samples completed before the flag, after {took:?}"));
        }
        assert!(!res.shares.is_empty());
        let sum: f32 = res.shares.iter().map(|s| s.value).sum();
        assert!((sum - 1.0).abs() < 1e-5, "shares sum to {sum}");
        assert!(matches!(res.method, Some(EquityMethod::MonteCarlo { .. })));
        if *took >= Duration::from_millis(200) {
            return Err(format!("cancelled {took:?} after a 1 ms flag, wanted < 200ms"));
        }
        Ok(())
    });
}

/// S9 (final review): the spec keeps EV in `f32` chips, but `terminal_payoff` must not do the
/// *arithmetic* in `f32` -- above `2^24` the pot itself stops being representable, so narrowing it
/// first loses a chip before the rake is even subtracted.
///
/// A pot of `2^24 + 1 = 16,777,217` with a 5% rake capped at 5,000 mchips is the smallest crisp
/// case. `16_777_217u32 as f32` rounds to `16,777,216` (f32 steps by 2 in `[2^24, 2^25)`), so the
/// f32 arithmetic returns `16,777,211`. Computed in f64 the payoff is `16,777,217 - 5 =
/// 16,777,212`, which *is* exactly representable in f32 (it is even, and below `2^25`), so the one
/// narrowing at the boundary returns it unchanged. One chip of the answer, recovered.
#[test]
fn terminal_payoff_computes_in_f64_and_narrows_once() {
    let raked = Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false };
    assert_eq!(
        terminal_payoff(1.0, 16_777_217, &raked),
        16_777_212.0,
        "the pot must reach the subtraction unnarrowed; f32 arithmetic gives 16777211.0"
    );
    // Half the pot of the same spot: 0.5 * (16,777,217 - 5) = 8,388,606, exactly representable.
    assert_eq!(terminal_payoff(0.5, 16_777_217, &raked), 8_388_606.0);
    // And the bound the review asked for, at the magnitude it named: the f32 *return* steps by 8
    // at 10^8, so half a chip is not achievable there; what is achievable, and what the f64
    // intermediate buys, is that the error never exceeds that half step.
    let pot = 100_000_000u32;
    let got = terminal_payoff(1.0, pot, &raked);
    let err = (f64::from(got) - (f64::from(pot) - 5.0)).abs();
    assert!(err <= 4.0, "a pot of {pot} came back as {got}, off by {err} chips (half an f32 step is 4)");
}

/// T3 (final review): `monte_carlo` with `max_samples == 0` never entered its loop and then
/// reported `BudgetExceeded` -- blaming the clock for a caller bug. `max_samples` is a caller's
/// contract, not data, so it is an always-on assertion like the rest of `check_request`.
#[test]
#[should_panic(expected = "max_samples is at least one sample")]
fn monte_carlo_rejects_a_zero_sample_cap() {
    let board = cards("QsJd7h3c2d");
    let req = EquityRequest::single_pot(
        board.clone(),
        vec![player(0, "AA", &board), player(1, "KK", &board)],
        EquityMode::MonteCarlo { seed: 1, max_samples: 0 },
    );
    equity(&req, Duration::from_millis(50), &no_cancel());
}

/// T2 (final review): `equity_budget_respected` proves the exact path *stops* on budget, but
/// nothing pinned its throughput, so S3's per-tuple allocation -- measured at 142.5 ns/tuple, about
/// 70% of exact-enumeration wall time -- was invisible to the suite and so was any future
/// regression. Spec section 7 selects exact enumeration when `exact_cost <= 2 * 10^7` and section
/// 13.1 gives that selection a 0.5 s budget: an enumeration at that ceiling must actually finish
/// inside the budget it was selected for.
///
/// Exhaustive-gated (spec 13.1): this is a throughput contract, not a correctness one, and it is
/// two orders of magnitude more work than any other test in the file.
///
/// Release-only as well: the section 7 ceiling is a **release-profile** budget, so measuring it in
/// a debug build asserts a number the spec never promised (this fixture reaches only 14,151,680 of
/// its 36,796,320 evaluations in 500 ms at `opt-level = 1`, and finishes in 0.34 s in release).
#[cfg(all(feature = "exhaustive", not(debug_assertions)))]
#[test]
fn exact_enumeration_at_the_selection_ceiling_finishes_inside_the_budget() {
    let board = cards("QsJd7h3c");
    let mut hero = Range1326::uniform();
    block_public(&mut hero, &board);
    // Every third combo: a support size chosen to put `exact_cost` just under the section 7 ceiling.
    let mut villain = Range1326::from_fn(|i| if i % 3 == 0 { 1.0 } else { 0.0 });
    block_public(&mut villain, &board);
    let req = EquityRequest::single_pot(
        board,
        vec![PlayerRange { seat: Seat(0), range: hero }, PlayerRange { seat: Seat(1), range: villain }],
        EquityMode::Exact,
    );
    let cost = exact_cost(&req);
    // 1,128 hero combos x 404 villain combos x 44 turn runouts = 20,051,328: the section 7 ceiling,
    // marginally above `2 * 10^7`, so this test is if anything stricter than the spec requires.
    assert_eq!(cost, 20_051_328, "the fixture must sit at the section 7 selection ceiling, not below it");
    let budget = Duration::from_millis(500);
    let start = Instant::now();
    let res = equity(&req, budget, &no_cancel());
    let took = start.elapsed();
    assert_eq!(
        res.status,
        EquityStatus::Ready,
        "an enumeration spec 7 selects (exact_cost {cost}) must finish inside its 0.5 s budget; it stopped after {took:?} at {} evaluations",
        res.samples
    );
    let sum: f32 = res.shares.iter().map(|s| s.value).sum();
    assert!((sum - 1.0).abs() < 1e-5, "shares sum to {sum}");
}

/// S10 (final review): `per_combo_equity` was the one public entry point of `core-eval` that could
/// not be cancelled and had no caller-supplied budget -- up to 1,326 exact enumerations behind a
/// hard-coded one-hour deadline and a permanently-false flag. Spec section 5 requires the work to
/// be abandonable when hero's decision changes, so it now takes the caller's clock and answers it
/// with `equity`'s own status semantics.
#[test]
fn per_combo_equity_reports_a_stop_with_equity_status_semantics() {
    let board = cards("QsJd7h3c2d");
    let hero = parse_range("AA,KK,QQ").unwrap();
    let villain = parse_range("QQ,JJ,54o").unwrap();

    // An already-raised flag stops the call before any work, like `exact`'s entry poll.
    let cancelled = AtomicBool::new(true);
    let (status, out) = per_combo_equity(&hero, &villain, &board, Duration::from_secs(10), &cancelled);
    assert_eq!(status, EquityStatus::Cancelled);
    assert!(out.iter().all(|v| *v == 0.0), "a cancelled call reports no equity at all");

    // A zero budget is exhausted on the first poll whatever the timer's resolution reports.
    let (status, out) = per_combo_equity(&hero, &villain, &board, Duration::ZERO, &no_cancel());
    assert_eq!(status, EquityStatus::BudgetExceeded);
    assert!(out.iter().all(|v| *v == 0.0));

    // An empty support on either side is a statement about the ranges, not about the clock.
    let (status, _) = per_combo_equity(&Range1326::zero(), &villain, &board, Duration::from_secs(10), &no_cancel());
    assert_eq!(status, EquityStatus::InvalidRanges);
    let (status, _) = per_combo_equity(&hero, &Range1326::zero(), &board, Duration::from_secs(10), &no_cancel());
    assert_eq!(status, EquityStatus::InvalidRanges);

    // The river fast path still completes, inside a budget a decision can actually wait for.
    // `Ready` from a 500 ms budget *is* the statement that it finished inside 500 ms -- the
    // deadline is what enforces it -- so no separate wall-clock assertion is made here.
    let (status, out) = per_combo_equity(&hero, &villain, &board, Duration::from_millis(500), &no_cancel());
    assert_eq!(status, EquityStatus::Ready, "the river fast path must finish inside its 500 ms budget");
    assert!(out[combo("AcAd") as usize] > 0.0, "a supported hero combo is scored");
    assert_eq!(out[combo("2h2d") as usize], 0.0, "an unsupported hero combo scores 0");
}

/// S10: a stop raised while the enumerations are running is observed between hero combos, and the
/// combos finished before it are kept -- the caller gets a partial answer, not an empty one.
///
/// Sized so the stop is never a race. On a **flop** every tuple carries 990 runouts, so one hero
/// combo against the six `AA` combos costs about 95 us in release, and the full 1,176-combo hero
/// range costs about 112 ms: a combo is three orders of magnitude cheaper than the stop window and
/// the whole run is an order of magnitude more expensive than it, on either build profile. An
/// earlier version of this test sized both windows at 20 ms against a 17 ms *river* run, which the
/// release build finished inside -- and it burned a core in a spin loop while doing so, which was
/// enough extra contention to destabilise the raw-`sleep` timing tests elsewhere in this file. The
/// budget half needs no helper thread at all.
#[test]
fn per_combo_equity_observes_a_stop_mid_run_and_keeps_what_it_finished() {
    let board = cards("Kh7d2c");
    let mut hero = Range1326::uniform();
    block_public(&mut hero, &board);
    let villain = parse_range("AA").unwrap();
    // 1,176 = C(49, 2): every combo that the three board cards do not block.
    let hero_combos = (0..1326u16).filter(|i| hero.get(*i) > 0.0).count();
    assert_eq!(hero_combos, 1176);

    // Budget, no helper thread: the per-combo poll stops the run and keeps the partial array.
    let (status, out) = per_combo_equity(&hero, &villain, &board, Duration::from_millis(5), &no_cancel());
    let scored = out.iter().filter(|v| **v > 0.0).count();
    assert_eq!(status, EquityStatus::BudgetExceeded);
    assert!(scored > 0, "the combos finished before the budget ran out are kept, not discarded");
    assert!(scored < hero_combos, "the run stopped before scoring every hero combo: {scored}");

    // The `Cancelled` variant of the same stop is deliberately *not* re-tested here with a helper
    // thread. Both statuses leave `per_combo_equity` through one path -- `Deadline`'s poll returns
    // `Some(status)` and the loop returns `(status, out)` -- so the budget above already exercises
    // the mid-run stop and the partial retention, and
    // `per_combo_equity_reports_a_stop_with_equity_status_semantics` pins `Cancelled` itself at the
    // entry poll. Racing a timer against the enumeration a second time would only add parallel load
    // to a test binary whose pre-existing raw-`sleep` timing tests are already sensitive to it.
}
