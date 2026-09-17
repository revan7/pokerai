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
    let start = Instant::now();
    let res = equity(&req, Duration::from_millis(1), &no_cancel());
    let took = start.elapsed();
    assert_eq!(res.status, EquityStatus::BudgetExceeded, "not InvalidRanges: the search never finished");
    assert!(res.shares.is_empty());
    assert_eq!(res.samples, 0, "no runout completes, so no rank is evaluated");
    assert!(took < Duration::from_millis(50), "spec 13.1 overrun limit: stopped after {took:?}");
}

/// R1: the same search must observe a cancellation raised while it is running, which only a poll
/// inside the assignment traversal can catch.
#[test]
fn assignment_search_observes_cancellation_with_no_evaluations() {
    let req = colliding_six_player();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let setter = std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(20)); flag.store(true, Ordering::Relaxed); });
    let start = Instant::now();
    let res = equity(&req, Duration::from_secs(10), &cancel);
    let took = start.elapsed();
    setter.join().unwrap();
    assert_eq!(res.status, EquityStatus::Cancelled);
    assert!(res.shares.is_empty());
    assert_eq!(res.samples, 0);
    assert!(took < Duration::from_millis(200), "cancelled {took:?} after a 20 ms flag");
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

    let eq_a = per_combo_equity(&a, &b, &board);
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
        let start = Instant::now();
        let res = equity(&req, Duration::from_secs(5), &no_cancel());
        let took = start.elapsed();
        assert_eq!(res.status, EquityStatus::InvalidRanges, "case {i}");
        assert!(res.shares.is_empty(), "case {i}");
        assert_eq!(res.samples, 0, "case {i}");
        assert_eq!(res.method, None, "case {i}");
        assert!(took < Duration::from_millis(100), "case {i}: took {took:?}");
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

/// T25-R1: the compatibility search runs inside the request's budget like every other phase. This
/// proof takes about 6 ms, so a 1 ms budget must stop it mid-proof and report `BudgetExceeded` -
/// never `InvalidRanges`, which a caller reads as a statement about its ranges, not about the clock.
#[test]
fn mc_compatibility_search_observes_the_budget() {
    let req = incompatible_six_player_mc(1);
    let start = Instant::now();
    let res = equity(&req, Duration::from_millis(1), &no_cancel());
    let took = start.elapsed();
    assert_eq!(res.status, EquityStatus::BudgetExceeded, "stopped after {took:?}");
    assert_eq!(res.samples, 0, "no tuple is sampled during the compatibility search");
    assert!(res.shares.is_empty());
    assert_eq!(res.method, None);
    assert!(took < Duration::from_millis(50), "spec 13.1 overrun limit: stopped after {took:?}");
}

/// T25-R1: the same search must observe a cancellation raised while it is running, which only a
/// poll inside the traversal can catch.
#[test]
fn mc_compatibility_search_observes_cancellation() {
    let req = incompatible_six_player_mc(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let setter = cancel_after(&cancel, Duration::from_millis(1));
    let start = Instant::now();
    let res = equity(&req, Duration::from_secs(5), &cancel);
    let took = start.elapsed();
    setter.join().unwrap();
    assert_eq!(res.status, EquityStatus::Cancelled, "stopped after {took:?}");
    assert_eq!(res.samples, 0);
    assert!(res.shares.is_empty());
    assert_eq!(res.method, None);
    assert!(took < Duration::from_millis(200), "cancelled {took:?} after a 1 ms flag");
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
    let start = Instant::now();
    let res = equity(&mc, budget, &no_cancel());
    let took = start.elapsed();
    assert_ne!(res.status, EquityStatus::InvalidRanges, "a compatible tuple exists; rejections prove nothing");
    assert_eq!(res.status, EquityStatus::BudgetExceeded, "stopped after {took:?}");
    assert_eq!(res.samples, 0);
    assert!(res.shares.is_empty());
    assert_eq!(res.method, None);
    assert!(took >= budget, "the loop must sample until the budget, not stop at a rejection count: {took:?}");
    assert!(took < budget + Duration::from_millis(100), "overran its budget: {took:?}");

    // Contrast: no disjoint tuple at all, which the completed proof does report.
    let impossible = EquityRequest::single_pot(board, vec![fixed(0, "AcAd"), fixed(1, "AcAd")], EquityMode::MonteCarlo { seed: 1, max_samples: 1 });
    let start = Instant::now();
    let res = equity(&impossible, Duration::from_secs(5), &no_cancel());
    assert_eq!(res.status, EquityStatus::InvalidRanges);
    assert_eq!(res.samples, 0);
    assert!(start.elapsed() < Duration::from_millis(100), "a completed proof is prompt");
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
    let start = Instant::now();
    let res = equity(&req(), budget, &no_cancel());
    let took = start.elapsed();
    assert_eq!(res.status, EquityStatus::Ready, "a budget stop with samples in hand is still an answer");
    assert!(res.samples > 0 && res.samples < u32::MAX as u64, "partial sample count {}", res.samples);
    assert!(took < budget + Duration::from_millis(50), "stopped {took:?} after a {budget:?} budget");
    let sum: f32 = res.shares.iter().map(|s| s.value).sum();
    assert!((sum - 1.0).abs() < 1e-5, "shares sum to {sum}");
    match res.method {
        Some(EquityMethod::MonteCarlo { samples, std_err }) => {
            assert_eq!(samples as u64, res.samples);
            assert!(std_err > 0.0);
        }
        other => panic!("{other:?}"),
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let setter = cancel_after(&cancel, Duration::from_millis(20));
    let start = Instant::now();
    let res = equity(&req(), Duration::from_secs(10), &cancel);
    let took = start.elapsed();
    setter.join().unwrap();
    assert_eq!(res.status, EquityStatus::Cancelled);
    assert!(res.samples > 0, "the samples taken before the flag are not thrown away");
    assert!(!res.shares.is_empty());
    let sum: f32 = res.shares.iter().map(|s| s.value).sum();
    assert!((sum - 1.0).abs() < 1e-5, "shares sum to {sum}");
    assert!(matches!(res.method, Some(EquityMethod::MonteCarlo { .. })));
    assert!(took < Duration::from_millis(200), "cancelled {took:?} after a 20 ms flag");
}
