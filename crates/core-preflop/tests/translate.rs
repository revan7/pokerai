//! P3.T10 -- wager interpolation (spec section 8.4's pseudo-harmonic formula) and legality-after-
//! mapping (destination choice, menu merging, EV/probability conservation) once a chip-domain
//! `ExpandedNode` (P3.T9) is checked against the live tree's actual `LegalAction`s.

use proto::{Action, LegalAction, Unavailable, UnsupportedReason};

// The exact test from the task brief (spec section 13.1 / section 8.4 boundaries).
#[test]
fn bet_translation_boundaries() {
    let t = core_preflop::interpolate(0.73, &[(0, 0.5), (1, 1.)]).unwrap();
    assert!((t.choices[0].1 - 81.0 / 173.0).abs() < 1e-12);
    assert!((t.deviation - 0.23).abs() < 1e-12);
    assert!(t.deviation > 0.10);
    assert_eq!(core_preflop::interpolate(0.2, &[(0, 0.5), (1, 1.)]).unwrap().choices, vec![(0, 1.)]);
    assert_eq!(core_preflop::interpolate(2., &[(0, 0.5), (1, 1.)]).unwrap().choices, vec![(1, 1.)]);
    assert_eq!(core_preflop::interpolate(0.73, &[(0, 0.5)]).unwrap().choices, vec![(0, 1.)]);
    assert_eq!(core_preflop::interpolate(0.5, &[(0, 0.5), (1, 1.)]).unwrap().choices, vec![(0, 1.)]);
    let jam = core_preflop::interpolate(1.5, &[(0, 0.5), (1, 1.), (2, 2.)]).unwrap();
    assert_eq!(jam.choices, vec![(1, 0.4), (2, 0.6)]);
}

#[test]
fn interpolate_flags_exact_match_as_unclamped_zero_deviation() {
    let t = core_preflop::interpolate(1.0, &[(0, 0.5), (1, 1.)]).unwrap();
    assert_eq!(t.choices, vec![(1, 1.0)]);
    assert_eq!(t.deviation, 0.0);
    assert!(!t.clamped);
}

#[test]
fn interpolate_below_smallest_is_clamped() {
    let t = core_preflop::interpolate(0.2, &[(0, 0.5), (1, 1.)]).unwrap();
    assert!(t.clamped);
    assert!((t.deviation - 0.3).abs() < 1e-12);
}

#[test]
fn interpolate_above_largest_is_clamped() {
    let t = core_preflop::interpolate(2., &[(0, 0.5), (1, 1.)]).unwrap();
    assert!(t.clamped);
    assert!((t.deviation - 1.0).abs() < 1e-12);
}

#[test]
fn interpolate_between_sizes_is_never_clamped() {
    let t = core_preflop::interpolate(0.73, &[(0, 0.5), (1, 1.)]).unwrap();
    assert!(!t.clamped);
}

#[test]
fn interpolate_rejects_out_of_domain_input() {
    assert!(core_preflop::interpolate(-0.1, &[(0, 0.5), (1, 1.)]).is_none(), "negative s");
    assert!(core_preflop::interpolate(f64::NAN, &[(0, 0.5), (1, 1.)]).is_none(), "non-finite s");
    assert!(core_preflop::interpolate(f64::INFINITY, &[(0, 0.5), (1, 1.)]).is_none(), "non-finite s");
    assert!(core_preflop::interpolate(0.5, &[]).is_none(), "empty menu");
    assert!(core_preflop::interpolate(0.5, &[(0, -0.1), (1, 1.)]).is_none(), "negative menu size");
    assert!(core_preflop::interpolate(0.5, &[(0, f64::NAN)]).is_none(), "non-finite menu size");
}

// --- R6 (fix round 1): accepted finite inputs never yield NaN or out-of-range weights ---

/// Every accepted interpolation is a probability split: each weight finite and in `[0, 1]`, the
/// weights summing to 1, the deviation finite.
fn assert_probability_split(t: &core_preflop::Interpolation) {
    assert!(t.choices.iter().all(|&(_, f)| f.is_finite() && (0.0..=1.0).contains(&f)), "weights {:?}", t.choices);
    let total: f64 = t.choices.iter().map(|&(_, f)| f).sum();
    assert!((total - 1.0).abs() < 1e-12, "weights {:?} sum to {total}", t.choices);
    assert!(t.deviation.is_finite(), "deviation {}", t.deviation);
}

#[test]
fn interpolate_large_finite_sizes_give_finite_weights() {
    // The review's exact repro: both products of the unfactored formula,
    // (B - s)(1 + A) and (B - A)(1 + s), overflow to infinity, and inf / inf is NaN.
    let t = core_preflop::interpolate(1.5e200, &[(0, 1e200), (1, 2e200)]).unwrap();
    assert_probability_split(&t);
    assert!(!t.clamped);
    assert_eq!((t.choices[0].0, t.choices[1].0), (0, 1));
    // f_A = ((B - s) / (B - A)) * ((1 + A) / (1 + s)) = 0.5 * (1e200 / 1.5e200) = 1/3.
    assert!((t.choices[0].1 - 1.0 / 3.0).abs() < 1e-12, "f_A = {}", t.choices[0].1);
}

#[test]
fn interpolate_at_the_top_of_the_f64_range_gives_finite_weights() {
    let a = f64::MAX / 2.0;
    let b = f64::MAX;
    let s = a + (b - a) / 2.0;
    let t = core_preflop::interpolate(s, &[(0, a), (1, b)]).unwrap();
    assert_probability_split(&t);
    // (B - s) / (B - A) = 1/2 and (1 + A) / (1 + s) = (MAX / 2) / (3 MAX / 4) = 2/3.
    assert!((t.choices[0].1 - 1.0 / 3.0).abs() < 1e-12, "f_A = {}", t.choices[0].1);
    // An exact hit on the largest representable size is still an exact match, not a clamp.
    let top = core_preflop::interpolate(b, &[(0, a), (1, b)]).unwrap();
    assert_eq!(top.choices, vec![(1, 1.0)]);
    assert_eq!(top.deviation, 0.0);
    assert!(!top.clamped);
}

#[test]
fn interpolate_across_a_two_ulp_interval_gives_finite_weights() {
    // The narrowest bracket that can hold a distinct `s`: A and B two ulps apart, s between them.
    let a = 1.0f64;
    let s = a.next_up();
    let b = s.next_up();
    let t = core_preflop::interpolate(s, &[(0, a), (1, b)]).unwrap();
    assert_probability_split(&t);
    assert!((t.choices[0].1 - 0.5).abs() < 1e-12, "f_A = {}", t.choices[0].1);
}

#[test]
fn interpolate_across_a_subnormal_interval_gives_finite_weights() {
    // A = 0 and B = two subnormal quanta: B - A and B - s are themselves subnormal.
    let s = f64::from_bits(1);
    let b = f64::from_bits(2);
    let t = core_preflop::interpolate(s, &[(0, 0.0), (1, b)]).unwrap();
    assert_probability_split(&t);
    assert!((t.choices[0].1 - 0.5).abs() < 1e-12, "f_A = {}", t.choices[0].1);
}

// --- wager_fraction (spec section 8.4: pot fraction at the parent node) ---

#[test]
fn wager_fraction_matches_the_boundary_test_size() {
    // own = 0, call = 0 (an opening bet after checks around), pot = 100, to = 73: s = 0.73,
    // exactly the size `bet_translation_boundaries` interpolates against.
    let s = core_preflop::wager_fraction(73, 0, 0, 100).unwrap();
    assert!((s - 0.73).abs() < 1e-12);
}

#[test]
fn wager_fraction_raise_to_uses_pot_plus_call_denominator() {
    // A raise-to: own = 20 (already committed this street), call = 30 (owed to match the
    // current wager), pot = 150 (before this actor's call is added). s = (to-own-call)/(pot+call).
    let s = core_preflop::wager_fraction(140, 20, 30, 150).unwrap();
    assert!((s - (140.0 - 20.0 - 30.0) / (150.0 + 30.0)).abs() < 1e-12);
}

// R4 (fix round 1): an input with no pot fraction is a recoverable translation-domain mismatch,
// reported as `None` so the caller can take its unmappable-branch path -- never a panic.

#[test]
fn wager_fraction_rejects_zero_pot_and_call() {
    assert_eq!(core_preflop::wager_fraction(0, 0, 0, 0), None);
    assert_eq!(core_preflop::wager_fraction(10, 0, 0, 0), None);
}

#[test]
fn wager_fraction_rejects_a_wager_below_a_call() {
    // to (5) is less than own (10) + call (0): not a wager at all.
    assert_eq!(core_preflop::wager_fraction(5, 10, 0, 100), None);
}

#[test]
fn wager_fraction_of_exactly_a_call_is_zero() {
    // to == own + call adds nothing beyond the call: the smallest valid fraction, not an error.
    assert_eq!(core_preflop::wager_fraction(30, 10, 20, 100), Some(0.0));
}

#[test]
fn wager_fraction_returns_none_for_a_mapped_parent_mismatch() {
    // The review's repro. Blinds 1/2, deep stacks: an observed open to 7 splits onto a source open
    // to 20. The next actor (own 0) actually re-raised to 12, the legal minimum over 7; on the
    // source-20 branch its mapped parent has it facing a call of 20 into a pot of 1 + 2 + 20 = 23.
    // An observed amount below the mapped call is a legal observed history that this branch cannot
    // express, so the fraction is `None`, never a panic.
    assert_eq!(core_preflop::wager_fraction(12, 0, 20, 23), None);
}

#[test]
fn wager_fraction_at_the_u32_extremes_is_finite() {
    let s = core_preflop::wager_fraction(u32::MAX, 0, 0, 1).unwrap();
    assert!(s.is_finite());
    assert_eq!(s, u32::MAX as f64);
    let s = core_preflop::wager_fraction(u32::MAX, 0, u32::MAX, u32::MAX).unwrap();
    assert_eq!(s, 0.0);
}

#[test]
fn menu_fractions_returns_none_when_an_actual_stack_all_in_is_below_the_mapped_call() {
    // A source AllIn carries the actor's ACTUAL maximum (P3.T9's `actor_max_to`), while `own`/
    // `call`/`pot` come from the mapped parent's source money: with an actual stack of 15 facing a
    // mapped call of 20, the AllIn is below the call and has no pot fraction at this parent. The
    // whole menu conversion fails rather than silently dropping that size (which would change the
    // interpolation bracket) or panicking.
    let actions = vec![Action::Fold, Action::Call, Action::Raise { to: 60 }, Action::AllIn { to: 15 }];
    assert_eq!(core_preflop::menu_fractions(&actions, 0, 20, 23), None);
    // The same menu at a zero pot and zero call has no fractions either.
    assert_eq!(core_preflop::menu_fractions(&[Action::Bet { to: 4 }], 0, 0, 0), None);
}

// --- menu_fractions: a source node's own wager-sized actions, as pot fractions ---

#[test]
fn menu_fractions_skips_non_wager_actions_and_keeps_original_indices() {
    let actions = vec![Action::Fold, Action::Check, Action::Bet { to: 50 }, Action::Bet { to: 100 }];
    let m = core_preflop::menu_fractions(&actions, 0, 0, 100).unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].0, 2);
    assert!((m[0].1 - 0.5).abs() < 1e-12);
    assert_eq!(m[1].0, 3);
    assert!((m[1].1 - 1.0).abs() < 1e-12);
}

#[test]
fn menu_fractions_includes_all_in_sizes() {
    let actions = vec![Action::Fold, Action::Call, Action::Raise { to: 250 }, Action::AllIn { to: 400 }];
    // own = 20, call = 30, pot = 150 (matches the raise_to test above).
    let m = core_preflop::menu_fractions(&actions, 20, 30, 150).unwrap();
    assert_eq!(m, vec![
        (2, core_preflop::wager_fraction(250, 20, 30, 150).unwrap()),
        (3, core_preflop::wager_fraction(400, 20, 30, 150).unwrap()),
    ]);
}

#[test]
fn menu_fractions_of_an_all_fold_check_menu_is_empty() {
    let actions = vec![Action::Fold, Action::Check];
    assert_eq!(core_preflop::menu_fractions(&actions, 0, 0, 100), Some(vec![]));
}

// --- destination_map / legalize_row: legality-after-mapping (spec section 8.4) ---

fn total_probability(advice: &core_preflop::MappedAdvice) -> f32 {
    advice.actions.iter().map(|a| a.probability).sum()
}

#[test]
fn destination_map_orders_the_menu_fold_check_call_wagers_allin() {
    // Scrambled input order; every action is directly legal.
    let actions = vec![Action::Raise { to: 50 }, Action::AllIn { to: 200 }, Action::Call, Action::Fold];
    let legal =
        vec![LegalAction::Fold, LegalAction::Call { cost: 10 }, LegalAction::Raise { min_to: 20, max_to: 200 }, LegalAction::AllIn { to: 200 }];
    let (menu, map) = core_preflop::destination_map(&actions, &legal).unwrap();
    assert_eq!(menu, vec![Action::Fold, Action::Call, Action::Raise { to: 50 }, Action::AllIn { to: 200 }]);
    // Every source action is legal on its own, so every destination is not `created`.
    assert!(map.iter().all(|d| !d.created));
    // source index 0 (Raise{50}) maps to menu index 2, etc.
    assert_eq!(map[0].index, 2);
    assert_eq!(map[1].index, 3);
    assert_eq!(map[2].index, 1);
    assert_eq!(map[3].index, 0);
}

#[test]
fn below_min_raise_maps_to_smallest_legal_menu_raise_preserving_its_ev() {
    // Source offers two raise sizes: 7 (below the live min-raise of 10) and 12 (legal). Raise(7)
    // has no EV of its own (it was never a real menu size at this table), so merging its mass
    // into Raise(12) must not disturb Raise(12)'s own recorded EV.
    let actions = vec![Action::Fold, Action::Call, Action::Raise { to: 7 }, Action::Raise { to: 12 }];
    let legal =
        vec![LegalAction::Fold, LegalAction::Call { cost: 10 }, LegalAction::Raise { min_to: 10, max_to: 200 }, LegalAction::AllIn { to: 200 }];
    let (menu, map) = core_preflop::destination_map(&actions, &legal).unwrap();
    assert_eq!(menu, vec![Action::Fold, Action::Call, Action::Raise { to: 12 }]);
    assert!(!map[2].created, "Raise(7) moves onto an already-legal destination, not a manufactured one");
    assert_eq!(map[2].index, 2);
    assert_eq!(map[3].index, 2);

    let probs = vec![0.1f32, 0.2, 0.15, 0.55];
    let evs = vec![Some(0.0), Some(-3.0), None, Some(5.0)];
    let mut notes = Vec::new();
    let advice = core_preflop::legalize_row(&menu, &map, &probs, &evs, &mut notes);
    let raise12 = &advice.actions[2];
    assert_eq!(raise12.action, Action::Raise { to: 12 });
    assert!((raise12.probability - (0.15 + 0.55)).abs() < 1e-6, "Raise(7)'s mass merges into Raise(12)");
    assert_eq!(raise12.ev_chips, Some(5.0), "Raise(12)'s own EV survives the merge");
    assert!(raise12.unavailable.is_none());
    assert!((total_probability(&advice) - probs.iter().sum::<f32>()).abs() < 1e-6);
}

#[test]
fn below_min_raise_with_no_legal_source_raise_maps_to_call() {
    // Only one source raise size (7), and it is below the live min-raise (20): no other source
    // Raise is legal to fall back to, so the mass moves to Call instead.
    let actions = vec![Action::Fold, Action::Call, Action::Raise { to: 7 }];
    let legal =
        vec![LegalAction::Fold, LegalAction::Call { cost: 10 }, LegalAction::Raise { min_to: 20, max_to: 200 }, LegalAction::AllIn { to: 200 }];
    let (menu, map) = core_preflop::destination_map(&actions, &legal).unwrap();
    assert_eq!(menu, vec![Action::Fold, Action::Call]);
    assert_eq!(map[2].index, 1);
    assert!(!map[2].created, "Call was already a legal destination on its own");

    let probs = vec![0.3f32, 0.3, 0.4];
    let evs = vec![Some(0.0), Some(-1.0), Some(9.0)];
    let mut notes = Vec::new();
    let advice = core_preflop::legalize_row(&menu, &map, &probs, &evs, &mut notes);
    assert!((advice.actions[1].probability - 0.7).abs() < 1e-6);
    assert!((total_probability(&advice) - 1.0).abs() < 1e-6, "no probability is dropped or invented");
}

#[test]
fn oversized_bet_creates_all_in_with_no_ev() {
    let actions = vec![Action::Check, Action::Bet { to: 120 }];
    let legal = vec![LegalAction::Check, LegalAction::Bet { min_to: 2, max_to: 100 }, LegalAction::AllIn { to: 100 }];
    let (menu, map) = core_preflop::destination_map(&actions, &legal).unwrap();
    assert_eq!(menu, vec![Action::Check, Action::AllIn { to: 100 }]);
    assert!(map[1].created, "AllIn(100) exists on the menu only because Bet(120) was moved there");

    let probs = vec![0.4f32, 0.6];
    let evs = vec![Some(0.0), Some(3.5)];
    let mut notes = Vec::new();
    let advice = core_preflop::legalize_row(&menu, &map, &probs, &evs, &mut notes);
    let allin = &advice.actions[1];
    assert_eq!(allin.ev_chips, None, "a manufactured destination never inherits a moved source's EV");
    // R3 (fix round 1): `MovedProbability::from` names the ORIGINAL source action, Bet(120), never
    // the destination it was moved to.
    assert_eq!(allin.unavailable, Some(Unavailable::MovedProbability { from: Action::Bet { to: 120 } }));
    assert!((allin.probability - 0.6).abs() < 1e-6);
    assert!(advice.notes.iter().any(|n| n.contains("Moved")));
}

// --- R1/R2/R3 (fix round 1): EV ownership, creation and move provenance ---

/// The legal actions at a facing-a-raise node: min re-raise to 10, stack allows up to 200.
fn facing_legal() -> Vec<LegalAction> {
    vec![LegalAction::Fold, LegalAction::Call { cost: 2 }, LegalAction::Raise { min_to: 10, max_to: 200 }, LegalAction::AllIn { to: 200 }]
}

/// The legal actions at an unopened node whose stack caps every wager at 100.
fn opening_legal() -> Vec<LegalAction> {
    vec![LegalAction::Check, LegalAction::Bet { min_to: 2, max_to: 100 }, LegalAction::AllIn { to: 100 }]
}

/// `destination_map` then `legalize_row` for one row, with a fresh notes buffer.
fn legalize(
    actions: &[Action],
    legal: &[LegalAction],
    probs: &[f32],
    evs: &[Option<f32>],
) -> (Vec<Action>, Vec<core_preflop::Destination>, core_preflop::MappedAdvice) {
    let (menu, map) = core_preflop::destination_map(actions, legal).unwrap();
    let mut notes = Vec::new();
    let advice = core_preflop::legalize_row(&menu, &map, probs, evs, &mut notes);
    (menu, map, advice)
}

fn find<'a>(advice: &'a core_preflop::MappedAdvice, action: Action) -> &'a core_preflop::MappedAction {
    advice.actions.iter().find(|a| a.action == action).unwrap_or_else(|| panic!("{action:?} not on {:?}", advice.actions))
}

#[test]
fn moved_source_ev_never_displaces_the_legal_destinations_own_ev() {
    // R1: Raise(7) is below the live minimum of 10 and moves onto the legal source Raise(12).
    // Raise(12) owns its destination and keeps ITS EV (5), in either source order; Raise(7)'s EV
    // (9) is never a candidate and never makes the payoff look ambiguous.
    for (actions, probs, evs) in [
        (vec![Action::Raise { to: 7 }, Action::Raise { to: 12 }], vec![0.4f32, 0.6], vec![Some(9.0f32), Some(5.0)]),
        (vec![Action::Raise { to: 12 }, Action::Raise { to: 7 }], vec![0.6f32, 0.4], vec![Some(5.0f32), Some(9.0)]),
    ] {
        let (menu, _, advice) = legalize(&actions, &facing_legal(), &probs, &evs);
        assert_eq!(menu, vec![Action::Raise { to: 12 }], "order {actions:?}");
        let r12 = find(&advice, Action::Raise { to: 12 });
        assert_eq!(r12.ev_chips, Some(5.0), "order {actions:?}");
        assert_eq!(r12.unavailable, None, "order {actions:?}");
        assert!((r12.probability - 1.0).abs() < 1e-6, "order {actions:?}");
    }
}

#[test]
fn legal_destination_without_an_ev_never_borrows_a_moved_sources_ev() {
    // R1: the owner, Raise(12), has no EV; the moved Raise(7) has one. The missing EV stays
    // missing in either source order -- it is never filled in from the moved source.
    for (actions, probs, evs) in [
        (vec![Action::Raise { to: 12 }, Action::Raise { to: 7 }], vec![0.6f32, 0.4], vec![None, Some(9.0f32)]),
        (vec![Action::Raise { to: 7 }, Action::Raise { to: 12 }], vec![0.4f32, 0.6], vec![Some(9.0f32), None]),
    ] {
        let (_, _, advice) = legalize(&actions, &facing_legal(), &probs, &evs);
        let r12 = find(&advice, Action::Raise { to: 12 });
        assert_eq!(r12.ev_chips, None, "order {actions:?}");
        assert_eq!(r12.unavailable, None, "an owned destination is not a moved one; order {actions:?}");
    }
}

#[test]
fn oversized_bet_moving_onto_an_existing_source_all_in_keeps_the_all_ins_ev() {
    // R1: Bet(120) exceeds the stack and moves onto AllIn(100), which is itself a legal source
    // action with EV 7: AllIn keeps 7 and absorbs Bet(120)'s mass, in either source order.
    for (actions, probs, evs) in [
        (vec![Action::Check, Action::AllIn { to: 100 }, Action::Bet { to: 120 }], vec![0.2f32, 0.5, 0.3], vec![Some(0.0f32), Some(7.0), Some(3.0)]),
        (vec![Action::Check, Action::Bet { to: 120 }, Action::AllIn { to: 100 }], vec![0.2f32, 0.3, 0.5], vec![Some(0.0f32), Some(3.0), Some(7.0)]),
    ] {
        let (menu, map, advice) = legalize(&actions, &opening_legal(), &probs, &evs);
        assert_eq!(menu, vec![Action::Check, Action::AllIn { to: 100 }], "order {actions:?}");
        assert!(map.iter().all(|d| !d.created), "AllIn(100) was a legal source action; order {actions:?}");
        let allin = find(&advice, Action::AllIn { to: 100 });
        assert_eq!(allin.ev_chips, Some(7.0), "order {actions:?}");
        assert_eq!(allin.unavailable, None, "order {actions:?}");
        assert!((allin.probability - 0.8).abs() < 1e-6, "order {actions:?}");
        assert!((total_probability(&advice) - 1.0).abs() < 1e-6);
    }
}

#[test]
fn two_oversized_wagers_create_all_in_with_no_ev() {
    // R2: neither Bet(120) nor Bet(140) is AllIn(100); the second move onto the destination the
    // first one created must not make it look source-owned. Equal and differing source EVs alike.
    for evs in [vec![Some(0.0f32), Some(3.5), Some(3.5)], vec![Some(0.0f32), Some(3.5), Some(4.0)]] {
        let actions = vec![Action::Check, Action::Bet { to: 120 }, Action::Bet { to: 140 }];
        let probs = vec![0.2f32, 0.5, 0.3];
        let (menu, map, advice) = legalize(&actions, &opening_legal(), &probs, &evs);
        assert_eq!(menu, vec![Action::Check, Action::AllIn { to: 100 }]);
        assert!(!map[0].created);
        assert!(map[1].created && map[2].created, "both moves land on a created destination: {map:?}");
        let allin = find(&advice, Action::AllIn { to: 100 });
        assert_eq!(allin.ev_chips, None, "evs {evs:?}");
        assert_eq!(allin.unavailable, Some(Unavailable::MovedProbability { from: Action::Bet { to: 120 } }), "evs {evs:?}");
        assert!((allin.probability - 0.8).abs() < 1e-6);
        assert!((total_probability(&advice) - 1.0).abs() < 1e-6);
    }
}

#[test]
fn two_below_min_raises_create_a_previously_absent_call_with_no_ev() {
    // R2: the source has no Call and no legal raise; Raise(7) and Raise(8) are both below the live
    // minimum of 10, so both move to Call, which exists only because of the moves.
    for evs in [vec![Some(0.0f32), Some(2.0), Some(2.0)], vec![Some(0.0f32), Some(2.0), Some(-1.0)]] {
        let actions = vec![Action::Fold, Action::Raise { to: 7 }, Action::Raise { to: 8 }];
        let probs = vec![0.5f32, 0.25, 0.25];
        let (menu, map, advice) = legalize(&actions, &facing_legal(), &probs, &evs);
        assert_eq!(menu, vec![Action::Fold, Action::Call]);
        assert_eq!((map[1].index, map[2].index), (1, 1));
        assert!(map[1].created && map[2].created, "Call exists only because of the moves: {map:?}");
        let call = find(&advice, Action::Call);
        assert_eq!(call.ev_chips, None, "evs {evs:?}");
        assert_eq!(call.unavailable, Some(Unavailable::MovedProbability { from: Action::Raise { to: 7 } }), "evs {evs:?}");
        assert!((call.probability - 0.5).abs() < 1e-6);
        assert_eq!(find(&advice, Action::Fold).ev_chips, Some(0.0));
    }
}

#[test]
fn identical_rounded_sources_keep_an_ev_only_when_it_is_unique() {
    // R1: two legal sources that round to the same Raise(33) co-own it. Equal EVs are one unique
    // payoff and are kept; a missing EV on either co-owner makes the payoff ambiguous in EITHER
    // order (never "first visited wins"), so it is omitted with a collision note.
    let actions = vec![Action::Raise { to: 33 }, Action::Raise { to: 33 }];
    let probs = vec![0.5f32, 0.5];
    let (_, _, kept) = legalize(&actions, &facing_legal(), &probs, &[Some(1.5), Some(1.5)]);
    assert_eq!(kept.actions[0].ev_chips, Some(1.5));
    for evs in [[Some(1.5f32), None], [None, Some(1.5f32)]] {
        let (_, _, advice) = legalize(&actions, &facing_legal(), &probs, &evs);
        assert_eq!(advice.actions[0].ev_chips, None, "evs {evs:?}");
        assert!(advice.notes.iter().any(|n| n.contains("collide") && n.contains("EV omitted")), "evs {evs:?}: {:?}", advice.notes);
    }
}

#[test]
fn destination_map_carries_each_source_actions_provenance() {
    // R1/R3: the map records, per source action, the ORIGINAL action and whether it was moved, so
    // ownership (unmoved sources only) and provenance survive the menu reordering.
    let actions = vec![Action::Bet { to: 120 }, Action::Check, Action::Bet { to: 50 }];
    let (menu, map) = core_preflop::destination_map(&actions, &opening_legal()).unwrap();
    assert_eq!(menu, vec![Action::Check, Action::Bet { to: 50 }, Action::AllIn { to: 100 }]);
    assert_eq!(map, vec![
        core_preflop::Destination { index: 2, created: true, source: Action::Bet { to: 120 }, moved: true },
        core_preflop::Destination { index: 0, created: false, source: Action::Check, moved: false },
        core_preflop::Destination { index: 1, created: false, source: Action::Bet { to: 50 }, moved: false },
    ]);
}

#[test]
fn move_notes_name_the_original_source_its_probability_and_its_destination() {
    // R3: exactly one note for the one actual move; Check, which maps to itself, gets none.
    let (_, _, advice) = legalize(&[Action::Check, Action::Bet { to: 120 }], &opening_legal(), &[0.4, 0.6], &[Some(0.0), Some(3.5)]);
    assert_eq!(advice.notes, vec!["Moved Bet { to: 120 } probability 0.6 to AllIn { to: 100 }".to_string()]);
    // A move onto an owned destination is disclosed the same way.
    let (_, _, advice) =
        legalize(&[Action::Call, Action::Raise { to: 7 }, Action::Raise { to: 12 }], &facing_legal(), &[0.2, 0.3, 0.5], &[None, None, None]);
    assert_eq!(advice.notes, vec!["Moved Raise { to: 7 } probability 0.3 to Raise { to: 12 }".to_string()]);
}

#[test]
fn existing_source_all_in_retains_its_own_ev() {
    let actions = vec![Action::Check, Action::Bet { to: 80 }, Action::AllIn { to: 100 }];
    let legal = vec![LegalAction::Check, LegalAction::Bet { min_to: 2, max_to: 100 }, LegalAction::AllIn { to: 100 }];
    let (menu, map) = core_preflop::destination_map(&actions, &legal).unwrap();
    assert_eq!(menu, vec![Action::Check, Action::Bet { to: 80 }, Action::AllIn { to: 100 }]);
    assert!(!map[2].created, "AllIn(100) was already a legal source action");

    let probs = vec![0.2f32, 0.3, 0.5];
    let evs = vec![Some(0.0), Some(1.0), Some(7.0)];
    let mut notes = Vec::new();
    let advice = core_preflop::legalize_row(&menu, &map, &probs, &evs, &mut notes);
    assert_eq!(advice.actions[2].ev_chips, Some(7.0));
    assert!((advice.actions[2].probability - 0.5).abs() < 1e-6);
}

#[test]
fn two_half_up_rounded_wagers_collide_and_omit_ev() {
    // Two distinct source sizes both round (half up, P3.T9) onto the same live chip amount; both
    // are independently legal, so they collide purely through `destination_map`'s own dedup, with
    // no illegal-history remapping involved at all.
    let actions = vec![Action::Fold, Action::Call, Action::Raise { to: 33 }, Action::Raise { to: 33 }];
    let legal =
        vec![LegalAction::Fold, LegalAction::Call { cost: 10 }, LegalAction::Raise { min_to: 10, max_to: 200 }, LegalAction::AllIn { to: 200 }];
    let (menu, map) = core_preflop::destination_map(&actions, &legal).unwrap();
    assert_eq!(menu, vec![Action::Fold, Action::Call, Action::Raise { to: 33 }]);
    assert!(!map[2].created && !map[3].created);
    assert_eq!(map[2].index, 2);
    assert_eq!(map[3].index, 2);

    let probs = vec![0.1f32, 0.2, 0.35, 0.35];
    let evs = vec![Some(0.0), Some(-1.0), Some(1.0), Some(2.0)];
    let mut notes = Vec::new();
    let advice = core_preflop::legalize_row(&menu, &map, &probs, &evs, &mut notes);
    let raise33 = &advice.actions[2];
    assert_eq!(raise33.ev_chips, None, "two colliding sources have no unique payoff");
    assert!((raise33.probability - 0.7).abs() < 1e-6);
    // `legalize_row` moves its accumulated notes into `advice.notes` (`mem::take`), leaving the
    // caller's `notes` buffer empty -- assert against the returned advice, not the emptied buffer.
    assert!(advice.notes.iter().any(|n| n.contains("collide") && n.contains("EV omitted")));
    assert!((total_probability(&advice) - 1.0).abs() < 1e-6);
}

#[test]
fn no_legal_destination_is_reported_as_unsupported_history_never_a_guessed_fold() {
    // Bet(1) is below the live opening-bet minimum (2); no fallback rule applies to a Bet (only
    // Raise has a below-min fallback per section 8.4), and it is not above the max either.
    let actions = vec![Action::Check, Action::Bet { to: 1 }];
    let legal = vec![LegalAction::Check, LegalAction::Bet { min_to: 2, max_to: 100 }, LegalAction::AllIn { to: 100 }];
    let err = core_preflop::destination_map(&actions, &legal).unwrap_err();
    match err {
        UnsupportedReason::UnsupportedHistory { reason } => assert!(reason.contains("Bet")),
        other => panic!("expected UnsupportedHistory, got {other:?}"),
    }
}

#[test]
fn short_stack_raise_at_exactly_the_shove_amount_maps_to_all_in() {
    // The live stack is too short for any partial raise (only AllIn is legal), and the source
    // materializes a Raise landing exactly on the shove amount: it must still resolve to AllIn,
    // not fall through as unsupported.
    let actions = vec![Action::Fold, Action::Call, Action::Raise { to: 40 }];
    let legal = vec![LegalAction::Fold, LegalAction::Call { cost: 10 }, LegalAction::AllIn { to: 40 }];
    let (menu, map) = core_preflop::destination_map(&actions, &legal).unwrap();
    assert_eq!(menu, vec![Action::Fold, Action::Call, Action::AllIn { to: 40 }]);
    assert!(map[2].created);
    assert_eq!(map[2].index, 2);
}

#[test]
fn short_stack_raise_below_the_shove_amount_maps_to_call() {
    let actions = vec![Action::Fold, Action::Call, Action::Raise { to: 25 }];
    let legal = vec![LegalAction::Fold, LegalAction::Call { cost: 10 }, LegalAction::AllIn { to: 40 }];
    let (menu, map) = core_preflop::destination_map(&actions, &legal).unwrap();
    assert_eq!(menu, vec![Action::Fold, Action::Call]);
    assert_eq!(map[2].index, 1);
}

// --- R5 (fix round 1): the row/map/menu relationship and the value domains are enforced ---

fn dest(index: usize, created: bool, source: Action, moved: bool) -> core_preflop::Destination {
    core_preflop::Destination { index, created, source, moved }
}

/// A one-entry Check menu and its identity map.
fn check_only() -> (Vec<Action>, Vec<core_preflop::Destination>) {
    (vec![Action::Check], vec![dest(0, false, Action::Check, false)])
}

#[test]
#[should_panic(expected = "probs has length 2 but the destination map has length 1")]
fn legalize_row_rejects_a_probability_row_longer_than_the_map() {
    // The review's repro: without the check this returned "supported" advice summing to 0.25.
    let (menu, map) = check_only();
    core_preflop::legalize_row(&menu, &map, &[0.25, 0.75], &[None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "probs has length 1 but the destination map has length 2")]
fn legalize_row_rejects_a_probability_row_shorter_than_the_map() {
    let (menu, map) = core_preflop::destination_map(&[Action::Check, Action::Bet { to: 50 }], &opening_legal()).unwrap();
    core_preflop::legalize_row(&menu, &map, &[1.0], &[None, None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "evs has length 1 but the destination map has length 2")]
fn legalize_row_rejects_an_ev_row_of_the_wrong_length() {
    let (menu, map) = core_preflop::destination_map(&[Action::Check, Action::Bet { to: 50 }], &opening_legal()).unwrap();
    core_preflop::legalize_row(&menu, &map, &[0.5, 0.5], &[None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "map[1].index 5 is out of range for a 2-action menu")]
fn legalize_row_rejects_an_out_of_range_destination_index() {
    let menu = vec![Action::Check, Action::Bet { to: 50 }];
    let map = vec![dest(0, false, Action::Check, false), dest(5, false, Action::Bet { to: 50 }, false)];
    core_preflop::legalize_row(&menu, &map, &[0.5, 0.5], &[None, None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "menu[1] Bet { to: 50 } has no source action mapped to it")]
fn legalize_row_rejects_a_menu_entry_no_source_maps_to() {
    let (_, map) = check_only();
    core_preflop::legalize_row(&[Action::Check, Action::Bet { to: 50 }], &map, &[1.0], &[None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "menu[0] and menu[1] are both Check")]
fn legalize_row_rejects_a_duplicated_menu_entry() {
    let menu = vec![Action::Check, Action::Check];
    let map = vec![dest(0, false, Action::Check, false), dest(1, false, Action::Check, false)];
    core_preflop::legalize_row(&menu, &map, &[0.5, 0.5], &[None, None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "map[0] source Raise { to: 7 } -> menu[0] Call has moved = false")]
fn legalize_row_rejects_a_moved_source_marked_unmoved() {
    // Marked unmoved, Raise(7) would own Call and hand it Raise(7)'s EV.
    core_preflop::legalize_row(&[Action::Call], &[dest(0, false, Action::Raise { to: 7 }, false)], &[1.0], &[Some(9.0)], &mut Vec::new());
}

#[test]
#[should_panic(expected = "map[0].created is true but menu[0] Check has an unmoved source")]
fn legalize_row_rejects_an_inconsistent_created_flag() {
    core_preflop::legalize_row(&[Action::Check], &[dest(0, true, Action::Check, false)], &[1.0], &[None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "probs[1] = NaN is not a probability in [0, 1]")]
fn legalize_row_rejects_a_nan_probability() {
    let (menu, map) = core_preflop::destination_map(&[Action::Check, Action::Bet { to: 50 }], &opening_legal()).unwrap();
    core_preflop::legalize_row(&menu, &map, &[0.5, f32::NAN], &[None, None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "probs[0] = -0.1 is not a probability in [0, 1]")]
fn legalize_row_rejects_a_negative_probability() {
    let (menu, map) = check_only();
    core_preflop::legalize_row(&menu, &map, &[-0.1], &[None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "probs[0] = 1.5 is not a probability in [0, 1]")]
fn legalize_row_rejects_a_probability_above_one() {
    let (menu, map) = check_only();
    core_preflop::legalize_row(&menu, &map, &[1.5], &[None], &mut Vec::new());
}

#[test]
#[should_panic(expected = "evs[1] = inf is not finite")]
fn legalize_row_rejects_a_non_finite_ev() {
    let (menu, map) = core_preflop::destination_map(&[Action::Check, Action::Bet { to: 50 }], &opening_legal()).unwrap();
    core_preflop::legalize_row(&menu, &map, &[0.5, 0.5], &[Some(0.0), Some(f32::INFINITY)], &mut Vec::new());
}

/// Whether `a` is one of the live node's `legal` actions (a test-local mirror of the rule).
fn is_legal_at(a: &Action, legal: &[LegalAction]) -> bool {
    legal.iter().any(|la| match (a, la) {
        (Action::Fold, LegalAction::Fold) | (Action::Check, LegalAction::Check) => true,
        (Action::Call, LegalAction::Call { .. }) => true,
        (Action::Bet { to }, LegalAction::Bet { min_to, max_to }) | (Action::Raise { to }, LegalAction::Raise { min_to, max_to }) => {
            min_to <= to && to <= max_to
        }
        (Action::AllIn { to }, LegalAction::AllIn { to: t }) => to == t,
        _ => false,
    })
}

#[test]
fn valid_rows_conserve_mass_and_land_only_on_legal_actions() {
    // Moves of every kind in one node: Raise(7) below min onto the legal Raise(12), Raise(300)
    // above the stack onto a created AllIn(200); Fold/Call/Raise(12) map to themselves.
    let actions = vec![Action::Fold, Action::Call, Action::Raise { to: 7 }, Action::Raise { to: 12 }, Action::Raise { to: 300 }];
    let legal = facing_legal();
    let (menu, map) = core_preflop::destination_map(&actions, &legal).unwrap();
    assert!(menu.iter().all(|a| is_legal_at(a, &legal)), "menu {menu:?}");
    let evs = vec![Some(0.0f32), Some(-1.0), Some(4.0), Some(6.0), Some(8.0)];
    for probs in [
        vec![0.1f32, 0.2, 0.3, 0.25, 0.15], // a reachable row
        vec![0.0f32; 5],                    // an explicitly unreachable (all-zero) row
        vec![0.1f32, 0.1, 0.1, 0.1, 0.1],   // a malformed row summing to 0.5
    ] {
        let advice = core_preflop::legalize_row(&menu, &map, &probs, &evs, &mut Vec::new());
        assert!(advice.actions.iter().all(|a| is_legal_at(&a.action, &legal)), "{:?}", advice.actions);
        let input: f32 = probs.iter().sum();
        assert!((total_probability(&advice) - input).abs() < 1e-6, "row {probs:?}: mass {} vs {input}", total_probability(&advice));
        if input == 0.0 {
            assert!(advice.actions.iter().all(|a| a.probability == 0.0), "no mass is invented for an unavailable row");
        }
        assert_eq!(find(&advice, Action::Raise { to: 12 }).ev_chips, Some(6.0));
        assert_eq!(find(&advice, Action::AllIn { to: 200 }).ev_chips, None);
        assert_eq!(advice.unsupported, None);
    }
}

// --- R7 (fix round 1): the notes buffer is drained into each row's advice ---

#[test]
fn legalize_row_drains_the_whole_notes_buffer_into_each_rows_advice() {
    let (menu, map) = core_preflop::destination_map(&[Action::Check, Action::Bet { to: 120 }], &opening_legal()).unwrap();
    let mut notes = vec!["node-level note".to_string()];
    let first = core_preflop::legalize_row(&menu, &map, &[0.4, 0.6], &[None, None], &mut notes);
    assert_eq!(first.notes, vec![
        "node-level note".to_string(),
        "Moved Bet { to: 120 } probability 0.6 to AllIn { to: 100 }".to_string(),
    ]);
    assert!(notes.is_empty(), "the caller's buffer is left empty");
    // Reusing the (now empty) buffer: the node-level note went to the first row only.
    let second = core_preflop::legalize_row(&menu, &map, &[1.0, 0.0], &[None, None], &mut notes);
    assert_eq!(second.notes, vec!["Moved Bet { to: 120 } probability 0 to AllIn { to: 100 }".to_string()]);
    assert!(notes.is_empty());
}
