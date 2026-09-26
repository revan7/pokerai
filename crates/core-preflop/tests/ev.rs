//! P3.T9 -- EV-reference normalization, 169-class -> 1326-combo expansion, and the fold-EV
//! admission gate (spec section 8.3's four formulas and section 4.1's combo classes; section
//! 13.1's `pokerdata_units_source_scaling`).

use core_model::state::BeginHand;
use core_preflop::{
    checked_envelope, load_bundle, BundleInfo, ComboClasses, EvReference, PreflopNode, PreflopNodeKey, PreflopStep,
    PreflopStore, RakeProfile, SourceKind,
};
use proto::{Action, ApproxReason, HandConfig, HandState, Position, Rake, Seat, UtgStraddle};
use std::path::{Path, PathBuf};

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

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Raw wire JSON for a single node reachable at `history`, actor UTG, menu `[fold, raise 22bb]`,
/// with a declared fold EV of `fold_ev` at every one of the 169 classes and a harmless raise EV
/// of `1.0` at every class -- structurally valid regardless of `fold_ev`'s value. `evs` here is
/// the wire's own action-major shape (matching `EnvelopeNode::evs`/`WideFoldNode::evs`), used to
/// exercise `checked_envelope`'s load-time fold-EV admission gate directly, on raw bytes, the way
/// `load_bundle` itself would receive them -- never through a typed `Envelope` (that would bypass
/// the ORIGINAL wire-precision cells the wide check must run on).
fn fold_node_envelope_json(history_json: &str, fold_ev: f64) -> String {
    let row = |v: String| format!("[{}]", vec![v; 169].join(","));
    format!(
        r#"{{"bundle_id":"t9_test","depth_bb":100,"rake_profile":"test","straddle":false,"class_order":"A-2 row-major, section 4.1","nodes":[{{"history":{history_json},"actor":"UTG","actions":[{{"step":"fold"}},{{"step":"raise","to_bb_x1000":22000}}],"weights":[{ones},{zeros}],"evs":[{fold_row},{raise_row}],"unreachable_classes":[]}}]}}"#,
        ones = row("1".into()),
        zeros = row("0".into()),
        fold_row = row(fold_ev.to_string()),
        raise_row = row("1.0".into()),
    )
}

/// P3.T9 fix round 1, R2: the load-time fold-EV admission gate (`check_fold_consistency_wide`,
/// run inside `checked_envelope`) decides on the ORIGINAL wire-precision (`f64`) EV cells, never
/// on a value that has already been narrowed to `f32` -- exercised here through raw JSON bytes
/// and `checked_envelope` directly, the same entry point `load_bundle` calls, rather than through
/// a typed `Envelope`/`build_node_map` (which no longer makes this admission decision at all: a
/// second, narrow-only check there could disagree with the wide one purely from rounding, which
/// is exactly the defect this fix corrects).
#[test]
fn fold_ev_consistency_gates_bundle_admission_through_the_loader() {
    let history = serde_json::to_string(&three_bet_history()).unwrap();

    // UTG's own prior 2.5-bb open (`to_bb_x1000 = 2500`) commits exactly 5 source SB
    // (`committed_before`'s Raise formula: `to_bb_x1000 / 500`) before the 3bet comes back.
    // Consistent: folding back exactly what was put in.
    let raw_ok = fold_node_envelope_json(&history, -5.0).into_bytes();
    let mut info_ok = bundle_info(EvReference::NetHandStartVerified, 100);
    info_ok.sha256 = sha256_hex(&raw_ok);
    assert!(checked_envelope(&info_ok, &raw_ok).is_ok(), "a consistent fold EV must load");

    // Inconsistent: off by 4.5 source SB.
    let raw_bad = fold_node_envelope_json(&history, -0.5).into_bytes();
    let mut info_bad = bundle_info(EvReference::NetHandStartVerified, 100);
    info_bad.sha256 = sha256_hex(&raw_bad);
    let err = checked_envelope(&info_bad, &raw_bad)
        .expect_err("a fold EV off by 4.5 source SB (tolerance 1e-3) must be rejected")
        .to_string();
    assert!(err.contains("class"), "{err}");
    assert!(err.contains("inconsistent"), "{err}");

    // A second, independent bundle -- a different EV reference, its own consistent fold value --
    // loads without any regard to the first bundle's rejection: each call validates only its own
    // bytes (spec section 8.2's quarantine rule). stack_dec = source_stack_sb(100) - committed(5)
    // = 200 - 5 = 195.
    let raw_other = fold_node_envelope_json(&history, 195.0).into_bytes();
    let mut info_other = bundle_info(EvReference::AbsoluteStackVerified, 100);
    info_other.sha256 = sha256_hex(&raw_other);
    assert!(checked_envelope(&info_other, &raw_other).is_ok(), "{:?}", checked_envelope(&info_other, &raw_other));

    // R2's exact wide-vs-narrow boundary cases (reproduced from the review's own numeric probe):
    // wire -5.0010001 is wide-inconsistent (residual 0.0010001 > 1e-3), but narrowing the fold
    // cell to f32 first would shift the residual to ~0.0009999275 < 1e-3 and wrongly accept it.
    let raw_reject = fold_node_envelope_json(&history, -5.0010001).into_bytes();
    let mut info_reject = bundle_info(EvReference::NetHandStartVerified, 100);
    info_reject.sha256 = sha256_hex(&raw_reject);
    assert!(
        checked_envelope(&info_reject, &raw_reject).is_err(),
        "the wide check must catch a residual just over 1e-3 that f32 narrowing would mask"
    );

    // The reverse boundary: wire 195.0009999 under AbsoluteStackVerified IS wide-consistent
    // (stack_dec = 195, residual 0.0009999 < 1e-3), but narrowing the fold cell to f32 first
    // would push the residual just over 1e-3 and wrongly reject it.
    let raw_accept = fold_node_envelope_json(&history, 195.0009999).into_bytes();
    let mut info_accept = bundle_info(EvReference::AbsoluteStackVerified, 100);
    info_accept.sha256 = sha256_hex(&raw_accept);
    assert!(
        checked_envelope(&info_accept, &raw_accept).is_ok(),
        "the wide check must accept a residual just under 1e-3 that f32 narrowing would push over"
    );
}

/// R3 (P3.T9 fix round 1): `PreflopNode`/`BundleInfo` are publicly constructible, and
/// `PreflopStore::from_sources` accepts a source without ever running the loader's wide admission
/// gate -- so `expand_node` itself must refuse to silently zero an inconsistent fold when called
/// directly on unvalidated data. The review's own reproduction: a verified incremental fold EV of
/// `1.0` with `committed = 0.0` (`|1.0| > 1e-3`), which the pre-fix code normalized and
/// force-zeroed without complaint. Now it panics instead, naming the offending class.
#[test]
#[should_panic(expected = "fold-EV consistency invariant violated")]
fn expand_node_panics_on_an_unvalidated_inconsistent_fold_from_direct_construction() {
    use core_preflop::expand_node;
    let probs: Vec<Vec<f32>> = (0..169).map(|_| vec![1.0]).collect();
    let mut ev_source_sb: Vec<Vec<Option<f32>>> = (0..169).map(|_| vec![Some(0.0)]).collect();
    ev_source_sb[0] = vec![Some(1.0)]; // inconsistent: |1.0| > 1e-3 under DecisionIncrementalVerified
    let node = PreflopNode {
        actor: Position::Utg,
        actions: vec![PreflopStep::Fold],
        probs,
        ev_source_sb: Some(ev_source_sb),
        unreachable: [false; 169],
        committed_by_actor_sb: 0.0,
    };
    let info = bundle_info(EvReference::DecisionIncrementalVerified, 100);
    let _ = expand_node(&node, &info, Seat(0), 100, 1000);
}

// --- R1 (P3.T9 fix round 1): the shared checked chip conversion ---

/// The review's own reproduction: an admitted, finite source EV whose chip conversion overflows
/// `f32` must panic (naming the values), never silently return `inf`.
#[test]
#[should_panic(expected = "does not fit a finite f32")]
fn ev_chips_positive_overflow_panics_never_silently_returns_infinity() {
    let _ = core_preflop::ev_chips(3.0e38, 4);
}

/// The negative-overflow twin of the case above.
#[test]
#[should_panic(expected = "does not fit a finite f32")]
fn ev_chips_negative_overflow_panics_never_silently_returns_infinity() {
    let _ = core_preflop::ev_chips(-3.0e38, 4);
}

/// A finite boundary control: large but representable (`1e37 * 0.5 * 4 = 2e37`, well inside
/// `f32::MAX` (~3.4e38)) converts normally and must never panic.
#[test]
fn ev_chips_finite_boundary_converts_normally() {
    let v = core_preflop::ev_chips(1.0e37, 4);
    assert!(v.is_finite(), "{v}");
    assert!(((v as f64) / 2.0e37 - 1.0).abs() < 1e-6, "{v}");
}

/// `expand_node` never calls the infallible, panicking `ev_chips` for a non-fold action: an
/// unrepresentable chip EV (the review's own reproduction, 3.0e38 at unit 4) must come out as
/// `None`, never `Some(inf)` and never a clamped or fabricated `Some(0.0)`.
#[test]
fn expand_node_propagates_an_unrepresentable_ev_as_none_never_infinite_or_zeroed() {
    use core_preflop::expand_node;
    let probs: Vec<Vec<f32>> = (0..169).map(|_| vec![1.0, 0.0]).collect();
    let mut ev_source_sb: Vec<Vec<Option<f32>>> = (0..169).map(|_| vec![None, None]).collect();
    ev_source_sb[0] = vec![Some(0.0), Some(3.0e38)]; // fold (consistent), raise (overflows at unit 4)
    let node = PreflopNode {
        actor: Position::Bb,
        actions: vec![PreflopStep::Fold, PreflopStep::Raise { to_bb_x1000: 2500 }],
        probs,
        ev_source_sb: Some(ev_source_sb),
        unreachable: [false; 169],
        committed_by_actor_sb: 0.0,
    };
    let info = bundle_info(EvReference::DecisionIncrementalVerified, 100);
    let expanded = expand_node(&node, &info, Seat(1), 4, 777);
    let classes = ComboClasses::build();
    let combo_class_0 = (0..1326).find(|&i| classes.class(i) == 0).unwrap();
    assert_eq!(expanded.ev_chips[combo_class_0][1], None, "an unrepresentable chip EV must be None");
    assert_ne!(expanded.ev_chips[combo_class_0][1], Some(0.0), "never a fabricated zero either");
}

// --- R4 (P3.T9 fix round 1): real integration tests through the loader and the store ---

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/preflop/synthetic_v2")
}

/// The committed synthetic PokerData bundle, loaded through the real `load_bundle` entry point
/// (standing ruling (e): a missing or unreadable fixture fails the test, never skipped).
fn synthetic_bundle() -> Box<dyn core_preflop::PreflopSource> {
    let dir = fixture_dir();
    load_bundle(&dir.join("manifest.json"), &dir.join("nodes.json")).expect("the committed synthetic_v2 bundle loads")
}

fn synthetic_store() -> PreflopStore {
    PreflopStore::from_sources(vec![synthetic_bundle()])
}

fn config(sb: u32, bb: u32, straddle: Option<u32>, rake: Rake) -> HandConfig {
    HandConfig { config_revision: 7, sb_chips: sb, bb_chips: bb, straddle: straddle.map(|amount_chips| UtgStraddle { amount_chips }), rake, chip_label: "$1".into() }
}

/// The fixture's own profile (5%, cap 0.5 bb, no-flop-no-drop) expressed in chips at `unit` chips
/// per source unit, so `rake_reason` sees an exact match and emits nothing.
fn exact_rake(unit: u32) -> Rake {
    Rake::PotRake { rate: 0.05, cap_mchips: unit * 500, no_flop_no_drop: true }
}

fn dealt_seats(n: usize) -> Vec<Seat> {
    (0..n as u8).map(Seat).collect()
}

fn button_of(n: usize) -> Seat {
    Seat(n as u8 - 1)
}

fn table(cfg: &HandConfig, n: usize, stacks: &[u32]) -> HandState {
    core_model::begin_hand(
        cfg,
        BeginHand { hand_id: 1, button: button_of(n), hero: Seat(0), dealt: dealt_seats(n), stacks_start: stacks.to_vec(), hero_cards: None },
    )
    .expect("the model admits this table")
}

fn act(state: &HandState, actions: &[Action]) -> HandState {
    let mut next = state.clone();
    for a in actions {
        next = core_model::apply_action(&next, *a).unwrap_or_else(|e| panic!("legal preflop action {a:?}: {e}"));
    }
    next
}

/// R4: the review's own complaint was that the "SB=1 vs SB=2" pure-function check called
/// `ev_chips` twice with IDENTICAL arguments -- proving nothing about the actual lookup path.
/// This is the real check: two genuine `PreflopStore::query` calls through the committed
/// synthetic_v2 bundle, at the UTG-opens root node, differing only in `sb_chips` (1 vs 2) at a
/// fixed BB unit of 5. `expanded` must come out bit-for-bit identical either way.
#[test]
fn expand_node_through_the_store_is_independent_of_the_actual_small_blind() {
    let unit = 5u32;
    let stacks = vec![100 * unit; 6];
    let store = synthetic_store();

    let cfg_sb1 = config(1, unit, None, exact_rake(unit));
    let answer_sb1 = store.query(&cfg_sb1, &table(&cfg_sb1, 6, &stacks), 0);
    assert_eq!(answer_sb1.unsupported, None, "{answer_sb1:?}");
    assert!(answer_sb1.expanded.is_some());

    let cfg_sb2 = config(2, unit, None, exact_rake(unit));
    let answer_sb2 = store.query(&cfg_sb2, &table(&cfg_sb2, 6, &stacks), 0);
    assert_eq!(answer_sb2.unsupported, None, "{answer_sb2:?}");

    assert_eq!(
        answer_sb1.expanded, answer_sb2.expanded,
        "changing the actual small blind must never change the expanded node at a fixed BB unit"
    );
}

/// R4: the re-raise node (the fixture's UTG-facing-HJ's-3bet node) exercised through the real
/// loader and store, not hand-built in memory. UTG's own prior raise commits exactly 5 source SB
/// before the 3bet comes back, and the fixture's own fold EV is exactly 0.0 at every class --
/// consistent under `DecisionIncrementalVerified` regardless of `committed`, and forced to the
/// exact chip zero section 6 requires.
#[test]
fn reraise_node_through_the_store_forces_exact_zero_fold() {
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let root = table(&cfg, 6, &vec![100 * unit; 6]);
    let store = synthetic_store();
    let three_bet_state = act(
        &root,
        &[Action::Raise { to: 2500 }, Action::Raise { to: 8750 }, Action::Fold, Action::Fold, Action::Fold, Action::Fold],
    );
    let answer = store.query(&cfg, &three_bet_state, 6);
    assert_eq!(answer.unsupported, None, "{answer:?}");
    let node = answer.node.as_ref().expect("the re-raise node is present in the fixture");
    assert_eq!(node.actor, Position::Utg);
    let expanded = answer.expanded.as_ref().expect("a present node is always expanded");
    let fold_idx = expanded.actions.iter().position(|a| *a == Action::Fold).expect("fold is on the menu");
    assert!(
        expanded.ev_chips.iter().all(|row| row[fold_idx] == Some(0.0)),
        "the fixture's uniform fold EV must force to exactly 0.0 chips at every combo"
    );
}

/// R4: the no-fold node (BB facing a limp after three folds and the SB's call) -- its menu is
/// `[check, raise]` with no fold action at all, so the exact-zero fold-forcing branch in
/// `expand_node` is never exercised here, and its raise action carries a real, class-specific EV
/// (including at a *zero-frequency* class) that must survive expansion unchanged -- exercised end
/// to end through the real loader/store, not hand-built data.
#[test]
fn no_fold_node_through_the_store_preserves_a_zero_frequency_ev() {
    let unit = 1000u32;
    let cfg = config(unit / 2, unit, None, exact_rake(unit));
    let root = table(&cfg, 6, &vec![100 * unit; 6]);
    let store = synthetic_store();
    let limped = act(&root, &[Action::Fold, Action::Fold, Action::Fold, Action::Fold, Action::Call]);
    let answer = store.query(&cfg, &limped, 5);
    assert_eq!(answer.unsupported, None, "{answer:?}");
    let node = answer.node.as_ref().expect("the no-fold BB-vs-limp node is present in the fixture");
    assert_eq!(node.actor, Position::Bb);
    let expanded = answer.expanded.as_ref().expect("a present node is always expanded");
    assert!(!expanded.actions.iter().any(|a| *a == Action::Fold), "this node's menu never offers fold: {:?}", expanded.actions);
    let raise_idx = expanded.actions.iter().position(|a| matches!(a, Action::Raise { .. })).expect("raise is on the menu");
    let classes = ComboClasses::build();
    let combo_class_1 = (0..1326).find(|&i| classes.class(i) == 1).unwrap();
    let combo_class_14 = (0..1326).find(|&i| classes.class(i) == 14).unwrap();
    // Class 1: declared raise EV 1.84 source SB -> 1.84 * 0.5 * 1000 = 920.0 chips.
    let ev1 = expanded.ev_chips[combo_class_1][raise_idx].expect("class 1's raise EV is declared");
    assert!((ev1 - 920.0).abs() < 1e-2, "{ev1}");
    // Class 14: raise is a ZERO-FREQUENCY action (weight 0.0) at this class, yet its declared EV
    // (2.31 source SB -> 1155.0 chips) survives unchanged.
    assert_eq!(expanded.probs[combo_class_14][raise_idx], 0.0, "raise is zero-frequency at class 14");
    let ev14 = expanded.ev_chips[combo_class_14][raise_idx].expect("class 14's raise EV is declared despite zero frequency");
    assert!((ev14 - 1155.0).abs() < 1e-2, "{ev14}");
}

/// R4: the straddle mapping reaching `expand_node` through the real store -- the physical HJ
/// seat's decision is looked up as the fixture's virtual-UTG-facing-a-raise node, which also
/// carries a uniform, exactly-consistent fold EV.
#[test]
fn straddle_mapped_node_through_the_store_exercises_expand_node() {
    let unit = 2000u32; // the straddle amount is the source unit
    let cfg = config(500, 1000, Some(unit), exact_rake(unit));
    let state = table(&cfg, 6, &vec![100 * unit; 6]);
    // HJ (virtual UTG) opens to 2.5 straddles; CO (virtual HJ) is next to act.
    let opened = act(&state, &[Action::Raise { to: 5000 }]);
    let store = synthetic_store();
    let answer = store.query(&cfg, &opened, 1);
    assert_eq!(answer.unsupported, None, "{answer:?}");
    assert!(answer.reasons.contains(&ApproxReason::StraddleMapped { posts: [0.25, 0.5, 1.0] }), "{:?}", answer.reasons);
    let node = answer.node.as_ref().expect("the virtual-HJ node is present in the fixture");
    assert_eq!(node.actor, Position::Hj);
    let expanded = answer.expanded.as_ref().expect("a present node is always expanded");
    let fold_idx = expanded.actions.iter().position(|a| *a == Action::Fold).expect("fold is on the menu");
    assert!(
        expanded.ev_chips.iter().all(|row| row[fold_idx] == Some(0.0)),
        "the fixture's uniform fold EV must force to exactly 0.0 chips even under the straddle mapping"
    );
}

/// R4/R2: `PreflopStore::open` on a directory holding one fold-inconsistent bundle (rejected by
/// the new wide admission gate) and one valid sibling (the real committed synthetic_v2 fixture,
/// byte-for-byte) -- the bad one is quarantined with a banner naming it, and the healthy sibling
/// still answers a direct key lookup, exactly as spec section 8.2's quarantine rule requires.
#[test]
fn fold_inconsistent_bundle_is_quarantined_with_a_healthy_sibling_through_open() {
    let dir = std::env::temp_dir().join(format!("core_preflop_p3t9_fix1_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("good")).unwrap();
    std::fs::create_dir_all(dir.join("bad")).unwrap();

    let good_manifest = fixture_dir().join("manifest.json");
    let good_nodes = fixture_dir().join("nodes.json");
    std::fs::copy(&good_manifest, dir.join("good/manifest.json")).unwrap();
    std::fs::copy(&good_nodes, dir.join("good/nodes.json")).unwrap();

    let history = serde_json::to_string(&three_bet_history()).unwrap();
    let raw_bad = fold_node_envelope_json(&history, -0.5).into_bytes(); // off by 4.5 source SB
    let mut info_bad = bundle_info(EvReference::NetHandStartVerified, 100);
    info_bad.sha256 = sha256_hex(&raw_bad);
    std::fs::write(dir.join("bad/manifest.json"), serde_json::to_string(&info_bad).unwrap()).unwrap();
    std::fs::write(dir.join("bad/nodes.json"), &raw_bad).unwrap();

    let (store, banners) = PreflopStore::open(&dir);
    assert_eq!(banners.len(), 1, "{banners:?}");
    assert!(banners[0].contains("inconsistent"), "{banners:?}");
    assert!(dir.join("bad.bad").exists(), "the fold-inconsistent bundle is quarantined");

    let good_info: BundleInfo = serde_json::from_str(&std::fs::read_to_string(&good_manifest).unwrap()).unwrap();
    let good = store.bundle_of(&good_info.bundle_id).expect("the healthy sibling must still load");
    let key = PreflopNodeKey { depth_bb: 100, rake_profile: good_info.rake_profile.clone(), straddle: false, history: vec![] };
    assert!(good.lookup(&key).is_some(), "the healthy sibling must still answer a direct key lookup");

    let _ = std::fs::remove_dir_all(&dir);
}
