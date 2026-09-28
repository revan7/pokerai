//! P3.T13 -- preflop replay with frozen missing-node stops (spec sections 8.3, 8.4, 9.1-9.3; the
//! preflop half of section 13.1's `replay_missing_continuation`).
//!
//! Every hand is built by Plan 1's `begin_hand`/`apply_action`/`set_board`, never by editing
//! `Derived`. Every source is the committed synthetic_v2 bundle (Task 2) loaded in memory through
//! the loader's own validation; where a test needs a missing node, a selected key is removed from
//! that map -- no source response is ever invented. A missing fixture fails the test (standing
//! ruling (e)).
//!
//! The fixture is deliberately sparse, and several tests lean on that: at every node every class
//! takes the first action (fold/check) except AKs (class 1), which takes the second action with
//! probability 0.35, and 22 (class 168), which is explicitly unreachable. Every node's third action
//! (the 3bet, 4bet, squeeze, ...) has probability 0 for every class, which is the zero-support
//! case of section 9.2. The CO node after a UTG open is absent from the source.

use core_model::state::BeginHand;
use core_preflop::{
    build_node_map, checked_envelope, node_key, BundleInfo, PokerDataJson, PreflopNode, PreflopNodeKey, PreflopStep,
    PreflopStore,
};
use core_replay::*;
use proto::{Action, ApproxReason, Card, HandConfig, HandState, Position, Rake, Range1326, Seat, Street, UnsupportedReason, COMBOS};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------
// Fixture and hand helpers.
// ---------------------------------------------------------------------------------------------

/// Chips per big blind: one source unit (no straddle).
const UNIT: u32 = 1000;
const SB: Seat = Seat(0);
const BB: Seat = Seat(1);
const UTG: Seat = Seat(2);
const HJ: Seat = Seat(3);
const CO: Seat = Seat(4);
const BTN: Seat = Seat(5);

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/preflop/synthetic_v2")
}

/// The committed synthetic bundle's manifest and validated node map, built in memory by the
/// loader's own admission functions (hash, content and fold-EV checks included).
fn synthetic_nodes() -> (BundleInfo, BTreeMap<String, PreflopNode>) {
    let dir = fixture_dir();
    let manifest = std::fs::read(dir.join("manifest.json")).expect("the committed synthetic_v2 manifest is readable");
    let info: BundleInfo = serde_json::from_slice(&manifest).expect("the committed manifest deserializes");
    let raw = std::fs::read(dir.join("nodes.json")).expect("the committed synthetic_v2 nodes are readable");
    let envelope = checked_envelope(&info, &raw).expect("the committed bundle passes admission");
    let nodes = build_node_map(&info, &envelope).expect("the committed bundle builds its node map");
    (info, nodes)
}

/// The synthetic source with the named keys removed (and nothing else changed).
fn store_without(removed: &[String]) -> PreflopStore {
    let (info, mut nodes) = synthetic_nodes();
    for key in removed {
        assert!(nodes.remove(key).is_some(), "the fixture holds the node being removed: {key}");
    }
    PreflopStore::from_sources(vec![Box::new(PokerDataJson { info, nodes })])
}

fn full_store() -> PreflopStore {
    store_without(&[])
}

/// A source key at the fixture's depth and rake.
fn key(history: Vec<(Position, PreflopStep)>) -> String {
    node_key(&PreflopNodeKey { depth_bb: 100, rake_profile: "5% cap 0.5bb".into(), straddle: false, history })
}

fn raise_bb_x1000(to_bb_x1000: u32) -> PreflopStep {
    PreflopStep::Raise { to_bb_x1000 }
}

/// A 1/2-style config at `unit` chips per big blind with the fixture's own rake profile, so no
/// mapping reason is emitted on a symmetric 100 bb table.
fn config_at(unit: u32) -> HandConfig {
    HandConfig {
        config_revision: 7,
        sb_chips: unit / 2,
        bb_chips: unit,
        straddle: None,
        rake: Rake::PotRake { rate: 0.05, cap_mchips: unit * 500, no_flop_no_drop: true },
        chip_label: "$1".into(),
    }
}

fn cfg() -> HandConfig {
    config_at(UNIT)
}

/// A six-max 100 bb table; seat 5 holds the button, so SB 0, BB 1, UTG 2, HJ 3, CO 4, BTN 5.
fn table_with(cfg: &HandConfig, hero: Seat, hero_cards: Option<[Card; 2]>) -> HandState {
    core_model::begin_hand(
        cfg,
        BeginHand {
            hand_id: 1,
            button: BTN,
            hero,
            dealt: (0..6).map(Seat).collect(),
            stacks_start: vec![100 * cfg.bb_chips; 6],
            hero_cards,
        },
    )
    .expect("the model admits the table")
}

fn table(cfg: &HandConfig) -> HandState {
    table_with(cfg, BB, None)
}

fn act(state: &HandState, actions: &[Action]) -> HandState {
    let mut next = state.clone();
    for a in actions {
        next = core_model::apply_action(&next, *a).unwrap_or_else(|e| panic!("legal action {a:?}: {e}"));
    }
    next
}

fn deal(state: &HandState, board: &str) -> HandState {
    let cards = core_model::parse_cards(board).expect("a board");
    core_model::set_board(state, &cards).unwrap_or_else(|e| panic!("board {board}: {e}"))
}

fn hand(text: &str) -> [Card; 2] {
    core_model::parse_hand(text).expect("a hand")
}

fn run(store: &PreflopStore, state: &HandState) -> ReplayOutput {
    replay(ReplayInput { cfg: &state.config, state, store, snapshots: &[] })
}

/// UTG opens to the source's 2.5 bb, HJ calls, CO/BTN/SB fold: the BB is to act.
fn open_call_folds() -> Vec<Action> {
    vec![Action::Raise { to: 2500 }, Action::Call, Action::Fold, Action::Fold, Action::Fold]
}

/// One source node's action column, expanded to 1326 combos, as `f64` (unavailable combos 0).
fn column(store: &PreflopStore, state: &HandState, prefix_len: usize, action: usize) -> Vec<f64> {
    let answer = store.query(&state.config, state, prefix_len);
    let node = answer.expanded.unwrap_or_else(|| panic!("the fixture node at prefix {prefix_len} is present: {}", answer.key));
    (0..COMBOS).map(|c| if node.available[c] { f64::from(node.probs[c][action]) } else { 0.0 }).collect()
}

fn seat_mass<'a>(b: &'a HistoryBranch, seat: Seat) -> &'a [f64] {
    &b.seats.iter().find(|s| s.seat == seat).expect("a dealt seat").mass
}

fn uniform() -> Range1326 {
    Range1326([1.0; COMBOS])
}

/// The expected range of a seat whose only evidence is one action column: the column scaled to
/// maximum 1, narrowed at the output boundary.
fn scaled(col: &[f64]) -> Range1326 {
    let max = col.iter().copied().fold(0.0_f64, f64::max);
    range_output(&col.iter().map(|p| p / max).collect::<Vec<f64>>())
}

fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() <= 1e-12 * b.abs().max(1.0), "{what}: {a} != {b}");
}

/// Everything a branch carries, bit for bit, for identity comparisons.
type Fingerprint = (u8, Option<u8>, Option<Seat>, Vec<(Seat, Action)>, u64, bool, Option<String>, Vec<(Seat, Option<String>, Vec<u64>)>);

fn fingerprint(b: &HistoryBranch) -> Fingerprint {
    (
        b.id,
        b.parent,
        b.split_by,
        b.translated.clone(),
        b.q.to_bits(),
        b.residual,
        b.stopped.clone(),
        b.seats.iter().map(|s| (s.seat, s.node.clone(), s.mass.iter().map(|w| w.to_bits()).collect())).collect(),
    )
}

fn unconditioned(street: Street, seat: Seat, cause: &str) -> ApproxReason {
    ApproxReason::UnconditionedPriorStreet { street, seat, cause: cause.into() }
}

/// AKs, the fixture's one class that takes a second action: its four combos.
fn aks() -> Vec<usize> {
    proto::class_combos(1).into_iter().map(usize::from).collect()
}

// ---------------------------------------------------------------------------------------------
// The brief's kernel-level test, verbatim.
// ---------------------------------------------------------------------------------------------

#[test]
fn preflop_stop_is_shared_and_frozen(){
    use core_preflop::branches::*;
    let mut b=initial(&[proto::Seat(0),proto::Seat(1),proto::Seat(2)]);
    b[0].q=0.27;
    b[0].stopped=Some("missing node UTG_2500_HJ_8750".into());
    for s in &mut b[0].seats{s.node=None;}
    let frozen=b[0].clone();
    for actor in [proto::Seat(0),proto::Seat(1),proto::Seat(2)]{
        let next=condition(&b[0],actor,&vec![0.2;1326],1.).unwrap();
        assert_eq!(next.q,frozen.q);
        for (a,z) in next.seats.iter().zip(&frozen.seats){assert_eq!(a.mass,z.mass);}
    }
}

// ---------------------------------------------------------------------------------------------
// The start state and the public-range contract.
// ---------------------------------------------------------------------------------------------

/// Spec section 9.2's start: one branch, `q = 1`, uniform masses for every dealt seat, nothing
/// else; a vacant seat has no range.
#[test]
fn a_fresh_hand_replays_to_one_uniform_branch() {
    let store = full_store();
    let state = table(&cfg());
    let out = run(&store, &state);
    assert_eq!(out.branches.len(), 1);
    let b = &out.branches[0];
    assert_eq!((b.id, b.parent, b.split_by, b.q, b.residual, b.stopped.clone()), (0, None, None, 1.0, false, None));
    assert!(b.translated.is_empty());
    assert_eq!(b.seats.iter().map(|s| s.seat).collect::<Vec<_>>(), state.dealt);
    for s in &b.seats {
        assert!(s.mass.iter().all(|w| *w == 1.0), "seat {:?} starts uniform", s.seat);
    }
    // The seat to act (UTG) holds its next node, the root; nobody else has one yet.
    for s in &b.seats {
        let expected = (s.seat == UTG).then(|| key(vec![]));
        assert_eq!(s.node, expected, "seat {:?}", s.seat);
    }
    for i in 0..6 {
        assert_eq!(out.ranges[i], Some(uniform()), "seat {i}");
    }
    assert_eq!(out.log_reach, vec![0.0; 6]);
    assert!(out.folded_ranges.is_empty());
    assert!(out.reasons.is_empty(), "{:?}", out.reasons);
    assert_eq!(out.unsupported, None);

    // A three-handed table: the vacant seats have no range and log reach 0.
    let three = core_model::begin_hand(
        &cfg(),
        BeginHand { hand_id: 2, button: BTN, hero: SB, dealt: vec![SB, BB, BTN], stacks_start: vec![100 * UNIT; 3], hero_cards: None },
    )
    .expect("a three-handed table");
    let out = run(&store, &three);
    for i in 0..6usize {
        let dealt = [0usize, 1, 5].contains(&i);
        assert_eq!(out.ranges[i].is_some(), dealt, "seat {i}");
        assert_eq!(out.log_reach[i], 0.0, "seat {i}");
    }
    assert_eq!(out.branches[0].seats.len(), 3);
}

/// Hero's cards never enter a public range (spec sections 2 and 9.2), and hero's own actions
/// condition hero's public range exactly like any seat's: the replay is identical whoever hero
/// is and whatever hero holds.
#[test]
fn public_replay_ignores_hero_identity_and_cards() {
    let store = full_store();
    let mut line = open_call_folds();
    line.push(Action::Fold); // the BB folds: UTG and HJ see a flop
    let variants = [
        (BB, None),
        (UTG, Some(hand("AhKh"))), // hero is the raiser and holds a combo of its own public range
        (UTG, Some(hand("QsJs"))),
        (HJ, Some(hand("AsKs"))),
        (CO, Some(hand("3c3d"))),
    ];
    for (preflop, board) in [(true, ""), (false, "2c7dTh")] {
        let outs: Vec<ReplayOutput> = variants
            .iter()
            .map(|(hero, cards)| {
                let state = act(&table_with(&cfg(), *hero, *cards), &line);
                let state = if preflop { state } else { deal(&state, board) };
                run(&store, &state)
            })
            .collect();
        let first = &outs[0];
        for (out, variant) in outs.iter().zip(&variants).skip(1) {
            assert_eq!(out.ranges, first.ranges, "{variant:?}");
            assert_eq!(out.folded_ranges, first.folded_ranges, "{variant:?}");
            assert_eq!(
                out.log_reach.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                first.log_reach.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                "{variant:?}"
            );
            assert_eq!(
                out.branches.iter().map(fingerprint).collect::<Vec<_>>(),
                first.branches.iter().map(fingerprint).collect::<Vec<_>>(),
                "{variant:?}"
            );
            assert_eq!(out.reasons, first.reasons, "{variant:?}");
            assert_eq!(out.unsupported, first.unsupported, "{variant:?}");
        }
        // Hero's action conditioned hero's range: UTG (hero in two variants) holds AKs only.
        let utg = first.ranges[2].as_ref().expect("UTG is dealt");
        for c in 0..COMBOS {
            assert_eq!(utg.0[c] > 0.0, aks().contains(&c), "combo {c}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Each observed action is applied exactly once, and folds condition before they are retained.
// ---------------------------------------------------------------------------------------------

/// Spec section 8.4's on-menu update and rescale: the actor's marginal is conditioned exactly
/// once (log reach `ln(0.35)`, not `ln(0.35^2)`), and every other seat's marginal falls by the
/// integrated likelihood through `q` alone.
#[test]
fn each_action_conditions_once_and_rescales() {
    let store = full_store();
    let root = table(&cfg());
    let opened = act(&root, &[Action::Raise { to: 2500 }]);
    let raise = column(&store, &root, 0, 1);
    let m: f64 = raise.iter().sum::<f64>() / COMBOS as f64; // uniform masses: M = mean likelihood
    assert!((m - 4.0 * 0.35_f32 as f64 / COMBOS as f64).abs() < 1e-15, "the fixture's AKs-only open");

    let out = run(&store, &opened);
    assert_eq!(out.branches.len(), 1);
    let b = &out.branches[0];
    assert_eq!(b.id, 0, "an on-menu action updates the branch in place");
    assert_eq!(b.translated, vec![(UTG, Action::Raise { to: 2500 })]);
    close(b.q, m, "q = M");
    assert_eq!(out.ranges[2], Some(scaled(&raise)), "UTG's range is its raise column, once");
    close(out.log_reach[2], (0.35_f32 as f64).ln(), "UTG's removed maximum");
    for seat in [SB, BB, HJ, CO, BTN] {
        close(out.log_reach[seat.0 as usize], m.ln(), "a non-acting seat falls through q");
        assert_eq!(out.ranges[seat.0 as usize], Some(uniform()), "seat {seat:?}");
    }
    // HJ, next to act, holds the node after the open.
    assert_eq!(seat_mass(b, HJ).len(), COMBOS);
    assert_eq!(
        b.seats.iter().find(|s| s.seat == HJ).unwrap().node,
        Some(key(vec![(Position::Utg, raise_bb_x1000(2500))]))
    );

    // HJ calls: the same once-only rule, and UTG's marginal now falls through q.
    let called = act(&opened, &[Action::Call]);
    let call = column(&store, &opened, 1, 1);
    let out = run(&store, &called);
    let b = &out.branches[0];
    close(b.q, m * m, "q = M_UTG * M_HJ");
    assert_eq!(out.ranges[3], Some(scaled(&call)));
    let ln35 = (0.35_f32 as f64).ln();
    close(out.log_reach[2], ln35 + m.ln(), "UTG");
    close(out.log_reach[3], m.ln() + ln35, "HJ");
    for seat in [SB, BB, CO, BTN] {
        close(out.log_reach[seat.0 as usize], 2.0 * m.ln(), "a seat that has not acted");
    }
}

/// Spec section 9.2: "Folds condition the folder's range (`folded_ranges`)" -- the retained range
/// is the conditioned one, and the folder's masses stay in the branches for provenance.
#[test]
fn folding_conditions_the_folder_before_its_range_is_retained() {
    let store = full_store();
    let opened = act(&table(&cfg()), &[Action::Raise { to: 2500 }]);
    let folded = act(&opened, &[Action::Fold]);
    let fold = column(&store, &opened, 1, 0);
    let out = run(&store, &folded);
    assert_eq!(out.ranges[3], Some(scaled(&fold)), "HJ's range is its fold column");
    assert_ne!(out.ranges[3], Some(uniform()));
    assert_eq!(out.folded_ranges, vec![scaled(&fold)], "exactly the one folded dealt seat");
    let b = &out.branches[0];
    assert!(b.seats.iter().any(|s| s.seat == HJ), "the folder's SeatMass remains");
    // CO's node after UTG open / HJ fold is absent from the source: the branch stops there.
    let co_key = key(vec![(Position::Utg, raise_bb_x1000(2500)), (Position::Hj, PreflopStep::Fold)]);
    assert_eq!(out.branches[0].stopped, None, "CO has not acted yet");
    assert!(b.seats.iter().all(|s| s.node.is_none()), "CO has no node in this branch: {:?}", b.seats.iter().map(|s| &s.node).collect::<Vec<_>>());
    let co_folds = run(&store, &act(&folded, &[Action::Fold]));
    assert_eq!(co_folds.branches[0].stopped, Some(format!("missing node {co_key}")));
    assert_eq!(co_folds.folded_ranges.len(), 2, "HJ and CO, in seat order");
    assert_eq!(co_folds.folded_ranges[0], scaled(&fold));
    assert_eq!(co_folds.folded_ranges[1], uniform(), "CO folded without a node: unconditioned");
}

// ---------------------------------------------------------------------------------------------
// Missing nodes freeze the branch for the rest of the preflop street (spec section 9.3).
// ---------------------------------------------------------------------------------------------

/// Section 13.1's `replay_missing_continuation`, preflop half: the source answers the first action
/// (UTG's open) and a later key (the BB's node after open, call and folds), but not the middle
/// one (HJ's node, removed from the in-memory map). The branch keeps its pre-missing-action `q`
/// and every seat's masses through every remaining preflop action, no seat has a node, the
/// reason names the missing prefix, and the present later node is never used to resume it.
#[test]
fn replay_missing_continuation() {
    let hj_key = key(vec![(Position::Utg, raise_bb_x1000(2500))]);
    let bb_key = key(vec![
        (Position::Utg, raise_bb_x1000(2500)),
        (Position::Hj, PreflopStep::Call),
        (Position::Co, PreflopStep::Fold),
        (Position::Btn, PreflopStep::Fold),
        (Position::Sb, PreflopStep::Fold),
    ]);
    let store = store_without(&[hj_key.clone()]);
    let root = table(&cfg());
    let opened = act(&root, &[Action::Raise { to: 2500 }]);
    let at_bb = act(&root, &open_call_folds());
    let closed = act(&at_bb, &[Action::Call]);

    // The source answers the first action and the later key.
    assert!(store.query(&cfg(), &root, 0).node.is_some(), "UTG's root node");
    assert!(store.query(&cfg(), &at_bb, 5).node.is_some(), "the BB's node after open, call, folds: {bb_key}");
    assert!(store.query(&cfg(), &opened, 1).node.is_none(), "HJ's node was removed");

    let after_open = run(&store, &opened);
    for state in [&at_bb, &closed] {
        let out = run(&store, state);
        assert_eq!(out.branches.len(), 1);
        let b = &out.branches[0];
        let frozen = &after_open.branches[0];
        assert_eq!(b.stopped, Some(format!("missing node {hj_key}")));
        assert_eq!(b.q.to_bits(), frozen.q.to_bits(), "q keeps its pre-missing-action value");
        for (s, z) in b.seats.iter().zip(&frozen.seats) {
            assert_eq!(s.mass, z.mass, "seat {:?} keeps its pre-missing-action masses", s.seat);
            assert_eq!(s.node, None, "seat {:?} has no node in a stopped branch", s.seat);
        }
        assert_eq!(b.translated, frozen.translated, "the stopped branch's history is not advanced");
        assert_eq!(out.log_reach, after_open.log_reach, "nothing after the stop is conditioned or rescaled");
        let missing = unconditioned(Street::Preflop, HJ, &format!("missing node {hj_key}"));
        assert_eq!(out.reasons.iter().filter(|r| **r == missing).count(), 1, "{:?}", out.reasons);
        assert!(hj_key.contains("UTG") && hj_key.contains("2500"), "the reason names the missing prefix: {hj_key}");
        // No resumption at the present later node: the BB's own action (in `closed`) is not applied.
        assert_eq!(out.ranges[1], Some(uniform()), "the BB is unconditioned");
        assert_eq!(out.ranges[3], Some(uniform()), "HJ's call had no node to condition it");
        assert_eq!(out.unsupported, None, "a missing node is a coverage gap, not an unsupported hand");
    }

    // Control: with the middle node present, HJ's call conditions HJ, and the branch continues
    // until the next absent node (CO's, which the source genuinely lacks).
    let out = run(&full_store(), &at_bb);
    let b = &out.branches[0];
    let co_key = key(vec![(Position::Utg, raise_bb_x1000(2500)), (Position::Hj, PreflopStep::Call)]);
    assert_eq!(b.stopped, Some(format!("missing node {co_key}")));
    assert_eq!(out.ranges[3], Some(scaled(&column(&full_store(), &opened, 1, 1))), "HJ's call conditioned HJ");
    assert!(out.reasons.contains(&unconditioned(Street::Preflop, CO, &format!("missing node {co_key}"))));
}

/// Section 9.3's "other branches continue": in one transaction, a branch whose node is present
/// is conditioned while a sibling whose node is missing stops, frozen apart from the seat-common
/// rescale that every branch shares.
///
/// The synthetic source offers one raise size per node, so no observed line can split a branch
/// through the public walk; the two starting branches are therefore built with the kernel's own
/// `split_action` over the initial branch (UTG's open mapped to the source's 2.5 bb in one child
/// and to a 3 bb size the source does not have in the other). Every lookup and likelihood below
/// comes from the source through `apply_preflop_action`.
#[test]
fn a_branch_whose_node_is_present_continues_independently() {
    let store = full_store();
    let root = table(&cfg());
    let state = act(&root, &[Action::Raise { to: 2750 }, Action::Call]);
    let raise = column(&store, &root, 0, 1);
    let branches = split_action(
        &initial(&state.dealt),
        UTG,
        &[(Action::Raise { to: 2500 }, 0.6, raise.clone()), (Action::Raise { to: 3000 }, 0.4, raise.clone())],
    );
    assert_eq!(branches.iter().map(|b| b.id).collect::<Vec<_>>(), vec![1, 2]);
    let mut out = ReplayOutput {
        ranges: vec![None; 6],
        branches,
        folded_ranges: vec![],
        log_reach: vec![0.0; 6],
        reasons: vec![],
        unsupported: None,
    };
    let before = out.clone();
    let input = ReplayInput { cfg: &state.config, state: &state, store: &store, snapshots: &[] };
    assert!(apply_preflop_action(&input, &mut out, 1, HJ, &Action::Call));
    assert_eq!(out.branches.len(), 2);
    let present = out.branches.iter().find(|b| b.id == 1).expect("the 2.5 bb branch");
    let missing = out.branches.iter().find(|b| b.id == 2).expect("the 3 bb branch");

    // The present branch: HJ conditioned by its call column, q times M.
    let call = column(&store, &act(&root, &[Action::Raise { to: 2500 }]), 1, 1);
    let m: f64 = call.iter().sum::<f64>() / COMBOS as f64;
    close(present.q, before.branches[0].q * m, "q_A *= M");
    assert_eq!(present.stopped, None);
    assert_eq!(present.translated, vec![(UTG, Action::Raise { to: 2500 }), (HJ, Action::Call)]);
    for c in 0..COMBOS {
        assert_eq!(seat_mass(present, HJ)[c] > 0.0, call[c] > 0.0, "combo {c}");
    }

    // The missing branch: frozen q, masses frozen up to each seat's common rescale factor.
    assert_eq!(missing.q.to_bits(), before.branches[1].q.to_bits());
    assert!(missing.stopped.as_deref().is_some_and(|s| s.starts_with("missing node") && s.contains("3000")), "{:?}", missing.stopped);
    assert!(missing.seats.iter().all(|s| s.node.is_none()));
    assert_eq!(missing.translated, before.branches[1].translated);
    for (s, z) in missing.seats.iter().zip(&before.branches[1].seats) {
        let factor = (out.log_reach[s.seat.0 as usize] - before.log_reach[s.seat.0 as usize]).exp();
        for c in 0..COMBOS {
            close(s.mass[c] * factor, z.mass[c], "a stopped branch's mass up to the seat-common rescale");
        }
    }
    assert!(out.reasons.iter().any(|r| matches!(r,
        ApproxReason::UnconditionedPriorStreet { street: Street::Preflop, seat, cause }
            if *seat == HJ && cause.starts_with("missing node") && cause.contains("3000"))), "{:?}", out.reasons);
    // Equal totals survive: every seat's mass total is the same in both branches.
    for seat in &state.dealt {
        let totals: Vec<f64> = out.branches.iter().map(|b| seat_mass(b, *seat).iter().sum()).collect();
        close(totals[0], totals[1], "equal-total invariant");
    }
}

// ---------------------------------------------------------------------------------------------
// Zero support (spec section 9.2).
// ---------------------------------------------------------------------------------------------

/// HJ 3bets to the source's own 8.75 bb, which the source plays with probability 0 for every
/// class: the update is rejected, every `q` and mass keeps its pre-action value, and the reason
/// is recorded. The 3bet is on the menu, so the branch's path still advances along it without a
/// guessed action, and the CO's (absent) node after the 3bet is where the branch stops.
#[test]
fn zero_support_rejects_the_update_and_keeps_every_weight() {
    let store = full_store();
    let root = table(&cfg());
    let opened = act(&root, &[Action::Raise { to: 2500 }]);
    let three_bet = act(&opened, &[Action::Raise { to: 8750 }]);
    let after_open = run(&store, &opened);

    let mut out = after_open.clone();
    out.ranges = vec![None; 6];
    let input = ReplayInput { cfg: &three_bet.config, state: &three_bet, store: &store, snapshots: &[] };
    let before = out.clone();
    assert!(!apply_preflop_action(&input, &mut out, 1, HJ, &Action::Raise { to: 8750 }), "the update is rejected");
    assert_eq!(out.branches.len(), 1);
    assert_eq!(out.branches[0].q.to_bits(), before.branches[0].q.to_bits());
    for (s, z) in out.branches[0].seats.iter().zip(&before.branches[0].seats) {
        assert_eq!(s.mass, z.mass, "seat {:?}", s.seat);
    }
    assert_eq!(out.log_reach, before.log_reach);
    assert_eq!(out.reasons.last(), Some(&zero_reason(Street::Preflop, HJ, &Action::Raise { to: 8750 })));
    assert_eq!(
        out.reasons.last(),
        Some(&unconditioned(Street::Preflop, HJ, "zero support after Raise { to: 8750 }"))
    );
    assert_eq!(out.branches[0].stopped, None, "an on-menu rejection does not stop the branch");
    assert_eq!(
        out.branches[0].translated,
        vec![(UTG, Action::Raise { to: 2500 }), (HJ, Action::Raise { to: 8750 })],
        "the path advances along the one mapped action, with no likelihood"
    );

    // The whole walk: the CO then folds at an absent node, and the branch stops there.
    let co_folds = act(&three_bet, &[Action::Fold]);
    let walked = run(&store, &co_folds);
    let co_key = key(vec![(Position::Utg, raise_bb_x1000(2500)), (Position::Hj, raise_bb_x1000(8750))]);
    assert_eq!(walked.branches[0].stopped, Some(format!("missing node {co_key}")));
    assert_eq!(walked.branches[0].q.to_bits(), after_open.branches[0].q.to_bits());
    assert_eq!(walked.ranges[3], Some(uniform()), "HJ's rejected 3bet left HJ unconditioned");
    assert!(walked.reasons.contains(&zero_reason(Street::Preflop, HJ, &Action::Raise { to: 8750 })));
}

/// An off-menu wager whose every mapped child has zero support: the update is rejected, and the
/// branch -- which could only advance along an unobserved menu size -- stops, keeping its `q` and
/// masses.
#[test]
fn a_rejected_translation_stops_the_branch_without_a_guessed_size() {
    let store = full_store();
    let root = table(&cfg());
    // A min open (2 bb) is below the source's only size: clamped to 2.5 bb.
    let opened = act(&root, &[Action::Raise { to: 2000 }]);
    let raised = act(&opened, &[Action::Raise { to: 4000 }]);
    let after_open = run(&store, &opened);
    assert_eq!(after_open.branches.len(), 1);
    let child = &after_open.branches[0];
    assert_eq!((child.id, child.parent, child.split_by), (1, Some(0), Some(UTG)), "the translated open is a split child");
    assert_eq!(child.translated, vec![(UTG, Action::Raise { to: 2500 })]);
    let bt = after_open.reasons.iter().find_map(|r| match r {
        ApproxReason::BetTranslation { street: Street::Preflop, seat, observed_pct, mapped, deviation, prominent } if *seat == UTG => {
            Some((*observed_pct, mapped.clone(), *deviation, *prominent))
        }
        _ => None,
    });
    let (observed, mapped, deviation, prominent) = bt.expect("the open's translation is disclosed");
    assert!((observed - 0.4).abs() < 1e-6 && mapped.len() == 1 && (mapped[0].0 - 0.6).abs() < 1e-6 && mapped[0].1 == 1.0);
    assert!((deviation - 0.2).abs() < 1e-6 && prominent);

    let mut out = after_open.clone();
    let before = out.clone();
    let input = ReplayInput { cfg: &raised.config, state: &raised, store: &store, snapshots: &[] };
    assert!(!apply_preflop_action(&input, &mut out, 1, HJ, &Action::Raise { to: 4000 }));
    let b = &out.branches[0];
    assert_eq!(b.stopped, Some("zero support after Raise { to: 4000 }".into()));
    assert_eq!(b.q.to_bits(), before.branches[0].q.to_bits());
    for (s, z) in b.seats.iter().zip(&before.branches[0].seats) {
        assert_eq!(s.mass, z.mass);
        assert_eq!(s.node, None);
    }
    assert_eq!(b.translated, before.branches[0].translated, "no unobserved menu action is appended");
    assert!(out.reasons.iter().any(|r| matches!(r, ApproxReason::BetTranslation { seat, .. } if *seat == HJ)));
    assert_eq!(out.reasons.last(), Some(&zero_reason(Street::Preflop, HJ, &Action::Raise { to: 4000 })));
}

// ---------------------------------------------------------------------------------------------
// Translated prefixes (spec section 8.4): the branch's mapped history drives every later lookup.
// ---------------------------------------------------------------------------------------------

/// UTG opens to 3 bb, off the source's only size: the open is translated (clamped to 2.5 bb,
/// disclosed with its deviation), and HJ's next node is looked up under the mapped 2.5 bb
/// history, which the observed prefix alone cannot reach.
#[test]
fn an_off_menu_open_translates_and_looks_up_under_the_mapped_size() {
    let store = full_store();
    let root = table(&cfg());
    let opened = act(&root, &[Action::Raise { to: 3000 }]);
    let called = act(&opened, &[Action::Call]);
    let hj_key = key(vec![(Position::Utg, raise_bb_x1000(2500))]);

    let out = run(&store, &opened);
    let b = &out.branches[0];
    assert_eq!((b.id, b.parent, b.split_by), (1, Some(0), Some(UTG)));
    assert_eq!(b.translated, vec![(UTG, Action::Raise { to: 2500 })]);
    assert_eq!(out.ranges[2], Some(scaled(&column(&store, &root, 0, 1))), "clamped with f = 1: the 2.5 bb column");
    assert!(out.reasons.iter().any(|r| matches!(r,
        ApproxReason::BetTranslation { street: Street::Preflop, seat, observed_pct, mapped, deviation, prominent: true }
            if *seat == UTG && (observed_pct - 0.8).abs() < 1e-6 && mapped.len() == 1
                && (mapped[0].0 - 0.6).abs() < 1e-6 && mapped[0].1 == 1.0 && (deviation - 0.2).abs() < 1e-6)),
        "{:?}", out.reasons);
    assert_eq!(b.seats.iter().find(|s| s.seat == HJ).unwrap().node, Some(hj_key.clone()), "HJ's next node is under the mapped size");

    // The observed prefix alone is off-menu; the branch's translated history finds the node.
    let observed = store.query(&cfg(), &called, 1);
    assert!(observed.node.is_none() && observed.key.contains("off-menu"), "{}", observed.key);
    let translated = query_translated(&store, &cfg(), &called, 1, b);
    assert_eq!(translated.key, hj_key);
    assert_eq!(translated.node.as_ref().map(|n| n.actor), Some(Position::Hj));
    assert!(translated.expanded.is_some());
    assert_eq!(translated.actor, Some(HJ));

    // HJ's call is then conditioned at that node.
    let out = run(&store, &called);
    assert_eq!(out.branches[0].translated, vec![(UTG, Action::Raise { to: 2500 }), (HJ, Action::Call)]);
    assert_eq!(out.ranges[3], Some(scaled(&column(&store, &act(&root, &[Action::Raise { to: 2500 }]), 1, 1))));
}

/// At a three-chip big blind the source's 2.5 bb open is 7.5 chips, so both a 7-chip and an
/// 8-chip open are on the menu (within half a chip). The branch keeps the OBSERVED open (spec
/// section 8.4: only translated wagers are replaced; ruling 13-R1: the observed on-menu history
/// stays faithful), and its next node is looked up under the source's own 2500, which a thousandth
/// conversion of 8 chips (2667) would miss: the history is resolved against the source's menus,
/// like the observed prefix (P3.T8 R3).
#[test]
fn translated_sizes_resolve_against_the_source_menu() {
    let store = full_store();
    let cfg = config_at(3);
    let root = table(&cfg);
    assert_eq!(core_preflop::to_source_step(&Action::Raise { to: 8 }, 3), raise_bb_x1000(2667), "the naive conversion");
    for open in [7u32, 8] {
        // Both 7 and 8 chips are within half a chip of 7.5: on the menu, kept as observed.
        let called = act(&root, &[Action::Raise { to: open }, Action::Call]);
        let out = run(&store, &called);
        assert_eq!(out.branches.len(), 1);
        let b = &out.branches[0];
        assert_eq!(b.id, 0, "{open}: an on-menu open is not a split");
        assert_eq!(b.translated, vec![(UTG, Action::Raise { to: open }), (HJ, Action::Call)], "{open}");
        assert!(!out.reasons.iter().any(|r| matches!(r, ApproxReason::BetTranslation { .. })), "{open}: {:?}", out.reasons);
        let call = column(&store, &act(&root, &[Action::Raise { to: 8 }]), 1, 1);
        assert_eq!(out.ranges[3], Some(scaled(&call)), "{open}: HJ conditioned at the source's 2500 node");
    }
}

/// Behind a UTG straddle the source unit is the straddle and every seat is looked up under its
/// virtual role (spec section 8.3): the HJ seat opens as the virtual UTG and the CO seat calls as
/// the virtual HJ. The mapping reason is recorded once for the replay, not once per lookup.
#[test]
fn a_straddled_hand_replays_under_virtual_roles() {
    let store = full_store();
    let unit = 2000;
    let cfg = HandConfig {
        straddle: Some(proto::UtgStraddle { amount_chips: unit }),
        rake: Rake::PotRake { rate: 0.05, cap_mchips: unit * 500, no_flop_no_drop: true },
        ..config_at(1000)
    };
    let root = core_model::begin_hand(
        &cfg,
        BeginHand { hand_id: 3, button: BTN, hero: BB, dealt: (0..6).map(Seat).collect(), stacks_start: vec![100 * unit; 6], hero_cards: None },
    )
    .expect("a straddled six-max table");
    let opened = act(&root, &[Action::Raise { to: 5000 }]); // the HJ seat, 2.5 straddles
    assert_eq!(opened.actions[0].seat, HJ);
    let called = act(&opened, &[Action::Call]); // the CO seat
    let out = run(&store, &called);
    assert_eq!(out.branches.len(), 1);
    let b = &out.branches[0];
    assert_eq!(b.stopped, None);
    assert_eq!(b.translated, vec![(HJ, Action::Raise { to: 5000 }), (CO, Action::Call)]);
    assert_eq!(out.ranges[3], Some(scaled(&column(&store, &root, 0, 1))), "the HJ seat conditioned as the virtual UTG");
    assert_eq!(out.ranges[4], Some(scaled(&column(&store, &opened, 1, 1))), "the CO seat conditioned as the virtual HJ");
    let straddle = ApproxReason::StraddleMapped { posts: [0.25, 0.5, 1.0] };
    assert_eq!(out.reasons.iter().filter(|r| **r == straddle).count(), 1, "{:?}", out.reasons);
    // The BTN seat (the virtual CO) is next; the source has no node for it after open and call.
    assert!(b.seats.iter().all(|s| s.node.is_none()));
    let stopped = run(&store, &act(&called, &[Action::Fold]));
    let btn_key = key(vec![(Position::Utg, raise_bb_x1000(2500)), (Position::Hj, PreflopStep::Call)]);
    assert_eq!(stopped.branches[0].stopped, Some(format!("missing node {btn_key}")));
    assert!(stopped.reasons.contains(&unconditioned(Street::Preflop, BTN, &format!("missing node {btn_key}"))));
}

/// The per-invocation mapping memo is keyed by the observed prefix index and the translated
/// history (never by final-hand flags): one computation per distinct pair.
#[test]
fn translated_lookups_are_memoized_per_prefix_and_history() {
    let store = full_store();
    let called = act(&table(&cfg()), &[Action::Raise { to: 3000 }, Action::Call]);
    let mapped = vec![(UTG, Action::Raise { to: 2500 })];
    let mut memo = core_preflop::PreflopInvocation::new();
    let first = memo.answer_history(&store, &cfg(), &called, 1, &mapped);
    let second = memo.answer_history(&store, &cfg(), &called, 1, &mapped);
    assert_eq!(first, second);
    assert_eq!(first, store.query_history(&cfg(), &called, 1, &mapped));
    assert_eq!((memo.lookups(), memo.hits(), memo.len()), (2, 1, 1));
    // The observed prefix at the same index is a different mapping.
    let observed = memo.answer(&store, &cfg(), &called, 1);
    assert_ne!(observed.key, first.key);
    assert_eq!((memo.lookups(), memo.hits(), memo.len()), (3, 1, 2));
    // A translated history that does not follow the observed actors is a typed refusal.
    let wrong = store.query_history(&cfg(), &called, 1, &[(HJ, Action::Raise { to: 2500 })]);
    assert!(matches!(wrong.unsupported, Some(UnsupportedReason::UnsupportedHistory { .. })), "{wrong:?}");
    assert!(wrong.node.is_none());
}

// ---------------------------------------------------------------------------------------------
// Street roots: board blocking on the output marginal only (spec section 9.2, revision 6 S13).
// ---------------------------------------------------------------------------------------------

/// Two branches with different UTG mass shapes, blocked by a board that removes every AKs combo:
/// the per-branch masses are never zeroed, each seat's masses are divided by one blocked maximum,
/// the equal-total invariant survives, and the published range is zero exactly on board combos.
///
/// The two branches are kernel scaffolding (`split_action` over the initial branch with the
/// root's raise and fold columns at 0.9/0.1), so that AKs carries UTG's maximum and the board
/// moves it.
#[test]
fn block_and_rescale_keeps_equal_totals_and_blocks_only_the_marginal() {
    let store = full_store();
    let root = table(&cfg());
    let raise = column(&store, &root, 0, 1);
    let fold = column(&store, &root, 0, 0);
    let mut branches = split_action(
        &initial(&root.dealt),
        UTG,
        &[(Action::Raise { to: 2500 }, 0.9, raise.clone()), (Action::Fold, 0.1, fold.clone())],
    );
    let mut log_reach = vec![0.0; 6];
    rescale(&mut branches, &mut log_reach);
    let mut out = ReplayOutput { ranges: vec![None; 6], branches, folded_ranges: vec![], log_reach, reasons: vec![], unsupported: None };
    let board: Vec<Card> = core_model::parse_cards("AsAhAdKc").unwrap();
    let before = out.clone();
    let mask = board_mask(&board);
    let blocked: Vec<usize> = (0..COMBOS).filter(|&c| mask.0[c] == 0.0).collect();
    assert_eq!(blocked.len(), 4 * 51 - 6, "four board cards block 198 combos");
    for c in aks() {
        assert_eq!(mask.0[c], 0.0, "every AKs combo holds a board card");
    }

    // The expected factor for UTG: the maximum of its marginal over unblocked combos.
    let r = marginal(&before.branches, UTG);
    let m = (0..COMBOS).filter(|c| mask.0[*c] != 0.0).map(|c| r[c]).fold(0.0_f64, f64::max);
    assert!(m < 1.0, "AKs carried UTG's maximum, and the board removes it: {m}");

    block_and_rescale(&mut out, &board);
    assert_eq!(out.unsupported, None);
    close(out.log_reach[2] - before.log_reach[2], m.ln(), "UTG's removed blocked maximum");
    for seat in &root.dealt {
        let totals: Vec<f64> = out.branches.iter().map(|b| seat_mass(b, *seat).iter().sum()).collect();
        close(totals[0], totals[1], "equal totals across branches after blocking");
        let r = marginal(&out.branches, *seat);
        let peak = (0..COMBOS).filter(|c| mask.0[*c] != 0.0).map(|c| r[c]).fold(0.0_f64, f64::max);
        close(peak, 1.0, "the blocked marginal has maximum 1");
    }
    for (b, z) in out.branches.iter().zip(&before.branches) {
        assert_eq!(b.q.to_bits(), z.q.to_bits(), "q is never rescaled");
        for c in &blocked {
            assert_eq!(seat_mass(b, UTG)[*c] > 0.0, seat_mass(z, UTG)[*c] > 0.0, "combo {c}: a board combo keeps its mass");
        }
    }
    let as_ks = proto::combo_index(Card::parse("As").unwrap(), Card::parse("Ks").unwrap()) as usize;
    assert!(seat_mass(&out.branches[0], UTG)[as_ks] > 0.0, "AsKs keeps its per-branch mass");

    // Publishing at a turn with that board zeroes exactly the board combos.
    let mut line = open_call_folds();
    line.push(Action::Fold);
    let flop = deal(&act(&root, &line), "AsAhAd");
    let turn = deal(&act(&flop, &[Action::Check, Action::Check]), "AsAhAdKc");
    publish(&mut out, &turn);
    let utg = out.ranges[2].as_ref().unwrap();
    for c in 0..COMBOS {
        if mask.0[c] == 0.0 {
            assert_eq!(utg.0[c], 0.0, "combo {c} is on the board");
        }
    }
    assert!(utg.0[as_ks] == 0.0 && out.ranges[0].as_ref().unwrap().0.iter().filter(|w| **w > 0.0).count() == COMBOS - blocked.len());
    assert_eq!(out.folded_ranges.len(), 4, "SB, BB, CO and BTN folded");
}

/// A seat whose every positive combo is blocked by the board has no public range at that root:
/// `InvalidRanges`.
#[test]
fn a_seat_blocked_out_of_its_range_is_invalid() {
    let store = full_store();
    let mut line = open_call_folds();
    line.push(Action::Fold);
    let flop = deal(&act(&table(&cfg()), &line), "AsAhAd"); // AKs: only AcKc survives
    let checked = act(&flop, &[Action::Check, Action::Check]);
    let at_flop = run(&store, &flop);
    assert_eq!(at_flop.unsupported, None);
    let ac_kc = proto::combo_index(Card::parse("Ac").unwrap(), Card::parse("Kc").unwrap()) as usize;
    assert_eq!(at_flop.ranges[2].as_ref().unwrap().0.iter().filter(|w| **w > 0.0).count(), 1);
    assert_eq!(at_flop.ranges[2].as_ref().unwrap().0[ac_kc], 1.0);
    let turn = deal(&checked, "AsAhAdKc");
    let out = run(&store, &turn);
    assert_eq!(out.unsupported, Some(UnsupportedReason::InvalidRanges));
}

// ---------------------------------------------------------------------------------------------
// Completed streets without snapshots, and the preflop stop's scope.
// ---------------------------------------------------------------------------------------------

/// A completed postflop street with no snapshot (none registered; hero folded preflop, so no hero
/// decision was ever solved on it): `UnconditionedPriorStreet{cause: "no compatible snapshot"}` per
/// actual acting seat, the preflop masses retained, each root blocked in order. The preflop stop
/// clears on entering the flop (section 9.3 scopes it to the preflop street).
#[test]
fn a_completed_street_falls_back_unconditioned_and_blocks_each_root() {
    let store = full_store();
    let mut line = open_call_folds();
    line.push(Action::Fold);
    let closed = act(&table(&cfg()), &line);
    let flop = deal(&closed, "2c7dTh");
    let checked = act(&flop, &[Action::Check, Action::Check]);
    let turn = deal(&checked, "2c7dThJs");

    // Preflop, the branch stopped at CO's absent node; the stop is scoped to preflop.
    let preflop = run(&store, &closed);
    assert!(preflop.branches[0].stopped.is_some(), "stopped while the hand is still preflop");
    for state in [&flop, &turn] {
        let out = run(&store, state);
        assert_eq!(out.branches[0].stopped, None, "a preflop stop clears on entering a postflop street");
        assert!(!out.branches[0].residual);
        assert_eq!(out.branches[0].q.to_bits(), preflop.branches[0].q.to_bits(), "no postflop likelihood");
    }

    let out = run(&store, &turn);
    let flop_reasons: Vec<&ApproxReason> =
        out.reasons.iter().filter(|r| matches!(r, ApproxReason::UnconditionedPriorStreet { street: Street::Flop, .. })).collect();
    assert_eq!(
        flop_reasons,
        vec![
            &unconditioned(Street::Flop, UTG, "no compatible snapshot"),
            &unconditioned(Street::Flop, HJ, "no compatible snapshot")
        ]
    );
    assert!(!out.reasons.iter().any(|r| matches!(r, ApproxReason::UnconditionedPriorStreet { street: Street::Turn, .. })), "the current street is not replayed");
    // The masses are the preflop masses: blocking never zeroes a per-branch mass.
    let board = core_model::parse_cards("2c7dThJs").unwrap();
    let mask = board_mask(&board);
    for s in &out.branches[0].seats {
        for c in 0..COMBOS {
            let pre = seat_mass(&preflop.branches[0], s.seat)[c];
            assert_eq!(s.mass[c] > 0.0, pre > 0.0, "seat {:?} combo {c}", s.seat);
        }
    }
    // Published ranges: board combos zero; AKs untouched by this board.
    for i in 0..6 {
        let r = out.ranges[i].as_ref().unwrap();
        for c in 0..COMBOS {
            if mask.0[c] == 0.0 {
                assert_eq!(r.0[c], 0.0, "seat {i} combo {c}");
            }
        }
    }
    for c in aks() {
        assert_eq!(out.ranges[2].as_ref().unwrap().0[c], 1.0);
    }
    assert_eq!(out.folded_ranges, vec![out.ranges[0].clone().unwrap(), out.ranges[1].clone().unwrap(), out.ranges[4].clone().unwrap(), out.ranges[5].clone().unwrap()]);
    assert_eq!(out.unsupported, None);
}

/// P3.T15: a completed street without a compatible heads-up snapshot keeps every mass the earlier
/// streets left and names why, for each seat that acted on it, in order of first action. Played
/// three-way from its root with no hero decision ever heads-up, the cause is `multiway prior
/// street`; heads-up with hero deciding but nothing registered, `no compatible snapshot` (replay
/// holds no engine failure, deadline or no-request provenance to be more specific). No preflop
/// source: every seat enters the flop uniform.
#[test]
fn a_completed_street_without_a_snapshot_names_multiway_or_no_snapshot() {
    let cfg = config_at(10);
    let none = PreflopStore::from_sources(vec![]);
    let seated = table_with(&cfg, BB, Some(hand("AsAd")));

    // UTG and HJ fold, CO limps, BTN folds, SB completes, BB checks: SB, BB and CO see the flop.
    let limped = act(&seated, &[Action::Fold, Action::Fold, Action::Call, Action::Fold, Action::Call, Action::Check]);
    let flop = deal(&limped, "Kh7d2c");
    let turn = deal(&act(&flop, &[Action::Check, Action::Check, Action::Check]), "Kh7d2c4c");
    let out = run(&none, &turn);
    let multiway = |seat| unconditioned(Street::Flop, seat, "multiway prior street");
    let flop_reasons: Vec<&ApproxReason> =
        out.reasons.iter().filter(|r| matches!(r, ApproxReason::UnconditionedPriorStreet { street: Street::Flop, .. })).collect();
    assert_eq!(flop_reasons, vec![&multiway(SB), &multiway(BB), &multiway(CO)]);
    let at_root = run(&none, &flop);
    assert_eq!(out.branches.len(), 1);
    assert_eq!(out.branches[0].q.to_bits(), at_root.branches[0].q.to_bits(), "nothing is conditioned");
    for (s, z) in out.branches[0].seats.iter().zip(&at_root.branches[0].seats) {
        assert_eq!(s.mass, z.mass, "seat {:?} keeps its flop-root masses", s.seat);
    }
    assert_eq!(out.log_reach, at_root.log_reach);

    // Heads-up: SB raises to 50, BB (hero) calls, both check the flop; nothing was registered.
    let heads_up = act(&seated, &[Action::Fold, Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 50 }, Action::Call]);
    let turn = deal(&act(&deal(&heads_up, "Kh7d2c"), &[Action::Check, Action::Check]), "Kh7d2c4c");
    let out = run(&none, &turn);
    let flop_reasons: Vec<&ApproxReason> =
        out.reasons.iter().filter(|r| matches!(r, ApproxReason::UnconditionedPriorStreet { street: Street::Flop, .. })).collect();
    assert_eq!(
        flop_reasons,
        vec![&unconditioned(Street::Flop, SB, "no compatible snapshot"), &unconditioned(Street::Flop, BB, "no compatible snapshot")]
    );
}

// ---------------------------------------------------------------------------------------------
// Input contract and snapshot records.
// ---------------------------------------------------------------------------------------------

/// `apply_preflop_action`'s explicit seat and action must be the recorded action at that prefix:
/// an always-on assertion, never a silently mismatched update.
#[test]
#[should_panic(expected = "apply_preflop_action: prefix 1 records")]
fn apply_preflop_action_refuses_an_action_that_was_not_recorded() {
    let store = full_store();
    let state = act(&table(&cfg()), &[Action::Raise { to: 2500 }, Action::Call]);
    let mut out = run(&store, &act(&table(&cfg()), &[Action::Raise { to: 2500 }]));
    let input = ReplayInput { cfg: &state.config, state: &state, store: &store, snapshots: &[] };
    let _ = apply_preflop_action(&input, &mut out, 1, HJ, &Action::Fold);
}

/// The snapshot records are plain serde data (spec section 9.1), for Task 14's store.
#[test]
fn snapshot_records_round_trip_through_serde() {
    let snapshot = StreetSnapshot {
        key: SnapshotKey {
            hand_id: 9,
            config_revision: 7,
            model_revision: 3,
            street: Street::Flop,
            root_board: core_model::parse_cards("2c7dTh").unwrap(),
            root_range_hashes: [[1; 32], [2; 32]],
            tree_signature: "sig".into(),
        },
        provenance: SnapshotProvenance {
            identity_at_solve: proto::DecisionIdentity { hand_id: 9, hand_revision: 4, decision_id: 2, config_revision: 7, model_revision: 3 },
            solved_prefix: vec![(UTG, Action::Check)],
            origin: "live".into(),
        },
        tree: proto::EffectiveTree {
            rules_version: 1,
            template_id: "t".into(),
            root_street: Street::Flop,
            menus: BTreeMap::new(),
            add_allin_threshold: 1.5,
            force_allin_threshold: 0.15,
            merging_threshold: 0.1,
            wager_cap: 3,
            inserted: vec![],
            materialized: vec![],
        },
        nodes: vec![],
        covered_paths: vec![vec![], vec![0]],
        exploitability_chips: 0.5,
        reasons: vec![ApproxReason::ChartRounded],
    };
    let text = serde_json::to_string(&snapshot).expect("serializes");
    let back: StreetSnapshot = serde_json::from_str(&text).expect("deserializes");
    assert_eq!(back, snapshot);
    let copy = snapshot.clone();
    assert_eq!(format!("{copy:?}"), format!("{snapshot:?}"));
}

// ---------------------------------------------------------------------------------------------
// P3.T13 fix round 1 (task-13-review.md R1-R3 and Q4): in-memory multi-size sources. Each source
// holds exactly the nodes listed (under the synthetic bundle's own manifest); nothing is invented
// for a key of the committed fixture.
// ---------------------------------------------------------------------------------------------

type MemNodes = Vec<(Vec<(Position, PreflopStep)>, PreflopNode)>;

/// A class-major probability table for a node with `n` actions: every entry positive, the pattern
/// distinct per `seed`, each class's row summing to 1 (in `f32`).
fn mixed(seed: usize, n: usize) -> Vec<Vec<f32>> {
    (0..169)
        .map(|c| {
            let w: Vec<f32> = (0..n).map(|a| 1.0 + ((c * (a + seed + 1) + 3 * a + seed) % 7) as f32).collect();
            let t: f32 = w.iter().sum();
            w.iter().map(|x| x / t).collect()
        })
        .collect()
}

/// An in-memory node with no EV and no unreachable class.
fn mem_node(actor: Position, actions: Vec<PreflopStep>, seed: usize) -> PreflopNode {
    let n = actions.len();
    PreflopNode {
        actor,
        actions,
        probs: mixed(seed, n),
        ev_source_sb: None,
        unreachable: [false; 169],
        committed_by_actor_sb: 0.0,
        fold_wide_verified: false,
    }
}

/// A source holding exactly `nodes`, each at its history, under the synthetic bundle's manifest
/// (100 bb, its rake profile).
fn store_of(nodes: MemNodes) -> PreflopStore {
    let (info, _) = synthetic_nodes();
    let nodes = nodes.into_iter().map(|(history, node)| (key(history), node)).collect();
    PreflopStore::from_sources(vec![Box::new(PokerDataJson { info, nodes })])
}

/// Column `a` of the in-memory node at `history`, expanded from its 169 classes to 1326 combos.
fn column_at(nodes: &MemNodes, history: &[(Position, PreflopStep)], a: usize) -> Vec<f64> {
    let node = &nodes.iter().find(|(h, _)| h.as_slice() == history).expect("the in-memory source holds this history").1;
    let mut col = vec![0.0; COMBOS];
    for class in 0..169u8 {
        for c in proto::class_combos(class) {
            col[usize::from(c)] = f64::from(node.probs[usize::from(class)][a]);
        }
    }
    col
}

/// A fold followed by raises to the given sizes (thousandths of a source unit).
fn raise_menu(sizes: &[u32]) -> Vec<PreflopStep> {
    std::iter::once(PreflopStep::Fold).chain(sizes.iter().map(|&s| raise_bb_x1000(s))).collect()
}

/// The integrated likelihood under uniform masses: the column's mean.
fn mean(p: &[f64]) -> f64 {
    p.iter().sum::<f64>() / COMBOS as f64
}

/// A published range equal to `unscaled / max(unscaled)`, at the output's `f32` precision.
fn assert_range_close(actual: &Option<Range1326>, unscaled: &[f64], what: &str) {
    let r = actual.as_ref().unwrap_or_else(|| panic!("{what}: a dealt seat has a range"));
    let max = unscaled.iter().copied().fold(0.0_f64, f64::max);
    for c in 0..COMBOS {
        let want = unscaled[c] / max;
        assert!((f64::from(r.0[c]) - want).abs() <= 1e-6, "{what}: combo {c}: {} != {want}", r.0[c]);
    }
}

/// Two likelihood columns that no published range could confuse at `f32` precision.
fn assert_columns_differ(a: &[f64], b: &[f64]) {
    let (ma, mb) = (a.iter().copied().fold(0.0, f64::max), b.iter().copied().fold(0.0, f64::max));
    assert!(a.iter().zip(b).any(|(x, y)| (x / ma - y / mb).abs() > 1e-3), "the continuation columns must differ");
}

/// UTG opens 2.5 or 2.6 bb; HJ's continuation after each size is its own node, with its own call
/// column.
fn sub_chip_nodes() -> MemNodes {
    let hj = || vec![PreflopStep::Fold, PreflopStep::Call];
    vec![
        (vec![], mem_node(Position::Utg, raise_menu(&[2500, 2600]), 1)),
        (vec![(Position::Utg, raise_bb_x1000(2500))], mem_node(Position::Hj, hj(), 2)),
        (vec![(Position::Utg, raise_bb_x1000(2600))], mem_node(Position::Hj, hj(), 5)),
    ]
}

/// UTG opens 2.5 or 3.5 bb; HJ faces each size with its own raise menu: 6 or 9 bb after 2.5, 7 or
/// 12 bb after 3.5.
fn ladder_nodes() -> MemNodes {
    let hj = |sizes: &[u32]| [PreflopStep::Fold, PreflopStep::Call].into_iter().chain(sizes.iter().map(|&s| raise_bb_x1000(s))).collect();
    vec![
        (vec![], mem_node(Position::Utg, raise_menu(&[2500, 3500]), 1)),
        (vec![(Position::Utg, raise_bb_x1000(2500))], mem_node(Position::Hj, hj(&[6000, 9000]), 2)),
        (vec![(Position::Utg, raise_bb_x1000(3500))], mem_node(Position::Hj, hj(&[7000, 12000]), 3)),
    ]
}

/// Ruling 13-R1 (task-13-review.md R1): a source offering 2.5 and 2.6 bb at a three-chip big
/// blind. A 7-chip open is exactly the 2500 edge (distances 500 and 800 in `to * 1000 - source *
/// unit`); an 8-chip open is exactly 2600's (the closer of two sizes within half a chip). Each
/// open keeps its observed chip amount in the branch, so HJ's next node is the continuation of the
/// edge the open was conditioned with -- never the other size's, which rounding 7.5 bb up to 8
/// chips selected before.
#[test]
fn an_exact_sub_chip_open_navigates_its_own_source_edge() {
    let nodes = sub_chip_nodes();
    let store = store_of(nodes.clone());
    let cfg = config_at(3);
    let root = table(&cfg);
    let hj_call = |edge: u32| column_at(&nodes, &[(Position::Utg, raise_bb_x1000(edge))], 1);
    assert_columns_differ(&hj_call(2500), &hj_call(2600));
    for (open, edge, raise) in [(7u32, 2500u32, 1usize), (8, 2600, 2)] {
        let called = act(&root, &[Action::Raise { to: open }, Action::Call]);
        let out = run(&store, &called);
        assert_eq!(out.branches.len(), 1, "{open}");
        let b = &out.branches[0];
        assert_eq!((b.id, b.parent, b.stopped.clone()), (0, None, None), "{open}: an on-menu open is not a split");
        assert_eq!(b.translated, vec![(UTG, Action::Raise { to: open }), (HJ, Action::Call)], "{open}: the observed history is kept");
        assert!(!out.reasons.iter().any(|r| matches!(r, ApproxReason::BetTranslation { .. })), "{open}: {:?}", out.reasons);
        assert_range_close(&out.ranges[2], &column_at(&nodes, &[], raise), &format!("{open}: UTG conditioned by the {edge} column"));
        assert_range_close(&out.ranges[3], &hj_call(edge), &format!("{open}: HJ conditioned at the node after {edge}"));
    }
}

/// Ruling 13-R1: a translated edge keeps the source size it chose, which its chip amount cannot
/// name. At a three-chip big blind a 6-chip open (below 2.5 bb) clamps to 2500 and a 9-chip open
/// (above 2.6 bb, no all-in on the menu) clamps to 2600; both sizes are the chip action
/// `Raise{to: 8}`. The walk carries each branch's source step, so HJ's call is conditioned at the
/// continuation of the edge the open was mapped to. A chip-only query of the same branch cannot
/// tell the two edges apart and says so (an explicit unresolved missing path, never either node),
/// and a standalone transaction, which holds only that chip history, stops the branch on it.
#[test]
fn a_translated_edge_keeps_its_source_size_past_chip_rounding() {
    let nodes = sub_chip_nodes();
    let store = store_of(nodes.clone());
    let cfg = config_at(3);
    let root = table(&cfg);
    for (open, edge, raise) in [(6u32, 2500u32, 1usize), (9, 2600, 2)] {
        let opened = act(&root, &[Action::Raise { to: open }]);
        let called = act(&opened, &[Action::Call]);
        let after_open = run(&store, &opened);
        assert_eq!(after_open.branches.len(), 1, "{open}");
        let b = &after_open.branches[0];
        assert_eq!((b.id, b.parent, b.split_by), (1, Some(0), Some(UTG)), "{open}: a clamped translation is a split child");
        assert_eq!(b.translated, vec![(UTG, Action::Raise { to: 8 })], "{open}: both source sizes are 8 chips");
        let hj_key = key(vec![(Position::Utg, raise_bb_x1000(edge))]);
        assert_eq!(b.seats.iter().find(|s| s.seat == HJ).unwrap().node, Some(hj_key), "{open}: HJ's next node is under {edge}");

        let out = run(&store, &called);
        assert_eq!(out.branches.len(), 1, "{open}");
        assert_eq!(out.branches[0].translated, vec![(UTG, Action::Raise { to: 8 }), (HJ, Action::Call)], "{open}");
        assert_eq!(out.branches[0].stopped, None, "{open}");
        assert_range_close(&out.ranges[2], &column_at(&nodes, &[], raise), &format!("{open}: UTG conditioned by the {edge} column"));
        let hj_call = column_at(&nodes, &[(Position::Utg, raise_bb_x1000(edge))], 1);
        assert_range_close(&out.ranges[3], &hj_call, &format!("{open}: HJ conditioned at the node after {edge}"));

        // The chip history alone is ambiguous: an explicit unresolved missing path.
        let chip_only = query_translated(&store, &cfg, &called, 1, b);
        assert!(chip_only.node.is_none() && chip_only.expanded.is_none(), "{open}: {chip_only:?}");
        assert!(
            chip_only.key.contains("unresolved") && chip_only.key.contains("2500") && chip_only.key.contains("2600"),
            "{open}: {}",
            chip_only.key
        );
        assert_eq!(chip_only.unsupported, Some(UnsupportedReason::MissingPreflopNode { key: chip_only.key.clone() }));
        assert_eq!(chip_only.actor, Some(HJ));
        let mut standalone = after_open.clone();
        let input = ReplayInput { cfg: &cfg, state: &called, store: &store, snapshots: &[] };
        assert!(apply_preflop_action(&input, &mut standalone, 1, HJ, &Action::Call), "{open}");
        let s = &standalone.branches[0];
        assert_eq!(s.stopped, Some(format!("missing node {}", chip_only.key)), "{open}");
        assert_eq!(s.q.to_bits(), b.q.to_bits(), "{open}");
        for (x, z) in s.seats.iter().zip(&b.seats) {
            assert_eq!(x.mass, z.mass, "{open}: seat {:?} is frozen", x.seat);
        }
        assert!(standalone.reasons.contains(&missing_reason(HJ, &chip_only.key)), "{open}: {:?}", standalone.reasons);
    }
}

/// A two-way translated open at the root of [`ladder_nodes`]: the disclosed pot fractions `(s, A,
/// B)`, the deviation and its prominence (`d > 0.10`, stated by the caller from the hand
/// calculation), the pseudo-harmonic weights `f_A` and `1 - f_A`, each child's `q = f_X * M_X` and
/// chip action, and the opener's marginal `f_A P_A + f_B P_B` combo by combo (spec section 8.4).
fn check_two_way_open(
    out: &ReplayOutput,
    nodes: &MemNodes,
    seat: Seat,
    (s, a, b): (f64, f64, f64),
    fa: f64,
    chips: [Action; 2],
    want_prominent: bool,
) {
    let fb = 1.0 - fa;
    let (pa, pb) = (column_at(nodes, &[], 1), column_at(nodes, &[], 2));
    let (observed, mapped, deviation, prominent) = out
        .reasons
        .iter()
        .find_map(|r| match r {
            ApproxReason::BetTranslation { street: Street::Preflop, seat: by, observed_pct, mapped, deviation, prominent } if *by == seat => {
                Some((*observed_pct, mapped.clone(), *deviation, *prominent))
            }
            _ => None,
        })
        .expect("the translation is disclosed");
    assert!((f64::from(observed) - s).abs() < 1e-6, "observed {observed} != {s}");
    assert_eq!(mapped.len(), 2, "{mapped:?}");
    for ((size, weight), (want_size, want_weight)) in mapped.iter().zip([(a, fa), (b, fb)]) {
        assert!((f64::from(*size) - want_size).abs() < 1e-6 && (f64::from(*weight) - want_weight).abs() < 1e-6, "{mapped:?}");
    }
    assert!((f64::from(deviation) - (s - a).abs().min((s - b).abs())).abs() < 1e-6, "{deviation}");
    assert_eq!(prominent, want_prominent, "prominent = d > 0.10 at d = {deviation}");
    assert_eq!(out.branches.len(), 2);
    for (k, (branch, (f, p, chip))) in out.branches.iter().zip([(fa, &pa, chips[0]), (fb, &pb, chips[1])]).enumerate() {
        assert_eq!((branch.id, branch.parent, branch.split_by), (k as u8 + 1, Some(0), Some(seat)));
        assert_eq!(branch.translated, vec![(seat, chip)]);
        close(branch.q, f * mean(p), "q = f_X * M_X");
    }
    let expected: Vec<f64> = (0..COMBOS).map(|c| fa * pa[c] + fb * pb[c]).collect();
    assert_range_close(&out.ranges[usize::from(seat.0)], &expected, "the opener's marginal is f_A P_A + f_B P_B");
}

/// Ruling 13-R2 (the review's example): at a three-chip big blind with a one-chip small blind, a
/// 9-chip open against a 2.5/3.5 bb menu is interpolated at the SOURCE parent -- posts 0.5 and 1
/// source bb, UTG owes 1, the pot is 1.5 -- with the observed and the menu sizes in one scale:
/// `s = (3 - 1) / (1.5 + 1) = 0.8`, `A = 0.6`, `B = 1.0`, so `f_A = 0.2 * 1.6 / (0.4 * 1.8) = 4/9`
/// and `f_B = 5/9` (the live posts and chip-rounded sizes gave `f_A = 8/13`).
#[test]
fn interpolation_uses_the_source_parents_money_at_a_three_chip_blind() {
    let nodes = ladder_nodes();
    let store = store_of(nodes.clone());
    let cfg = config_at(3);
    assert_eq!(cfg.sb_chips, 1);
    let opened = act(&table(&cfg), &[Action::Raise { to: 9 }]);
    let out = run(&store, &opened);
    // d = min(0.2, 0.2) = 0.2: prominent.
    check_two_way_open(&out, &nodes, UTG, (0.8, 0.6, 1.0), 4.0 / 9.0, [Action::Raise { to: 8 }, Action::Raise { to: 11 }], true);
}

/// Ruling 13-R2 behind a straddle (spec section 8.3): the source unit is the 4-chip straddle, the
/// HJ seat opens as the virtual UTG, and the source parent is the VIRTUAL tree's -- the virtual SB
/// (the physical BB, 2 chips = 0.5 S) and the virtual BB (the straddler, 1 S) post, while the
/// physical SB's chip is not represented. An 11-chip open is `s = (2.75 - 1) / (1.5 + 1) = 0.7`
/// against `A = 0.6` and `B = 1.0`: `f_A = 0.3 * 1.6 / (0.4 * 1.7) = 12/17` (counting the physical
/// SB's chip in the pot gave `17/24`).
#[test]
fn a_straddled_off_menu_open_interpolates_in_the_virtual_tree() {
    let nodes = ladder_nodes();
    let store = store_of(nodes.clone());
    let straddle = 4;
    let cfg = HandConfig {
        config_revision: 7,
        sb_chips: 1,
        bb_chips: 2,
        straddle: Some(proto::UtgStraddle { amount_chips: straddle }),
        rake: Rake::PotRake { rate: 0.05, cap_mchips: straddle * 500, no_flop_no_drop: true },
        chip_label: "$1".into(),
    };
    let root = core_model::begin_hand(
        &cfg,
        BeginHand { hand_id: 4, button: BTN, hero: BB, dealt: (0..6).map(Seat).collect(), stacks_start: vec![100 * straddle; 6], hero_cards: None },
    )
    .expect("a straddled six-max table");
    let opened = act(&root, &[Action::Raise { to: 11 }]);
    assert_eq!(opened.actions[0].seat, HJ);
    let out = run(&store, &opened);
    // d = min(0.1, 0.3) = 0.1, which is not above the 0.10 prominence threshold.
    check_two_way_open(&out, &nodes, HJ, (0.7, 0.6, 1.0), 12.0 / 17.0, [Action::Raise { to: 10 }, Action::Raise { to: 14 }], false);
    assert!(out.reasons.contains(&ApproxReason::StraddleMapped { posts: [0.25, 0.5, 1.0] }), "{:?}", out.reasons);
}

/// Ruling 13-R3 through the public transaction: two live branches at ids 252 and 253 (UTG's 3 bb
/// open translated to 2.5 bb and to 3.5 bb), each facing HJ's 7.5 bb raise at its own node with its
/// own menu (6/9 bb after 2.5, 7/12 bb after 3.5). Both split in ONE generation: the ids are
/// compacted once (252 -> 0, 253 -> 1) and all four children keep their parents' links. Hand
/// calculation at each source parent (posts 0.5/1, HJ owes UTG's size): after 2.5 bb,
/// `s = 5 / 6.5 = 10/13` between `7/13` and `1`, `f = (3/13)(20/13) / ((6/13)(23/13)) = 10/23`;
/// after 3.5 bb, `s = 4 / 8.5 = 8/17` between `7/17` and `1`, `f = (9/17)(24/17) / ((10/17)(25/17))
/// = 0.864`. HJ's marginal is `sum_k q_k (f_kA P_kA + f_kB P_kB)`.
#[test]
fn several_branches_split_at_one_action_in_one_generation() {
    let nodes = ladder_nodes();
    let store = store_of(nodes.clone());
    let cfg = config_at(10);
    let state = act(&table(&cfg), &[Action::Raise { to: 30 }, Action::Raise { to: 75 }]);
    let parent = |id: u8, q: f64, open: u32| {
        let mut b = initial(&state.dealt).remove(0);
        b.id = id;
        b.q = q;
        b.translated = vec![(UTG, Action::Raise { to: open })];
        b
    };
    let mut out = ReplayOutput {
        ranges: vec![None; 6],
        branches: vec![parent(252, 0.4, 25), parent(253, 0.3, 35)],
        folded_ranges: vec![],
        log_reach: vec![0.0; 6],
        reasons: vec![],
        unsupported: None,
    };
    let input = ReplayInput { cfg: &cfg, state: &state, store: &store, snapshots: &[] };
    assert!(apply_preflop_action(&input, &mut out, 1, HJ, &Action::Raise { to: 75 }));
    assert_eq!(
        out.branches.iter().map(|b| (b.id, b.parent)).collect::<Vec<_>>(),
        vec![(2, Some(0)), (3, Some(0)), (4, Some(1)), (5, Some(1))],
        "one generation-wide allocation keeps every parent link"
    );
    let after = |open: u32| vec![(Position::Utg, raise_bb_x1000(open * 100))];
    let children = [
        (0.4, 10.0 / 23.0, 25u32, 2usize, 60u32),
        (0.4, 13.0 / 23.0, 25, 3, 90),
        (0.3, 0.864, 35, 2, 70),
        (0.3, 0.136, 35, 3, 120),
    ];
    let mut expected = vec![0.0; COMBOS];
    for (b, &(q, f, open, a, chips)) in out.branches.iter().zip(&children) {
        let p = column_at(&nodes, &after(open), a);
        assert_eq!(b.split_by, Some(HJ));
        assert_eq!(b.translated, vec![(UTG, Action::Raise { to: open }), (HJ, Action::Raise { to: chips })]);
        close(b.q, q * f * mean(&p), "q = q_k * f_X * M_kX");
        for (e, x) in expected.iter_mut().zip(&p) {
            *e += q * f * x;
        }
    }
    let r = marginal(&out.branches, HJ);
    let peak = expected.iter().copied().fold(0.0_f64, f64::max);
    for c in 0..COMBOS {
        close(r[c], expected[c] / peak, "HJ's rescaled marginal is sum_k q_k (f_A P_A + f_B P_B)");
    }
    let translations = out.reasons.iter().filter(|r| matches!(r, ApproxReason::BetTranslation { seat, .. } if *seat == HJ)).count();
    assert_eq!(translations, 2, "one disclosure per splitting branch: {:?}", out.reasons);
}

/// Q4 (task-13-review.md): a recoverable translation-domain mismatch through the public walk. UTG's
/// 2 bb open clamps to the source's only size, 5 bb; HJ's legal min-raise to 3 bb is then below the
/// 5 bb HJ owes at the mapped source parent, so the observed wager has no pot fraction there. The
/// branch stops with `unmappable size at <key>` -- a disclosed stop, no guessed size and no
/// zero-support rejection -- keeping its q and masses. (The menu-side twin, a source all-in below
/// the mapped call, cannot follow a legal observed wager: the observed raise never exceeds the
/// actor's own maximum.)
#[test]
fn a_raise_below_the_mapped_call_stops_as_an_unmappable_size() {
    let hj = vec![PreflopStep::Fold, PreflopStep::Call, raise_bb_x1000(12000)];
    let store = store_of(vec![
        (vec![], mem_node(Position::Utg, raise_menu(&[5000]), 1)),
        (vec![(Position::Utg, raise_bb_x1000(5000))], mem_node(Position::Hj, hj, 2)),
    ]);
    let cfg = config_at(10);
    let opened = act(&table(&cfg), &[Action::Raise { to: 20 }]);
    let raised = act(&opened, &[Action::Raise { to: 30 }]);
    let after_open = run(&store, &opened);
    let out = run(&store, &raised);
    let cause = format!("unmappable size at {}", key(vec![(Position::Utg, raise_bb_x1000(5000))]));
    assert_eq!(out.branches.len(), 1);
    let b = &out.branches[0];
    assert_eq!(b.stopped, Some(cause.clone()));
    assert_eq!(b.translated, vec![(UTG, Action::Raise { to: 50 })], "the clamped open, and no guessed size for HJ");
    assert_eq!(b.q.to_bits(), after_open.branches[0].q.to_bits());
    for (s, z) in b.seats.iter().zip(&after_open.branches[0].seats) {
        assert_eq!(s.mass, z.mass, "seat {:?}", s.seat);
    }
    assert_eq!(out.log_reach, after_open.log_reach);
    assert!(out.reasons.contains(&unconditioned(Street::Preflop, HJ, &cause)), "{:?}", out.reasons);
    assert!(!out.reasons.iter().any(|r| matches!(r, ApproxReason::BetTranslation { seat, .. } if *seat == HJ)));
    assert!(!out.reasons.contains(&zero_reason(Street::Preflop, HJ, &Action::Raise { to: 30 })));
    assert_eq!(out.unsupported, None);
}
