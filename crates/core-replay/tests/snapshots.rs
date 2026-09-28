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

// =============================================================================================
// P3.T15 -- the postflop walk over partial snapshot exports (spec sections 8.4, 9.2 and 9.3;
// section 13.1's `replay_snapshot_prefix_reuse`).
//
// Every hand below is built by Plan 1's `begin_hand`/`apply_action`/`set_board`. There is no
// preflop source, so the preflop walk stops at the first action (`missing node no bundle`), the
// stop clears on the flop, and every seat enters the flop with uniform masses on all 1326 combos.
// Every expected branch weight, mass, marginal and `log_reach` is computed independently of the
// crate's kernel with the Task 11 formulas (`Expected`, plain f64 sums). No worker is used.
// =============================================================================================

use core_preflop::PreflopStore;
use core_replay::{board_mask, marginal, replay, ReplayInput, ReplayOutput, SnapshotKey, SnapshotProvenance};
use proto::worker::NodeStrategy;
use proto::{ApproxReason, EffectiveTree, MaterializedNode, OrdinalPath, COMBOS};

const SB: Seat = Seat(0);

/// The walk's hand: the table above with hero the BB. UTG, HJ, CO and BTN fold, the SB raises to 50
/// and the BB calls: a heads-up flop Kh 7d 2c with a 100-chip root pot, the SB out of position and
/// first to act, the BB (hero) in position; 950 chips behind each.
fn walk_flop() -> HandState {
    let s = act(&table(BB), &[Action::Fold, Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 50 }, Action::Call]);
    let s = core_model::set_board(&s, &flop_board()).expect("the flop");
    assert_eq!((s.derived.to_act, s.derived.pot), (Some(SB), 100));
    s
}

/// `line` on the flop of [`walk_flop`], then the turn 4c.
fn walked_turn(line: &[Action]) -> HandState {
    core_model::set_board(&act(&walk_flop(), line), &turn_board()).expect("the turn")
}

/// Section 13.1's flop line: the SB checks, the BB bets 73 into 100, the SB calls.
fn check_bet73_call() -> Vec<Action> {
    vec![Action::Check, Action::Bet { to: 73 }, Action::Call]
}

/// Replays `state` with no preflop source and the given registered snapshots.
fn replay_with(state: &HandState, snapshots: &[StreetSnapshot]) -> ReplayOutput {
    let store = PreflopStore::from_sources(vec![]);
    replay(ReplayInput { cfg: &state.config, state, store: &store, snapshots, missing: &[] })
}

/// The combos `board` leaves: the `available` set of every exported node solved on it.
fn live_on(board: &[Card]) -> Vec<bool> {
    board_mask(board).0.iter().map(|w| *w > 0.0).collect()
}

/// The combos the flop leaves.
fn flop_live() -> Vec<bool> {
    live_on(&flop_board())
}

/// One node of a walk fixture's materialized tree. It is exported when it carries probabilities:
/// one `[even, odd]` pair per menu action, a combo taking the entry of its index's parity.
struct TreeNode {
    path: OrdinalPath,
    actor: &'static str,
    actions: Vec<Action>,
    terminal: Vec<Option<u32>>,
    probs: Option<Vec<[f32; 2]>>,
}

fn tree_node(path: &[u8], actor: &'static str, actions: Vec<Action>, terminal: Vec<Option<u32>>, probs: Option<&[[f32; 2]]>) -> TreeNode {
    assert_eq!(actions.len(), terminal.len());
    if let Some(p) = probs {
        assert_eq!(p.len(), actions.len());
    }
    TreeNode { path: path.to_vec(), actor, actions, terminal, probs: probs.map(|p| p.to_vec()) }
}

/// A menu action's likelihood column over 1326 combos: its parity entry on every `live` combo, 0
/// on every other (unavailable to the solve).
fn column_on(live: &[bool], pairs: &[[f32; 2]], a: usize) -> Vec<f64> {
    (0..COMBOS).map(|c| if live[c] { f64::from(pairs[a][c % 2]) } else { 0.0 }).collect()
}

/// [`column_on`] the combos the flop leaves.
fn column(pairs: &[[f32; 2]], a: usize) -> Vec<f64> {
    column_on(&flop_live(), pairs, a)
}

/// The root: the SB checks or bets 100 with probability .5 each.
const ROOT: [[f32; 2]; 2] = [[0.5, 0.5], [0.5, 0.5]];
/// The BB after the check, menu Check / Bet50 / Bet100 (the brief's `(.9, .3)` and `(.1, .5)`).
const IP_MENU: [[f32; 2]; 3] = [[0.0, 0.2], [0.9, 0.3], [0.1, 0.5]];
/// The SB facing 50: Fold / Call `(.5, 1)`.
const AFTER_50: [[f32; 2]; 2] = [[0.5, 0.0], [0.5, 1.0]];
/// The SB facing 100: Fold / Call `(.2, .8)`.
const AFTER_100: [[f32; 2]; 2] = [[0.8, 0.2], [0.2, 0.8]];
/// The BB facing the SB's 100-chip bet (off the observed line).
const FACING_ROOT_BET: [[f32; 2]; 2] = [[0.5, 0.5], [0.5, 0.5]];
/// The BB after the check with 73 inserted: Check / Bet50 / Bet73 `(.4, .2)` / Bet100.
const INSERTED_MENU: [[f32; 2]; 4] = [[0.1, 0.3], [0.4, 0.3], [0.4, 0.2], [0.1, 0.2]];
/// The same node forced to Bet73 with probability 1.
const FORCED_MENU: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 0.0], [1.0, 1.0], [0.0, 0.0]];
/// The SB facing 73: Fold / Call `(.3, .6)`.
const AFTER_73: [[f32; 2]; 2] = [[0.7, 0.4], [0.3, 0.6]];

/// The prefix-reuse skeleton (menu Bet50/Bet100 after the check), exporting exactly `exported`.
fn menu_tree(exported: &[&[u8]]) -> Vec<TreeNode> {
    let probs = |path: &[u8], pairs: &'static [[f32; 2]]| exported.contains(&path).then_some(pairs);
    vec![
        tree_node(&[], "oop", vec![Action::Check, Action::Bet { to: 100 }], vec![None, None], probs(&[], &ROOT)),
        tree_node(&[1], "ip", vec![Action::Fold, Action::Call], vec![Some(200), Some(300)], probs(&[1], &FACING_ROOT_BET)),
        tree_node(&[0], "ip", vec![Action::Check, Action::Bet { to: 50 }, Action::Bet { to: 100 }], vec![Some(100), None, None], probs(&[0], &IP_MENU)),
        tree_node(&[0, 1], "oop", vec![Action::Fold, Action::Call], vec![Some(150), Some(200)], probs(&[0, 1], &AFTER_50)),
        tree_node(&[0, 2], "oop", vec![Action::Fold, Action::Call], vec![Some(200), Some(300)], probs(&[0, 2], &AFTER_100)),
    ]
}

/// The same street with 73 inserted into the BB's menu after the check, every node exported.
fn inserted_tree(ip_menu: &[[f32; 2]; 4]) -> Vec<TreeNode> {
    vec![
        tree_node(&[], "oop", vec![Action::Check, Action::Bet { to: 100 }], vec![None, None], Some(&ROOT)),
        tree_node(&[1], "ip", vec![Action::Fold, Action::Call], vec![Some(200), Some(300)], Some(&FACING_ROOT_BET)),
        tree_node(
            &[0],
            "ip",
            vec![Action::Check, Action::Bet { to: 50 }, Action::Bet { to: 73 }, Action::Bet { to: 100 }],
            vec![Some(100), None, None, None],
            Some(ip_menu),
        ),
        tree_node(&[0, 1], "oop", vec![Action::Fold, Action::Call], vec![Some(150), Some(200)], Some(&AFTER_50)),
        tree_node(&[0, 2], "oop", vec![Action::Fold, Action::Call], vec![Some(173), Some(246)], Some(&AFTER_73)),
        tree_node(&[0, 3], "oop", vec![Action::Fold, Action::Call], vec![Some(200), Some(300)], Some(&AFTER_100)),
    ]
}

/// The provenance reason every walk fixture snapshot carries, to be inherited by the replay.
fn inherited() -> ApproxReason {
    ApproxReason::DeadlineBestSoFar { reached_bp: 80, target_bp: 50 }
}

/// The public ranges this replay computes at the flop root, hashed as the engine keys a snapshot:
/// `[OOP, IP]`.
fn flop_root_hashes(oop: Seat, ip: Seat) -> [[u8; 32]; 2] {
    let incoming = replay_with(&walk_flop(), &[]);
    let hash = |seat: Seat| core_ranges::hash_scaled(incoming.ranges[usize::from(seat.0)].as_ref().expect("a dealt seat's range"));
    [hash(oop), hash(ip)]
}

/// The snapshot hero registers at its flop decision after the SB's check (solved prefix
/// `[SB Check]`, decision `decision_id`), keyed by the flop-root public ranges, over `nodes`: every
/// node with probabilities is exported, rows `available` exactly on the combos the flop leaves.
fn walk_snapshot(nodes: &[TreeNode], decision_id: u64) -> StreetSnapshot {
    let decision = act(&walk_flop(), &[Action::Check]);
    let root = core_model::street_root(&decision).expect("hero's decision after the check");
    assert_eq!((root.oop, root.ip, root.pot_root, root.history.clone()), (SB, BB, 100, vec![(SB, Action::Check)]));
    snapshot_at(&decision, nodes, &flop_live(), flop_root_hashes(SB, BB), decision_id)
}

/// The snapshot the engine registers at hero's decision in `decision` (decision `decision_id`):
/// keyed by that street's root board and the incoming public-range `hashes` (OOP, IP), its solved
/// prefix the street root's history, over the tree `nodes`; every node with probabilities is
/// exported, its rows `available` exactly on the `live` combos.
fn snapshot_at(decision: &HandState, nodes: &[TreeNode], live: &[bool], hashes: [[u8; 32]; 2], decision_id: u64) -> StreetSnapshot {
    let root = core_model::street_root(decision).expect("a hero decision with an admitted root");
    let at = |path: &[u8]| nodes.iter().find(|n| n.path == path).unwrap_or_else(|| panic!("the fixture has a node at {path:?}"));
    let chips = |path: &[u8]| -> Vec<Action> { (0..path.len()).map(|k| at(&path[..k]).actions[usize::from(path[k])]).collect() };
    let exported: Vec<&TreeNode> = nodes.iter().filter(|n| n.probs.is_some()).collect();
    let strategies = exported
        .iter()
        .map(|n| {
            let pairs = n.probs.as_ref().expect("exported");
            let width = n.actions.len();
            NodeStrategy {
                path: chips(&n.path),
                actor: n.actor.into(),
                actions: n.actions.clone(),
                probs: (0..COMBOS).map(|c| if live[c] { pairs.iter().map(|p| p[c % 2]).collect() } else { vec![0.0; width] }).collect(),
                ev_chips: vec![vec![0.0; width]; COMBOS],
                available: live.to_vec(),
            }
        })
        .collect();
    let materialized = nodes
        .iter()
        .map(|n| MaterializedNode { path: n.path.clone(), street: root.street, actor: n.actor.into(), actions: n.actions.clone(), terminal_pots: n.terminal.clone() })
        .collect();
    let inserted = if nodes.iter().any(|n| n.path == [0] && n.actions.contains(&Action::Bet { to: 73 })) {
        vec![(vec![Action::Check], "ip".to_string(), Action::Bet { to: 73 })]
    } else {
        vec![]
    };
    StreetSnapshot {
        key: SnapshotKey {
            hand_id: decision.hand_id,
            config_revision: decision.config.config_revision,
            model_revision: 0,
            street: root.street,
            root_board: root.board.clone(),
            root_range_hashes: hashes,
            tree_signature: "walk_test_v1".into(),
        },
        provenance: SnapshotProvenance {
            identity_at_solve: id(decision.hand_id, 8, decision_id),
            solved_prefix: root.history.clone(),
            origin: "live".into(),
        },
        tree: EffectiveTree {
            rules_version: 3,
            template_id: "walk_test_v1".into(),
            root_street: root.street,
            menus: std::collections::BTreeMap::new(),
            add_allin_threshold: 0.0,
            force_allin_threshold: 0.0,
            merging_threshold: 0.0,
            wager_cap: 3,
            inserted,
            materialized,
        },
        nodes: strategies,
        covered_paths: exported.iter().map(|n| n.path.clone()).collect(),
        exploitability_chips: 0.1,
        reasons: vec![inherited()],
    }
}

/// A history branch computed independently of the crate's kernel with the Task 11 formulas
/// (plain f64 sums): its weight `q`, every seat's masses (indexed by seat id) and its mapped line.
#[derive(Clone, Debug)]
struct Expected {
    q: f64,
    masses: Vec<Vec<f64>>,
    line: Vec<(Seat, Action)>,
}

impl Expected {
    /// The flop root of every walk fixture: one branch, `q = 1`, uniform masses for all six seats.
    fn start() -> Self {
        Expected { q: 1.0, masses: vec![vec![1.0; COMBOS]; 6], line: vec![] }
    }

    /// `seat` takes `action` with likelihood `p` at interpolation weight `f`:
    /// `M = sum_c w[c] p[c] / sum_c w[c]`, `q *= f * M`, `w[c] *= p[c] / M`; other seats unchanged.
    fn take(&self, seat: Seat, action: Action, p: &[f64], f: f64) -> Self {
        let slot = usize::from(seat.0);
        let w = &self.masses[slot];
        let m = w.iter().zip(p).map(|(w, p)| w * p).sum::<f64>() / w.iter().sum::<f64>();
        assert!(m > 0.0, "the expected model applies only supported actions");
        let mut next = self.clone();
        next.q *= f * m;
        next.masses[slot] = w.iter().zip(p).map(|(w, p)| w * p / m).collect();
        next.line.push((seat, action));
        next
    }

    /// `seat`'s `action` navigated through an uncovered skeleton node: recorded on the mapped line,
    /// never applied (no likelihood, `q` and every mass unchanged).
    fn skip(&self, seat: Seat, action: Action) -> Self {
        let mut next = self.clone();
        next.line.push((seat, action));
        next
    }
}

/// Spec section 8.4's pseudo-harmonic weights of an observed pot fraction `s` between menu
/// fractions `a < s < b`, from the formula itself.
fn harmonic(s: f64, a: f64, b: f64) -> (f64, f64) {
    let fa = (b - s) * (1.0 + a) / ((b - a) * (1.0 + s));
    (fa, 1.0 - fa)
}

/// `a` equals `b` within a relative 1e-10 (exactly, when `b` is 0): weights and masses are checked
/// at their own magnitude, however small.
fn close_to(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() <= 1e-10 * b.abs(), "{what}: {a} != {b}");
}

/// `a` equals `b` within an absolute 1e-10: for logarithms.
fn close_log(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() <= 1e-10, "{what}: {a} != {b}");
}

/// [`assert_walked_at`] a replay ending at the turn root.
fn assert_walked(out: &ReplayOutput, expected: &[Expected], what: &str) {
    assert_walked_at(out, expected, &turn_board(), what);
}

/// Asserts that `out` (a replay ending at the street root of `board`) holds exactly the `expected`
/// branches: in order, each branch's mapped line and weight `q` (never rescaled), and every seat's
/// masses equal to the expected ones divided by that seat's accumulated rescale `exp(log_reach)`;
/// and, per seat, `log_reach`, the board-free marginal and the published range equal to the
/// expected marginal `sum_k q_k w_k` with `board` removed and scaled to maximum 1. The expected
/// branches start from uniform masses and `log_reach = 0`.
fn assert_walked_at(out: &ReplayOutput, expected: &[Expected], board: &[Card], what: &str) {
    let summary: Vec<(u8, f64, &Vec<(Seat, Action)>)> = out.branches.iter().map(|b| (b.id, b.q, &b.translated)).collect();
    assert_eq!(out.branches.len(), expected.len(), "{what}: branches {summary:?}");
    for (b, e) in out.branches.iter().zip(expected) {
        assert!(!b.residual && b.stopped.is_none(), "{what}: branch {} is live", b.id);
        assert_eq!(b.translated, e.line, "{what}: branch {} mapped line", b.id);
        close_to(b.q, e.q, &format!("{what}: branch {} q", b.id));
        for s in &b.seats {
            let slot = usize::from(s.seat.0);
            let scale = out.log_reach[slot].exp();
            for c in 0..COMBOS {
                close_to(s.mass[c] * scale, e.masses[slot][c], &format!("{what}: branch {} seat {slot} mass[{c}]", b.id));
            }
        }
    }
    let mask = board_mask(board);
    for slot in 0..6 {
        let unscaled: Vec<f64> =
            (0..COMBOS).map(|c| if mask.0[c] == 0.0 { 0.0 } else { expected.iter().map(|e| e.q * e.masses[slot][c]).sum() }).collect();
        let peak = unscaled.iter().copied().fold(0.0_f64, f64::max);
        assert!(peak > 0.0);
        close_log(out.log_reach[slot], peak.ln(), &format!("{what}: seat {slot} log_reach"));
        let got = marginal(&out.branches, Seat(slot as u8));
        let range = out.ranges[slot].as_ref().expect("a dealt seat's range");
        for c in 0..COMBOS {
            let want = unscaled[c] / peak;
            if mask.0[c] > 0.0 {
                close_to(got[c], want, &format!("{what}: seat {slot} marginal[{c}]"));
            }
            assert!((f64::from(range.0[c]) - want).abs() <= 1e-6, "{what}: seat {slot} range[{c}] = {} != {want}", range.0[c]);
        }
    }
}

fn flop_reason(seat: Seat, cause: &str) -> ApproxReason {
    ApproxReason::UnconditionedPriorStreet { street: Street::Flop, seat, cause: cause.into() }
}

/// Every flop `UnconditionedPriorStreet` of `out`, as `(seat, cause)`.
fn flop_unconditioned(out: &ReplayOutput) -> Vec<(Seat, String)> {
    out.reasons
        .iter()
        .filter_map(|r| match r {
            ApproxReason::UnconditionedPriorStreet { street: Street::Flop, seat, cause } => Some((*seat, cause.clone())),
            _ => None,
        })
        .collect()
}

/// A flop `BetTranslation`'s fields: seat, observed fraction, mapped `(size, weight)`s, deviation,
/// prominence.
type Translation = (Seat, f32, Vec<(f32, f32)>, f32, bool);

/// The flop `BetTranslation`s of `out`.
fn flop_translations(out: &ReplayOutput) -> Vec<Translation> {
    out.reasons
        .iter()
        .filter_map(|r| match r {
            ApproxReason::BetTranslation { street: Street::Flop, seat, observed_pct, mapped, deviation, prominent } => {
                Some((*seat, *observed_pct, mapped.clone(), *deviation, *prominent))
            }
            _ => None,
        })
        .collect()
}

/// Section 13.1's `replay_snapshot_prefix_reuse`: the flop line [check, bet 73, call] against one
/// snapshot solved at prefix [check] with menu 50/100, under three exports (spec section 9.2 case 3
/// per node), plus section 13.1 T3's inserted-size clause. In every case the applied prefix is kept
/// and nothing is re-solved, and an omitted action changes neither `q` nor any mass in its branch:
/// each expected branch below does not apply it, and the replay must match that branch (masses up
/// to the seat-common rescale).
#[test]
fn replay_snapshot_prefix_reuse() {
    let turn = walked_turn(&check_bet73_call());
    let (fa, fb) = harmonic(73.0 / 100.0, 0.5, 1.0);
    let (check, bet50, bet100, call) = (Action::Check, Action::Bet { to: 50 }, Action::Bet { to: 100 }, Action::Call);

    // (a) Requested-node-only: only [check] (the BB's node) is exported. The SB's check at the
    // absent root is not applied ("uncovered path []"), the BB's 73 is translated over 50/100 from
    // the covered node, and the SB's call at [0,1] / [0,2] is not applied either.
    let requested = replay_with(&turn, &[walk_snapshot(&menu_tree(&[&[0]]), 1)]);
    let root = Expected::start().skip(SB, check);
    let expected = [
        root.take(BB, bet50, &column(&IP_MENU, 1), fa).skip(SB, call),
        root.take(BB, bet100, &column(&IP_MENU, 2), fb).skip(SB, call),
    ];
    assert_walked(&requested, &expected, "requested-node-only");
    for b in &requested.branches {
        let sb = &b.seats[0].mass;
        assert!(sb.iter().all(|w| *w == sb[0]), "the SB's check and call are never applied: its masses stay uniform");
    }
    let unconditioned = flop_unconditioned(&requested);
    for path in ["[]", "[0, 1]", "[0, 2]"] {
        let reason = (SB, format!("uncovered path {path}"));
        assert_eq!(unconditioned.iter().filter(|r| **r == reason).count(), 1, "{reason:?} in {unconditioned:?}");
    }
    assert_eq!(unconditioned.len(), 3, "{unconditioned:?}");
    let translations = flop_translations(&requested);
    assert_eq!(translations.len(), 1, "{translations:?}");
    let (seat, observed, mapped, deviation, prominent) = &translations[0];
    assert_eq!((*seat, *prominent), (BB, true));
    assert!((f64::from(*observed) - 0.73).abs() < 1e-6 && (f64::from(*deviation) - 0.23).abs() < 1e-6);
    assert_eq!(mapped.len(), 2);
    assert!((f64::from(mapped[0].0) - 0.5).abs() < 1e-6 && (f64::from(mapped[0].1) - fa).abs() < 1e-6);
    assert!((f64::from(mapped[1].0) - 1.0).abs() < 1e-6 && (f64::from(mapped[1].1) - fb).abs() < 1e-6);
    assert_eq!(requested.reasons.iter().filter(|r| **r == inherited()).count(), 1, "the snapshot's reasons are inherited once");

    // (b) Root-only: only [] is exported. The SB's check is conditioned from the root; the BB's 73 at
    // the uncovered [check] has no likelihood and no guessed split; the SB's call is not applied.
    let root_only = replay_with(&turn, &[walk_snapshot(&menu_tree(&[&[]]), 2)]);
    let expected = [Expected::start().take(SB, check, &column(&ROOT, 0), 1.0)];
    assert_walked(&root_only, &expected, "root-only");
    let bb = &root_only.branches[0].seats[1].mass;
    assert!(bb.iter().all(|w| *w == bb[0]), "the BB's uncovered bet is never applied: its masses stay uniform");
    assert!(flop_unconditioned(&root_only).contains(&(BB, "uncovered path [0]".to_string())));
    assert!(flop_translations(&root_only).is_empty(), "no guessed split");
    assert_eq!(root_only.reasons.iter().filter(|r| **r == inherited()).count(), 1);

    // (c) Complete street: [], [0], [0,1], [0,2]. The check, the translated 73 and the call from
    // each covered continuation all condition their actors, each exactly once.
    let complete = replay_with(&turn, &[walk_snapshot(&menu_tree(&[&[], &[0], &[0, 1], &[0, 2]]), 3)]);
    let root = Expected::start().take(SB, check, &column(&ROOT, 0), 1.0);
    let expected = [
        root.take(BB, bet50, &column(&IP_MENU, 1), fa).take(SB, call, &column(&AFTER_50, 1), 1.0),
        root.take(BB, bet100, &column(&IP_MENU, 2), fb).take(SB, call, &column(&AFTER_100, 1), 1.0),
    ];
    assert_walked(&complete, &expected, "complete");
    assert!(flop_unconditioned(&complete).is_empty(), "{:?}", complete.reasons);
    assert_eq!(flop_translations(&complete).len(), 1);

    // (d) Section 13.1 T3: an inserted observed size is a tree action with a solved probability.
    // With 73 on the BB's menu (solved `(.4, .2)`), the same line conditions on that probability,
    // never on 1, and is never translated.
    let inserted = replay_with(&turn, &[walk_snapshot(&inserted_tree(&INSERTED_MENU), 4)]);
    let bet73 = Action::Bet { to: 73 };
    let expected = [Expected::start()
        .take(SB, check, &column(&ROOT, 0), 1.0)
        .take(BB, bet73, &column(&INSERTED_MENU, 2), 1.0)
        .take(SB, call, &column(&AFTER_73, 1), 1.0)];
    assert_walked(&inserted, &expected, "inserted");
    assert!(flop_translations(&inserted).is_empty() && flop_unconditioned(&inserted).is_empty(), "{:?}", inserted.reasons);
    let turn_live = board_mask(&turn_board());
    let ip = inserted.ranges[1].as_ref().expect("the BB's range");
    for c in (0..COMBOS).filter(|&c| turn_live.0[c] > 0.0) {
        assert_eq!(ip.0[c], if c % 2 == 0 { 1.0 } else { 0.5 }, "the BB's range is the .4/.2 conditioning, combo {c}");
    }
    // Forcing P(Bet73 | c) = 1 is a different, wrong answer: it leaves the BB's range flat.
    let forced = replay_with(&turn, &[walk_snapshot(&inserted_tree(&FORCED_MENU), 5)]);
    let (solved, flat) = (marginal(&inserted.branches, BB), marginal(&forced.branches, BB));
    let gap = (0..COMBOS).filter(|&c| turn_live.0[c] > 0.0).map(|c| (solved[c] - flat[c]).abs()).fold(0.0_f64, f64::max);
    assert!(gap > 1e-10, "forcing the inserted size to 1 must differ from its solved probability (gap {gap})");
}

/// Plan-3 final review F-M2 through the postflop walk (spec 8.4: `prominent = d > 0.10`, decided in
/// exact integers). After the SB's check the BB bets 110 into the 100-chip pot against the
/// snapshot's Bet50/Bet100 menu: `s = 110 / 100` lies above the largest size, so it clamps to
/// Bet100 with `d = |110 - 100| / 100 = 1/10` exactly, which is NOT prominent (in `f64`,
/// `1.1 - 1.0` is `0.10000000000000009`, above the literal `0.1`). One chip more, `d = 11/100` is
/// prominent.
#[test]
fn a_flop_bet_a_tenth_of_the_pot_above_the_menu_is_not_prominent() {
    for (to, want_d, want_prominent) in [(110, 0.10, false), (111, 0.11, true)] {
        let turn = walked_turn(&[Action::Check, Action::Bet { to }, Action::Call]);
        let out = replay_with(&turn, &[walk_snapshot(&menu_tree(&[&[], &[0], &[0, 1], &[0, 2]]), 6)]);
        let translations = flop_translations(&out);
        assert_eq!(translations.len(), 1, "{translations:?}");
        let (seat, observed, mapped, deviation, prominent) = &translations[0];
        assert_eq!((*seat, mapped.clone()), (BB, vec![(1.0, 1.0)]), "the clamp to Bet100");
        assert!((f64::from(*observed) - f64::from(to) / 100.0).abs() < 1e-6, "s = {observed}");
        assert!((f64::from(*deviation) - want_d).abs() < 1e-6, "d = {deviation}");
        assert_eq!(*prominent, want_prominent, "a bet to {to}: d = {deviation}");
    }
}

// ---------------------------------------------------------------------------------------------
// Frozen navigation (spec section 9.2 as amended by revision 6, S14): "A branch whose mapped
// continuation is not covered freezes navigation for that branch for the remainder of the street;
// later observed actions in other branches are unaffected."
// ---------------------------------------------------------------------------------------------

use core_replay::{uncovered, walk_postflop, HistoryBranch};

/// The flop root's replay output (no snapshot consumed), then [`walk_postflop`] over the flop of
/// [`walk_flop`] followed by `line`, with `snapshots` registered: the walk after each prefix of a
/// street, which the whole-hand replay only shows once the street is complete.
fn walk_flop_line(line: &[Action], snapshots: &[StreetSnapshot]) -> ReplayOutput {
    let mut out = replay_with(&walk_flop(), &[]);
    let state = act(&walk_flop(), line);
    let store = PreflopStore::from_sources(vec![]);
    walk_postflop(&ReplayInput { cfg: &state.config, state: &state, store: &store, snapshots, missing: &[] }, Street::Flop, &mut out);
    out
}

/// A branch bit for bit: id, parent, mapped line, `q`, residual and stop flags, every seat's masses.
type Bits = (u8, Option<u8>, Vec<(Seat, Action)>, u64, bool, Option<String>, Vec<Vec<u64>>);

fn bits(b: &HistoryBranch) -> Bits {
    (
        b.id,
        b.parent,
        b.translated.clone(),
        b.q.to_bits(),
        b.residual,
        b.stopped.clone(),
        b.seats.iter().map(|s| s.mass.iter().map(|w| w.to_bits()).collect()).collect(),
    )
}

/// The branch whose mapped line holds `step`.
fn branch_with<'a>(out: &'a ReplayOutput, step: (Seat, Action)) -> &'a HistoryBranch {
    out.branches.iter().find(|b| b.translated.contains(&step)).unwrap_or_else(|| panic!("a branch mapped through {step:?}"))
}

/// Asserts that `later`'s copy of `b` has `b`'s `q` bit for bit and `b`'s masses up to each seat's
/// common rescale between the two outputs (`exp` of the `log_reach` difference): the actions in
/// between were not applied in `b`, only the seat-common factor every branch shares moved.
fn assert_unchanged(before: &ReplayOutput, b: &HistoryBranch, later: &ReplayOutput, what: &str) {
    let now = later.branches.iter().find(|x| x.translated == b.translated).unwrap_or_else(|| panic!("{what}: the branch survives"));
    assert_eq!(now.q.to_bits(), b.q.to_bits(), "{what}: q");
    for (s, z) in now.seats.iter().zip(&b.seats) {
        let slot = usize::from(s.seat.0);
        let factor = (later.log_reach[slot] - before.log_reach[slot]).exp();
        for c in 0..COMBOS {
            close_to(s.mass[c] * factor, z.mass[c], &format!("{what}: seat {slot} mass[{c}]"));
        }
    }
}

/// Root-only export: the BB's 73 at the unexported [check] is off that node's menu, so the walk
/// has neither a likelihood nor a unique next ordinal. The branch keeps `q` and every mass (bit for
/// bit: nothing is applied, so nothing is rescaled) for the rest of the street, and each later action
/// on it is disclosed with the cause that froze it ("OOP's call is uncovered too", section 13.1).
#[test]
fn an_uncovered_off_menu_wager_freezes_its_branch_for_the_rest_of_the_street() {
    assert_eq!(uncovered(Street::Flop, BB, &[0]), flop_reason(BB, "uncovered path [0]"));
    assert_eq!(uncovered(Street::Turn, SB, &[]), ApproxReason::UnconditionedPriorStreet { street: Street::Turn, seat: SB, cause: "uncovered path []".into() });
    let snapshots = [walk_snapshot(&menu_tree(&[&[]]), 2)];
    let checked = walk_flop_line(&[Action::Check], &snapshots);
    let bet = walk_flop_line(&[Action::Check, Action::Bet { to: 73 }], &snapshots);
    let called = walk_flop_line(&check_bet73_call(), &snapshots);
    assert_eq!(checked.branches.len(), 1);
    assert_eq!(checked.branches[0].translated, vec![(SB, Action::Check)]);
    for (later, what) in [(&bet, "after the bet"), (&called, "after the call")] {
        assert_eq!(later.branches.iter().map(bits).collect::<Vec<_>>(), checked.branches.iter().map(bits).collect::<Vec<_>>(), "{what}");
        assert_eq!(later.log_reach, checked.log_reach, "{what}: nothing applied, nothing rescaled");
        assert!(flop_translations(later).is_empty(), "{what}: no guessed split");
        assert!(later.reasons.contains(&uncovered(Street::Flop, BB, &[0])), "{what}: {:?}", later.reasons);
    }
    assert!(!bet.reasons.contains(&flop_reason(SB, "uncovered path [0]")));
    assert!(called.reasons.contains(&flop_reason(SB, "uncovered path [0]")), "the SB's call on the frozen branch: {:?}", called.reasons);
    assert_eq!(flop_unconditioned(&called).len(), 2, "{:?}", called.reasons);
}

/// Off the observed line: the SB's raise over the BB's 50.
const RAISE_OVER_100: [[f32; 2]; 3] = [[0.5, 0.2], [0.3, 0.3], [0.2, 0.5]];
/// The BB facing the SB's raise to 300: Fold / Call `(.6, .9)`.
const CALL_300: [[f32; 2]; 2] = [[0.4, 0.1], [0.6, 0.9]];

/// The sibling fixture: [0,1] (the SB facing 50) is not exported, [0,2] (facing 100) is, and each
/// holds one raise size.
fn sibling_tree() -> Vec<TreeNode> {
    let raise = |to: u32| vec![Action::Fold, Action::Call, Action::Raise { to }];
    vec![
        tree_node(&[], "oop", vec![Action::Check, Action::Bet { to: 100 }], vec![None, None], Some(&ROOT)),
        tree_node(&[1], "ip", vec![Action::Fold, Action::Call], vec![Some(200), Some(300)], None),
        tree_node(&[0], "ip", vec![Action::Check, Action::Bet { to: 50 }, Action::Bet { to: 100 }], vec![Some(100), None, None], Some(&IP_MENU)),
        tree_node(&[0, 1], "oop", raise(150), vec![Some(150), Some(200), None], None),
        tree_node(&[0, 2], "oop", raise(300), vec![Some(200), Some(300), None], Some(&RAISE_OVER_100)),
        tree_node(&[0, 1, 2], "ip", vec![Action::Fold, Action::Call], vec![Some(250), Some(400)], None),
        tree_node(&[0, 2, 2], "ip", vec![Action::Fold, Action::Call], vec![Some(400), Some(700)], Some(&CALL_300)),
    ]
}

/// Spec section 9.2 (S14) with two branches on one street: the BB's 73 splits into Bet50 ([0,1],
/// unexported) and Bet100 ([0,2], exported). The SB's raise to 250 is off both menus: in the Bet50
/// branch it meets an unexported node, so that branch freezes (no likelihood, no next ordinal); in
/// the Bet100 branch it is translated (clamped onto 300, below the only size) and conditions the
/// SB. The BB's call then conditions the BB in the Bet100 branch only; the frozen branch keeps its
/// `q` bit for bit and its masses up to the seat-common rescale, and the call is disclosed there.
#[test]
fn a_frozen_branch_keeps_its_weights_while_a_defined_sibling_keeps_conditioning() {
    let line = [Action::Check, Action::Bet { to: 73 }, Action::Raise { to: 250 }, Action::Call];
    let snapshots = [walk_snapshot(&sibling_tree(), 6)];
    let (fa, fb) = harmonic(0.73, 0.5, 1.0);
    let (bet50, bet100) = (Action::Bet { to: 50 }, Action::Bet { to: 100 });

    let out = replay_with(&walked_turn(&line), &snapshots);
    let root = Expected::start().take(SB, Action::Check, &column(&ROOT, 0), 1.0);
    let expected = [
        root.take(BB, bet50, &column(&IP_MENU, 1), fa),
        root.take(BB, bet100, &column(&IP_MENU, 2), fb)
            .take(SB, Action::Raise { to: 300 }, &column(&RAISE_OVER_100, 2), 1.0)
            .take(BB, Action::Call, &column(&CALL_300, 1), 1.0),
    ];
    assert_walked(&out, &expected, "frozen sibling");
    let unconditioned = flop_unconditioned(&out);
    assert_eq!(unconditioned, vec![(SB, "uncovered path [0, 1]".to_string()), (BB, "uncovered path [0, 1]".to_string())]);
    let translations = flop_translations(&out);
    assert_eq!(translations.len(), 2, "{translations:?}");
    let (seat, observed, mapped, deviation, prominent) = &translations[1];
    assert_eq!((*seat, *prominent, mapped.len()), (SB, true, 1));
    assert!((f64::from(*observed) - 0.5).abs() < 1e-6, "(250 - 100) / (200 + 100)");
    assert!((f64::from(mapped[0].0) - 2.0 / 3.0).abs() < 1e-6 && mapped[0].1 == 1.0, "clamped onto 300: (300 - 100) / 300");
    assert!((f64::from(*deviation) - 1.0 / 6.0).abs() < 1e-6);

    // Action by action: the frozen branch's `q` never moves again, its masses only by the common
    // rescale; the defined sibling's `q` moves at each of the two later actions.
    let after_bet = walk_flop_line(&line[..2], &snapshots);
    let after_raise = walk_flop_line(&line[..3], &snapshots);
    let after_call = walk_flop_line(&line, &snapshots);
    let frozen = branch_with(&after_bet, (BB, bet50));
    assert_unchanged(&after_bet, frozen, &after_raise, "the frozen branch across the raise");
    assert_unchanged(&after_bet, frozen, &after_call, "the frozen branch across the call");
    let sibling_q = |o: &ReplayOutput| branch_with(o, (BB, bet100)).q;
    assert!(sibling_q(&after_raise) < sibling_q(&after_bet) && sibling_q(&after_call) < sibling_q(&after_raise));
}

/// The turn root: the SB checks `(.6, .3)` or bets 150.
const TURN_ROOT: [[f32; 2]; 2] = [[0.6, 0.3], [0.4, 0.7]];

fn river_board() -> Vec<Card> {
    vec![Card(46), Card(21), Card(0), Card(8), Card(12)]
}

/// A path frozen on the flop is scoped to the flop: the turn is walked from its own snapshot's
/// root. Here the flop freezes at the BB's 73 (root-only export), and on the turn the SB's check
/// at the exported turn root still conditions the SB; the BB's check at the unexported [check]
/// advances without a likelihood.
#[test]
fn a_frozen_flop_branch_restarts_at_the_next_street_root() {
    let flop = [walk_snapshot(&menu_tree(&[&[]]), 2)];
    let turn_root = walked_turn(&check_bet73_call());
    let incoming = replay_with(&turn_root, &flop);
    let hash = |seat: Seat| core_ranges::hash_scaled(incoming.ranges[usize::from(seat.0)].as_ref().expect("a dealt seat's range"));
    let turn_tree = vec![
        tree_node(&[], "oop", vec![Action::Check, Action::Bet { to: 150 }], vec![None, None], Some(&TURN_ROOT)),
        tree_node(&[1], "ip", vec![Action::Fold, Action::Call], vec![Some(396), Some(546)], None),
        tree_node(&[0], "ip", vec![Action::Check, Action::Bet { to: 150 }], vec![Some(246), None], None),
    ];
    let turn = snapshot_at(&act(&turn_root, &[Action::Check]), &turn_tree, &live_on(&turn_board()), [hash(SB), hash(BB)], 9);
    assert_eq!((turn.key.street, turn.provenance.solved_prefix.clone()), (Street::Turn, vec![(SB, Action::Check)]));
    let river = core_model::set_board(&act(&turn_root, &[Action::Check, Action::Check]), &river_board()).expect("the river");

    let out = replay_with(&river, &[flop[0].clone(), turn]);
    let expected = [Expected::start()
        .take(SB, Action::Check, &column(&ROOT, 0), 1.0)
        .take(SB, Action::Check, &column_on(&live_on(&turn_board()), &TURN_ROOT, 0), 1.0)
        .skip(BB, Action::Check)];
    assert_walked_at(&out, &expected, &river_board(), "flop frozen, turn walked");
    assert!(out.reasons.contains(&uncovered(Street::Flop, BB, &[0])));
    assert!(out.reasons.contains(&flop_reason(SB, "uncovered path [0]")), "{:?}", out.reasons);
    assert!(out.reasons.contains(&uncovered(Street::Turn, BB, &[0])));
}

/// The walk of section 13.1's `replay_missing_continuation` (postflop half; the preflop half is
/// `tests/replay.rs`): every observed action is a skeleton action, but the strategy of one node,
/// [0,1] (the SB facing 50), is not exported. The SB's raise there is not applied in that branch
/// (reason "uncovered path [0, 1]", masses as conditioned so far, `q` unchanged, no invented
/// likelihood) and the branch advances along its exact ordinal; the BB's re-raise at the covered
/// [0,1,2] conditions the BB, the SB's later call at the covered [0,1,2,2] conditions the SB again,
/// and the other branch conditions every action.
const RAISE_AFTER_100: [[f32; 2]; 3] = [[0.5, 0.3], [0.3, 0.3], [0.2, 0.4]];
const RERAISE_A: [[f32; 2]; 3] = [[0.2, 0.5], [0.3, 0.3], [0.5, 0.2]];
const RERAISE_B: [[f32; 2]; 3] = [[0.4, 0.4], [0.4, 0.2], [0.2, 0.4]];
const CALL_A: [[f32; 2]; 2] = [[0.1, 0.6], [0.9, 0.4]];
const CALL_B: [[f32; 2]; 2] = [[0.3, 0.7], [0.7, 0.3]];

#[test]
fn replay_missing_continuation_postflop() {
    let menu = |to: u32| vec![Action::Fold, Action::Call, Action::Raise { to }];
    let tree = vec![
        tree_node(&[], "oop", vec![Action::Check, Action::Bet { to: 100 }], vec![None, None], Some(&ROOT)),
        tree_node(&[1], "ip", vec![Action::Fold, Action::Call], vec![Some(200), Some(300)], None),
        tree_node(&[0], "ip", vec![Action::Check, Action::Bet { to: 50 }, Action::Bet { to: 100 }], vec![Some(100), None, None], Some(&IP_MENU)),
        tree_node(&[0, 1], "oop", menu(300), vec![Some(150), Some(200), None], None),
        tree_node(&[0, 2], "oop", menu(300), vec![Some(200), Some(300), None], Some(&RAISE_AFTER_100)),
        tree_node(&[0, 1, 2], "ip", menu(900), vec![Some(350), Some(700), None], Some(&RERAISE_A)),
        tree_node(&[0, 2, 2], "ip", menu(900), vec![Some(400), Some(700), None], Some(&RERAISE_B)),
        tree_node(&[0, 1, 2, 2], "oop", vec![Action::Fold, Action::Call], vec![Some(1000), Some(1900)], Some(&CALL_A)),
        tree_node(&[0, 2, 2, 2], "oop", vec![Action::Fold, Action::Call], vec![Some(1000), Some(1900)], Some(&CALL_B)),
    ];
    let line = [Action::Check, Action::Bet { to: 73 }, Action::Raise { to: 300 }, Action::Raise { to: 900 }, Action::Call];
    let out = replay_with(&walked_turn(&line), &[walk_snapshot(&tree, 7)]);
    let (fa, fb) = harmonic(0.73, 0.5, 1.0);
    let (raise, reraise) = (Action::Raise { to: 300 }, Action::Raise { to: 900 });
    let root = Expected::start().take(SB, Action::Check, &column(&ROOT, 0), 1.0);
    let expected = [
        root.take(BB, Action::Bet { to: 50 }, &column(&IP_MENU, 1), fa)
            .skip(SB, raise)
            .take(BB, reraise, &column(&RERAISE_A, 2), 1.0)
            .take(SB, Action::Call, &column(&CALL_A, 1), 1.0),
        root.take(BB, Action::Bet { to: 100 }, &column(&IP_MENU, 2), fb)
            .take(SB, raise, &column(&RAISE_AFTER_100, 2), 1.0)
            .take(BB, reraise, &column(&RERAISE_B, 2), 1.0)
            .take(SB, Action::Call, &column(&CALL_B, 1), 1.0),
    ];
    assert_walked(&out, &expected, "missing continuation");
    assert_eq!(flop_unconditioned(&out), vec![(SB, "uncovered path [0, 1]".to_string())], "{:?}", out.reasons);
}

// ---------------------------------------------------------------------------------------------
// Zero support, missing or unreproducible snapshots, the current street, the financial root,
// and T6 through snapshot nodes (plan 3 Task 15 Step 5).
// ---------------------------------------------------------------------------------------------

use core_replay::{initial, posterior, publish, snapshot_node_at, snapshot_root};
use proto::{StreetRootSnapshot, UnsupportedReason};

/// The complete-street skeleton with the root's `(Check, Bet100)` probabilities replaced.
fn complete_tree(root: &'static [[f32; 2]]) -> Vec<TreeNode> {
    let mut tree = menu_tree(&[&[0], &[0, 1], &[0, 2]]);
    tree[0].probs = Some(root.to_vec());
    tree
}

/// Section 9.2's zero-support rule inside the walk: the SB never checks at the exported root, yet
/// the SB checked. Every pre-action `q` and mass survives (bit for bit), the cause is `zero support
/// after Check`, and navigation stays truthful without a guessed action: the observed check is on
/// the root's menu, so the branch still advances along it, and the BB's 73 and the SB's call are
/// conditioned from the covered nodes after it. A positive likelihood of 1e-30 is support.
#[test]
fn a_covered_action_without_support_is_rejected_and_the_walk_continues() {
    const NEVER_CHECKS: [[f32; 2]; 2] = [[0.0, 0.0], [1.0, 1.0]];
    const RARELY_CHECKS: [[f32; 2]; 2] = [[1e-30, 1e-30], [1.0, 1.0]];
    let (fa, fb) = harmonic(0.73, 0.5, 1.0);
    let (check, call) = (Action::Check, Action::Call);
    let turn = walked_turn(&check_bet73_call());

    let zero = [walk_snapshot(&complete_tree(&NEVER_CHECKS), 12)];
    let out = replay_with(&turn, &zero);
    let root = Expected::start().skip(SB, check);
    let expected = [
        root.take(BB, Action::Bet { to: 50 }, &column(&IP_MENU, 1), fa).take(SB, call, &column(&AFTER_50, 1), 1.0),
        root.take(BB, Action::Bet { to: 100 }, &column(&IP_MENU, 2), fb).take(SB, call, &column(&AFTER_100, 1), 1.0),
    ];
    assert_walked(&out, &expected, "zero support");
    assert_eq!(flop_unconditioned(&out), vec![(SB, "zero support after Check".to_string())]);
    assert_eq!(core_replay::zero_reason(Street::Flop, SB, &check), flop_reason(SB, "zero support after Check"));
    let incoming = replay_with(&walk_flop(), &[]);
    let rejected = walk_flop_line(&[check], &zero);
    assert_eq!(rejected.branches.iter().map(bits).collect::<Vec<_>>(), incoming.branches.iter().map(|b| {
        let mut b = b.clone();
        b.translated.push((SB, check));
        bits(&b)
    }).collect::<Vec<_>>(), "every pre-action q and mass survives; only the observed check is recorded");
    assert_eq!(rejected.log_reach, incoming.log_reach);

    let tiny = replay_with(&turn, &[walk_snapshot(&complete_tree(&RARELY_CHECKS), 13)]);
    let root = Expected::start().take(SB, check, &column(&RARELY_CHECKS, 0), 1.0);
    let expected = [
        root.take(BB, Action::Bet { to: 50 }, &column(&IP_MENU, 1), fa).take(SB, call, &column(&AFTER_50, 1), 1.0),
        root.take(BB, Action::Bet { to: 100 }, &column(&IP_MENU, 2), fb).take(SB, call, &column(&AFTER_100, 1), 1.0),
    ];
    assert_walked(&tiny, &expected, "a 1e-30 likelihood");
    assert!(flop_unconditioned(&tiny).is_empty(), "positive reach is valid at any magnitude: {:?}", tiny.reasons);
    assert!(tiny.branches.iter().all(|b| b.q > 0.0 && b.q < 1e-29));
}

/// A compatible snapshot whose solved prefix no cutoff of the observed street reproduces is not a
/// candidate (its financial root is unknown, and chips are never adjusted to fit); with no other,
/// the street is unconditioned with `snapshot root not reproducible` for each actor and every mass
/// kept as the flop root left it.
#[test]
fn a_snapshot_whose_root_the_model_cannot_reproduce_is_never_walked() {
    let mut odd = walk_snapshot(&menu_tree(&[&[], &[0], &[0, 1], &[0, 2]]), 14);
    odd.provenance.solved_prefix = vec![(SB, Action::Bet { to: 30 })];
    let turn = walked_turn(&check_bet73_call());
    let out = replay_with(&turn, &[odd.clone()]);
    let cause = "snapshot root not reproducible".to_string();
    assert_eq!(flop_unconditioned(&out), vec![(SB, cause.clone()), (BB, cause)]);
    assert_walked(&out, &[Expected::start()], "not reproducible");
    assert!(!out.reasons.contains(&inherited()), "an unused snapshot's reasons are not inherited");
    assert_eq!(snapshot_root(&turn, &odd), Err(UnsupportedReason::UnsupportedHistory { reason: "snapshot root not reproducible".into() }));
    // The reproducible one beside it is selected.
    let good = walk_snapshot(&menu_tree(&[&[]]), 15);
    let out = replay_with(&turn, &[odd, good]);
    assert_eq!(out.branches[0].translated, vec![(SB, Action::Check)], "the root-only snapshot was walked");
}

/// `snapshot_root` recovers the root a snapshot was solved on from its solved prefix through the
/// model, never from the final pot: after check, bet 73, call and the turn, the flop snapshot's root
/// is still the 100-chip root with 950 behind each. A spec section 10.2 projection is recovered with
/// its dead money. Another root board, an unreproducible prefix, or a state without hero's cards
/// (ruling 14f-C2: a cutoff is a decision only with them) reproduces nothing.
#[test]
fn snapshot_root_recovers_the_solved_financial_root_through_the_model() {
    let snap = walk_snapshot(&menu_tree(&[&[]]), 2);
    let turn = walked_turn(&check_bet73_call());
    let solved = core_model::street_root(&act(&walk_flop(), &[Action::Check])).expect("hero's decision");
    let root = snapshot_root(&turn, &snap).expect("the solved root replays");
    assert_eq!(root, StreetRootSnapshot { history: vec![], ..solved });
    assert_eq!((root.pot_root, root.stack_oop_root, root.stack_ip_root, root.dead_this_street, root.oop, root.ip), (100, 950, 950, 0, SB, BB));
    assert_eq!(turn.derived.pot, 246, "the final flop pot is not the root");

    // Spec 10.2's dead-money projection: C's call closes the street; the root keeps B's 50 dead.
    let flop = three_way_flop(BTN);
    let at = act(&flop, &[Action::Bet { to: 50 }, Action::Call, Action::Raise { to: 150 }, Action::Raise { to: 250 }, Action::Fold]);
    let projected = core_model::street_root(&at).expect("the admitted projection");
    let (_, snapshot) = at_decision(&at, 30, 1);
    let recovered = snapshot_root(&act(&at, &[Action::Call]), &snapshot).expect("the projection replays");
    assert_eq!(recovered, StreetRootSnapshot { history: vec![], ..projected });
    assert_eq!((recovered.dead_this_street, recovered.projected_from, recovered.oop, recovered.ip), (50, 3, BB, BTN));

    let not_reproducible = Err(UnsupportedReason::UnsupportedHistory { reason: "snapshot root not reproducible".into() });
    let mut other_board = snap.clone();
    other_board.key.root_board = vec![Card(46), Card(21), Card(4)];
    assert_eq!(snapshot_root(&turn, &other_board), not_reproducible);
    let mut other_prefix = snap.clone();
    other_prefix.provenance.solved_prefix = vec![];
    assert_eq!(snapshot_root(&turn, &other_prefix), not_reproducible, "the root itself was the SB's decision, not hero's");
    assert_eq!(snapshot_root(&HandState { hero_cards: None, ..turn.clone() }, &snap), not_reproducible);
    let mut preflop = snap.clone();
    preflop.key.street = Street::Preflop;
    preflop.key.root_board = vec![];
    assert_eq!(snapshot_root(&turn, &preflop), not_reproducible);
}

/// `snapshot_node_at` finds an exported node's strategy by ordinal path through `covered_paths`,
/// and nothing for a node that is not exported.
#[test]
fn snapshot_node_at_reads_exported_nodes_by_ordinal_path() {
    let snap = walk_snapshot(&menu_tree(&[&[0], &[0, 2]]), 16);
    assert_eq!(snapshot_node_at(&snap, &[0]).map(|n| n.path.clone()), Some(vec![Action::Check]));
    assert_eq!(snapshot_node_at(&snap, &[0, 2]).map(|n| n.path.clone()), Some(vec![Action::Check, Action::Bet { to: 100 }]));
    assert!(snapshot_node_at(&snap, &[]).is_none());
    assert!(snapshot_node_at(&snap, &[0, 1]).is_none());
    assert!(snapshot_node_at(&snap, &[7]).is_none());
}

/// Only completed streets are walked: on the flop the flop's own actions are inserted exactly by
/// Plan 2's street-root solve, so appending the BB's 73 and the SB's raise leaves every published
/// range, branch and `log_reach` unchanged even with a covering snapshot registered, while the
/// street root's history (the solve's effective history) grows.
#[test]
fn the_current_street_is_never_replayed() {
    let snapshots = [walk_snapshot(&menu_tree(&[&[], &[0], &[0, 1], &[0, 2]]), 3)];
    let at_check = act(&walk_flop(), &[Action::Check]);
    let later = act(&at_check, &[Action::Bet { to: 73 }, Action::Raise { to: 250 }]);
    let root_out = replay_with(&walk_flop(), &[]);
    for state in [&at_check, &later] {
        let out = replay_with(state, &snapshots);
        assert_eq!(out.ranges, root_out.ranges);
        assert_eq!(out.branches.iter().map(bits).collect::<Vec<_>>(), root_out.branches.iter().map(bits).collect::<Vec<_>>());
        assert_eq!((out.log_reach.clone(), out.reasons.clone()), (root_out.log_reach.clone(), root_out.reasons.clone()));
    }
    let (early, late) = (core_model::street_root(&at_check).unwrap(), core_model::street_root(&later).unwrap());
    assert_eq!(early.history, vec![(SB, Action::Check)]);
    assert_eq!(late.history, vec![(SB, Action::Check), (BB, Action::Bet { to: 73 }), (SB, Action::Raise { to: 250 })]);
    assert_eq!(StreetRootSnapshot { history: vec![], ..early }, StreetRootSnapshot { history: vec![], ..late }, "the same root, a longer history");
}

/// The roles and the money of the walk come from the admitted projected root, never from the
/// preflop positions or the final pot (spec section 10.2's second worked case): A (the BB) bets 50,
/// B (the CO) calls, C (the BTN, hero) raises to 150, A re-raises to 250, B folds; hero's snapshot
/// was solved there (root history `A Bet 50, C Raise 150, A Raise 250`, B's 50 dead). Then C raises
/// to 700 (off the 600 / 900 menu) and A calls. B's two actions are outside the heads-up tree:
/// disclosed once, never applied. C's 700 is translated at its mapped parent with the dead money in
/// the pot: `s = (700 - 150 - 100) / (35 + 50 + 400 + 100)`.
#[test]
fn a_projected_root_is_walked_in_its_heads_up_line_with_its_dead_money() {
    const OPENS: [[f32; 2]; 2] = [[0.4, 0.6], [0.6, 0.4]];
    const FACING_50: [[f32; 2]; 3] = [[0.2, 0.3], [0.5, 0.4], [0.3, 0.3]];
    const FACING_150: [[f32; 2]; 3] = [[0.3, 0.2], [0.3, 0.5], [0.4, 0.3]];
    const FACING_250: [[f32; 2]; 4] = [[0.1, 0.2], [0.3, 0.3], [0.4, 0.1], [0.2, 0.4]];
    const FACING_600: [[f32; 2]; 2] = [[0.5, 0.2], [0.5, 0.8]];
    const FACING_900: [[f32; 2]; 2] = [[0.7, 0.4], [0.3, 0.6]];
    let flop = three_way_flop(BTN);
    let line = [
        Action::Bet { to: 50 },
        Action::Call,
        Action::Raise { to: 150 },
        Action::Raise { to: 250 },
        Action::Fold,
        Action::Raise { to: 700 },
        Action::Call,
    ];
    let decision = act(&flop, &line[..5]);
    let incoming = replay_with(&flop, &[]);
    let hash = |seat: Seat| core_ranges::hash_scaled(incoming.ranges[usize::from(seat.0)].as_ref().expect("a dealt seat's range"));
    let wagers = |menu: Vec<Action>| {
        let terminal = menu.iter().map(|a| if matches!(a, Action::Fold | Action::Call) { Some(0) } else { None }).collect();
        (menu, terminal)
    };
    let (m1, t1) = wagers(vec![Action::Fold, Action::Call, Action::Raise { to: 150 }]);
    let (m2, t2) = wagers(vec![Action::Fold, Action::Call, Action::Raise { to: 250 }]);
    let (m3, t3) = wagers(vec![Action::Fold, Action::Call, Action::Raise { to: 600 }, Action::Raise { to: 900 }]);
    let tree = vec![
        tree_node(&[], "oop", vec![Action::Check, Action::Bet { to: 50 }], vec![None, None], Some(&OPENS)),
        tree_node(&[0], "ip", vec![Action::Check, Action::Bet { to: 35 }], vec![Some(135), None], None),
        tree_node(&[1], "ip", m1, t1, Some(&FACING_50)),
        tree_node(&[1, 2], "oop", m2, t2, Some(&FACING_150)),
        tree_node(&[1, 2, 2], "ip", m3, t3, Some(&FACING_250)),
        tree_node(&[1, 2, 2, 2], "oop", vec![Action::Fold, Action::Call], vec![Some(0), Some(0)], Some(&FACING_600)),
        tree_node(&[1, 2, 2, 3], "oop", vec![Action::Fold, Action::Call], vec![Some(0), Some(0)], Some(&FACING_900)),
    ];
    let snapshot = snapshot_at(&decision, &tree, &flop_live(), [hash(BB), hash(BTN)], 21);
    assert_eq!(
        snapshot.provenance.solved_prefix,
        vec![(BB, Action::Bet { to: 50 }), (BTN, Action::Raise { to: 150 }), (BB, Action::Raise { to: 250 })]
    );
    let turn = core_model::set_board(&act(&flop, &line), &turn_board()).expect("the turn");
    let out = replay_with(&turn, &[snapshot]);

    let (s, a, b) = (450.0 / 585.0, 350.0 / 585.0, 650.0 / 585.0);
    let (fa, fb) = harmonic(s, a, b);
    let root = Expected::start()
        .take(BB, Action::Bet { to: 50 }, &column(&OPENS, 1), 1.0)
        .take(BTN, Action::Raise { to: 150 }, &column(&FACING_50, 2), 1.0)
        .take(BB, Action::Raise { to: 250 }, &column(&FACING_150, 2), 1.0);
    let expected = [
        root.take(BTN, Action::Raise { to: 600 }, &column(&FACING_250, 2), fa).take(BB, Action::Call, &column(&FACING_600, 1), 1.0),
        root.take(BTN, Action::Raise { to: 900 }, &column(&FACING_250, 3), fb).take(BB, Action::Call, &column(&FACING_900, 1), 1.0),
    ];
    assert_walked(&out, &expected, "projected root");
    assert_eq!(flop_unconditioned(&out), vec![(CO, "not in the heads-up street root".to_string())]);
    let translations = flop_translations(&out);
    assert_eq!(translations.len(), 1, "{translations:?}");
    assert_eq!(translations[0].0, BTN);
    assert!((f64::from(translations[0].1) - s).abs() < 1e-6, "dead money is in the pot: {} vs {s}", translations[0].1);
    assert!((f64::from(translations[0].2[0].0) - a).abs() < 1e-6 && (f64::from(translations[0].2[1].0) - b).abs() < 1e-6);
}

/// Ruling 15-I2 (brief Step 5): the missing-snapshot cause follows how the street OPENED. The
/// admitted-projection hand above opened three-way (BB, CO, BTN), and hero's decision after B's
/// fold has an admitted heads-up root -- which is a financial root, not a snapshot. Replayed with
/// no snapshot, or with only an incompatible one (keyed by other ranges), the street is
/// `multiway prior street` once per actual actor in order of first action, every incoming `q` and
/// mass kept. A compatible snapshot whose root the model cannot reproduce still reports that
/// concrete provenance. A street that opened heads-up without a compatible snapshot stays
/// `no compatible snapshot`.
#[test]
fn a_street_that_opened_multiway_without_a_compatible_snapshot_is_multiway_even_with_a_projected_root() {
    let flop = three_way_flop(BTN);
    let line = [
        Action::Bet { to: 50 },
        Action::Call,
        Action::Raise { to: 150 },
        Action::Raise { to: 250 },
        Action::Fold,
        Action::Raise { to: 700 },
        Action::Call,
    ];
    let decision = act(&flop, &line[..5]);
    assert_eq!(core_model::street_root(&decision).expect("an admitted projection").projected_from, 3);
    let turn = core_model::set_board(&act(&flop, &line), &turn_board()).expect("the turn");
    let incoming = replay_with(&flop, &[]);
    let hash = |seat: Seat| core_ranges::hash_scaled(incoming.ranges[usize::from(seat.0)].as_ref().expect("a dealt seat's range"));
    let reproducible_but_incompatible = snapshot_at(&decision, &menu_tree(&[&[]]), &flop_live(), [[7; 32]; 2], 22);
    let multiway = |seat: Seat| (seat, "multiway prior street".to_string());

    for (snapshots, what) in [(vec![], "no snapshot"), (vec![reproducible_but_incompatible], "an incompatible snapshot")] {
        let out = replay_with(&turn, &snapshots);
        assert_eq!(flop_unconditioned(&out), vec![multiway(BB), multiway(CO), multiway(BTN)], "{what}");
        assert_eq!(out.branches.iter().map(bits).collect::<Vec<_>>(), incoming.branches.iter().map(bits).collect::<Vec<_>>(), "{what}: incoming q and masses");
        assert_eq!(out.log_reach, incoming.log_reach, "{what}");
        assert!(cap_reasons(&out).is_empty() && !out.reasons.contains(&inherited()), "{what}: {:?}", out.reasons);
    }

    // Concrete reconstruction provenance is preferred: compatible, but its prefix reproduces nothing.
    let mut unreproducible = snapshot_at(&decision, &menu_tree(&[&[]]), &flop_live(), [hash(BB), hash(BTN)], 23);
    unreproducible.provenance.solved_prefix = vec![(BB, Action::Check)];
    let out = replay_with(&turn, &[unreproducible]);
    let cause = "snapshot root not reproducible".to_string();
    assert_eq!(flop_unconditioned(&out), vec![(BB, cause.clone()), (CO, cause.clone()), (BTN, cause)]);

    // A street that opened heads-up keeps `no compatible snapshot`, with or without an incompatible one.
    let heads_up = walked_turn(&check_bet73_call());
    let mut other_ranges = walk_snapshot(&menu_tree(&[&[]]), 24);
    other_ranges.key.root_range_hashes = [[7; 32]; 2];
    for snapshots in [vec![], vec![other_ranges]] {
        let out = replay_with(&heads_up, &snapshots);
        let cause = "no compatible snapshot".to_string();
        assert_eq!(flop_unconditioned(&out), vec![(SB, cause.clone()), (BB, cause)]);
    }
}

/// The cap runs once per action inside the walk, and each surviving branch keeps its own path.
/// Three branches enter the flop (weights 0.5 / 0.3 / 0.2 from a combo-independent split on the
/// UTG, so every mass stays uniform); the BB's 73 splits each into Bet50 and Bet100, six live
/// branches, and the cap keeps the four heaviest and merges the other two into the residual. The
/// SB's call is then conditioned in each survivor at its own node ([0,1] after Bet50, [0,2] after
/// Bet100) and never in the frozen residual.
#[test]
fn the_cap_inside_the_walk_keeps_each_survivor_on_its_own_path() {
    let snapshots = [walk_snapshot(&menu_tree(&[&[], &[0], &[0, 1], &[0, 2]]), 3)];
    let (fa, fb) = harmonic(0.73, 0.5, 1.0);
    let utg = Seat(2);
    let splits = [(Action::Fold, 0.5), (Action::Call, 0.3), (Action::Raise { to: 30 }, 0.2)];
    // `seeded`: reasons already on the output when the walk starts (an earlier boundary's).
    let walked_with = |snaps: &[StreetSnapshot], line: &[Action], seeded: &[ApproxReason]| {
        let mut out = replay_with(&walk_flop(), &[]);
        out.branches = core_replay::split_action(&out.branches, utg, &splits.iter().map(|&(a, f)| (a, f, vec![1.0; COMBOS])).collect::<Vec<_>>());
        out.reasons.extend_from_slice(seeded);
        let state = act(&walk_flop(), line);
        let store = PreflopStore::from_sources(vec![]);
        walk_postflop(&ReplayInput { cfg: &state.config, state: &state, store: &store, snapshots: snaps, missing: &[] }, Street::Flop, &mut out);
        out
    };
    let walked_from = |line: &[Action], seeded: &[ApproxReason]| walked_with(&snapshots, line, seeded);
    let walked = |line: &[Action]| walked_from(line, &[]);
    let before_call = walked(&[Action::Check, Action::Bet { to: 73 }]);
    let after_call = walked(&check_bet73_call());

    // Independently: each entering branch checks, then splits.
    let entering: Vec<Expected> = splits
        .iter()
        .map(|&(a, f)| Expected { q: f, masses: vec![vec![1.0; COMBOS]; 6], line: vec![(utg, a)] }.take(SB, Action::Check, &column(&ROOT, 0), 1.0))
        .collect();
    let a = |e: &Expected| e.take(BB, Action::Bet { to: 50 }, &column(&IP_MENU, 1), fa);
    let b = |e: &Expected| e.take(BB, Action::Bet { to: 100 }, &column(&IP_MENU, 2), fb);
    // Children ids 4..=9 in creation order: (4 A, 5 B) of 0.5, (6 A, 7 B) of 0.3, (8 A, 9 B) of 0.2;
    // the heaviest four are 4, 6, 5 and 8, and 7 becomes the residual that absorbs 9.
    let (a0, b0, a1, b1, a2, b2) = (a(&entering[0]), b(&entering[0]), a(&entering[1]), b(&entering[1]), a(&entering[2]), b(&entering[2]));
    let lightest_survivor = [a0.q, b0.q, a1.q, a2.q].into_iter().fold(f64::INFINITY, f64::min);
    assert!(lightest_survivor > b1.q && b1.q > b2.q, "the fixture's weights order as described");
    let ids = |o: &ReplayOutput| o.branches.iter().map(|b| (b.id, b.residual)).collect::<Vec<_>>();
    assert_eq!(ids(&before_call), vec![(4, false), (5, false), (6, false), (8, false), (7, true)]);
    assert_eq!(ids(&after_call), ids(&before_call));
    let survivors = [(&a0, &AFTER_50), (&b0, &AFTER_100), (&a1, &AFTER_50), (&a2, &AFTER_50)];
    for (k, (e, call)) in survivors.iter().enumerate() {
        close_to(before_call.branches[k].q, e.q, &format!("survivor {k} before the call"));
        let called = e.take(SB, Action::Call, &column(*call, 1), 1.0);
        close_to(after_call.branches[k].q, called.q, &format!("survivor {k} conditioned at its own node"));
        assert_eq!(after_call.branches[k].translated, called.line);
    }
    let residual = &after_call.branches[4];
    close_to(before_call.branches[4].q, b1.q + b2.q, "the residual merges the two lightest");
    assert_eq!(residual.q.to_bits(), before_call.branches[4].q.to_bits(), "the residual is frozen through the call");
    assert!(residual.translated.is_empty());

    // Ruling 15-I1 (spec 8.4): the cap is disclosed as `BranchResidual{seat: hero, cause: "cap"}`,
    // once, with the residual's share of the total weight as the street leaves it -- recomputed
    // after the call (the live weights shrink, the frozen residual does not), never the share at
    // the cap. Independently: `100 * q_R / sum_k q_k` over the expected weights.
    let merged = b1.q + b2.q;
    let live_at_cap: f64 = [&a0, &b0, &a1, &a2].iter().map(|e| e.q).sum();
    let at_cap = 100.0 * merged / (live_at_cap + merged);
    let called: f64 = survivors.iter().map(|(e, call)| e.take(SB, Action::Call, &column(*call, 1), 1.0).q).sum();
    let after = 100.0 * merged / (called + merged);
    for (out, want, rounded, what) in [(&before_call, at_cap, 18.1102, "at the cap"), (&after_call, after, 24.1470, "after the call")] {
        let caps = cap_reasons(out);
        assert_eq!(caps.len(), 1, "{what}: one cap disclosure, {:?}", out.reasons);
        let (seat, pct, cause) = &caps[0];
        assert_eq!((*seat, cause.as_str()), (BB, "cap"), "{what}: hero's seat and the cap cause");
        assert!((f64::from(*pct) - want).abs() < 1e-4, "{what}: {pct} != {want}");
        assert!((want - rounded).abs() < 1e-3, "{what}: {want} vs the review's {rounded}");
    }
    // No residual, no cap disclosure: three live branches after the check alone.
    let uncapped = walked(&[Action::Check]);
    assert!(uncapped.branches.iter().all(|b| !b.residual) && cap_reasons(&uncapped).is_empty(), "{:?}", uncapped.reasons);

    // A disclosure an earlier boundary recorded is replaced in place by the current share, never
    // repeated; with no residual left to disclose, a stale one is removed.
    let stale = ApproxReason::BranchResidual { seat: BB, residual_mass_pct: 99.0, cause: "cap".into() };
    let other_cause = ApproxReason::BranchResidual { seat: BB, residual_mass_pct: 5.0, cause: "missing node k".into() };
    let updated = walked_from(&check_bet73_call(), &[stale.clone(), other_cause.clone()]);
    let caps = cap_reasons(&updated);
    assert_eq!(caps.len(), 2, "{:?}", updated.reasons);
    assert!(caps[0].2 == "cap" && (f64::from(caps[0].1) - after).abs() < 1e-4, "replaced in place: {caps:?}");
    assert_eq!(caps[1], (BB, 5.0, "missing node k".to_string()), "another cause is not the cap's");
    let cleared = walked_from(&[Action::Check], &[stale]);
    assert!(!cleared.reasons.iter().any(|r| matches!(r, ApproxReason::BranchResidual { cause, .. } if cause == "cap")), "{:?}", cleared.reasons);

    // Ruling 15-N1 (the re-review's probe P2): a snapshot may inherit a cap disclosure with another
    // share (a cache-origin snapshot carries its stored entry's reasons). Exactly one hero cap
    // reason survives, with the current share, at the first cap disclosure's position.
    let mut inheriting = snapshots.to_vec();
    inheriting[0].reasons.push(ApproxReason::BranchResidual { seat: BB, residual_mass_pct: 50.0, cause: "cap".into() });
    let earlier = ApproxReason::BranchResidual { seat: BB, residual_mass_pct: 10.0, cause: "cap".into() };
    for seeded in [vec![], vec![earlier]] {
        let out = walked_with(&inheriting, &check_bet73_call(), &seeded);
        let caps: Vec<(usize, &ApproxReason)> = out.reasons.iter().enumerate().filter(|(_, r)| matches!(r, ApproxReason::BranchResidual { .. })).collect();
        assert_eq!(caps.len(), 1, "seeded {}: {:?}", seeded.len(), out.reasons);
        match caps[0] {
            (i, ApproxReason::BranchResidual { seat, residual_mass_pct, cause }) => {
                assert_eq!((*seat, cause.as_str()), (BB, "cap"));
                assert!((f64::from(*residual_mass_pct) - after).abs() < 1e-4, "the current share, not 50 or 10: {residual_mass_pct}");
                if !seeded.is_empty() {
                    assert_eq!(i, 1, "in place of the earlier boundary's disclosure (after the preflop reason)");
                }
            }
            other => panic!("{other:?}"),
        }
    }
}

/// Ruling 15-N2: the disclosure also runs on the walk's no-snapshot exit. A residual that already
/// exists when a street has no compatible snapshot (here five branches entering the flop, weights
/// 0.3 / 0.25 / 0.2 / 0.15 / 0.1, capped into four plus a 0.1 residual) is disclosed once with its
/// share, `100 * 0.1 / 1.0 = 10%`, and every branch is left bit for bit.
#[test]
fn the_cap_residual_is_disclosed_on_the_no_snapshot_exit_too() {
    let mut out = replay_with(&walk_flop(), &[]);
    let weights = [(Action::Fold, 0.3), (Action::Call, 0.25), (Action::Raise { to: 30 }, 0.2), (Action::Raise { to: 40 }, 0.15), (Action::Raise { to: 50 }, 0.1)];
    out.branches = core_replay::split_action(&out.branches, Seat(2), &weights.iter().map(|&(a, f)| (a, f, vec![1.0; COMBOS])).collect::<Vec<_>>());
    core_replay::cap_branches(&mut out.branches);
    assert_eq!(out.branches.iter().filter(|b| b.residual).count(), 1);
    let before: Vec<Bits> = out.branches.iter().map(bits).collect();
    let state = walked_turn(&check_bet73_call());
    let store = PreflopStore::from_sources(vec![]);
    walk_postflop(&ReplayInput { cfg: &state.config, state: &state, store: &store, snapshots: &[], missing: &[] }, Street::Flop, &mut out);
    assert_eq!(out.branches.iter().map(bits).collect::<Vec<_>>(), before, "no snapshot: nothing is conditioned");
    assert_eq!(flop_unconditioned(&out), vec![(SB, "no compatible snapshot".to_string()), (BB, "no compatible snapshot".to_string())]);
    let caps = cap_reasons(&out);
    assert_eq!(caps.len(), 1, "{:?}", out.reasons);
    assert_eq!((caps[0].0, caps[0].2.as_str()), (BB, "cap"));
    assert!((f64::from(caps[0].1) - 10.0).abs() < 1e-5, "{}", caps[0].1);
}

/// Every `BranchResidual` reason of `out`, as `(seat, residual_mass_pct, cause)`.
fn cap_reasons(out: &ReplayOutput) -> Vec<(Seat, f32, String)> {
    out.reasons
        .iter()
        .filter_map(|r| match r {
            ApproxReason::BranchResidual { seat, residual_mass_pct, cause } => Some((*seat, *residual_mass_pct, cause.clone())),
            _ => None,
        })
        .collect()
}

/// The engine hands replay one identity's snapshots (`SnapshotStore::for_identity`); a slice
/// mixing model revisions is a caller bug, refused loudly rather than keyed by an arbitrary one.
#[test]
#[should_panic(expected = "replay takes one identity's snapshots")]
fn a_slice_mixing_model_revisions_is_refused() {
    let a = walk_snapshot(&menu_tree(&[&[]]), 2);
    let mut b = walk_snapshot(&menu_tree(&[&[]]), 3);
    b.key.model_revision = 1;
    let _ = replay_with(&walked_turn(&check_bet73_call()), &[a, b]);
}

/// Section 13.1's T6 (`replay_cross_actor_branches`) through snapshot nodes: the villain's off-menu
/// wager against a snapshot menu. Hero is the SB with As Ad, the villain the BB; SB raises to 75
/// preflop, BB calls (150 in the pot, 925 behind each), flop Kh 7d 3s. Both players hold the two
/// combos 2c2d and 2c2h with masses `(1, 1)`. Hero checks (probability 1: `M = 1`), the villain bets
/// 100 into 150 (`s = 2/3` between 75 and 150: `f = 0.6 / 0.4`) with `P_A = (0.8, 0.1)` and
/// `P_B = (0.1, 0.4)`, hero shoves at the unexported nodes after 75 and after 150 (not applied: each
/// branch judges it at its own node), and the villain calls with `0.9` after 75 and `0.1` after 150.
/// A branch with `M_X = 0` is dropped for every seat.
#[test]
fn replay_cross_actor_branches_through_snapshot_nodes() {
    const HERO_CHECKS: [[f32; 2]; 3] = [[1.0, 1.0], [0.0, 0.0], [0.0, 0.0]];
    const VILLAIN: [[f32; 2]; 3] = [[0.1, 0.5], [0.8, 0.1], [0.1, 0.4]];
    const VILLAIN_NEVER_150: [[f32; 2]; 3] = [[0.2, 0.9], [0.8, 0.1], [0.0, 0.0]];
    const CALLS_A: [[f32; 2]; 2] = [[0.1, 0.1], [0.9, 0.9]];
    const CALLS_B: [[f32; 2]; 2] = [[0.9, 0.9], [0.1, 0.1]];
    let board = vec![Card(46), Card(21), Card(7)];
    let flop = core_model::set_board(&act(&table(SB), &[Action::Fold, Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 75 }, Action::Call]), &board)
        .expect("the flop");
    assert_eq!((flop.derived.to_act, flop.derived.pot), (Some(SB), 150));
    let shove = Action::AllIn { to: 925 };
    let line = [Action::Check, Action::Bet { to: 100 }, shove, Action::Call];
    let tree = |villain: &'static [[f32; 2]]| {
        let facing = |to: u32| vec![Action::Fold, Action::Call, Action::AllIn { to }];
        vec![
            tree_node(&[], "oop", vec![Action::Check, Action::Bet { to: 75 }, Action::Bet { to: 150 }], vec![None, None, None], Some(&HERO_CHECKS)),
            tree_node(&[1], "ip", vec![Action::Fold, Action::Call], vec![Some(150), Some(300)], None),
            tree_node(&[2], "ip", vec![Action::Fold, Action::Call], vec![Some(150), Some(450)], None),
            tree_node(&[0], "ip", vec![Action::Check, Action::Bet { to: 75 }, Action::Bet { to: 150 }], vec![Some(150), None, None], Some(villain)),
            tree_node(&[0, 1], "oop", facing(925), vec![Some(225), Some(300), None], None),
            tree_node(&[0, 2], "oop", facing(925), vec![Some(300), Some(450), None], None),
            tree_node(&[0, 1, 2], "ip", vec![Action::Fold, Action::Call], vec![Some(1075), Some(2000)], Some(&CALLS_A)),
            tree_node(&[0, 2, 2], "ip", vec![Action::Fold, Action::Call], vec![Some(1150), Some(2000)], Some(&CALLS_B)),
        ]
    };
    // The two-combo start, published at the flop root to key the snapshot.
    let mut incoming = ReplayOutput { ranges: vec![None; 6], branches: initial(&flop.dealt), folded_ranges: vec![], log_reach: vec![0.0; 6], reasons: vec![], unsupported: None, snapshots_used: vec![] };
    for s in incoming.branches[0].seats.iter_mut().filter(|s| s.seat == SB || s.seat == BB) {
        s.mass.fill(0.0);
        s.mass[0] = 1.0;
        s.mass[1] = 1.0;
    }
    let mut published = incoming.clone();
    publish(&mut published, &flop);
    let hash = |seat: Seat| core_ranges::hash_scaled(published.ranges[usize::from(seat.0)].as_ref().expect("a dealt seat's range"));
    let live: Vec<bool> = (0..COMBOS).map(|c| c < 2).collect();
    let store = PreflopStore::from_sources(vec![]);
    let walk = |snapshot: &StreetSnapshot, n: usize| {
        let state = act(&flop, &line[..n]);
        let mut out = incoming.clone();
        walk_postflop(&ReplayInput { cfg: &state.config, state: &state, store: &store, snapshots: std::slice::from_ref(snapshot), missing: &[] }, Street::Flop, &mut out);
        out
    };
    let near = |a: f64, b: f64| assert!((a - b).abs() < 5e-4, "{a} is not within 5e-4 of {b}");
    let snapshot = snapshot_at(&flop, &tree(&VILLAIN), &live, [hash(SB), hash(BB)], 11);
    assert_eq!(snapshot.provenance.solved_prefix, vec![], "hero's decision at the flop root");
    // The exact figures come from the snapshot's own f32 probabilities, widened (0.8f32 is
    // 0.800000011920929); the section 13.1 figures, rounded, are checked beside them.
    let (pa, pb) = ([f64::from(0.8f32), f64::from(0.1f32)], [f64::from(0.1f32), f64::from(0.4f32)]);
    let (fa, fb) = harmonic(100.0 / 150.0, 0.5, 1.0);
    let (ma, mb) = ((pa[0] + pa[1]) / 2.0, (pb[0] + pb[1]) / 2.0);
    let (qa, qb) = (fa * ma, fb * mb);
    let villain = [fa * pa[0] + fb * pb[0], fa * pa[1] + fb * pb[1]];

    // The villain's off-menu bet splits the branch: q = 0.27 / 0.10.
    let split = walk(&snapshot, 2);
    assert_eq!(split.branches.iter().map(|b| b.translated.clone()).collect::<Vec<_>>(), vec![
        vec![(SB, Action::Check), (BB, Action::Bet { to: 75 })],
        vec![(SB, Action::Check), (BB, Action::Bet { to: 150 })],
    ]);
    close_to(fa, 0.6, "f_A");
    close_to(split.branches[0].q, qa, "q_A");
    close_to(split.branches[1].q, qb, "q_B");
    near(split.branches[0].q, 0.27);
    near(split.branches[1].q, 0.10);
    let (bb, sb) = (usize::from(BB.0), usize::from(SB.0));
    let villain_a = &split.branches[0].seats[bb].mass;
    let villain_b = &split.branches[1].seats[bb].mass;
    let scale = split.log_reach[bb].exp();
    for c in [0, 1] {
        close_to(villain_a[c] * scale, pa[c] / ma, "villain mass in A");
        close_to(villain_b[c] * scale, pb[c] / mb, "villain mass in B");
    }
    near(villain_a[0] * scale, 1.7778);
    near(villain_a[1] * scale, 0.2222);
    near(villain_b[0] * scale, 0.4);
    near(villain_b[1] * scale, 1.6);
    close_log(split.log_reach[bb], villain[0].ln(), "villain log_reach");
    close_log(split.log_reach[sb], (qa + qb).ln(), "hero log_reach");
    near(villain[0], 0.52);
    near(qa + qb, 0.37);
    close_to(marginal(&split.branches, BB)[1], villain[1] / villain[0], "villain marginal");
    near(marginal(&split.branches, BB)[1], 0.4231);
    close_to(posterior(&split.branches, BB, 0)[0], fa * pa[0] / villain[0], "villain posterior, combo 1");
    close_to(posterior(&split.branches, BB, 1)[0], fa * pa[1] / villain[1], "villain posterior, combo 2");
    near(posterior(&split.branches, BB, 0)[0], 0.923);
    near(posterior(&split.branches, BB, 1)[0], 0.273);
    for combo in [0, 1] {
        close_to(posterior(&split.branches, SB, combo)[0], qa / (qa + qb), "hero posterior");
        near(posterior(&split.branches, SB, combo)[0], 0.7297);
    }
    assert_eq!(split.branches[0].seats[sb].mass, split.branches[1].seats[sb].mass, "hero's masses are copied into both children");
    let translation = flop_translations(&split);
    assert_eq!(translation.len(), 1);
    assert!((f64::from(translation[0].1) - 2.0 / 3.0).abs() < 1e-6);
    assert!((f64::from(translation[0].2[0].1) - 0.6).abs() < 1e-6 && (f64::from(translation[0].2[1].1) - 0.4).abs() < 1e-6);

    // Hero's shove is judged at each branch's own node, [0, 1] and [0, 2]: neither is exported.
    let shoved = walk(&snapshot, 3);
    assert_eq!(shoved.branches.iter().map(bits).collect::<Vec<_>>(), split.branches.iter().map(|b| {
        let mut b = b.clone();
        b.translated.push((SB, shove));
        bits(&b)
    }).collect::<Vec<_>>());
    assert_eq!(flop_unconditioned(&shoved), vec![(SB, "uncovered path [0, 1]".to_string()), (SB, "uncovered path [0, 2]".to_string())]);

    // The villain's later on-menu call: 0.9 in branch A, 0.1 in branch B.
    let called = walk(&snapshot, 4);
    let (ca, cb) = (f64::from(0.9f32), f64::from(0.1f32));
    let (qa2, qb2) = (qa * ca, qb * cb);
    let villain2 = [fa * pa[0] * ca + fb * pb[0] * cb, fa * pa[1] * ca + fb * pb[1] * cb];
    close_to(called.branches[0].q, qa2, "q_A");
    close_to(called.branches[1].q, qb2, "q_B");
    near(called.branches[0].q, 0.243);
    near(called.branches[1].q, 0.010);
    close_log(called.log_reach[bb], villain2[0].ln(), "villain log_reach");
    close_log(called.log_reach[sb], (qa2 + qb2).ln(), "hero log_reach");
    near(villain2[0], 0.436);
    near(villain2[1], 0.070);
    close_to(marginal(&called.branches, BB)[1], villain2[1] / villain2[0], "villain marginal");
    for combo in [0, 1] {
        close_to(posterior(&called.branches, SB, combo)[0], qa2 / (qa2 + qb2), "hero posterior");
        near(posterior(&called.branches, SB, combo)[0], 0.9605);
    }
    let (a, b) = (&called.branches[0].seats, &called.branches[1].seats);
    close_to(a[bb].mass[0] / a[bb].mass[1], pa[0] / pa[1], "villain's masses keep their shape in A");
    close_to(b[bb].mass[0] / b[bb].mass[1], pb[0] / pb[1], "villain's masses keep their shape in B");
    near(a[bb].mass[0] / a[bb].mass[1], 8.0);
    near(b[bb].mass[0] / b[bb].mass[1], 0.25);
    assert_eq!(a[sb].mass, b[sb].mass, "hero's masses are untouched");
    assert_eq!(a[sb].mass[0], a[sb].mass[1]);

    // A branch with M_X = 0 is never created: the villain never bets 150 here.
    let dropped = walk(&snapshot_at(&flop, &tree(&VILLAIN_NEVER_150), &live, [hash(SB), hash(BB)], 12), 2);
    assert_eq!(dropped.branches.len(), 1);
    assert_eq!(dropped.branches[0].translated, vec![(SB, Action::Check), (BB, Action::Bet { to: 75 })]);
    close_to(dropped.branches[0].q, qa, "q_A");
}

// ---------------------------------------------------------------------------------------------
// Section 13.1 names whose postflop halves live in this file (plan 3 Task 19's test-name audit;
// ruling 15-D2 placed both tests here rather than in `tests/branches.rs`). Each spec-named test
// runs the test that asserts that half, with its numeric assertions unchanged.
// ---------------------------------------------------------------------------------------------

/// Section 13.1's `replay_missing_continuation`, postflop half: a deleted continuation leaves the
/// action unapplied in that branch (the actor's masses as conditioned so far, `q_k` unchanged),
/// `UnconditionedPriorStreet{cause: "uncovered path [0, 1]"}` persists, nothing is invented, and the
/// other seat's actions, the same seat's later action at a covered path and the other branch keep
/// conditioning (`replay_missing_continuation_postflop`). The preflop half -- a branch stopped on a
/// missing node is frozen for the rest of the street and hero's lookup in it is unresolved mass --
/// is `tests/replay.rs`'s test of the same name.
#[test]
fn replay_missing_continuation() {
    replay_missing_continuation_postflop();
}

/// Section 13.1's `replay_cross_actor_branches` (T6), last clause: "the same rules apply to a
/// postflop off-menu wager against a snapshot menu" -- the villain's 100-into-150 bet over the
/// snapshot's 75/150 menu gives `q = 0.27 / 0.10`, the rest of T6's figures and the dropped
/// `M_X = 0` branch (`replay_cross_actor_branches_through_snapshot_nodes`). The kernel example
/// itself is `tests/branches.rs`'s test of the same name.
#[test]
fn replay_cross_actor_branches() {
    replay_cross_actor_branches_through_snapshot_nodes();
}
