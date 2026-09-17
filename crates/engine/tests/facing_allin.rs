use engine::allin::{facing_allin, AllInInput};
use proto::{combo_index, Action, Card, Rake, Range1326};
use std::sync::atomic::AtomicBool;
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
