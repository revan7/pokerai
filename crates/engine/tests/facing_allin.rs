//! Spec section 6's facing-an-all-in analytic fallback and the equity adapter underneath it
//! (`engine::equity`), which this task also owns: the two section 4.4 populations, mode selection
//! by `exact_cost`, the single equity-phase deadline of section 7, and the failure labels.

use engine::allin::{facing_allin, AllInInput};
use engine::clock::Clock;
use engine::equity::{equity_summary, equity_summary_with_clock, hero_combo_equity, pending_summary, range_vs_range, EQUITY_BUDGET_MS};
use proto::{combo_index, Action, Availability, Card, EquityEstimate, EquityMethod, Rake, Range1326, Seat};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

fn c(s: &str) -> Card { Card::parse(s).unwrap() }
fn range(entries: &[(&str, &str, f32)]) -> Range1326 { let mut r = Range1326([0.0; 1326]); for (a, b, w) in entries { r.0[combo_index(c(a), c(b)) as usize] = *w; } r }
fn qq_54o(w54: f32) -> Range1326 {
    let mut e = vec![("Qc", "Qd", 1.0), ("Qc", "Qh", 1.0), ("Qd", "Qh", 1.0)];
    for f in ["5c", "5d", "5h", "5s"] { for g in ["4c", "4d", "4h", "4s"] { if f.as_bytes()[1] != g.as_bytes()[1] { e.push((f, g, w54)); } } }
    range(&e)
}
fn input(opp: Range1326, rake: Rake) -> AllInInput {
    AllInInput { hero: [c("Ah"), c("Ad")], board: "Qs Jd 7h 3c 2d".split(' ').map(c).collect(), opp_public: opp, hero_public: None, call_cost: 73, pot: 173, facing: 73, rake, bb_chips: 5 }
}

#[test]
fn facing_allin_golden() {
    let cancel = AtomicBool::new(false);
    let unraked = Rake::TimeCharge;
    let raked = Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false };
    // T1: AhAd versus QQ + 54o at weight 1/12: equity 0.25, W = 246, EV(call) = -11.5 unraked, -12.75 with cap 5; -2.30 bb at a 5-chip BB
    let a = facing_allin(&input(qq_54o(1.0 / 12.0), unraked.clone()), Duration::from_secs(2), &cancel).unwrap();
    assert!((a.equity - 0.25).abs() < 1e-6 && a.w == 246 && a.r == 0.0);
    assert!((a.ev_call_chips + 11.5).abs() < 1e-3);
    let call = a.actions.iter().find(|x| x.action == Action::Call).unwrap();
    let fold = a.actions.iter().find(|x| x.action == Action::Fold).unwrap();
    assert!((call.ev_bb.unwrap() + 2.30).abs() < 1e-3 && fold.ev_bb == Some(0.0));
    assert_eq!((fold.frequency, call.frequency, fold.headline, call.headline), (Some(1.0), Some(0.0), true, false));
    let b = facing_allin(&input(qq_54o(1.0 / 12.0), raked.clone()), Duration::from_secs(2), &cancel).unwrap();
    assert!((b.r - 5.0).abs() < 1e-6 && (b.ev_call_chips + 12.75).abs() < 1e-3);
    // 54o at weight 0.25: equity 0.5, +50 unraked, +47.5 raked; call 100% and the headline
    let d = facing_allin(&input(qq_54o(0.25), unraked), Duration::from_secs(2), &cancel).unwrap();
    assert!((d.ev_call_chips - 50.0).abs() < 1e-3);
    assert!(d.actions.iter().find(|x| x.action == Action::Call).unwrap().headline);
    let e = facing_allin(&input(qq_54o(0.25), raked), Duration::from_secs(2), &cancel).unwrap();
    assert!((e.ev_call_chips - 47.5).abs() < 1e-3);
    // section 13.3 "hero's strategic range includes other hands and the headline uses AhAd": passing a non-trivial hero
    // public range alongside changes nothing, because only hero's actual combo enters the analytic fallback.
    let mut with_range = input(qq_54o(1.0 / 12.0), Rake::TimeCharge);
    with_range.hero_public = Some(range(&[("Ah", "Ad", 1.0), ("Kh", "Kd", 1.0), ("7c", "7d", 1.0), ("As", "Ks", 1.0)]));
    let g = facing_allin(&with_range, Duration::from_secs(2), &cancel).unwrap();
    assert_eq!((g.w, g.equity == a.equity), (a.w, true));
    assert!((g.ev_call_chips - a.ev_call_chips).abs() < 1e-6);
    assert!(g.actions.iter().find(|x| x.action == Action::Fold).unwrap().headline, "the headline still comes from AhAd's EV");
    // an opponent range with no compatible combo is InvalidRanges
    assert!(matches!(facing_allin(&input(range(&[("Ah", "Ad", 1.0)]), Rake::TimeCharge), Duration::from_secs(2), &cancel), Err(proto::UnsupportedReason::InvalidRanges)));
}

/// The two branches `facing_allin_golden`'s fixture cannot reach, because it calls the whole wager
/// (`call_cost == facing`) into a rake that is already at its cap:
/// * the uncalled excess going back before the showdown, so `W = pot + 2C - facing` and not
///   `pot + C` (verified by mutation: `pot + C` passes the golden and fails here);
/// * `rate * W` below the cap, so the rake is the rate and not the cap.
#[test]
fn facing_allin_short_hero_and_uncapped_rake() {
    let cancel = AtomicBool::new(false);
    // Hero calls 40 of a 73-chip all-in: 33 chips go back uncalled, so W = 173 + 80 - 73 = 180.
    let short = |rake| AllInInput { call_cost: 40, ..input(qq_54o(1.0 / 12.0), rake) };
    // Unraked: 0.25 * 180 - 40 = +5 chips = +1.00 bb at a 5-chip BB, so calling is the headline.
    let a = facing_allin(&short(Rake::TimeCharge), Duration::from_secs(2), &cancel).unwrap();
    assert_eq!((a.w, a.r), (180, 0.0));
    assert!((a.ev_call_chips - 5.0).abs() < 1e-3);
    let call = a.actions.iter().find(|x| x.action == Action::Call).unwrap();
    assert!((call.ev_bb.unwrap() - 1.0).abs() < 1e-3 && call.headline && call.frequency == Some(1.0));
    // 5% of 180 is 9 chips, under a 50-chip cap: R = 9, EV = 0.25 * 171 - 40 = +2.75 chips.
    let b = facing_allin(&short(Rake::PotRake { rate: 0.05, cap_mchips: 50_000, no_flop_no_drop: false }), Duration::from_secs(2), &cancel).unwrap();
    assert!((b.r - 9.0).abs() < 1e-6 && (b.ev_call_chips - 2.75).abs() < 1e-3);
}

// ---------------------------------------------------------------------------------------------
// `engine::equity` adapter contracts (review R1/R2). This task is the single owner of
// `range_vs_range` and of the two section 4.4 populations, so each is pinned with a
// hand-computable answer rather than with `is_some()`.
// ---------------------------------------------------------------------------------------------

/// A river with no possible straight or flush for any range below, so every rank is checkable by
/// hand: `KcKd` is a set of kings, `QcQd` a pair of queens, `AcAd` a pair of aces, `4c4d` a pair of
/// fours, and the board's own five cards are the kicker sequence `K 9 7` for all of them.
fn adapter_board() -> Vec<Card> { "Ks 7d 2c 3h 9s".split(' ').map(c).collect() }
fn seats(v: &[(Seat, EquityEstimate)]) -> Vec<Seat> { v.iter().map(|(s, _)| *s).collect() }
fn values(v: &[(Seat, EquityEstimate)]) -> Vec<Option<f32>> { v.iter().map(|(_, e)| e.value).collect() }
fn value_of(v: &[(Seat, EquityEstimate)], s: Seat) -> Option<f32> { v.iter().find(|(t, _)| *t == s).expect("the seat is in the summary").1.value }
fn expired(e: &EquityEstimate) -> bool { matches!(&e.availability, Availability::Unavailable { reason } if reason == "equity deadline expired before this estimate") }

#[test]
fn range_vs_range_weights_the_joint_population() {
    let cancel = AtomicBool::new(false);
    let board = adapter_board();
    // Hero: AcAd at full weight, 4c4d at half. Opponent: KcKd (beats both) and QcQd (loses to AA
    // only). Joint weights 1, 1, 0.5, 0.5 over the four dealable tuples and hero wins exactly one
    // of them, so equity is 1/3 -- an unweighted count would answer 1/4 instead.
    let hero = range(&[("Ac", "Ad", 1.0), ("4c", "4d", 0.5)]);
    let opp = range(&[("Kc", "Kd", 1.0), ("Qc", "Qd", 1.0)]);
    let (before_hero, before_opp) = (hero.clone(), opp.clone());
    let (v, m) = range_vs_range(&hero, &opp, &board, Duration::from_secs(5), &cancel).expect("both ranges are dealable");
    assert!((v - 1.0 / 3.0).abs() < 1e-6, "weighted range-versus-range equity was {v}, expected 1/3");
    assert_eq!(m, EquityMethod::Exact);
    // Both public ranges are read, neither is written (spec section 2).
    assert!(hero.0 == before_hero.0 && opp.0 == before_opp.0, "range_vs_range must not mutate its inputs");
}

#[test]
fn blocked_tuples_leave_the_population_and_an_undealable_pair_has_no_value() {
    let cancel = AtomicBool::new(false);
    let board = adapter_board();
    let budget = Duration::from_secs(5);
    let hero = range(&[("Ac", "Ad", 1.0)]);
    // AcKc cannot be dealt against hero's Ac, so it must leave the denominator instead of counting
    // as a loss: only QcQd remains and AcAd beats it, making equity exactly 1.
    let (v, _) = range_vs_range(&hero, &range(&[("Ac", "Kc", 1.0), ("Qc", "Qd", 1.0)]), &board, budget, &cancel).expect("QcQd is dealable");
    assert!((v - 1.0).abs() < 1e-6, "blocked tuples must leave the denominator, got {v}");
    // The same combo on both sides can never reach a showdown: proven incompatible, no value.
    assert!(range_vs_range(&hero, &hero, &board, budget, &cancel).is_none());
    // A one-combo public range and hero's actual combo describe the same population: 0.5 against
    // {KcKd, QcQd} at equal weight (a loss to the set, a win over the pair).
    let opp = range(&[("Kc", "Kd", 1.0), ("Qc", "Qd", 1.0)]);
    let (pub_v, _) = range_vs_range(&hero, &opp, &board, budget, &cancel).expect("dealable");
    let (combo_v, combo_m) = hero_combo_equity([c("Ac"), c("Ad")], &opp, &board, budget, &cancel).expect("dealable");
    assert!((combo_v - 0.5).abs() < 1e-6, "hero-combo equity was {combo_v}, expected 0.5");
    assert!((pub_v - combo_v).abs() < 1e-6, "public {pub_v} and combo {combo_v} populations must agree here");
    assert_eq!(combo_m, EquityMethod::Exact);
}

#[test]
fn mode_is_chosen_by_exact_cost_not_by_street() {
    let cancel = AtomicBool::new(false);
    let flop: Vec<Card> = "Ks 7d 2c".split(' ').map(c).collect();
    // Two one-combo ranges on a flop cost 1 * 1 * C(45, 2) = 990 evaluations, far under 2e7.
    let (_, m) = range_vs_range(&range(&[("Ac", "Ad", 1.0)]), &range(&[("Qc", "Qd", 1.0)]), &flop, Duration::from_secs(5), &cancel).expect("dealable");
    assert_eq!(m, EquityMethod::Exact, "a cheap flop request must still enumerate exactly");
    // Two uniform ranges on the same flop price ~1.4e9, so the adapter must switch to Monte Carlo
    // and keep its metadata. Uniform against uniform is symmetric, so the answer is 0.5.
    let (v, m) = range_vs_range(&Range1326::uniform(), &Range1326::uniform(), &flop, Duration::from_secs(20), &cancel).expect("dealable");
    match m {
        EquityMethod::MonteCarlo { samples, std_err } => assert!(samples > 0 && std_err > 0.0, "Monte Carlo metadata was samples {samples}, std_err {std_err}"),
        EquityMethod::Exact => panic!("a request priced above 2e7 must not enumerate exactly"),
    }
    assert!((v - 0.5).abs() < 0.02, "uniform against uniform is symmetric, got {v}");
}

#[test]
fn summary_maps_seats_and_keeps_the_public_population_independent_of_hero_cards() {
    let cancel = AtomicBool::new(false);
    let board = adapter_board();
    let budget = Duration::from_secs(10);
    let hero_public = range(&[("Ac", "Ad", 1.0), ("4c", "4d", 0.5)]);
    let opps = vec![(Seat(3), range(&[("Kc", "Kd", 1.0), ("Qc", "Qd", 1.0)])), (Seat(5), range(&[("Qc", "Qd", 1.0)]))];
    let before = opps.clone();
    let a = equity_summary(Some([c("Ac"), c("Ad")]), &hero_public, &opps, &board, budget, &cancel);
    let b = equity_summary(Some([c("4c"), c("4d")]), &hero_public, &opps, &board, budget, &cancel);
    // Both populations report every opponent, in the caller's order; per-pot shares are the
    // multiway path's, not the summary's.
    assert_eq!((seats(&a.hero_combo_vs_each), seats(&a.hero_range_vs_each)), (vec![Seat(3), Seat(5)], vec![Seat(3), Seat(5)]));
    assert!(a.per_pot_shares.is_empty() && b.per_pot_shares.is_empty());
    // Hero's actual cards never reach a public range, so the range-versus-range population is
    // identical for two different hero holdings...
    assert_eq!(values(&a.hero_range_vs_each), values(&b.hero_range_vs_each));
    assert!(values(&a.hero_range_vs_each).iter().all(|v| v.is_some()));
    // ...while the combo population does move with them: AcAd beats QcQd and splits with {KK, QQ},
    // 4c4d loses to both.
    assert_eq!((value_of(&a.hero_combo_vs_each, Seat(5)), value_of(&b.hero_combo_vs_each, Seat(5))), (Some(1.0), Some(0.0)));
    assert_eq!((value_of(&a.hero_combo_vs_each, Seat(3)), value_of(&b.hero_combo_vs_each, Seat(3))), (Some(0.5), Some(0.0)));
    // Neither call wrote into the caller's ranges.
    assert!(opps.iter().zip(&before).all(|((s, r), (s0, r0))| s == s0 && r.0 == r0.0), "equity_summary must not mutate its inputs");
}

#[test]
fn pending_and_unknown_hero_are_labelled_not_guessed() {
    let cancel = AtomicBool::new(false);
    let board = adapter_board();
    let opps = vec![(Seat(1), range(&[("Qc", "Qd", 1.0)])), (Seat(4), range(&[("Kc", "Kd", 1.0)]))];
    let p = pending_summary(&[Seat(1), Seat(4)]);
    assert_eq!((seats(&p.hero_combo_vs_each), seats(&p.hero_range_vs_each)), (vec![Seat(1), Seat(4)], vec![Seat(1), Seat(4)]));
    assert!(p.hero_combo_vs_each.iter().chain(&p.hero_range_vs_each).all(|(_, e)| e.value.is_none() && e.method.is_none() && e.availability == Availability::Pending));
    assert!(p.per_pot_shares.is_empty());
    // Hero's cards are not entered: the combo population says exactly that, and never "no
    // compatible holdings"; the public population still answers.
    let s = equity_summary(None, &Range1326::uniform(), &opps, &board, Duration::from_secs(10), &cancel);
    assert!(s.hero_combo_vs_each.iter().all(|(_, e)| e.value.is_none() && e.method.is_none() && matches!(&e.availability, Availability::Unavailable { reason } if reason == "hero's cards are not entered")), "{s:?}");
    assert!(s.hero_range_vs_each.iter().all(|(_, e)| e.value.is_some() && e.availability == Availability::Ready && e.method.is_some()), "{s:?}");
}

#[test]
fn a_stop_is_reported_as_a_stop_not_as_invalid_ranges() {
    let board = adapter_board();
    let live = AtomicBool::new(false);
    let cancelled = AtomicBool::new(true);
    let opp = range(&[("Kc", "Kd", 1.0), ("Qc", "Qd", 1.0)]);
    // A zero budget is exhausted on core-eval's entry poll whatever the timer resolution reports,
    // so this needs no sleep and no wall-clock margin.
    let stopped = facing_allin(&input(qq_54o(1.0 / 12.0), Rake::TimeCharge), Duration::ZERO, &live);
    assert!(matches!(stopped, Err(proto::UnsupportedReason::DeadlineExceeded { .. })), "a budget stop is not bad input: {stopped:?}");
    let cut = facing_allin(&input(qq_54o(1.0 / 12.0), Rake::TimeCharge), Duration::from_secs(5), &cancelled);
    assert!(matches!(cut, Err(proto::UnsupportedReason::EngineError { retryable: true, .. })), "a cancellation is retryable, not bad input: {cut:?}");
    // ...while a genuinely undealable pairing stays InvalidRanges.
    let proven = facing_allin(&input(range(&[("Ah", "Ad", 1.0)]), Rake::TimeCharge), Duration::from_secs(5), &live);
    assert!(matches!(proven, Err(proto::UnsupportedReason::InvalidRanges)), "{proven:?}");
    // Both adapter entry points report a stop as no value, with the ranges themselves valid.
    assert!(hero_combo_equity([c("Ac"), c("Ad")], &opp, &board, Duration::ZERO, &live).is_none());
    assert!(range_vs_range(&Range1326::uniform(), &opp, &board, Duration::ZERO, &live).is_none());
    assert!(hero_combo_equity([c("Ac"), c("Ad")], &opp, &board, Duration::from_secs(5), &cancelled).is_none());
}

/// A clock the test drives: every reading advances by `step` ms. A summary that measures elapsed
/// time against one deadline sees it pass; one that renews the duration per estimate never does.
struct StepClock { now: AtomicU64, step: u64 }

impl Clock for StepClock {
    fn now_ms(&self) -> u64 { self.now.fetch_add(self.step, Ordering::Relaxed) }
    fn wait_until(&self, _t_ms: u64) { unreachable!("the equity summary never waits") }
}

/// Spec section 7: the equity phase has one 0.5 s budget and "every phase receives only the
/// remaining time" -- not 0.5 s per opponent per population.
#[test]
fn the_summary_budget_is_one_deadline_for_the_whole_phase() {
    let cancel = AtomicBool::new(false);
    let board = adapter_board();
    let budget = Duration::from_millis(EQUITY_BUDGET_MS);
    let hero = Some([c("Ac"), c("Ad")]);
    let hero_public = range(&[("Ac", "Ad", 1.0)]);
    let opps = vec![(Seat(1), range(&[("Qc", "Qd", 1.0)])), (Seat(4), range(&[("Kc", "Kd", 1.0)]))];
    // A frozen clock: no time passes, so all four estimates run inside the one budget.
    let frozen = StepClock { now: AtomicU64::new(0), step: 0 };
    let all = equity_summary_with_clock(&frozen, hero, &hero_public, &opps, &board, budget, &cancel);
    assert!(all.hero_combo_vs_each.iter().chain(&all.hero_range_vs_each).all(|(_, e)| e.availability == Availability::Ready && e.value.is_some()), "{all:?}");
    // 300 ms of the 500 ms phase per reading: the first estimate fits, the other three are out of
    // time and are reported as such instead of each being handed a fresh 500 ms.
    let ticking = StepClock { now: AtomicU64::new(0), step: 300 };
    let s = equity_summary_with_clock(&ticking, hero, &hero_public, &opps, &board, budget, &cancel);
    assert_eq!((seats(&s.hero_combo_vs_each), seats(&s.hero_range_vs_each)), (vec![Seat(1), Seat(4)], vec![Seat(1), Seat(4)]));
    // The finished estimate is preserved...
    assert_eq!(s.hero_combo_vs_each[0].1.availability, Availability::Ready);
    assert_eq!(s.hero_combo_vs_each[0].1.value, Some(1.0));
    // ...and the unstarted ones are unavailable for the stated reason, never a guessed number.
    assert!(expired(&s.hero_combo_vs_each[1].1) && expired(&s.hero_range_vs_each[0].1) && expired(&s.hero_range_vs_each[1].1), "{s:?}");
    assert!(s.hero_combo_vs_each[1].1.value.is_none() && s.hero_range_vs_each.iter().all(|(_, e)| e.value.is_none()), "{s:?}");
}
