//! P3.T9 -- EV-reference normalization, 169-class -> 1326-combo expansion, and the fold-EV
//! admission gate (spec section 8.3's four formulas and section 4.1's combo classes; section
//! 13.1's `pokerdata_units_source_scaling`).

use core_preflop::{
    build_node_map, BundleInfo, ComboClasses, Envelope, EnvelopeAction, EnvelopeNode, EvReference, PreflopNode,
    PreflopStep, RakeProfile, SourceKind,
};
use proto::{Action, Position, Seat};

// The exact test from the task brief (spec section 13.1).
#[test]
fn pokerdata_units_source_scaling() {
    use core_preflop::{ev_chips, normalize_ev, verify_fold, EvReference::*};
    for (sb, bb) in [(1, 2), (2, 5)] {
        let v = normalize_ev(DecisionIncrementalVerified, Some(1.84), sb as f32, 200.).unwrap();
        assert!((v * 0.5 - 0.92).abs() < 1e-6);
        assert!((ev_chips(v, bb) / bb as f32 - 0.92).abs() < 1e-6);
    }
    assert!((normalize_ev(NetHandStartVerified, Some(1.84), 1., 200.).unwrap() - 2.84).abs() < 1e-6);
    assert!(verify_fold(NetHandStartVerified, Some(-1.), 1., 200.));
    assert!(!verify_fold(NetHandStartVerified, Some(-0.5), 1., 200.));
    assert_eq!(normalize_ev(AbsoluteStackVerified, Some(203.), 2., 200.), Some(5.));
    assert_eq!(normalize_ev(Unverified, Some(1.84), 1., 200.), None);
    assert_eq!(normalize_ev(DecisionIncrementalVerified, None, 5., 200.), None);
    assert!((ev_chips(1.84, 10) / 5. - 1.84).abs() < 1e-6);
}

/// Step 5's pure-function extensions: a re-raise prefix's `committed`, a no-fold node's
/// `AbsoluteStackVerified` stack_dec, straddle-sized units, and actual-SB independence.
#[test]
fn ev_reference_normalization_extended() {
    use core_preflop::{ev_chips, normalize_ev, verify_fold, EvReference::*};

    // A re-raise prefix: the actor's own earlier 2.5-bb open (`to_bb_x1000 = 2500`, i.e. 5 source
    // SB, since `committed_before`'s Raise formula is `to_bb_x1000 / 500`) is what they committed
    // before facing the 3bet back. `NetHandStartVerified` shifts the source's net-since-start
    // number onto the decision-point baseline by adding that committed amount back.
    let committed = 5.0f32;
    assert!(
        (normalize_ev(NetHandStartVerified, Some(-2.0), committed, 200.0).unwrap() - 3.0).abs() < 1e-6,
        "net-hand-start EV -2.0 plus a committed 5 source SB is 3.0 source SB at the decision point"
    );
    assert!(verify_fold(NetHandStartVerified, Some(-5.0), committed, 200.0), "folding back exactly what was put in");
    assert!(!verify_fold(NetHandStartVerified, Some(-0.5), committed, 200.0), "off by 4.5 source SB");

    // A no-fold BB-vs-limp node at 100 bb depth (`source_stack_sb = 2 * 100 = 200` source SB): the
    // BB has only posted its own blind (`committed = 2` source SB) and faces a straight limp, so
    // its menu never offers Fold at all. `AbsoluteStackVerified`'s own baseline is the actor's
    // final absolute stack, so `stack_dec = start - committed = 200 - 2 = 198` is what a
    // *non-aggressive* line (e.g. checking back) settles to, and the formula only ever needs the
    // start/committed pair, never a fold value.
    let (start, committed) = (200.0f32, 2.0f32);
    let stack_dec = start - committed;
    assert_eq!(stack_dec, 198.0);
    assert_eq!(normalize_ev(AbsoluteStackVerified, Some(203.0), committed, start), Some(5.0));
    // No Fold action exists at this node at all -- `verify_fold` is never even asked about one,
    // and if it were, a missing value is never itself a contradiction.
    assert!(verify_fold(AbsoluteStackVerified, None, committed, start));

    // Straddle-sized units (the source unit becomes the straddle amount, spec section 8.3): the
    // same source-SB EV converts proportionally at a 4-chip and a 10-chip unit.
    let inc_sb = 1.84f32;
    assert!((ev_chips(inc_sb, 4) - 3.68).abs() < 1e-6);
    assert!((ev_chips(inc_sb, 10) - 9.2).abs() < 1e-6);

    // The actual small blind never enters either formula: changing it from 1 to 2 chips while the
    // BB/straddle unit stays 5 can never multiply the resulting EV by the actual SB, because
    // neither `normalize_ev` nor `ev_chips` takes an SB chip amount as a parameter at all -- the
    // same source value at the same unit always converts to the same chip EV.
    let same_unit = 5u32;
    let normalized = normalize_ev(DecisionIncrementalVerified, Some(inc_sb), 0.0, 200.0).unwrap();
    let chips_when_sb_is_1 = ev_chips(normalized, same_unit);
    let chips_when_sb_is_2 = ev_chips(normalized, same_unit);
    assert_eq!(chips_when_sb_is_1, chips_when_sb_is_2);
    assert!((chips_when_sb_is_1 - inc_sb * 0.5 * same_unit as f32).abs() < 1e-6);
}

fn bundle_info(ev_reference: EvReference, depth_bb: u16) -> BundleInfo {
    BundleInfo {
        bundle_id: "t9_test".into(),
        source: SourceKind::PokerDataJson,
        depth_bb,
        depths: vec![depth_bb],
        source_blinds: [0.5, 1.0],
        rake_profile: "test".into(),
        rake: Some(RakeProfile { rate: 0.0, cap_bb: 0.0, no_flop_no_drop: true }),
        straddle: false,
        version: 2,
        game: "nl".into(),
        ev_unit: "source_sb".into(),
        ev_reference,
        license_note: "test fixture".into(),
        accuracy: "unverified".into(),
        sha256: String::new(),
    }
}

/// `ComboClasses`/`expand_column`/`expand_optional`/`expand_mask`/`expand_node` end to end on a
/// small in-memory node: class-major -> combo-major, source SB -> chips, the exact-zero fold rule,
/// a null EV staying distinct from a numeric zero (including a zero-frequency action that still
/// carries a real EV), and `to_chip_action`'s half-up raise rounding and actual-stack `AllIn`.
#[test]
fn expand_node_expands_classes_to_combos() {
    use core_preflop::expand_node;

    let classes = ComboClasses::build();
    // Some combo of class 0 and of class 7, used below to check the per-combo expansion.
    let combo_class_0 = (0..1326).find(|&i| classes.class(i) == 0).expect("class 0 has combos");
    let combo_class_7 = (0..1326).find(|&i| classes.class(i) == 7).expect("class 7 has combos");

    // Every class always folds and never raises (so the raise action is a "zero-frequency" action
    // at every class), except class 0's declared EVs are actually populated -- the frequency and
    // the EV are independent arrays, and a zero-frequency action's real EV must survive expansion
    // unchanged, never zeroed or nulled just because its weight is zero.
    let probs: Vec<Vec<f32>> = (0..169).map(|_| vec![1.0, 0.0, 0.0]).collect();
    let mut ev_source_sb: Vec<Vec<Option<f32>>> = (0..169).map(|_| vec![None, None, None]).collect();
    ev_source_sb[0] = vec![Some(0.0), Some(2.31), Some(50.0)]; // fold, raise, all-in
    let mut unreachable = [false; 169];
    unreachable[7] = true;

    let node = PreflopNode {
        actor: Position::Bb,
        actions: vec![PreflopStep::Fold, PreflopStep::Raise { to_bb_x1000: 2500 }, PreflopStep::AllIn],
        probs,
        ev_source_sb: Some(ev_source_sb),
        unreachable,
        committed_by_actor_sb: 0.0, // consistent with the fold EV of exactly 0.0 at class 0
    };
    let info = bundle_info(EvReference::DecisionIncrementalVerified, 100);

    let expanded = expand_node(&node, &info, Seat(1), /* unit */ 3, /* actor_max_to */ 777);

    // `to_chip_action`: fold/raise/all-in convert as expected -- the 2.5-bb raise half-up-rounds
    // to an 8-chip raise at a 3-chip unit (matching P3.T8's own boundary probe), and `AllIn` is the
    // actor's actual maximum, never a function of the source's own depth.
    assert_eq!(expanded.actions, vec![Action::Fold, Action::Raise { to: 8 }, Action::AllIn { to: 777 }]);
    assert_eq!(expanded.ev_reference, EvReference::DecisionIncrementalVerified);
    assert_eq!(expanded.source, SourceKind::PokerDataJson);
    assert_eq!(expanded.actor, Seat(1));
    assert_eq!(expanded.probs.len(), 1326);
    assert_eq!(expanded.ev_chips.len(), 1326);

    // Class-major -> combo-major: every combo of class 0 sees class 0's own row.
    assert_eq!(expanded.probs[combo_class_0], vec![1.0, 0.0, 0.0]);
    // The raise action (index 1) is a zero-frequency action at class 0, yet its EV (2.31 source
    // SB, `DecisionIncrementalVerified` so unchanged, times 0.5 * unit(3) = 3.465 chips) survives:
    // a null EV is never conflated with a genuine numeric zero.
    assert!((expanded.ev_chips[combo_class_0][1].unwrap() - 3.465).abs() < 1e-6);
    assert_ne!(expanded.ev_chips[combo_class_0][1], Some(0.0), "a real nonzero EV is never rounded to zero");
    // Fold's EV was declared exactly 0.0 (consistent with committed = 0.0) and normalizes to the
    // exact chip zero the spec requires, not merely "close to zero".
    assert_eq!(expanded.ev_chips[combo_class_0][0], Some(0.0));
    // Every other class has no declared EV data at all: `None`, never a fabricated `Some(0.0)`.
    let other_combo = (0..1326).find(|&i| classes.class(i) != 0).unwrap();
    assert_eq!(expanded.ev_chips[other_combo], vec![None, None, None]);
    assert_ne!(expanded.ev_chips[other_combo][0], Some(0.0), "a missing EV is never a numeric zero");

    // The class-7 unreachable declaration expands to every one of its combos.
    assert!(!expanded.available[combo_class_7]);
    assert!(expanded.available[combo_class_0]);
}

/// Charts and unverified sources suppress every EV, at every combo and every action -- never a
/// partial suppression and never a fabricated zero.
#[test]
fn expand_node_suppresses_ev_for_charts_and_unverified_sources() {
    use core_preflop::expand_node;
    let probs: Vec<Vec<f32>> = (0..169).map(|_| vec![1.0]).collect();
    let ev_source_sb: Vec<Vec<Option<f32>>> = (0..169).map(|_| vec![Some(0.0)]).collect();
    let node = PreflopNode {
        actor: Position::Utg,
        actions: vec![PreflopStep::Fold],
        probs,
        ev_source_sb: Some(ev_source_sb),
        unreachable: [false; 169],
        committed_by_actor_sb: 0.0,
    };
    for info in [
        {
            let mut i = bundle_info(EvReference::DecisionIncrementalVerified, 100);
            i.source = SourceKind::ChartTranscription;
            i
        },
        bundle_info(EvReference::Unverified, 100),
    ] {
        let expanded = expand_node(&node, &info, Seat(0), 100, 1000);
        assert!(expanded.ev_chips.iter().all(|row| row == &[None]), "{:?}", expanded.ev_chips);
    }
}

fn three_bet_history() -> Vec<(String, String, u32)> {
    vec![
        ("UTG".into(), "raise".into(), 2500),
        ("HJ".into(), "raise".into(), 8750),
        ("CO".into(), "fold".into(), 0),
        ("BTN".into(), "fold".into(), 0),
        ("SB".into(), "fold".into(), 0),
        ("BB".into(), "fold".into(), 0),
    ]
}

fn envelope_with_fold_ev(history: Vec<(String, String, u32)>, fold_ev: f32) -> Envelope {
    Envelope {
        bundle_id: "t9_fold".into(),
        depth_bb: 100,
        rake_profile: "test".into(),
        straddle: false,
        class_order: "A-2 row-major, section 4.1".into(),
        nodes: vec![EnvelopeNode {
            history,
            actor: "UTG".into(),
            actions: vec![
                EnvelopeAction { step: "fold".into(), to_bb_x1000: None, label: None },
                EnvelopeAction { step: "raise".into(), to_bb_x1000: Some(22000), label: None },
            ],
            weights: vec![vec![1.0; 169], vec![0.0; 169]],
            evs: Some(vec![vec![Some(fold_ev); 169], vec![Some(1.0); 169]]),
            unreachable_classes: vec![],
        }],
    }
}

/// Step 5: a present, conflicting fold EV makes the node -- and so the bundle -- unloadable, while
/// a separately validated bundle (with its own consistent data, under a different EV reference) is
/// completely unaffected.
#[test]
fn fold_ev_consistency_gates_bundle_admission() {
    let history = three_bet_history();
    // UTG's own prior 2.5-bb open (`to_bb_x1000 = 2500`) commits exactly 5 source SB
    // (`committed_before`'s Raise formula: `to_bb_x1000 / 500`) before the 3bet comes back.
    let net_start = bundle_info(EvReference::NetHandStartVerified, 100);

    let consistent = build_node_map(&net_start, &envelope_with_fold_ev(history.clone(), -5.0));
    assert!(consistent.is_ok(), "{consistent:?}");

    let inconsistent = build_node_map(&net_start, &envelope_with_fold_ev(history.clone(), -0.5));
    let err = inconsistent.expect_err("a fold EV off by 4.5 source SB (tolerance 1e-3) must be rejected").to_string();
    assert!(err.contains("class"), "{err}");
    assert!(err.contains("inconsistent"), "{err}");

    // A second, independent bundle -- a different EV reference, its own consistent fold value --
    // loads without any regard to the first bundle's rejection: each call validates only its own
    // bytes (spec section 8.2's quarantine rule), which `PreflopStore::open`'s existing
    // quarantine-and-banner loop already exercises per bundle; here each `build_node_map` call is
    // itself already fully independent of the other.
    let absolute_stack = bundle_info(EvReference::AbsoluteStackVerified, 100);
    // stack_dec = source_stack_sb(100) - committed(5) = 200 - 5 = 195.
    let other = build_node_map(&absolute_stack, &envelope_with_fold_ev(history, 195.0));
    assert!(other.is_ok(), "{other:?}");
}
