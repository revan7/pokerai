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

// --- wager_fraction (spec section 8.4: pot fraction at the parent node) ---

#[test]
fn wager_fraction_matches_the_boundary_test_size() {
    // own = 0, call = 0 (an opening bet after checks around), pot = 100, to = 73: s = 0.73,
    // exactly the size `bet_translation_boundaries` interpolates against.
    let s = core_preflop::wager_fraction(73, 0, 0, 100);
    assert!((s - 0.73).abs() < 1e-12);
}

#[test]
fn wager_fraction_raise_to_uses_pot_plus_call_denominator() {
    // A raise-to: own = 20 (already committed this street), call = 30 (owed to match the
    // current wager), pot = 150 (before this actor's call is added). s = (to-own-call)/(pot+call).
    let s = core_preflop::wager_fraction(140, 20, 30, 150);
    assert!((s - (140.0 - 20.0 - 30.0) / (150.0 + 30.0)).abs() < 1e-12);
}

#[test]
#[should_panic(expected = "pot")]
fn wager_fraction_rejects_zero_pot_and_call() {
    core_preflop::wager_fraction(0, 0, 0, 0);
}

#[test]
#[should_panic(expected = "negative")]
fn wager_fraction_rejects_a_wager_below_a_call() {
    // to (5) is less than own (10) + call (0): not a wager at all.
    core_preflop::wager_fraction(5, 10, 0, 100);
}

// --- menu_fractions: a source node's own wager-sized actions, as pot fractions ---

#[test]
fn menu_fractions_skips_non_wager_actions_and_keeps_original_indices() {
    let actions = vec![Action::Fold, Action::Check, Action::Bet { to: 50 }, Action::Bet { to: 100 }];
    let m = core_preflop::menu_fractions(&actions, 0, 0, 100);
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
    let m = core_preflop::menu_fractions(&actions, 20, 30, 150);
    assert_eq!(m, vec![
        (2, core_preflop::wager_fraction(250, 20, 30, 150)),
        (3, core_preflop::wager_fraction(400, 20, 30, 150)),
    ]);
}

#[test]
fn menu_fractions_of_an_all_fold_check_menu_is_empty() {
    let actions = vec![Action::Fold, Action::Check];
    assert!(core_preflop::menu_fractions(&actions, 0, 0, 100).is_empty());
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
    // `legalize_row` has no access to the original per-source action (only `Destination`'s
    // index/created flag), so `MovedProbability::from` names the destination itself, not the
    // original `Bet(120)` -- the only original-action detail available at this point in the
    // pipeline. It still tags every row landing on a created destination as moved.
    assert_eq!(allin.unavailable, Some(Unavailable::MovedProbability { from: Action::AllIn { to: 100 } }));
    assert!((allin.probability - 0.6).abs() < 1e-6);
    assert!(advice.notes.iter().any(|n| n.contains("Moved")));
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
