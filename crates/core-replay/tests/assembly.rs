//! P3.T16 -- branch-supported strategy, EV and unresolved mass (spec sections 8.4, 6 and 8.3;
//! the section 13.1 rows `replay_incomplete_branch_ev` (T7) and `replay_hero_out_of_support`).
//!
//! `replay_incomplete_branch_ev` opens with the task brief's Step 1 body, verbatim, and then runs
//! the same T7 example through `mix_nodes` (the brief's Step 5 list). The tests after it cover the
//! Step 3 precedence rules one by one (NotInMenu, BranchSupportIncomplete including covered = 0,
//! NoEvReference, ChartNoEv, EvReferenceUnverified, MovedProbability, mixed references), unequal
//! menus of the same kind at different chip amounts, the no-node key selection, the frozen
//! stopped branch, the range-level mix and the always-on input checks.
//!
//! Fix round 1: T16-R1's source-rounding policy (rows admitted by spec 8.2's `1 +- 1e-3` are
//! normalized once at the mapped-node boundary, fed here from real `legalize_row` output; rows
//! outside it are rejected) and T16-R2's retained key for a heaviest residual.

use core_preflop::branches::{initial, rescale, HistoryBranch};
use core_preflop::{destination_map, legalize_row, mix_action, mix_nodes, BranchNode, EvReference, ExpandedNode, MixedNode, SourceKind};
use proto::{Action, ActionAdvice, ApproxReason, LegalAction, Seat, Unavailable, UnsupportedReason, COMBOS};

const V: Seat = Seat(0);
const H: Seat = Seat(1);
const FOLD: Action = Action::Fold;
const CHECK: Action = Action::Check;
const CALL: Action = Action::Call;

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

/// Branches with the given weights, ids in creation order; every seat uniform, so hero's branch
/// posterior is `q_k / sum_j q_j` for every combo.
fn branches(qs: &[f64]) -> Vec<HistoryBranch> {
    let base = initial(&[V, H]).remove(0);
    qs.iter().enumerate().map(|(i, &q)| HistoryBranch { id: i as u8, q, ..base.clone() }).collect()
}

/// Hero's node with the same `(action, probability, EV chips)` row for every combo.
fn node(row: &[(Action, f32, Option<f32>)], reference: EvReference, source: SourceKind) -> ExpandedNode {
    ExpandedNode {
        actor: H,
        actions: row.iter().map(|a| a.0).collect(),
        probs: vec![row.iter().map(|a| a.1).collect(); COMBOS],
        ev_chips: vec![row.iter().map(|a| a.2).collect(); COMBOS],
        available: vec![true; COMBOS],
        ev_reference: reference,
        source,
    }
}

fn verified(row: &[(Action, f32, Option<f32>)]) -> ExpandedNode {
    node(row, EvReference::DecisionIncrementalVerified, SourceKind::PokerDataJson)
}

fn at(branch_id: u8, node: Option<ExpandedNode>, key: &str) -> BranchNode {
    BranchNode { branch_id, node, key: key.into(), created: vec![] }
}

fn advice(m: &MixedNode, a: Action) -> &ActionAdvice {
    m.actions.iter().find(|x| x.action == a).unwrap_or_else(|| panic!("{a:?} missing from {:?}", m.actions))
}

fn frequency_total(m: &MixedNode) -> f64 {
    m.actions.iter().map(|a| f64::from(a.frequency.expect("hero supported: every action has a frequency"))).sum()
}

fn covered(a: &ActionAdvice) -> f32 {
    match a.unavailable {
        Some(Unavailable::BranchSupportIncomplete { covered_posterior }) => covered_posterior,
        ref other => panic!("{:?}: expected BranchSupportIncomplete, got {other:?}", a.action),
    }
}

#[test]
fn replay_incomplete_branch_ev(){
    use core_preflop::mix_action;
    let (p,ev,why)=mix_action(&[0.2,0.8],&[Some(0.5),None],&[Some(10.),None],true);
    assert!((p-0.1).abs()<1e-12);assert_eq!(ev,None);
    assert!(matches!(why,Some(proto::Unavailable::BranchSupportIncomplete{covered_posterior})
        if (covered_posterior-0.2).abs()<1e-6));
    let (_,ev,_)=mix_action(&[0.2,0.8],&[Some(0.5),Some(0.2)],&[Some(10.),Some(-2.)],true);
    assert!((ev.unwrap()-0.4).abs()<1e-12);
    let (_,ev,why)=mix_action(&[0.19,0.76,0.05],&[Some(0.5),Some(0.2),None],
        &[Some(10.),Some(-2.),None],true);
    assert_eq!(ev,None);
    assert!(matches!(why,Some(proto::Unavailable::BranchSupportIncomplete{covered_posterior})
        if (covered_posterior-0.95).abs()<1e-6));

    // Step 5: the same T7 example assembled through `mix_nodes`. Posterior 0.2 / 0.8; the raise
    // exists only in the 0.2 branch; fold and call exist in both with EV. bb_chips = 2.
    let raise = Action::Raise { to: 60 };
    let a_row = [(FOLD, 0.3, Some(0.0)), (CALL, 0.5, Some(20.0)), (raise, 0.2, Some(100.0))];
    let b_row = [(FOLD, 0.4, Some(0.0)), (CALL, 0.6, Some(-10.0))];
    let bs = branches(&[0.2, 0.8]);
    let nodes = vec![at(0, Some(verified(&a_row)), "k0"), at(1, Some(verified(&b_row)), "k1")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, None);
    assert_eq!(m.unresolved_mass, 0.0);
    assert_eq!(m.actions.iter().map(|a| a.action).collect::<Vec<_>>(), vec![FOLD, CALL, raise]);
    let r = advice(&m, raise);
    close(f64::from(r.frequency.unwrap()), 0.2 * 0.2); // 0.2 * P_1(raise)
    assert_eq!(r.ev_bb, None, "the raise's +100 chips is never reported over incomplete support");
    close(f64::from(covered(r)), 0.2);
    let f = advice(&m, FOLD);
    close(f64::from(f.frequency.unwrap()), 0.2 * 0.3 + 0.8 * 0.4);
    assert_eq!(f.ev_bb, Some(0.0));
    assert_eq!(f.unavailable, None);
    let c = advice(&m, CALL);
    close(f64::from(c.frequency.unwrap()), 0.2 * 0.5 + 0.8 * 0.6);
    close(f64::from(c.ev_bb.unwrap()), (0.2 * 20.0 + 0.8 * -10.0) / 2.0); // chips -> bb once
    assert_eq!(c.unavailable, None);
    close(frequency_total(&m) + f64::from(m.unresolved_mass), 1.0);
    assert!(m.actions.iter().all(|a| !a.headline), "mix_nodes never sets a headline");
    assert!(m.reasons.is_empty() && m.notes.is_empty(), "{:?} {:?}", m.reasons, m.notes);

    // A 0.05 residual: every EV is None, coverage is incomplete (<= 0.95) and unresolved = 0.05.
    let mut bs = branches(&[0.19, 0.76, 0.05]);
    bs[2].residual = true;
    let nodes = vec![at(0, Some(verified(&a_row)), "k0"), at(1, Some(verified(&b_row)), "k1"), at(2, None, "")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, None);
    close(f64::from(m.unresolved_mass), 0.05);
    for a in &m.actions {
        assert_eq!(a.ev_bb, None, "{:?}", a.action);
        assert!(covered(a) <= 0.95 + 1e-6, "{:?}", a.action);
    }
    close(f64::from(covered(advice(&m, FOLD))), 0.95);
    close(f64::from(covered(advice(&m, CALL))), 0.95);
    close(f64::from(covered(advice(&m, raise))), 0.19);
    close(frequency_total(&m) + f64::from(m.unresolved_mass), 1.0);
    match m.reasons.as_slice() {
        [ApproxReason::BranchResidual { seat, residual_mass_pct, cause }] => {
            assert_eq!(*seat, H);
            close(f64::from(*residual_mass_pct), 5.0);
            // Both live branches have their node; only the cap residual lacks one, so its share is
            // disclosed as the cap (ruling 19-I1, spec 8.4), never as a missing node that is present.
            assert_eq!(cause, "cap");
        }
        other => panic!("expected one BranchResidual reason, got {other:?}"),
    }
    assert!(m.notes.iter().any(|n| n == "5.0% of the posterior has no strategy"), "{:?}", m.notes);
}

#[test]
fn replay_hero_out_of_support() {
    // Hero's combo 0 has zero public mass in every branch; the rest of the range is uniform.
    let mut bs = branches(&[0.25, 0.75]);
    for b in &mut bs {
        b.seats[1].mass[0] = 0.0;
    }
    let nodes = vec![
        at(0, Some(verified(&[(FOLD, 0.2, Some(0.0)), (CALL, 0.8, Some(4.0))])), "k0"),
        at(1, Some(verified(&[(FOLD, 0.6, Some(0.0)), (CALL, 0.4, Some(6.0))])), "k1"),
    ];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, Some(UnsupportedReason::HeroComboOutOfSupport));
    let mix = m.range_mix.as_ref().expect("range-level mix present under HeroComboOutOfSupport");
    assert_eq!(mix.iter().map(|x| x.0).collect::<Vec<_>>(), vec![FOLD, CALL]);
    close(f64::from(mix[0].1), 0.25 * 0.2 + 0.75 * 0.6);
    close(f64::from(mix[1].1), 0.25 * 0.8 + 0.75 * 0.4);
    assert!(mix.iter().any(|x| x.1 > 0.0), "a positive range-level mix remains");
    assert_eq!(m.actions.len(), 2);
    for a in &m.actions {
        assert_eq!(a.frequency, None, "no per-combo frequency for {:?}", a.action);
        assert_eq!(a.ev_bb, None, "no per-combo EV for {:?}", a.action);
        assert_eq!(a.unavailable, Some(Unavailable::HeroOutOfSupport));
        assert!(!a.headline);
    }
    assert_eq!(m.unresolved_mass, 0.0);
}

/// Section 13.1's `replay_off_tree_pseudo_harmonic`, last clause (plan 3 Task 19's test-name
/// audit): "node translation mixes strategies with `pi` and EV with the complete-support rule". The
/// row's other figures are `tests/branches.rs`'s test of the same name; this clause is an assembly
/// rule, so it is asserted here. The row's split -- 73 into 100 over menu sizes 50/100 (`f =
/// 81/173, 92/173`), `P_A = (0.9, 0.3)`, `P_B = (0.1, 0.5)` on the actor's two combos -- gives the
/// actor a branch posterior that differs per combo (`pi_A = 0.888` for combo 1, `0.346` for combo
/// 2). The actor's next node differs per branch, and a raise exists only after 50: each combo's
/// advice mixes the two nodes with that combo's own posterior, the raise gets its known frequency
/// but no EV (only its covered posterior), and fold and call average their EVs over both branches.
#[test]
fn replay_off_tree_pseudo_harmonic() {
    use core_preflop::branches::{condition, posterior};
    let menu = [Action::Bet { to: 50 }, Action::Bet { to: 100 }];
    let s = core_preflop::wager_fraction(73, 0, 0, 100).expect("73 into 100");
    let t = core_preflop::interpolate(s, &core_preflop::menu_fractions(&menu, 0, 0, 100).expect("both sizes")).expect("73 lies between 50 and 100");
    let (fa, fb) = (t.choices[0].1, t.choices[1].1);
    close(fa, 81.0 / 173.0);
    close(fb, 92.0 / 173.0);
    let mut start = initial(&[V, H]).remove(0);
    for seat in &mut start.seats {
        seat.mass.fill(0.0);
        seat.mass[0] = 1.0;
        seat.mass[1] = 1.0;
    }
    let pair = |a: f64, b: f64| {
        let mut p = vec![0.0; COMBOS];
        p[0] = a;
        p[1] = b;
        p
    };
    let mut bs = vec![condition(&start, V, &pair(0.9, 0.3), fa).expect("M_A = 0.6"), condition(&start, V, &pair(0.1, 0.5), fb).expect("M_B = 0.3")];
    bs[1].id = 1;
    close(bs[0].q, fa * 0.6);
    close(bs[1].q, fb * 0.3);
    // The actor's own next node in each branch: after bet 50 it may raise, after bet 100 it may not.
    let raise = Action::Raise { to: 300 };
    let actor = |row: &[(Action, f32, Option<f32>)]| ExpandedNode { actor: V, ..verified(row) };
    let nodes = vec![
        at(0, Some(actor(&[(FOLD, 0.2, Some(0.0)), (CALL, 0.5, Some(12.0)), (raise, 0.3, Some(30.0))])), "after bet 50"),
        at(1, Some(actor(&[(FOLD, 0.6, Some(0.0)), (CALL, 0.4, Some(-8.0))])), "after bet 100"),
    ];
    let pi_a = [fa * 0.9 / (fa * 0.9 + fb * 0.1), fa * 0.3 / (fa * 0.3 + fb * 0.5)];
    assert!((pi_a[0] - 0.888).abs() < 5e-4 && (pi_a[1] - 0.346).abs() < 5e-4, "the row's rounded posteriors: {pi_a:?}");
    let mut frequencies = vec![];
    for (combo, &pa) in pi_a.iter().enumerate() {
        let pb = 1.0 - pa;
        let pi = posterior(&bs, V, combo);
        close(pi[0], pa);
        close(pi[1], pb);
        let m = mix_nodes(&bs, &nodes, V, combo, 10);
        assert_eq!(m.unsupported, None);
        assert_eq!(m.unresolved_mass, 0.0);
        assert_eq!(m.actions.iter().map(|a| a.action).collect::<Vec<_>>(), vec![FOLD, CALL, raise]);
        let fold = advice(&m, FOLD);
        close(f64::from(fold.frequency.unwrap()), pa * 0.2 + pb * 0.6);
        assert_eq!((fold.ev_bb, fold.unavailable.clone()), (Some(0.0), None));
        let call = advice(&m, CALL);
        close(f64::from(call.frequency.unwrap()), pa * 0.5 + pb * 0.4);
        close(f64::from(call.ev_bb.unwrap()), (pa * 12.0 + pb * -8.0) / 10.0);
        assert_eq!(call.unavailable, None);
        let r = advice(&m, raise);
        close(f64::from(r.frequency.unwrap()), pa * 0.3);
        assert_eq!(r.ev_bb, None, "the raise exists in branch A only: no EV over incomplete support");
        close(f64::from(covered(r)), pa);
        close(frequency_total(&m), 1.0);
        frequencies.push(m.actions.iter().map(|a| a.frequency.unwrap()).collect::<Vec<_>>());
    }
    assert_ne!(frequencies[0], frequencies[1], "each combo mixes with its own branch posterior");
}

#[test]
fn explicitly_unreachable_hero_class_is_out_of_support() {
    // Positive public mass, but the source marks hero's class unreachable at the node: never
    // replaced by a nearby hand's strategy.
    let bs = branches(&[1.0]);
    let mut n = verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(2.0))]);
    n.available[7] = false;
    n.probs[7] = vec![0.0, 0.0];
    n.ev_chips[7] = vec![None, None];
    let m = mix_nodes(&bs, &[at(0, Some(n), "k0")], H, 7, 2);
    assert_eq!(m.unsupported, Some(UnsupportedReason::HeroComboOutOfSupport));
    assert!(m.range_mix.is_some());
    assert!(m.actions.iter().all(|a| a.frequency.is_none() && a.ev_bb.is_none()
        && a.unavailable == Some(Unavailable::HeroOutOfSupport)));
}

#[test]
fn range_mix_weights_combos_by_branch_mass_independently_of_hero_combo() {
    // One branch: hero's mass is 1 on combo 0 (always folds) and 3 on combo 1 (always calls).
    let mut bs = branches(&[1.0]);
    bs[0].seats[1].mass.fill(0.0);
    bs[0].seats[1].mass[0] = 1.0;
    bs[0].seats[1].mass[1] = 3.0;
    let mut n = verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(2.0))]);
    n.probs[0] = vec![1.0, 0.0];
    n.probs[1] = vec![0.0, 1.0];
    let m = mix_nodes(&bs, &[at(0, Some(n), "k0")], H, 0, 2);
    assert_eq!(m.unsupported, None);
    let mix = m.range_mix.as_ref().unwrap();
    close(f64::from(mix[0].1), 0.25);
    close(f64::from(mix[1].1), 0.75);
    assert_eq!(advice(&m, FOLD).frequency, Some(1.0));
    assert_eq!(advice(&m, CALL).frequency, Some(0.0));
}

#[test]
fn range_mix_excludes_and_discloses_the_unresolved_range_mass() {
    let mut bs = branches(&[0.6, 0.4]);
    bs[1].stopped = Some("missing node kB".into());
    let nodes = vec![at(0, Some(verified(&[(FOLD, 0.25, Some(0.0)), (CALL, 0.75, Some(2.0))])), "kA"), at(1, None, "kB")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    let mix = m.range_mix.as_ref().unwrap();
    close(f64::from(mix[0].1), 0.25); // renormalized over the node-covered mass only
    close(f64::from(mix[1].1), 0.75);
    assert!(m.notes.iter().any(|n| n == "range mix excludes 40.0% of hero's public range mass (no strategy)"), "{:?}", m.notes);
    close(f64::from(m.unresolved_mass), 0.4);
    close(f64::from(advice(&m, FOLD).frequency.unwrap()), 0.6 * 0.25); // never renormalized
    close(frequency_total(&m) + f64::from(m.unresolved_mass), 1.0);
}

#[test]
fn unequal_menus_of_one_kind_at_different_chips_are_different_actions() {
    let r12 = Action::Raise { to: 12 };
    let r13 = Action::Raise { to: 13 };
    let bs = branches(&[0.5, 0.5]);
    let nodes = vec![
        at(0, Some(verified(&[(FOLD, 0.5, Some(0.0)), (r12, 0.5, Some(30.0))])), "k0"),
        at(1, Some(verified(&[(FOLD, 0.5, Some(0.0)), (r13, 0.5, Some(40.0))])), "k1"),
    ];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.actions.iter().map(|a| a.action).collect::<Vec<_>>(), vec![FOLD, r12, r13]);
    for r in [r12, r13] {
        let a = advice(&m, r);
        close(f64::from(a.frequency.unwrap()), 0.25);
        assert_eq!(a.ev_bb, None, "{r:?} is not averaged with the other raise size");
        close(f64::from(covered(a)), 0.5);
    }
    assert_eq!(advice(&m, FOLD).ev_bb, Some(0.0));
    close(frequency_total(&m), 1.0);
}

#[test]
fn mixed_reference_variants_give_no_ev_even_when_the_numbers_agree() {
    let row = [(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(10.0))];
    let bs = branches(&[0.5, 0.5]);
    let nodes = vec![
        at(0, Some(node(&row, EvReference::DecisionIncrementalVerified, SourceKind::PokerDataJson)), "k0"),
        at(1, Some(node(&row, EvReference::NetHandStartVerified, SourceKind::PokerDataJson)), "k1"),
    ];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    for a in &m.actions {
        assert_eq!(a.ev_bb, None, "{:?}", a.action);
        assert_eq!(a.unavailable, Some(Unavailable::NoEvReference), "{:?}", a.action);
    }
    close(f64::from(advice(&m, CALL).frequency.unwrap()), 0.5);
    // mix_action itself refuses to average over a mixed reference set.
    let (_, ev, why) = mix_action(&[0.5, 0.5], &[Some(0.5), Some(0.5)], &[Some(10.0), Some(10.0)], false);
    assert_eq!(ev, None);
    assert_eq!(why, Some(Unavailable::NoEvReference));
}

#[test]
fn complete_support_with_an_unnormalizable_ev_is_no_ev_reference() {
    let bs = branches(&[0.5, 0.5]);
    let nodes = vec![
        at(0, Some(verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(10.0))])), "k0"),
        at(1, Some(verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, None)])), "k1"),
    ];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    let c = advice(&m, CALL);
    assert_eq!(c.ev_bb, None);
    assert_eq!(c.unavailable, Some(Unavailable::NoEvReference));
    assert_eq!(advice(&m, FOLD).ev_bb, Some(0.0));
}

#[test]
fn incomplete_support_with_no_ev_anywhere_reports_covered_zero() {
    // The one node present has the call but no EV for it; the other branch is stopped.
    let mut bs = branches(&[0.6, 0.4]);
    bs[1].stopped = Some("missing node k1".into());
    let nodes = vec![at(0, Some(verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, None)])), "k0"), at(1, None, "k1")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    let c = advice(&m, CALL);
    assert_eq!(c.ev_bb, None);
    assert_eq!(covered(c), 0.0);
    close(f64::from(covered(advice(&m, FOLD))), 0.6);
    assert!(matches!(m.reasons.as_slice(),
        [ApproxReason::BranchResidual { cause, .. }] if cause == "missing node k1"), "{:?}", m.reasons);
}

#[test]
fn an_action_in_no_positive_branch_is_not_in_menu() {
    // The raise exists only in a branch where hero's combo has zero posterior.
    let r20 = Action::Raise { to: 20 };
    let mut bs = branches(&[0.7, 0.3]);
    bs[1].seats[1].mass[0] = 0.0;
    let nodes = vec![
        at(0, Some(verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(4.0))])), "k0"),
        at(1, Some(verified(&[(FOLD, 0.2, Some(0.0)), (CALL, 0.3, Some(2.0)), (r20, 0.5, Some(8.0))])), "k1"),
    ];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    let r = advice(&m, r20);
    assert_eq!(r.frequency, Some(0.0));
    assert_eq!(r.ev_bb, None);
    assert_eq!(r.unavailable, Some(Unavailable::NotInMenu));
    assert_eq!(advice(&m, CALL).ev_bb, Some(2.0)); // the zero-posterior branch contributes nothing
    close(frequency_total(&m), 1.0);
    let (p, ev, why) = mix_action(&[1.0, 0.0], &[None, Some(0.5)], &[None, Some(8.0)], true);
    assert_eq!((p, ev, why), (0.0, None, Some(Unavailable::NotInMenu)));
}

#[test]
fn chart_complete_nodes_use_chart_no_ev_and_chart_rounded() {
    let row = [(FOLD, 0.4, None), (CALL, 0.6, None)];
    let bs = branches(&[0.5, 0.5]);
    let chart = || Some(node(&row, EvReference::Unverified, SourceKind::ChartTranscription));
    let m = mix_nodes(&bs, &[at(0, chart(), "k0"), at(1, chart(), "k1")], H, 0, 2);
    for a in &m.actions {
        assert_eq!(a.ev_bb, None);
        assert_eq!(a.unavailable, Some(Unavailable::ChartNoEv), "{:?}", a.action);
    }
    assert_eq!(m.reasons, vec![ApproxReason::ChartRounded]);
    close(f64::from(advice(&m, CALL).frequency.unwrap()), 0.6);
}

#[test]
fn unverified_complete_nodes_use_no_ev_reference_and_ev_reference_unverified() {
    let row = [(FOLD, 0.4, None), (CALL, 0.6, None)];
    let bs = branches(&[0.5, 0.5]);
    let unverified = || Some(node(&row, EvReference::Unverified, SourceKind::PokerDataJson));
    let m = mix_nodes(&bs, &[at(0, unverified(), "k0"), at(1, unverified(), "k1")], H, 0, 2);
    for a in &m.actions {
        assert_eq!(a.ev_bb, None);
        assert_eq!(a.unavailable, Some(Unavailable::NoEvReference), "{:?}", a.action);
    }
    assert_eq!(m.reasons, vec![ApproxReason::EvReferenceUnverified]);
}

#[test]
fn a_legality_created_destination_keeps_moved_probability() {
    let bet120 = Action::Bet { to: 120 };
    let allin = Action::AllIn { to: 100 };
    let created = |id: u8, key: &str| BranchNode {
        branch_id: id,
        node: Some(verified(&[(CHECK, 0.6, Some(1.0)), (allin, 0.4, None)])),
        key: key.into(),
        created: vec![(allin, bet120)],
    };
    // One branch: Task 10's MovedProbability survives the assembly.
    let m = mix_nodes(&branches(&[1.0]), &[created(0, "k0")], H, 0, 2);
    let a = advice(&m, allin);
    close(f64::from(a.frequency.unwrap()), 0.4);
    assert_eq!(a.ev_bb, None);
    assert_eq!(a.unavailable, Some(Unavailable::MovedProbability { from: bet120 }));
    assert_eq!(advice(&m, CHECK).ev_bb, Some(0.5));
    // Two branches, one created and one with the source all-in's own EV: the move is the reason.
    let own = at(1, Some(verified(&[(CHECK, 0.6, Some(1.0)), (allin, 0.4, Some(50.0))])), "k1");
    let m = mix_nodes(&branches(&[0.5, 0.5]), &[created(0, "k0"), own], H, 0, 2);
    assert_eq!(advice(&m, allin).unavailable, Some(Unavailable::MovedProbability { from: bet120 }));
    // Missing support outranks the move.
    let mut bs = branches(&[0.5, 0.5]);
    bs[1].stopped = Some("missing node k1".into());
    let m = mix_nodes(&bs, &[created(0, "k0"), at(1, None, "k1")], H, 0, 2);
    assert_eq!(covered(advice(&m, allin)), 0.0);
    // A missing source EV beside the move is not "because of the move": NoEvReference.
    let no_ev = at(1, Some(verified(&[(CHECK, 0.6, None), (allin, 0.4, None)])), "k1");
    let m = mix_nodes(&branches(&[0.5, 0.5]), &[created(0, "k0"), no_ev], H, 0, 2);
    assert_eq!(advice(&m, allin).unavailable, Some(Unavailable::NoEvReference));
    // A chart has no EV whether or not it moved: ChartNoEv.
    let chart = BranchNode {
        branch_id: 0,
        node: Some(node(&[(CHECK, 0.6, None), (allin, 0.4, None)], EvReference::Unverified, SourceKind::ChartTranscription)),
        key: "k0".into(),
        created: vec![(allin, bet120)],
    };
    let m = mix_nodes(&branches(&[1.0]), &[chart], H, 0, 2);
    assert_eq!(advice(&m, allin).unavailable, Some(Unavailable::ChartNoEv));
}

#[test]
fn missing_node_in_every_positive_branch_names_the_heaviest_retained_key() {
    let stopped = |bs: &mut Vec<HistoryBranch>, k: usize, key: &str| bs[k].stopped = Some(format!("missing node {key}"));
    // Heaviest q wins.
    let mut bs = branches(&[0.3, 0.5, 0.2]);
    stopped(&mut bs, 0, "k0");
    stopped(&mut bs, 1, "k1");
    bs[2].residual = true;
    let nodes = vec![at(0, None, "k0"), at(1, None, "k1"), at(2, None, "")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, Some(UnsupportedReason::MissingPreflopNode { key: "k1".into() }));
    assert!(m.actions.is_empty());
    assert_eq!(m.range_mix, None);
    close(f64::from(m.unresolved_mass), 1.0);
    // Ties keep creation order.
    let mut bs = branches(&[0.4, 0.4, 0.2]);
    stopped(&mut bs, 0, "k0");
    stopped(&mut bs, 1, "k1");
    bs[2].residual = true;
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, Some(UnsupportedReason::MissingPreflopNode { key: "k0".into() }));
    // The residual is heaviest but has no source key: the heaviest known stopped/live key.
    let mut bs = branches(&[0.5, 0.3, 0.2]);
    bs[0].residual = true;
    stopped(&mut bs, 1, "k1");
    stopped(&mut bs, 2, "k2");
    let nodes = vec![at(0, None, ""), at(1, None, "k1"), at(2, None, "k2")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, Some(UnsupportedReason::MissingPreflopNode { key: "k1".into() }));
    // A node only in a branch where hero's combo has zero posterior does not count.
    let mut bs = branches(&[0.6, 0.4]);
    bs[0].seats[1].mass[0] = 0.0;
    stopped(&mut bs, 1, "k1");
    let nodes = vec![at(0, Some(verified(&[(FOLD, 1.0, Some(0.0))])), "k0"), at(1, None, "k1")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, Some(UnsupportedReason::MissingPreflopNode { key: "k1".into() }));
}

#[test]
fn a_heaviest_residual_names_the_heaviest_known_key_across_all_branches() {
    // T16-R2, the reviewer's combined case. q = [0.6 residual, 0.1 stopped "positive-key", 0.3 live
    // with a present node but zero mass for hero's combo 0]. Both positive-posterior branches lack
    // a node, so the decision is MissingPreflopNode; the heaviest positive-posterior branch is the
    // residual, which has no source key, so the key is the heaviest known stopped/live key across
    // ALL branches: branch 2's (q = 0.3), not the lighter positive branch 1's (q = 0.1).
    let mut bs = branches(&[0.6, 0.1, 0.3]);
    bs[0].residual = true;
    bs[1].stopped = Some("missing node positive-key".into());
    // Equal per-seat branch totals: combo 1 carries the mass combo 0 gives up in branch 2.
    bs[2].seats[1].mass[0] = 0.0;
    bs[2].seats[1].mass[1] = 2.0;
    rescale(&mut bs, &mut [0.0; 2]);
    let live = verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(2.0))]);
    let nodes = vec![at(0, None, ""), at(1, None, "positive-key"), at(2, Some(live.clone()), "heaviest-known-key")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, Some(UnsupportedReason::MissingPreflopNode { key: "heaviest-known-key".into() }));
    assert!(m.actions.is_empty());
    assert_eq!(m.range_mix, None);
    close(f64::from(m.unresolved_mass), 1.0);

    // Guard: the partial-coverage reason keeps its preference for an actual missing key. q = [0.5
    // residual, 0.1 stopped "k-missing", 0.4 live "k-live"]: hero has a node in branch 2 only, and
    // the missing-node BranchResidual names the stopped branch's missing key, although the residual
    // is heavier and the live key is the heaviest known key. Ruling 19-I1: the residual's own share
    // is a second reason, the cap's (50%), and the missing node's share is its own (10%).
    let mut bs = branches(&[0.5, 0.1, 0.4]);
    bs[0].residual = true;
    bs[1].stopped = Some("missing node k-missing".into());
    let nodes = vec![at(0, None, ""), at(1, None, "k-missing"), at(2, Some(live), "k-live")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, None);
    close(f64::from(m.unresolved_mass), 0.6);
    assert!(matches!(m.reasons.as_slice(),
        [ApproxReason::BranchResidual { cause: cap, residual_mass_pct: cap_pct, .. },
         ApproxReason::BranchResidual { cause, residual_mass_pct: missing_pct, .. }]
            if cap == "cap" && (cap_pct - 50.0).abs() < 1e-4 && cause == "missing node k-missing" && (missing_pct - 10.0).abs() < 1e-4),
        "{:?}", m.reasons);
}

// ---------------------------------------------------------------------------------------------
// Ruling 19-I1 (P3.T19 fix round 1): `BranchResidual` names only causes actually incurred (spec
// section 2), with spec section 8.4's vocabulary -- the persistent cap residual's share is `"cap"`,
// the share of positive-posterior branches whose key has no node is `"missing node <key>"` (the
// heaviest such key) -- one reason per distinct cause, each with its own share, so both shares sum
// to the unresolved mass; the note keeps the total.
// ---------------------------------------------------------------------------------------------

/// The residual share as `(residual_mass_pct, cause)` pairs, in the order the reasons list them.
fn residual_causes(m: &MixedNode) -> Vec<(f32, String)> {
    m.reasons
        .iter()
        .filter_map(|r| match r {
            ApproxReason::BranchResidual { seat, residual_mass_pct, cause } => {
                assert_eq!(*seat, H, "hero's residual");
                Some((*residual_mass_pct, cause.clone()))
            }
            _ => None,
        })
        .collect()
}

fn assert_causes(m: &MixedNode, expected: &[(f64, &str)]) {
    let got = residual_causes(m);
    assert_eq!(got.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>(), expected.iter().map(|(_, c)| *c).collect::<Vec<_>>(), "{:?}", m.reasons);
    for ((pct, cause), (want, _)) in got.iter().zip(expected) {
        close(f64::from(*pct), *want);
        assert!(!cause.is_empty());
    }
}

/// T7's residual case: both live branches have hero's node; only the cap residual (q = 0.05) has no
/// strategy. The reason is the cap's, never a "missing node" naming a node that is present.
#[test]
fn a_cap_residual_alone_is_disclosed_as_the_cap() {
    let raise = Action::Raise { to: 60 };
    let mut bs = branches(&[0.19, 0.76, 0.05]);
    bs[2].residual = true;
    let nodes = vec![
        at(0, Some(verified(&[(FOLD, 0.3, Some(0.0)), (CALL, 0.5, Some(20.0)), (raise, 0.2, Some(100.0))])), "k0"),
        at(1, Some(verified(&[(FOLD, 0.4, Some(0.0)), (CALL, 0.6, Some(-10.0))])), "k1"),
        at(2, None, ""),
    ];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, None);
    close(f64::from(m.unresolved_mass), 0.05);
    assert_causes(&m, &[(5.0, "cap")]);
    assert!(m.reasons.iter().all(|r| !matches!(r, ApproxReason::BranchResidual { cause, .. } if cause.starts_with("missing node"))), "{:?}", m.reasons);
    assert!(m.notes.iter().any(|n| n == "5.0% of the posterior has no strategy"), "{:?}", m.notes);
}

/// The cap residual (q = 0.5) beside a stopped branch whose node is missing (q = 0.1, key
/// "k-missing") and a live branch with hero's node (q = 0.4): two reasons, one per cause, each with
/// its own share (50% and 10%), summing to the unresolved 60%, which the note states once.
#[test]
fn a_cap_residual_beside_a_missing_node_discloses_both_causes() {
    let mut bs = branches(&[0.5, 0.1, 0.4]);
    bs[0].residual = true;
    bs[1].stopped = Some("missing node k-missing".into());
    let live = verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(2.0))]);
    let nodes = vec![at(0, None, ""), at(1, None, "k-missing"), at(2, Some(live), "k-live")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_eq!(m.unsupported, None);
    close(f64::from(m.unresolved_mass), 0.6);
    assert_causes(&m, &[(50.0, "cap"), (10.0, "missing node k-missing")]);
    let shares: f64 = residual_causes(&m).iter().map(|(pct, _)| f64::from(*pct)).sum();
    close(shares / 100.0, f64::from(m.unresolved_mass)); // the two causes' shares sum to the unresolved mass
    assert_eq!(m.notes.iter().filter(|n| n.ends_with("% of the posterior has no strategy")).collect::<Vec<_>>(), vec!["60.0% of the posterior has no strategy"]);
}

/// No residual: a missing node alone keeps its cause and its whole share, exactly as before.
#[test]
fn a_missing_node_alone_keeps_its_cause() {
    let mut bs = branches(&[0.6, 0.4]);
    bs[1].stopped = Some("missing node kB".into());
    let nodes = vec![at(0, Some(verified(&[(FOLD, 0.25, Some(0.0)), (CALL, 0.75, Some(2.0))])), "kA"), at(1, None, "kB")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_causes(&m, &[(40.0, "missing node kB")]);
    assert!(m.notes.iter().any(|n| n == "40.0% of the posterior has no strategy"), "{:?}", m.notes);
}

/// A positive branch with no node and no retained key (no entry at all) is still a missing node,
/// but its reason never borrows another branch's key, whose node is present: it names none.
#[test]
fn a_missing_node_without_a_retained_key_names_no_present_key() {
    let bs = branches(&[0.7, 0.3]);
    let nodes = vec![at(0, Some(verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(2.0))])), "k-present")];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    assert_causes(&m, &[(30.0, "missing node no retained key")]);
}

/// Hero's node built from real P3.T10 output, one source row repeated on every combo: source
/// actions `Call` and raises to 10, 11, ... chips, every raise below the live minimum raise (20)
/// with no legal source raise, so `destination_map` moves each raise into the existing `Call` and
/// `legalize_row` sums their probabilities there in `f32` (spec 8.4's legality-after-mapping
/// rule). The `Call` keeps its own 2-chip EV; the moved raises' EVs are never used.
fn legalized_call_node(source: &[f32]) -> ExpandedNode {
    let actions: Vec<Action> = (0..source.len()).map(|i| if i == 0 { CALL } else { Action::Raise { to: 9 + i as u32 } }).collect();
    let legal = [LegalAction::Fold, LegalAction::Call { cost: 2 }, LegalAction::Raise { min_to: 20, max_to: 100 }, LegalAction::AllIn { to: 100 }];
    let (menu, map) = destination_map(&actions, &legal).expect("every raise has a legal destination");
    assert_eq!(menu, vec![CALL], "every raise was moved into the existing call");
    let evs: Vec<Option<f32>> = (0..source.len()).map(|i| Some(2.0 + i as f32)).collect();
    let row = legalize_row(&menu, &map, source, &evs, &mut vec![]);
    assert_eq!(row.unsupported, None);
    ExpandedNode {
        actor: H,
        actions: row.actions.iter().map(|m| m.action).collect(),
        probs: vec![row.actions.iter().map(|m| m.probability).collect(); COMBOS],
        ev_chips: vec![row.actions.iter().map(|m| m.ev_chips).collect(); COMBOS],
        available: vec![true; COMBOS],
        ev_reference: EvReference::DecisionIncrementalVerified,
        source: SourceKind::PokerDataJson,
    }
}

/// T16-R1: a source row admitted by spec 8.2 (`|sum - 1| <= 1e-3`, the wide gate's own formula)
/// whose `legalize_row` output sums above 1 is normalized once and mixes to finite advice, on one
/// branch and beside a stopped branch whose posterior stays unresolved (never renormalized away).
fn assert_admitted_row_mixes(source: &[f32]) {
    let wide: f64 = source.iter().map(|&p| f64::from(p)).sum();
    assert!((wide - 1.0).abs() <= 1e-3, "spec 8.2 admits the source row: sum {wide}");
    let n = legalized_call_node(source);
    let carried = n.probs[0][0];
    assert!(carried > 1.0, "legalize_row's merged Call probability exceeds 1: {carried}");
    let m = mix_nodes(&branches(&[1.0]), &[at(0, Some(n.clone()), "k0")], H, 0, 2);
    assert_eq!(m.unsupported, None);
    let c = advice(&m, CALL);
    assert_eq!(c.frequency, Some(1.0), "the admitted row is normalized to sum 1 before mixing");
    assert_eq!(c.ev_bb, Some(1.0), "the Call's own 2 chips at 2 chips per bb");
    assert_eq!(c.unavailable, None);
    assert_eq!(m.unresolved_mass, 0.0);
    let mix = m.range_mix.as_ref().expect("a node strategy exists");
    assert_eq!(mix.len(), 1);
    close(f64::from(mix[0].1), 1.0);
    // Beside a stopped 0.25 branch: the posterior is not renormalized over the covered branch.
    let mut bs = branches(&[0.75, 0.25]);
    bs[1].stopped = Some("missing node k1".into());
    let m = mix_nodes(&bs, &[at(0, Some(n), "k0"), at(1, None, "k1")], H, 0, 2);
    assert_eq!(m.unsupported, None);
    let c = advice(&m, CALL);
    close(f64::from(c.frequency.unwrap()), 0.75);
    assert_eq!(c.ev_bb, None);
    close(f64::from(covered(c)), 0.75);
    close(f64::from(m.unresolved_mass), 0.25);
    close(frequency_total(&m) + f64::from(m.unresolved_mass), 1.0);
    close(f64::from(m.range_mix.as_ref().unwrap()[0].1), 1.0);
}

#[test]
fn admitted_rounded_halves_moved_into_the_call_mix_without_panicking() {
    // Two 0.5002 probabilities (source sum 1.0004, admitted); the raise moves into the call.
    assert_admitted_row_mixes(&[0.5002, 0.5002]);
}

#[test]
fn exact_decimal_tenths_accumulated_in_f32_mix_without_panicking() {
    // Ten 0.1 probabilities sum to 1 in decimal; f32 accumulation in legalize_row gives 1.0000001.
    assert_admitted_row_mixes(&[0.1; 10]);
}

#[test]
fn a_row_on_the_admission_boundary_survives_its_f32_carrier() {
    // Two 0.5005 weights: spec 8.2's wide (f64) gate admits the sum 1.001, but the f32 values the
    // mapped node carries sum to 1.0010000467 -- outside 1 +- 1e-3 by f32 rounding alone. An
    // admitted row must never abort the recommendation, so the assembly admits it too.
    assert!((0.5005_f64 + 0.5005_f64 - 1.0).abs() <= 1e-3, "spec 8.2 admits the wide row");
    let n = legalized_call_node(&[0.5005, 0.5005]);
    let carried = f64::from(n.probs[0][0]);
    assert!((carried - 1.0).abs() > 1e-3, "the f32 carrier lies just outside 1 +- 1e-3: {carried}");
    let m = mix_nodes(&branches(&[1.0]), &[at(0, Some(n), "k0")], H, 0, 2);
    assert_eq!(m.unsupported, None);
    assert_eq!(advice(&m, CALL).frequency, Some(1.0));
}

#[test]
#[should_panic(expected = "mix_nodes: branch 0's node \"k0\" combo 0 probabilities sum to 1.0011000633239746, outside spec 8.2's 1 +- 1e-3")]
fn a_row_just_above_the_admission_tolerance_is_rejected() {
    // Source sum 1.0011: outside 1 +- 1e-3, so spec 8.2 never admits it; the assembly rejects it
    // with a diagnostic naming the node and the sum instead of normalizing it.
    let n = legalized_call_node(&[0.5, 0.5011]);
    mix_nodes(&branches(&[1.0]), &[at(0, Some(n), "k0")], H, 0, 2);
}

#[test]
#[should_panic(expected = "mix_nodes: branch 0's node \"k0\" combo 0 probabilities sum to 0.9988000392913818, outside spec 8.2's 1 +- 1e-3")]
fn a_row_just_below_the_admission_tolerance_is_rejected() {
    let n = legalized_call_node(&[0.5, 0.4988]);
    mix_nodes(&branches(&[1.0]), &[at(0, Some(n), "k0")], H, 0, 2);
}

#[test]
#[should_panic(expected = "mix_nodes: branch 0's node \"k0\" combo 3 is explicitly unreachable but its probabilities sum to 1, not exactly 0")]
fn an_unreachable_row_that_is_not_all_zero_is_rejected() {
    // Spec 8.2: an unreachable class sums to exactly 0; only that all-zero row is kept as is.
    let mut n = verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(2.0))]);
    n.available[3] = false;
    mix_nodes(&branches(&[1.0]), &[at(0, Some(n), "k0")], H, 0, 2);
}

#[test]
fn a_stopped_branch_never_resumes_at_a_present_node() {
    // Section 9.3: a stopped branch has no preflop node for any later lookup, even if one exists.
    let mut bs = branches(&[0.5, 0.5]);
    bs[1].stopped = Some("missing node kOld".into());
    let nodes = vec![
        at(0, Some(verified(&[(FOLD, 0.5, Some(0.0)), (CALL, 0.5, Some(2.0))])), "k0"),
        at(1, Some(verified(&[(FOLD, 1.0, Some(0.0))])), "k1"),
    ];
    let m = mix_nodes(&bs, &nodes, H, 0, 2);
    close(f64::from(m.unresolved_mass), 0.5);
    close(f64::from(advice(&m, FOLD).frequency.unwrap()), 0.25);
    close(frequency_total(&m) + f64::from(m.unresolved_mass), 1.0);
    assert!(matches!(m.reasons.as_slice(),
        [ApproxReason::BranchResidual { residual_mass_pct, cause, .. }]
            if (*residual_mass_pct - 50.0).abs() < 1e-4 && cause == "missing node k1"), "{:?}", m.reasons);
    assert!(m.notes.iter().any(|n| n == "50.0% of the posterior has no strategy"), "{:?}", m.notes);
}

#[test]
#[should_panic(expected = "mix_action: posterior has 2 entries but probs has 1")]
fn mix_action_rejects_mismatched_lengths() {
    mix_action(&[0.5, 0.5], &[Some(0.5)], &[None, None], true);
}

#[test]
#[should_panic(expected = "mix_action: probs[1] = 1.5 is not a probability in [0, 1]")]
fn mix_action_rejects_an_out_of_range_probability() {
    mix_action(&[0.5, 0.5], &[Some(0.5), Some(1.5)], &[None, None], true);
}

#[test]
#[should_panic(expected = "mix_action: evs[0] = NaN is not finite")]
fn mix_action_rejects_a_non_finite_ev() {
    mix_action(&[1.0], &[Some(0.5)], &[Some(f64::NAN)], true);
}

#[test]
#[should_panic(expected = "mix_action: evs[1] is present for an action absent from branch 1")]
fn mix_action_rejects_an_ev_without_its_action() {
    mix_action(&[0.5, 0.5], &[Some(0.5), None], &[Some(1.0), Some(2.0)], true);
}

#[test]
#[should_panic(expected = "mix_action: posterior sums to 0.5, not 1")]
fn mix_action_rejects_a_posterior_that_is_not_a_distribution() {
    mix_action(&[0.25, 0.25], &[Some(0.5), Some(0.5)], &[None, None], true);
}

#[test]
#[should_panic(expected = "mix_nodes: node for branch 9 names no branch in the list")]
fn mix_nodes_rejects_an_unknown_branch_id() {
    mix_nodes(&branches(&[1.0]), &[at(9, None, "k9")], H, 0, 2);
}

#[test]
#[should_panic(expected = "mix_nodes: branch 0 has more than one node entry")]
fn mix_nodes_rejects_a_duplicate_branch_entry() {
    mix_nodes(&branches(&[1.0]), &[at(0, None, "k0"), at(0, None, "k0")], H, 0, 2);
}

#[test]
#[should_panic(expected = "mix_nodes: branch 0's node is seat Seat(0)'s, not hero Seat(1)'s")]
fn mix_nodes_rejects_another_seats_node() {
    let mut n = verified(&[(FOLD, 1.0, Some(0.0))]);
    n.actor = V;
    mix_nodes(&branches(&[1.0]), &[at(0, Some(n), "k0")], H, 0, 2);
}

#[test]
#[should_panic(expected = "mix_nodes: branch 0's node lists Call twice (actions 0 and 1)")]
fn mix_nodes_rejects_an_unmapped_duplicate_action() {
    let n = verified(&[(CALL, 0.5, Some(1.0)), (CALL, 0.5, Some(1.0))]);
    mix_nodes(&branches(&[1.0]), &[at(0, Some(n), "k0")], H, 0, 2);
}

#[test]
#[should_panic(expected = "mix_nodes: bb_chips is 0")]
fn mix_nodes_rejects_a_zero_big_blind() {
    mix_nodes(&branches(&[1.0]), &[at(0, None, "k0")], H, 0, 0);
}
