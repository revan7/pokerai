//! P3.T14 -- compatible snapshot selection and prefix-valid provenance (spec sections 9.1 and 9.2;
//! section 13.1's `replay_snapshot_compatibility`).
//!
//! `snapshot_prefix_predicate`, the `identity`/`snapshot` helpers and `replay_snapshot_compatibility`
//! are the task brief's Step 1 bodies, verbatim. The tests after them cover the rest of Steps 1, 4
//! and 6: every compared field of the compatibility predicate (and the two that are not compared),
//! the selection order (longest covered observed prefix, then raw exploitability, then the larger
//! decision id), export coverage including an off-menu wager's two interpolation children, the
//! active-identity registration gate, `for_identity`/`for_hand`/`invalidate_hand`, and the
//! prefix-based mutation invalidation over hands built by Plan 1's `begin_hand`/`apply_action`/
//! `set_board` (append-only survival with the original identity, undo, later streets, a changed
//! board, another hand), with the cache origins behaving exactly as `live`. No worker is used.
//!
//! Fix round 1 (ruling 14-I1): every invalidation test registers its snapshot at a genuine hero
//! decision with the street root `core_model::street_root` returns there, and spec 10.2's two
//! admitted multiway projections are validated in their projected history (append, hero-card change,
//! undo across the prefix or the projection, and no blind stripping of folded players' actions).

use core_model::state::BeginHand;
use core_replay::{compatible, covered_prefix, select_snapshot, street_number, CompatKey, SnapshotStore, StreetSnapshot};
use proto::{Action, Card, DecisionIdentity, HandConfig, HandState, Rake, Seat, Street};

#[test]
fn snapshot_prefix_predicate(){
    use proto::{Seat,Action};
    let old=vec![(Seat(1),Action::Check)];
    let longer=vec![(Seat(1),Action::Check),(Seat(0),Action::Bet{to:73})];
    assert!(longer.starts_with(&old));
    assert!(!old.starts_with(&longer));
    assert!(!vec![(Seat(1),Action::Bet{to:50})].starts_with(&old));
}

fn identity(decision_id:u64)->proto::DecisionIdentity{
    proto::DecisionIdentity{hand_id:1,hand_revision:7,decision_id,config_revision:1,model_revision:0}
}
fn snapshot(id:proto::DecisionIdentity,covered:Vec<proto::OrdinalPath>,exploitability:f32)
    ->core_replay::StreetSnapshot{
    use proto::{Action::*,Card,MaterializedNode,Street};
    let board=vec![Card(46),Card(21),Card(0)];
    let mut public=proto::Range1326([1.0;1326]);core_ranges::block_public(&mut public,&board);
    let specs=vec![
        (vec![],vec![],"oop",vec![Check],vec![None]),
        (vec![0],vec![Check],"ip",vec![Check,Bet{to:50},Bet{to:100}],vec![Some(100),None,None]),
        (vec![0,1],vec![Check,Bet{to:50}],"oop",vec![Fold,Call],vec![Some(100),Some(200)]),
        (vec![0,2],vec![Check,Bet{to:100}],"oop",vec![Fold,Call],vec![Some(100),Some(300)]),
    ];
    let materialized=specs.iter().map(|(p,_,actor,actions,terminal)|MaterializedNode{
        path:p.clone(),street:Street::Flop,actor:(*actor).into(),actions:actions.clone(),terminal_pots:terminal.clone()
    }).collect();
    let nodes=covered.iter().map(|path|{
        let (_,chips,actor,actions,_)=specs.iter().find(|s|s.0==*path).unwrap();
        let available:Vec<bool>=public.0.iter().map(|&w|w>0.0).collect();
        proto::worker::NodeStrategy{path:chips.clone(),actor:(*actor).into(),actions:actions.clone(),
            probs:available.iter().map(|&a|vec![if a{1.0/actions.len() as f32}else{0.0};actions.len()]).collect(),
            ev_chips:vec![vec![0.0;actions.len()];1326],available}
    }).collect();
    core_replay::StreetSnapshot{
        key:core_replay::SnapshotKey{hand_id:1,config_revision:1,model_revision:0,street:Street::Flop,
            root_board:board,root_range_hashes:[core_ranges::hash_scaled(&public);2],tree_signature:"snapshot_test_v1".into()},
        provenance:core_replay::SnapshotProvenance{identity_at_solve:id,
            solved_prefix:vec![(proto::Seat(1),Check)],origin:"live".into()},
        tree:proto::EffectiveTree{rules_version:3,template_id:"snapshot_test_v1".into(),root_street:Street::Flop,
            menus:std::collections::BTreeMap::new(),add_allin_threshold:0.0,force_allin_threshold:0.0,
            merging_threshold:0.0,wager_cap:1,inserted:vec![],materialized},
        nodes,covered_paths:covered,exploitability_chips:exploitability,reasons:vec![],
    }
}
#[test]
fn replay_snapshot_compatibility(){
    use proto::{Action::*,Seat};
    let history=vec![(Seat(1),Check),(Seat(0),Bet{to:50}),(Seat(1),Call)];
    let short=snapshot(identity(1),vec![vec![]],0.1);
    let long=snapshot(identity(2),vec![vec![],vec![0],vec![0,1],vec![0,2]],0.5);
    let key=core_replay::CompatKey::of(&short.key);let mut all=vec![short,long];
    assert_eq!(core_replay::select_snapshot(&all,&key,&history).unwrap().provenance.identity_at_solve.decision_id,2);
    all[1].key.model_revision=1;
    assert_eq!(core_replay::select_snapshot(&all,&key,&history).unwrap().provenance.identity_at_solve.decision_id,1);
    all[0].key.hand_id=2;
    assert!(core_replay::select_snapshot(&all,&key,&history).is_none());
    // A later hand_revision does not break compatibility: it is not a compared field.
    all[0].key.hand_id=1;
    let mut newer=all[0].clone();newer.provenance.identity_at_solve.hand_revision=9;
    assert!(core_replay::compatible(&newer.key,&key));
}

// ---------------------------------------------------------------------------------------------
// Shared helpers for the tests below.
// ---------------------------------------------------------------------------------------------

/// Every node of the fixture skeleton, in `covered_paths` order.
fn complete() -> Vec<proto::OrdinalPath> {
    vec![vec![], vec![0], vec![0, 1], vec![0, 2]]
}

/// The decision id of the snapshot `select_snapshot` picks, `None` when no candidate is compatible.
fn chosen(all: &[StreetSnapshot], key: &CompatKey, history: &[(Seat, Action)]) -> Option<u64> {
    select_snapshot(all, key, history).map(|s| s.provenance.identity_at_solve.decision_id)
}

/// One edit of a snapshot key.
type KeyEdit = Box<dyn Fn(&mut core_replay::SnapshotKey)>;

/// OOP (seat 1) checks, IP (seat 0) wagers to `to`, OOP calls: the fixture's street line.
fn line(to: u32) -> Vec<(Seat, Action)> {
    vec![(Seat(1), Action::Check), (Seat(0), Action::Bet { to }), (Seat(1), Action::Call)]
}

// ---------------------------------------------------------------------------------------------
// The compatibility predicate: exactly six compared fields (spec section 9.2).
// ---------------------------------------------------------------------------------------------

#[test]
fn compat_key_carries_exactly_the_six_compared_fields() {
    let s = snapshot(identity(1), complete(), 0.1);
    let key = CompatKey::of(&s.key);
    assert_eq!(
        (key.hand_id, key.config_revision, key.model_revision, key.street, &key.root_board, key.root_range_hashes),
        (s.key.hand_id, s.key.config_revision, s.key.model_revision, s.key.street, &s.key.root_board, s.key.root_range_hashes)
    );
    assert!(compatible(&s.key, &key));

    // Not compared: the tree signature (different trees compete when their incoming roots match),
    // and every identity field besides hand/config/model (hand_revision, decision_id).
    let mut other_tree = s.clone();
    other_tree.key.tree_signature = "another_template_v9".into();
    other_tree.provenance.identity_at_solve.hand_revision = 12;
    other_tree.provenance.identity_at_solve.decision_id = 99;
    assert!(compatible(&other_tree.key, &key), "tree signature, hand revision and decision id are provenance only");

    // Each compared field, changed alone, breaks compatibility.
    let mut other_hashes = s.key.root_range_hashes;
    other_hashes[1][0] ^= 1;
    let edits: Vec<(&str, KeyEdit)> = vec![
        ("hand_id", Box::new(|k| k.hand_id = 2)),
        ("config_revision", Box::new(|k| k.config_revision = 2)),
        ("model_revision", Box::new(|k| k.model_revision = 1)),
        ("street", Box::new(|k| k.street = Street::Turn)),
        ("root_board", Box::new(|k| k.root_board = vec![Card(46), Card(21), Card(1)])),
        ("oop range hash", Box::new(|k| k.root_range_hashes[0][31] ^= 0x80)),
        ("ip range hash", Box::new(move |k| k.root_range_hashes = other_hashes)),
    ];
    for (what, edit) in &edits {
        let mut changed = s.key.clone();
        edit(&mut changed);
        assert!(!compatible(&changed, &key), "{what} is a compared field");
        let mut candidate = s.clone();
        candidate.key = changed;
        assert_eq!(chosen(&[candidate], &key, &line(50)), None, "{what}: an incompatible snapshot is never selected");
    }
}

/// Step 6: the same solved prefix on a different board is another root, and the same displayed
/// revision in another hand is another hand; neither is selected however well it covers the line.
#[test]
fn same_prefix_on_another_board_and_same_revision_in_another_hand_are_incompatible() {
    let ours = snapshot(identity(1), vec![vec![]], 0.4);
    let key = CompatKey::of(&ours.key);
    let mut other_board = snapshot(identity(2), complete(), 0.01);
    other_board.key.root_board = vec![Card(46), Card(21), Card(4)];
    assert_eq!(other_board.provenance.solved_prefix, ours.provenance.solved_prefix);
    let mut other_hand = snapshot(DecisionIdentity { hand_id: 2, ..identity(3) }, complete(), 0.01);
    other_hand.key.hand_id = 2;
    assert_eq!(other_hand.provenance.identity_at_solve.hand_revision, ours.provenance.identity_at_solve.hand_revision);
    assert_eq!(chosen(&[other_board.clone(), other_hand.clone(), ours.clone()], &key, &line(50)), Some(1));
    assert_eq!(chosen(&[other_board, other_hand], &key, &line(50)), None);
}

// ---------------------------------------------------------------------------------------------
// Selection order.
// ---------------------------------------------------------------------------------------------

#[test]
fn longest_covered_prefix_then_raw_exploitability_then_larger_decision_id() {
    let history = line(50);
    // Longest covered prefix wins even against a far better exploitability.
    let root_only = snapshot(identity(1), vec![vec![]], 0.001);
    let two = snapshot(identity(2), vec![vec![], vec![0]], 3.0);
    let key = CompatKey::of(&root_only.key);
    assert_eq!(chosen(&[root_only.clone(), two.clone()], &key, &history), Some(2));
    assert_eq!(chosen(&[two.clone(), root_only.clone()], &key, &history), Some(2), "order-independent");

    // Equal coverage: the older decision with the better exploitability wins over a newer one.
    let older_better = snapshot(identity(3), complete(), 0.2);
    let newer_worse = snapshot(identity(4), complete(), 0.3);
    assert_eq!(chosen(&[older_better.clone(), newer_worse.clone()], &key, &history), Some(3));
    assert_eq!(chosen(&[newer_worse.clone(), older_better.clone()], &key, &history), Some(3));

    // Raw exploitability, never a display rounding: 0.1 and the next f32 above it both display as
    // 0.10, and the raw smaller one wins against a larger decision id.
    let exact = snapshot(identity(5), complete(), 0.1);
    let above = snapshot(identity(6), complete(), f32::from_bits(0.1f32.to_bits() + 1));
    assert_eq!(chosen(&[exact.clone(), above.clone()], &key, &history), Some(5));
    assert_eq!(chosen(&[above, exact], &key, &history), Some(5));

    // Equal coverage and exploitability: the larger decision id wins, in either order.
    let first = snapshot(identity(7), complete(), 0.25);
    let second = snapshot(identity(8), complete(), 0.25);
    assert_eq!(chosen(&[first.clone(), second.clone()], &key, &history), Some(8));
    assert_eq!(chosen(&[second, first], &key, &history), Some(8));
}

/// A snapshot is never required to cover all later actions: one covering nothing of the observed
/// line is still the selection when it is the only compatible candidate.
#[test]
fn reuse_never_requires_coverage_of_the_whole_line() {
    let key = CompatKey::of(&snapshot(identity(1), complete(), 0.1).key);
    let requested_only = snapshot(identity(1), vec![vec![0]], 0.1);
    assert_eq!(covered_prefix(&requested_only, &line(50)), 0);
    assert_eq!(chosen(&[requested_only], &key, &line(50)), Some(1));
    let root_only = snapshot(identity(2), vec![vec![]], 0.1);
    assert_eq!(chosen(&[root_only], &key, &line(50)), Some(2));
    assert_eq!(chosen(&[], &key, &line(50)), None);
}

// ---------------------------------------------------------------------------------------------
// Export coverage (spec section 9.2's "longest covered prefix").
// ---------------------------------------------------------------------------------------------

#[test]
fn covered_prefix_counts_consecutive_covered_actions_from_the_street_root() {
    let cover = |paths: Vec<proto::OrdinalPath>, history: &[(Seat, Action)]| covered_prefix(&snapshot(identity(1), paths, 0.1), history);
    assert_eq!(cover(complete(), &line(50)), 3);
    assert_eq!(cover(complete(), &[]), 0, "an empty street history covers nothing");
    assert_eq!(cover(vec![vec![]], &line(50)), 1, "root-only export");
    assert_eq!(cover(vec![vec![0]], &line(50)), 0, "requested-node-only export: counting stops at the uncovered root");
    assert_eq!(cover(vec![vec![], vec![0]], &line(50)), 2);
    assert_eq!(cover(vec![vec![], vec![0], vec![0, 2]], &line(50)), 2, "Bet50's continuation [0,1] is not exported");
    assert_eq!(cover(vec![vec![], vec![0], vec![0, 1]], &line(50)), 3, "the other size's node is not needed");
    assert_eq!(cover(vec![vec![], vec![0], vec![0, 2]], &line(100)), 3);
    // Coverage stops where the line leaves the skeleton: Check-Check closes this fixture's street at
    // a terminal edge, so a third action has no node.
    let checked_through = [(Seat(1), Action::Check), (Seat(0), Action::Check), (Seat(1), Action::Check)];
    assert_eq!(cover(complete(), &checked_through), 2);
    // An off-menu action that is not a wager has no interpolation: counting stops there.
    assert_eq!(cover(complete(), &[(Seat(1), Action::Check), (Seat(0), Action::Fold)]), 1);
}

/// An off-menu wager counts only when every child it maps to with a nonzero interpolation
/// coefficient is covered: both bracketing sizes strictly between two menu sizes, the nearest size
/// alone when it clamps below the smallest or above the largest (spec section 8.4).
#[test]
fn an_off_menu_wager_counts_only_when_all_its_interpolation_children_are_covered() {
    let cover = |paths: Vec<proto::OrdinalPath>, to: u32| covered_prefix(&snapshot(identity(1), paths, 0.1), &line(to));
    let after_50 = vec![vec![], vec![0], vec![0, 1]];
    let after_100 = vec![vec![], vec![0], vec![0, 2]];
    // 73 lies strictly between 50 and 100: both children carry a nonzero coefficient.
    assert_eq!(cover(complete(), 73), 3);
    assert_eq!(cover(after_50.clone(), 73), 2);
    assert_eq!(cover(after_100.clone(), 73), 2);
    // Below the smallest size: clamped onto Bet50's child alone.
    assert_eq!(cover(after_50.clone(), 40), 3);
    assert_eq!(cover(after_100.clone(), 40), 2);
    // Above the largest size: clamped onto Bet100's child alone.
    assert_eq!(cover(after_100.clone(), 150), 3);
    assert_eq!(cover(after_50.clone(), 150), 2);
    // On the menu: the exact child only.
    assert_eq!(cover(after_50, 50), 3);
    assert_eq!(cover(after_100, 100), 3);
}

// ---------------------------------------------------------------------------------------------
// The store: the active-identity registration gate (spec sections 4.4 and 9.2).
// ---------------------------------------------------------------------------------------------

fn id(hand_id: u64, hand_revision: u32, decision_id: u64) -> DecisionIdentity {
    DecisionIdentity { hand_id, hand_revision, decision_id, config_revision: 1, model_revision: 0 }
}

/// `snapshot` re-keyed for `identity`'s hand and `street`, solved at `prefix` on `board`.
fn solved(identity: DecisionIdentity, street: Street, board: &[Card], prefix: Vec<(Seat, Action)>, exploitability: f32) -> StreetSnapshot {
    let mut s = snapshot(identity.clone(), complete(), exploitability);
    s.key.hand_id = identity.hand_id;
    s.key.config_revision = identity.config_revision;
    s.key.model_revision = identity.model_revision;
    s.key.street = street;
    s.key.root_board = board.to_vec();
    s.provenance.solved_prefix = prefix;
    s
}

fn flop_board() -> Vec<Card> {
    vec![Card(46), Card(21), Card(0)]
}

#[test]
fn only_the_active_identity_registers_and_a_stale_result_is_refused_even_when_its_prefix_fits() {
    let mut store = SnapshotStore::new();
    let a = id(1, 7, 10);
    let prefix = vec![(Seat(1), Action::Check)];
    assert!(store.register(&a, solved(a.clone(), Street::Flop, &flop_board(), prefix.clone(), 0.1)));
    assert_eq!(store.for_hand(1).len(), 1);
    // A stale identity is refused outright, although its key, board and prefix all fit.
    let stale = id(1, 6, 9);
    assert!(!store.register(&a, solved(stale, Street::Flop, &flop_board(), prefix.clone(), 0.01)));
    assert_eq!(store.for_hand(1).len(), 1);
    // An identity differing from the active one in any single field is refused.
    for other in [
        DecisionIdentity { hand_id: 2, ..a.clone() },
        DecisionIdentity { hand_revision: 8, ..a.clone() },
        DecisionIdentity { decision_id: 11, ..a.clone() },
        DecisionIdentity { config_revision: 2, ..a.clone() },
        DecisionIdentity { model_revision: 1, ..a.clone() },
    ] {
        let mut refused = solved(a.clone(), Street::Flop, &flop_board(), prefix.clone(), 9.0);
        refused.provenance.identity_at_solve = other.clone();
        assert!(!store.register(&a, refused), "{other:?} is not the active identity");
    }
    // A key that disagrees with its own identity is refused too.
    let edits: Vec<(&str, KeyEdit)> = vec![
        ("hand_id", Box::new(|k| k.hand_id = 2)),
        ("config_revision", Box::new(|k| k.config_revision = 2)),
        ("model_revision", Box::new(|k| k.model_revision = 1)),
    ];
    for (what, edit) in &edits {
        let mut refused = solved(a.clone(), Street::Flop, &flop_board(), prefix.clone(), 9.0);
        edit(&mut refused.key);
        assert!(!store.register(&a, refused), "a key whose {what} is not the identity's is refused");
    }
    let kept = store.for_hand(1);
    assert_eq!((kept.len(), kept[0].exploitability_chips), (1, 0.1), "refused results leave the store untouched");
}

/// Re-registering the same decision on the same street keeps the newest (a Provisional replaced by
/// the same decision's Final); another decision of the same street, or the same decision on another
/// street, is kept alongside.
#[test]
fn re_registering_a_decision_replaces_it_and_other_decisions_remain_candidates() {
    let mut store = SnapshotStore::new();
    let a = id(3, 2, 5);
    assert!(store.register(&a, solved(a.clone(), Street::Turn, &[Card(46), Card(21), Card(0), Card(8)], vec![], 0.3)));
    assert!(store.register(&a, solved(a.clone(), Street::Turn, &[Card(46), Card(21), Card(0), Card(8)], vec![], 0.05)));
    let kept = store.for_hand(3);
    assert_eq!((kept.len(), kept[0].exploitability_chips), (1, 0.05));
    assert!(store.register(&a, solved(a.clone(), Street::Flop, &flop_board(), vec![], 0.2)));
    assert_eq!(store.for_hand(3).len(), 2, "the same decision on another street is another entry");
    let b = id(3, 2, 6);
    assert!(store.register(&b, solved(b.clone(), Street::Turn, &[Card(46), Card(21), Card(0), Card(8)], vec![], 0.4)));
    assert_eq!(store.for_hand(3).len(), 3, "a different decision remains a selection candidate");
    let other = id(4, 3, 7);
    assert!(store.register(&other, solved(other.clone(), Street::Turn, &[Card(46), Card(21), Card(0), Card(8)], vec![], 0.1)));
    store.invalidate_hand(3);
    assert_eq!((store.for_hand(3).len(), store.for_hand(4).len()), (0, 1), "invalidate_hand drops exactly that hand");
}

/// `for_identity` requires hand, config and model; not the hand revision or the decision id. A
/// config or model change hides the old snapshots without deleting them.
#[test]
fn for_identity_filters_by_hand_config_and_model_only() {
    let mut store = SnapshotStore::new();
    let a = id(1, 7, 10);
    assert!(store.register(&a, solved(a.clone(), Street::Flop, &flop_board(), vec![], 0.1)));
    let b = id(1, 9, 12);
    assert!(store.register(&b, solved(b.clone(), Street::Flop, &flop_board(), vec![], 0.2)));
    let later = id(1, 11, 15);
    let seen: Vec<u64> = store.for_identity(&later).iter().map(|s| s.provenance.identity_at_solve.decision_id).collect();
    assert_eq!(seen, vec![10, 12], "every revision and decision of the hand is visible, in registration order");
    assert!(store.for_identity(&DecisionIdentity { config_revision: 2, ..later.clone() }).is_empty());
    assert!(store.for_identity(&DecisionIdentity { model_revision: 1, ..later.clone() }).is_empty());
    assert!(store.for_identity(&DecisionIdentity { hand_id: 2, ..later.clone() }).is_empty());
    assert_eq!(store.for_hand(1).len(), 2, "hidden, not deleted");
    let unchanged = store.for_identity(&later);
    assert_eq!((unchanged[0].provenance.identity_at_solve.clone(), unchanged[1].provenance.identity_at_solve.clone()), (a, b));
}

// ---------------------------------------------------------------------------------------------
// Mutation invalidation over real hands (spec sections 9.2 and 10.2).
//
// Every snapshot below is registered at a genuine hero decision, keyed and prefixed by the street
// root `core_model::street_root` returns there (the history domain the engine registers in), and
// every mutation is a state built by Plan 1's `begin_hand`/`apply_action`/`set_board`/
// `set_hero_cards`; an undo is an earlier state under a fresh revision, as the engine's.
// ---------------------------------------------------------------------------------------------

const BB: Seat = Seat(1);
const CO: Seat = Seat(4);
const BTN: Seat = Seat(5);
/// Hero's cards: As Ad (never on any board below).
const HERO_CARDS: [Card; 2] = [Card(51), Card(49)];

fn turn_board() -> Vec<Card> {
    vec![Card(46), Card(21), Card(0), Card(8)]
}

/// A six-max table at 5/10 chips, hand 1, config revision 1, 1,000 chips each; seat 5 holds the
/// button (SB 0, BB 1, UTG 2, HJ 3, CO 4, BTN 5); `hero` holds As Ad.
fn table(hero: Seat) -> HandState {
    let cfg = HandConfig { config_revision: 1, sb_chips: 5, bb_chips: 10, straddle: None, rake: Rake::TimeCharge, chip_label: "$1".into() };
    core_model::begin_hand(&cfg, BeginHand { hand_id: 1, button: BTN, hero, dealt: (0..6).map(Seat).collect(), stacks_start: vec![1000; 6], hero_cards: Some(HERO_CARDS) })
        .expect("the model admits the table")
}

/// Heads-up: UTG, HJ and CO fold, BTN (hero) opens to 25, SB folds, BB calls; closed, before the
/// flop is dealt.
fn preflop_closed() -> HandState {
    act(&table(BTN), &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 25 }, Action::Fold, Action::Call])
}

/// The heads-up flop Kh 7d 2c (the fixture's board): BB (seat 1) is out of position and to act,
/// hero (BTN) in position.
fn flop_state() -> HandState {
    let s = core_model::set_board(&preflop_closed(), &flop_board()).expect("the flop");
    assert_eq!(s.derived.to_act, Some(BB));
    s
}

/// Three-way: UTG and HJ fold, CO and BTN call 10, SB folds, BB checks; the flop Kh 7d 2c is dealt
/// with a pot of 35. Postflop order is BB, CO, BTN: spec 10.2's A, B and C.
fn three_way_flop(hero: Seat) -> HandState {
    let s = act(&table(hero), &[Action::Fold, Action::Fold, Action::Call, Action::Call, Action::Fold, Action::Check]);
    let s = core_model::set_board(&s, &flop_board()).expect("the flop");
    assert_eq!((s.derived.to_act, s.derived.pot), (Some(BB), 35));
    s
}

fn act(state: &HandState, actions: &[Action]) -> HandState {
    let mut next = state.clone();
    for a in actions {
        next = core_model::apply_action(&next, *a).unwrap_or_else(|e| panic!("legal action {a:?}: {e}"));
    }
    next
}

/// The engine's undo: an earlier state under a fresh hand revision.
fn undone(earlier: &HandState, revision: u32) -> HandState {
    HandState { hand_revision: revision, ..earlier.clone() }
}

/// The snapshot the engine registers at hero's decision in `state` (solved under hand revision
/// `revision`, decision `decision_id`): keyed by the street root `core_model::street_root` returns
/// there, with that root's `history` as the solved prefix.
fn at_decision(state: &HandState, revision: u32, decision_id: u64) -> (DecisionIdentity, StreetSnapshot) {
    let root = core_model::street_root(state).unwrap_or_else(|e| panic!("hero's decision has an admitted street root: {e:?}"));
    let identity = id(state.hand_id, revision, decision_id);
    let snapshot = solved(identity.clone(), root.street, &root.board, root.history, 0.1);
    (identity, snapshot)
}

fn register_at(store: &mut SnapshotStore, state: &HandState, revision: u32, decision_id: u64) -> DecisionIdentity {
    let (identity, snapshot) = at_decision(state, revision, decision_id);
    assert!(store.register(&identity, snapshot));
    identity
}

fn decisions(store: &SnapshotStore) -> Vec<(u64, u32)> {
    store.for_hand(1).iter().map(|s| (s.provenance.identity_at_solve.decision_id, s.provenance.identity_at_solve.hand_revision)).collect()
}

#[test]
fn an_append_only_action_keeps_every_snapshot_whose_prefix_still_fits_with_its_original_identity() {
    let checked = act(&flop_state(), &[Action::Check]);
    let mut store = SnapshotStore::new();
    let after_check = register_at(&mut store, &checked, 8, 1);
    assert_eq!(store.for_hand(1)[0].provenance.solved_prefix, vec![(BB, Action::Check)]);
    // Hero bets: the street history grows past the solved decision.
    let bet = act(&checked, &[Action::Bet { to: 30 }]);
    store.invalidate(&bet);
    assert_eq!(decisions(&store), vec![(1, 8)]);
    // BB check-raises: hero's second decision on the street.
    let raised = act(&bet, &[Action::Raise { to: 90 }]);
    let after_raise = register_at(&mut store, &raised, 10, 2);
    store.invalidate(&raised);
    assert_eq!(decisions(&store), vec![(1, 8), (2, 10)], "retained with the identity they were solved under");
    // Hero calls, the turn comes: the flop's history only grew, and the flop is an earlier street.
    let called = act(&raised, &[Action::Call]);
    store.invalidate(&called);
    let turn = core_model::set_board(&called, &turn_board()).expect("the turn");
    store.invalidate(&turn);
    let retained = store.for_hand(1);
    assert_eq!((retained.len(), retained[0].provenance.identity_at_solve.clone(), retained[1].provenance.identity_at_solve.clone()), (2, after_check, after_raise));
}

#[test]
fn undo_drops_later_streets_and_same_street_snapshots_whose_prefix_no_longer_fits() {
    let flop = flop_state();
    let checked = act(&flop, &[Action::Check]);
    let checked_through = act(&checked, &[Action::Check]);
    let turn = core_model::set_board(&checked_through, &turn_board()).expect("the turn");
    let turn_checked = act(&turn, &[Action::Check]);
    let mut store = SnapshotStore::new();
    register_at(&mut store, &checked, 8, 1);
    register_at(&mut store, &turn_checked, 11, 2);
    store.invalidate(&turn_checked);
    assert_eq!(decisions(&store), vec![(1, 8), (2, 11)]);

    // Undo the turn check: the turn decision was solved after it; the flop decision stays.
    store.invalidate(&undone(&turn, 12));
    assert_eq!(decisions(&store), vec![(1, 8)]);

    // Undo the turn card: the hand awaits the turn; the flop's history still holds the decision.
    let awaiting = undone(&checked_through, 13);
    assert!(matches!(awaiting.phase, proto::HandPhase::AwaitingBoard { street: Street::Turn }));
    store.invalidate(&awaiting);
    assert_eq!(decisions(&store), vec![(1, 8)]);

    // Undo hero's check: back at the solved decision itself, under a new revision that never
    // rewrites the snapshot's own.
    store.invalidate(&undone(&checked, 14));
    assert_eq!(decisions(&store), vec![(1, 8)]);

    // Rule (2) on its own: a later street's snapshot is dropped by its street alone, even one whose
    // key would pass the board rule (a turn key carrying only the flop's cards).
    let mut later = SnapshotStore::new();
    let odd = id(1, 14, 9);
    assert!(later.register(&odd, solved(odd.clone(), Street::Turn, &flop_board(), vec![], 0.1)));
    later.invalidate(&undone(&checked, 14));
    assert!(later.for_hand(1).is_empty(), "a street later than the current one is dropped");

    // Undo BB's check: the decision solved after it is gone.
    store.invalidate(&undone(&flop, 15));
    assert!(decisions(&store).is_empty());

    // BB bets instead: a result solved before the undo is stale for the new active decision, even
    // though its key and board fit: refused.
    let bet_instead = undone(&act(&flop, &[Action::Bet { to: 20 }]), 16);
    let (active, fresh) = at_decision(&bet_instead, 16, 3);
    let (_, stale) = at_decision(&bet_instead, 15, 2);
    assert!(!store.register(&active, stale));
    assert!(store.register(&active, fresh));
    store.invalidate(&bet_instead);
    assert_eq!(decisions(&store), vec![(3, 16)]);

    // Undo the flop cards: the hand awaits the flop, whose root board no longer matches.
    let awaiting_flop = undone(&preflop_closed(), 17);
    assert!(matches!(awaiting_flop.phase, proto::HandPhase::AwaitingBoard { street: Street::Flop }));
    store.invalidate(&awaiting_flop);
    assert!(decisions(&store).is_empty());

    // Undo into the preflop betting (BB has not called yet): every postflop snapshot is a later street.
    let mut postflop = SnapshotStore::new();
    register_at(&mut postflop, &checked, 8, 1);
    register_at(&mut postflop, &turn_checked, 11, 2);
    let before_call = undone(&act(&table(BTN), &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 25 }, Action::Fold]), 18);
    assert_eq!(before_call.derived.street, Street::Preflop);
    postflop.invalidate(&before_call);
    assert!(decisions(&postflop).is_empty());
}

/// A same-street snapshot whose root board no longer matches that street's board is dropped even
/// though its solved prefix fits; a snapshot of another hand is dropped by any mutation of this one.
#[test]
fn a_changed_root_board_or_another_hand_is_dropped_by_invalidate() {
    let checked = act(&flop_state(), &[Action::Check]);
    let mut store = SnapshotStore::new();
    register_at(&mut store, &checked, 7, 1);
    let (other_board, other_hand) = (id(1, 7, 2), id(2, 3, 3));
    let prefix = vec![(BB, Action::Check)];
    assert!(store.register(&other_board, solved(other_board.clone(), Street::Flop, &[Card(46), Card(21), Card(4)], prefix.clone(), 0.1)));
    assert!(store.register(&other_hand, solved(other_hand.clone(), Street::Flop, &flop_board(), prefix.clone(), 0.1)));
    store.invalidate(&checked);
    assert_eq!(decisions(&store), vec![(1, 7)]);
    assert!(store.for_hand(2).is_empty());
}

/// Ruling 14-I1, spec 10.2's first worked case: "A bets 50, B folds, C raises to 150, A to act"
/// projects to `A Bet(50), C Raise(150)`, and that projected history is the snapshot's solved
/// prefix. Returns the flop, the state at A's decision, and a store holding the snapshot registered
/// there (solved under revision 20, decision 1) with its identity.
fn projected_case() -> (HandState, HandState, SnapshotStore, DecisionIdentity) {
    let flop = three_way_flop(BB);
    let at_decision_state = act(&flop, &[Action::Bet { to: 50 }, Action::Fold, Action::Raise { to: 150 }]);
    let root = core_model::street_root(&at_decision_state).expect("spec 10.2 admits this projection");
    assert_eq!((root.oop, root.ip, root.projected_from, root.dead_this_street), (BB, BTN, 3, 0));
    assert_eq!(root.history, vec![(BB, Action::Bet { to: 50 }), (BTN, Action::Raise { to: 150 })]);
    let mut store = SnapshotStore::new();
    let identity = register_at(&mut store, &at_decision_state, 20, 1);
    assert_eq!(store.for_hand(1)[0].provenance.solved_prefix, root.history, "the real root history is the solved prefix");
    (flop, at_decision_state, store, identity)
}

fn identities(store: &SnapshotStore) -> Vec<DecisionIdentity> {
    store.for_hand(1).iter().map(|s| s.provenance.identity_at_solve.clone()).collect()
}

/// 14-I1 (a): A calls. The full history interleaves B's fold; the projected one still holds the
/// solved decision, so the snapshot is retained under its original identity, and again once the
/// next street is dealt.
#[test]
fn a_projected_snapshot_survives_an_appended_action_under_its_original_identity() {
    let (_, at_decision_state, mut store, identity) = projected_case();
    let called = act(&at_decision_state, &[Action::Call]);
    store.invalidate(&undone(&called, 21));
    assert_eq!(identities(&store), vec![identity.clone()], "an appended action keeps the projected snapshot");
    let turn = core_model::set_board(&called, &turn_board()).expect("the turn");
    store.invalidate(&undone(&turn, 22));
    assert_eq!(identities(&store), vec![identity]);
}

/// 14-I1 (b): only hero's cards change; public history is untouched.
#[test]
fn a_projected_snapshot_survives_a_hero_card_change() {
    let (_, at_decision_state, mut store, identity) = projected_case();
    let recarded = core_model::set_hero_cards(&at_decision_state, [Card(44), Card(45)]).expect("Kc Kd are not on the board");
    store.invalidate(&undone(&recarded, 21));
    assert_eq!(identities(&store), vec![identity], "a hero-card change keeps the projected snapshot");
}

/// 14-I1 (c): an undo of C's raise crosses the solved prefix; an undo of B's fold as well crosses
/// the projection. Either removes the snapshot.
#[test]
fn an_undo_across_the_projected_prefix_or_the_projection_removes_the_snapshot() {
    let (flop, _, mut store, _) = projected_case();
    store.invalidate(&undone(&act(&flop, &[Action::Bet { to: 50 }, Action::Fold]), 22));
    assert!(decisions(&store).is_empty(), "an undo across the solved prefix removes it");
    let (flop, _, mut store, _) = projected_case();
    store.invalidate(&undone(&act(&flop, &[Action::Bet { to: 50 }]), 23));
    assert!(decisions(&store).is_empty(), "an undo across the projection removes it");
}

/// Spec 10.2's second worked case, with dead money: "A bets 50, B calls 50, C raises to 150, A
/// raises to 250, B folds, C to act" projects to `A Bet(50), C Raise(150), A Raise(250)` (B's 50
/// dead). Hero is C; the projected snapshot survives C's call and a hero-card change, and an undo of
/// B's fold (across the projection) removes it.
#[test]
fn a_projected_root_with_dead_money_is_validated_in_its_projected_history() {
    let flop = three_way_flop(BTN);
    let line = [Action::Bet { to: 50 }, Action::Call, Action::Raise { to: 150 }, Action::Raise { to: 250 }, Action::Fold];
    let at_decision_state = act(&flop, &line);
    let root = core_model::street_root(&at_decision_state).expect("spec 10.2 admits this projection");
    assert_eq!((root.projected_from, root.dead_this_street), (3, 50));
    assert_eq!(root.history, vec![(BB, Action::Bet { to: 50 }), (BTN, Action::Raise { to: 150 }), (BB, Action::Raise { to: 250 })]);

    let mut store = SnapshotStore::new();
    let identity = register_at(&mut store, &at_decision_state, 30, 1);
    store.invalidate(&undone(&act(&at_decision_state, &[Action::Call]), 31));
    assert_eq!(identities(&store), vec![identity.clone()], "C's call keeps the projected snapshot");
    store.invalidate(&undone(&core_model::set_hero_cards(&at_decision_state, [Card(44), Card(45)]).unwrap(), 32));
    assert_eq!(identities(&store), vec![identity], "a hero-card change keeps it");
    store.invalidate(&undone(&act(&flop, &line[..4]), 33));
    assert!(decisions(&store).is_empty(), "an undo of B's fold crosses the projection");
}

/// The projected history is recovered by the model at a cutoff, never by stripping the actions of
/// whoever has folded by now: here B calls A's bet, C raises, A calls and only then B folds. B's
/// actions stripped, the history would start with the solved prefix `A Bet(50), C Raise(150)`, but
/// A's decision after C's raise was three-way in this line (multiway, no projected root), so the
/// decision the snapshot was solved for is not in this history and it is removed.
#[test]
fn a_folded_players_actions_are_never_stripped_blindly() {
    let (flop, _, mut store, _) = projected_case();
    let late_fold = act(&flop, &[Action::Bet { to: 50 }, Action::Call, Action::Raise { to: 150 }, Action::Call, Action::Fold]);
    let stripped: Vec<(Seat, Action)> =
        late_fold.actions.iter().filter(|a| a.street == Street::Flop && a.seat != CO).map(|a| (a.seat, a.action)).collect();
    assert!(stripped.starts_with(&store.for_hand(1)[0].provenance.solved_prefix), "the stripping heuristic would keep it");
    store.invalidate(&undone(&late_fold, 21));
    assert!(decisions(&store).is_empty());
}

/// The solved decision itself must be in the new history, not merely a later decision whose
/// projected history extends the solved prefix: here B calls A's bet, C raises to 150 (A's decision
/// there is three-way, no projected root), A re-raises to 400, B folds and C raises to 900. A's
/// decision now is an admitted projection whose history `A Bet(50), C Raise(150), A Raise(400),
/// C Raise(900)` starts with the solved prefix `A Bet(50), C Raise(150)`, but the decision solved at
/// that prefix does not exist in this line, so the snapshot is removed.
#[test]
fn a_later_decision_extending_the_prefix_does_not_stand_in_for_the_solved_one() {
    let (flop, _, mut store, _) = projected_case();
    let line = [Action::Bet { to: 50 }, Action::Call, Action::Raise { to: 150 }, Action::Raise { to: 400 }, Action::Fold, Action::Raise { to: 900 }];
    let later = act(&flop, &line);
    let root = core_model::street_root(&later).expect("the later decision is an admitted projection");
    assert_eq!(root.projected_from, 3);
    assert!(root.history.starts_with(&store.for_hand(1)[0].provenance.solved_prefix));
    assert!(root.history.len() > store.for_hand(1)[0].provenance.solved_prefix.len());
    store.invalidate(&undone(&later, 24));
    assert!(decisions(&store).is_empty());
}

/// Step 6: a cache-origin snapshot (`cache_exact`, `cache_approximate`, `cache_provisional`)
/// registers, is selected and is invalidated exactly as a `live` one.
#[test]
fn cache_origins_behave_exactly_as_live() {
    let origins = ["cache_exact", "live", "cache_provisional", "cache_approximate"];
    let history = line(50);
    let checked = act(&flop_state(), &[Action::Check]);
    let mut store = SnapshotStore::new();
    let mut all = vec![];
    for (i, origin) in origins.iter().enumerate() {
        let (identity, mut s) = at_decision(&checked, 7, 10 + i as u64);
        s.provenance.origin = (*origin).into();
        assert!(store.register(&identity, s.clone()), "{origin} registers");
        all.push(s);
    }
    let key = CompatKey::of(&all[0].key);
    // Equal coverage and exploitability: the largest decision id wins whatever each origin is.
    for rotation in 0..origins.len() {
        let mut candidates = all.clone();
        candidates.rotate_left(rotation);
        assert_eq!(chosen(&candidates, &key, &history), Some(13));
        for s in &mut candidates {
            s.provenance.origin = "live".into();
        }
        assert_eq!(chosen(&candidates, &key, &history), Some(13));
    }
    // Better exploitability wins for any origin.
    for (i, origin) in origins.iter().enumerate() {
        let mut candidates = all.clone();
        candidates[i].exploitability_chips = 0.05;
        assert_eq!(chosen(&candidates, &key, &history), Some(10 + i as u64), "{origin}");
    }
    // Invalidation does not read the origin either.
    store.invalidate(&act(&checked, &[Action::Bet { to: 30 }]));
    assert_eq!(store.for_hand(1).len(), 4);
    store.invalidate(&flop_state());
    assert!(store.for_hand(1).is_empty());
}

#[test]
fn street_numbers_order_the_streets() {
    assert_eq!(
        [Street::Preflop, Street::Flop, Street::Turn, Street::River].map(street_number),
        [0, 1, 2, 3]
    );
}
