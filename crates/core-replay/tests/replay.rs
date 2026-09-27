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

/// At a three-chip big blind the source's 2.5 bb open is the chip action `Raise{to: 8}` (7.5
/// rounded half up). A branch carrying that chip action must look its next node up under the
/// source's own 2500, which a thousandth conversion of 8 chips (2667) would miss: the translated
/// history is resolved against the source's menus, like the observed prefix (P3.T8 R3).
#[test]
fn translated_sizes_resolve_against_the_source_menu() {
    let store = full_store();
    let cfg = config_at(3);
    let root = table(&cfg);
    assert_eq!(core_preflop::to_source_step(&Action::Raise { to: 8 }, 3), raise_bb_x1000(2667), "the naive conversion");
    for open in [7u32, 8] {
        // Both 7 and 8 chips are within half a chip of 7.5: on the menu, mapped to the menu action.
        let called = act(&root, &[Action::Raise { to: open }, Action::Call]);
        let out = run(&store, &called);
        assert_eq!(out.branches.len(), 1);
        let b = &out.branches[0];
        assert_eq!(b.id, 0, "{open}: an on-menu open is not a split");
        assert_eq!(b.translated, vec![(UTG, Action::Raise { to: 8 }), (HJ, Action::Call)], "{open}");
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

/// Task 13's complete fallback for a completed postflop street (no snapshot is consumed yet):
/// `UnconditionedPriorStreet{cause: "no compatible snapshot"}` per actual acting seat, the
/// preflop masses retained, each root blocked in order. The preflop stop clears on entering the
/// flop (section 9.3 scopes it to the preflop street).
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
