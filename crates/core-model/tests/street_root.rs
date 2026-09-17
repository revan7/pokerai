use core_model::state::BeginHand; // explicit: `proto::BeginHand` is the DTO of the same name
use core_model::*;
use proto::*;

fn cfg(sb: u32, bb: u32) -> HandConfig {
    HandConfig { config_revision: 1, sb_chips: sb, bb_chips: bb, straddle: None, rake: Rake::TimeCharge, chip_label: "$1".into() }
}
fn seats(ids: &[u8]) -> Vec<Seat> { ids.iter().map(|i| Seat(*i)).collect() }
fn begin(cfg: &HandConfig, dealt: &[u8], stacks: &[u32]) -> HandState {
    begin_hand(cfg, BeginHand { hand_id: 1, button: Seat(5), hero: Seat(dealt[0]), dealt: seats(dealt), stacks_start: stacks.to_vec(), hero_cards: None }).unwrap()
}
/// The same state seen by `hero_seat` holding two cards.
fn seen_by(state: &HandState, hero_seat: u8) -> HandState {
    let mut s = state.clone();
    s.hero = Seat(hero_seat);
    s.hero_cards = Some(parse_hand("QhQd").unwrap());
    s
}
fn act(state: HandState, action: Action) -> HandState { apply_action(&state, action).unwrap() }
fn flop(state: HandState) -> HandState { set_board(&state, &parse_cards("Ks7c2s").unwrap()).unwrap() }

/// Blinds 25/50, BTN folds, SB completes, BB checks: flop root pot 100, stacks 500/500, OOP = SB(0), IP = BB(1).
fn hu_flop_root() -> HandState {
    let s = begin(&cfg(25, 50), &[5, 0, 1], &[600, 550, 550]);
    let s = act(s, Action::Fold);
    let s = act(s, Action::Call);
    let s = act(s, Action::Check);
    flop(s)
}

#[test]
fn street_root_reconstruction() {
    let s = hu_flop_root();
    let root = street_root(&seen_by(&s, 0)).unwrap();
    assert_eq!((root.pot_root, root.stack_oop_root, root.stack_ip_root, root.projected_from, root.dead_this_street), (100, 500, 500, 2, 0));
    assert_eq!((root.oop, root.ip), (Seat(0), Seat(1)));
    assert!(root.history.is_empty());
    let after_bet = act(s.clone(), Action::Bet { to: 50 });
    assert_eq!((after_bet.derived.pot, after_bet.derived.stacks_remaining[0], after_bet.derived.stacks_remaining[1]), (150, 450, 500));
    let snap = street_root(&seen_by(&after_bet, 1)).unwrap();
    assert_eq!((snap.pot_root, snap.stack_oop_root, snap.stack_ip_root), (100, 500, 500), "the root, never the current pot");
    assert_eq!(snap.history, vec![(Seat(0), Action::Bet { to: 50 })]);
    let replayed = replay_root(&snap).unwrap();
    assert_eq!((replayed.pot, replayed.to_act, replayed.facing), (150, Some(Seat(1)), 50));
    assert_eq!(replayed.legal, after_bet.derived.legal);
    let after_raise = act(after_bet, Action::Raise { to: 150 });
    assert_eq!((after_raise.derived.pot, after_raise.derived.stacks_remaining[0], after_raise.derived.stacks_remaining[1]), (300, 450, 350));
    assert!(after_raise.derived.legal.contains(&LegalAction::Call { cost: 100 }));
    let snap = street_root(&seen_by(&after_raise, 0)).unwrap();
    assert_eq!((snap.pot_root, snap.stack_oop_root, snap.stack_ip_root), (100, 500, 500));
    assert_eq!(snap.history.len(), 2);
    assert_eq!(replay_root(&snap).unwrap().pot, 300);
    let after_call = act(after_raise, Action::Call);
    assert_eq!((after_call.derived.pot, after_call.derived.stacks_remaining[0], after_call.derived.stacks_remaining[1]), (400, 350, 350));
    assert_eq!(after_call.phase, HandPhase::AwaitingBoard { street: Street::Turn });
    let turn = set_board(&after_call, &parse_cards("Ks7c2s9d").unwrap()).unwrap();
    let snap = street_root(&seen_by(&turn, 0)).unwrap();
    assert_eq!((snap.street, snap.pot_root, snap.stack_oop_root, snap.stack_ip_root), (Street::Turn, 400, 350, 350));
    // check-prefix: the actor changes at an unchanged pot
    let checked = act(hu_flop_root(), Action::Check);
    let snap = street_root(&seen_by(&checked, 1)).unwrap();
    assert_eq!(snap.history, vec![(Seat(0), Action::Check)]);
    let replayed = replay_root(&snap).unwrap();
    assert_eq!((replayed.pot, replayed.to_act), (100, Some(Seat(1))));
    assert_eq!(replayed.legal, checked.derived.legal);
    // hero all-in: no decision; the opponent facing the jam has one
    let jammed = act(hu_flop_root(), Action::AllIn { to: 500 });
    assert_eq!(street_root(&seen_by(&jammed, 0)), Err(RootError::NoDecision));
    let snap = street_root(&seen_by(&jammed, 1)).unwrap();
    assert_eq!(snap.history, vec![(Seat(0), Action::AllIn { to: 500 })]);
    assert_eq!(replay_root(&snap).unwrap().legal, vec![LegalAction::Fold, LegalAction::Call { cost: 500 }]);
    // preflop decisions have no street root
    let pre = begin(&cfg(25, 50), &[5, 0, 1], &[600, 550, 550]);
    assert_eq!(street_root(&seen_by(&pre, 5)), Err(RootError::Preflop));
    // a third player all-in preflop keeps the flop multiway although the side pot is HU
    let s = begin(&cfg(25, 50), &[5, 0, 1], &[100, 550, 550]);
    let s = act(s, Action::AllIn { to: 100 });
    let s = act(s, Action::Call);
    let s = act(s, Action::Call);
    assert_eq!(s.phase, HandPhase::AwaitingBoard { street: Street::Flop });
    let s = flop(s);
    assert_eq!(s.derived.to_act, Some(Seat(0)));
    assert_eq!(street_root(&seen_by(&s, 0)), Err(RootError::Multiway { pot_eligible: 3 }));
}

/// Postflop order A = SB(0), B = BB(1), C = BTN(5); every seat holds 1,000 at the flop root (limped pot of 6).
fn three_way_flop() -> HandState {
    let s = begin(&cfg(1, 2), &[5, 0, 1], &[1002, 1002, 1002]);
    let s = act(s, Action::Call);
    let s = act(s, Action::Call);
    let s = act(s, Action::Check);
    let s = flop(s);
    assert_eq!(s.derived.stacks_remaining, vec![1000, 1000, 0, 0, 0, 1000]);
    assert_eq!(s.derived.pot, 6);
    s
}

#[test]
fn multiway_root_projection() {
    // case 1: A bets 50, B folds, C raises to 150, A to act -> admitted, dead 0
    let s = act(act(act(three_way_flop(), Action::Bet { to: 50 }), Action::Fold), Action::Raise { to: 150 });
    let real = seen_by(&s, 0);
    let snap = street_root(&real).unwrap();
    assert_eq!((snap.projected_from, snap.dead_this_street, snap.pot_root), (3, 0, 6));
    assert_eq!(snap.history, vec![(Seat(0), Action::Bet { to: 50 }), (Seat(5), Action::Raise { to: 150 })]);
    let replayed = replay_root(&snap).unwrap();
    assert_eq!(replayed.pot, real.derived.pot);
    assert_eq!(replayed.legal, real.derived.legal);
    assert_eq!(replayed.legal, vec![LegalAction::Fold, LegalAction::Call { cost: 100 }, LegalAction::Raise { min_to: 250, max_to: 1000 }, LegalAction::AllIn { to: 1000 }]);
    // case 2: A bets 50, B calls, C raises to 150, A raises to 250, B folds, C to act -> admitted, dead 50, min re-raise 350
    let s = three_way_flop();
    let s = act(s, Action::Bet { to: 50 });
    let s = act(s, Action::Call);
    let s = act(s, Action::Raise { to: 150 });
    let s = act(s, Action::Raise { to: 250 });
    let s = act(s, Action::Fold);
    let real = seen_by(&s, 5);
    assert_eq!(real.derived.to_act, Some(Seat(5)));
    let snap = street_root(&real).unwrap();
    assert_eq!((snap.projected_from, snap.dead_this_street), (3, 50));
    assert_eq!(snap.history, vec![(Seat(0), Action::Bet { to: 50 }), (Seat(5), Action::Raise { to: 150 }), (Seat(0), Action::Raise { to: 250 })]);
    let replayed = replay_root(&snap).unwrap();
    assert_eq!(replayed.pot, real.derived.pot);
    assert_eq!(replayed.pot, 6 + 50 + 250 + 150);
    assert_eq!(replayed.facing, 250);
    assert_eq!(replayed.legal, real.derived.legal);
    assert!(replayed.legal.contains(&LegalAction::Call { cost: 100 }));
    assert!(replayed.legal.contains(&LegalAction::Raise { min_to: 350, max_to: 1000 }));
    // case 3: A bets 50, B calls, C raises to 150, A folds, B to act -> a call at an unbet root is illegal at step 1
    let s = three_way_flop();
    let s = act(s, Action::Bet { to: 50 });
    let s = act(s, Action::Call);
    let s = act(s, Action::Raise { to: 150 });
    let s = act(s, Action::Fold);
    let real = seen_by(&s, 1);
    assert_eq!(real.derived.to_act, Some(Seat(1)));
    assert_eq!(street_root(&real), Err(RootError::ProjectionNotReproducing { step: 1 }));
    // an out-of-turn sequence is rejected by the replay, never projected
    let mut tampered = s.clone();
    tampered.actions.swap(0, 1);
    assert!(core_model::lifecycle::simulate(&tampered).is_err());
    assert!(matches!(apply_action(&three_way_flop(), Action::Call), Err(RulesError::IllegalAction { .. })));
}

/// The folded player's raise is what these two projections cannot reproduce: it sits in the real
/// `last_full_raise` but is dropped from the survivors-only replay (spec section 10.2).
#[test]
fn projection_rejected_when_a_folded_raise_is_load_bearing() {
    // A(0) bets 50, B(1) raises to 150, C(5) raises to 400, A raises to 700.
    let prefix = || {
        let s = act(three_way_flop(), Action::Bet { to: 50 });
        let s = act(s, Action::Raise { to: 150 });
        let s = act(s, Action::Raise { to: 400 });
        act(s, Action::Raise { to: 700 })
    };
    // B folds: the survivors are A and C, and without B's raise to 150 the projected minimum re-raise
    // at step 3 is 750, so A's real raise to 700 is illegal there.
    let real = seen_by(&act(prefix(), Action::Fold), 5);
    assert_eq!((real.derived.to_act, real.derived.facing, real.derived.last_full_raise), (Some(Seat(5)), 700, 300));
    assert_eq!(street_root(&real), Err(RootError::ProjectionNotReproducing { step: 3 }));
    // B jams 1,000 and C folds: the survivors are A and B, every step of the projection is legal, and
    // the pot, both contributions, both stacks, the facing amount, the actor and the legal set all
    // match -- but the projected `last_full_raise` is 550 where the real one is 300, so the projection
    // is rejected at the step after its last (5).
    let real = seen_by(&act(act(prefix(), Action::AllIn { to: 1000 }), Action::Fold), 0);
    assert_eq!((real.derived.to_act, real.derived.facing, real.derived.last_full_raise), (Some(Seat(0)), 1000, 300));
    assert_eq!(real.derived.pot, 6 + 700 + 1000 + 400);
    let projected = StreetRootSnapshot {
        street: Street::Flop,
        board: parse_cards("Ks7c2s").unwrap(),
        oop: Seat(0),
        ip: Seat(1),
        pot_root: 6,
        stack_oop_root: 1000,
        stack_ip_root: 1000,
        dead_this_street: 400,
        projected_from: 3,
        history: vec![
            (Seat(0), Action::Bet { to: 50 }),
            (Seat(1), Action::Raise { to: 150 }),
            (Seat(0), Action::Raise { to: 700 }),
            (Seat(1), Action::AllIn { to: 1000 }),
        ],
        bb_chips: 2,
    };
    let replayed = replay_root(&projected).unwrap();
    assert_eq!((replayed.pot, replayed.to_act, replayed.facing), (real.derived.pot, Some(Seat(0)), 1000));
    assert_eq!(replayed.stacks_remaining[0], real.derived.stacks_remaining[0]);
    assert_eq!(replayed.stacks_remaining[1], real.derived.stacks_remaining[1]);
    assert_eq!(replayed.committed_this_street[0], real.derived.committed_this_street[0]);
    assert_eq!(replayed.committed_this_street[1], real.derived.committed_this_street[1]);
    assert_eq!(replayed.legal, real.derived.legal);
    assert_eq!((replayed.last_full_raise, real.derived.last_full_raise), (550, 300), "only the minimum re-raise differs");
    assert_eq!(street_root(&real), Err(RootError::ProjectionNotReproducing { step: 5 }));
}

/// A snapshot can be deserialized from outside the crate, and [`core_model::Round`]'s preconditions are
/// always-on assertions, so `replay_root` admits it into typed errors instead of aborting the process;
/// `street_root` likewise refuses a stored decision that its own history does not replay to.
#[test]
fn replay_root_admits_its_snapshot_and_street_root_refuses_a_stale_decision() {
    let base = StreetRootSnapshot {
        street: Street::Flop,
        board: parse_cards("Ks7c2s").unwrap(),
        oop: Seat(0),
        ip: Seat(1),
        pot_root: 100,
        stack_oop_root: 500,
        stack_ip_root: 500,
        dead_this_street: 0,
        projected_from: 2,
        history: vec![],
        bb_chips: 50,
    };
    assert_eq!(replay_root(&base).unwrap().to_act, Some(Seat(0)));
    let bad = |edit: fn(&mut StreetRootSnapshot)| {
        let mut snap = base.clone();
        edit(&mut snap);
        replay_root(&snap).unwrap_err()
    };
    for err in [
        bad(|s| s.street = Street::Preflop),
        bad(|s| s.ip = s.oop),
        bad(|s| s.oop = Seat(9)),
        bad(|s| s.bb_chips = 0),
        bad(|s| s.stack_ip_root = 0),
        bad(|s| s.pot_root = u32::MAX),
    ] {
        assert!(matches!(err, RulesError::InvalidConfig { .. }), "{err}");
    }
    // an action after the street closed names its own 1-based step
    let mut closed = base.clone();
    closed.history = vec![(Seat(0), Action::Check), (Seat(1), Action::Check), (Seat(0), Action::Bet { to: 50 })];
    let err = replay_root(&closed).unwrap_err().to_string();
    assert!(err.contains("step 3"), "{err}");
    // the stored decision must be the one the recorded history replays to
    let mut stale = seen_by(&hu_flop_root(), 0);
    stale.derived.pot += 1;
    assert_eq!(street_root(&stale), Err(RootError::Inconsistent { step: 0 }));
}
