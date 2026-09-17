use core_model::settlement::{layer_pots, refund_uncalled};
use core_model::state::BeginHand; // explicit: `proto::BeginHand` is the DTO of the same name
use core_model::*;
use proto::*;

fn cfg(sb: u32, bb: u32, straddle: Option<u32>) -> HandConfig {
    HandConfig { config_revision: 1, sb_chips: sb, bb_chips: bb, straddle: straddle.map(|a| UtgStraddle { amount_chips: a }), rake: Rake::TimeCharge, chip_label: "$1".into() }
}
fn seats(ids: &[u8]) -> Vec<Seat> { ids.iter().map(|i| Seat(*i)).collect() }
fn begin(cfg: &HandConfig, button: u8, dealt: &[u8], stacks: &[u32]) -> HandState {
    begin_hand(cfg, BeginHand { hand_id: 1, button: Seat(button), hero: Seat(dealt[0]), dealt: seats(dealt), stacks_start: stacks.to_vec(), hero_cards: None }).unwrap()
}
fn total(d: &Derived) -> u32 { d.stacks_remaining.iter().sum::<u32>() + d.committed_this_street.iter().sum::<u32>() + d.pots.iter().map(|p| p.amount).sum::<u32>() }
fn play(state: HandState, action: Action, start_total: u32) -> HandState {
    let next = apply_action(&state, action).unwrap();
    assert_eq!(total(&next.derived), start_total, "conservation after {action:?}");
    next
}
fn pot(amount: u32, eligible: &[u8]) -> Pot { Pot { amount, eligible: seats(eligible) } }

#[test]
fn side_pot_three_allins() {
    // dealt BTN(5)=200, SB(0)=50, BB(1)=100; blinds 1/2; preflop order BTN, SB, BB
    let s = begin(&cfg(1, 2, None), 5, &[5, 0, 1], &[200, 50, 100]);
    assert_eq!(total(&s.derived), 350);
    let s = play(s, Action::AllIn { to: 200 }, 350);
    assert_eq!(s.derived.legal, vec![LegalAction::Fold, LegalAction::Call { cost: 49 }]);
    let s = play(s, Action::Call, 350);
    assert_eq!(s.phase, HandPhase::Betting { street: Street::Preflop });
    // the invariant between refund and settlement, on the closing committed amounts
    let mut committed = [50, 100, 0, 0, 0, 200];
    let mut stacks = [0, 0, 0, 0, 0, 0];
    assert_eq!(refund_uncalled(&mut committed, &mut stacks), Some((Seat(5), 100)));
    assert_eq!(stacks[5], 100);
    assert_eq!(stacks.iter().sum::<u32>() + committed.iter().sum::<u32>(), 350, "after the refund: stacks 100, live 250");
    let pots = layer_pots(&committed, &[false, false, true, true, true, false]);
    assert_eq!(pots, vec![pot(150, &[0, 1, 5]), pot(100, &[1, 5])]);
    assert_eq!(stacks.iter().sum::<u32>() + pots.iter().map(|p| p.amount).sum::<u32>(), 350);
    // the state machine does the same at closure
    let s = play(s, Action::Call, 350);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(s.derived.pots, vec![pot(150, &[0, 1, 5]), pot(100, &[1, 5])]);
    assert_eq!(s.derived.stacks_remaining, vec![0, 0, 0, 0, 0, 100]);
    assert_eq!(s.derived.all_in, vec![true, true, false, false, false, false]);
    assert_eq!(settle_pots(&s).returned, vec![(Seat(5), 100)]);
    assert_eq!(s.derived.pot, 250);
    assert!(matches!(set_board(&s, &parse_cards("AsKd2c").unwrap()), Err(RulesError::NotAwaitingBoard)));
}

#[test]
fn side_pot_two_contested() {
    // dealt BTN(5)=200, SB(0)=50, BB(1)=100, CO(2)=200; preflop order CO, BTN, SB, BB
    let s = begin(&cfg(1, 2, None), 5, &[5, 0, 1, 2], &[200, 50, 100, 200]);
    let s = play(s, Action::AllIn { to: 200 }, 550);
    let s = play(s, Action::Call, 550);
    let s = play(s, Action::Call, 550);
    let s = play(s, Action::Call, 550);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(settle_pots(&s).returned, vec![]);
    assert_eq!(s.derived.pots, vec![pot(200, &[0, 1, 2, 5]), pot(150, &[1, 2, 5]), pot(200, &[2, 5])]);
    assert_eq!(s.derived.pots.iter().map(|p| p.amount).sum::<u32>(), 550);
}

#[test]
fn allin_runout_single_survivor() {
    // BTN(5)=300 folds, SB(0)=100 shoves, BB(1)=300 calls
    let s = begin(&cfg(1, 2, None), 5, &[5, 0, 1], &[300, 100, 300]);
    let s = play(s, Action::Fold, 700);
    let s = play(s, Action::AllIn { to: 100 }, 700);
    let s = play(s, Action::Call, 700);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(settle_pots(&s).returned, vec![], "no refund: the call matched exactly");
    assert_eq!(s.derived.pots, vec![pot(200, &[0, 1])]);
    assert_eq!(s.derived.stacks_remaining[1], 200, "the survivor keeps 200 chips and the hand is still complete");
    // the caller shorter: BTN shoves 300, SB calls for 100, BB folds
    let s = begin(&cfg(1, 2, None), 5, &[5, 0, 1], &[300, 100, 300]);
    let s = play(s, Action::AllIn { to: 300 }, 700);
    let s = play(s, Action::Call, 700);
    assert_eq!(s.derived.all_in[0], true);
    let s = play(s, Action::Fold, 700);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(settle_pots(&s).returned, vec![(Seat(5), 200)]);
    assert_eq!(s.derived.pots, vec![pot(202, &[0, 5])]);
    assert_eq!(s.derived.stacks_remaining, vec![0, 298, 0, 0, 0, 200]);
}

#[test]
fn lifecycle_streets_and_board() {
    let c = cfg(1, 2, Some(4));
    let s = begin(&c, 5, &[0, 1, 2, 3, 4, 5], &[200; 6]);
    assert_eq!(s.derived.to_act, Some(Seat(3)), "HJ opens over the straddle");
    assert_eq!(s.derived.pot, 7);
    assert_eq!(s.derived.committed_this_street, vec![1, 2, 4, 0, 0, 0]);
    assert!(!is_decision_point(&s), "hero (seat 0) is not to act");
    let mut s = s;
    for _ in 0..5 { s = play(s, Action::Fold, 1200); }
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::FoldedOut });
    assert_eq!(s.derived.pots, vec![pot(5, &[2])]);
    assert_eq!(s.derived.stacks_remaining[2], 198);
    assert_eq!(settle_pots(&s).returned, vec![(Seat(2), 2)]);
    assert!(apply_action(&s, Action::Check).is_err());
    // three-way limped pot to the flop
    let c = cfg(1, 2, None);
    let s = begin(&c, 5, &[5, 0, 1], &[200; 3]);
    let s = play(s, Action::Call, 600);
    let s = play(s, Action::Call, 600);
    assert_eq!(s.derived.legal, vec![LegalAction::Check, LegalAction::Raise { min_to: 4, max_to: 200 }, LegalAction::AllIn { to: 200 }], "BB option");
    let s = play(s, Action::Check, 600);
    assert_eq!(s.phase, HandPhase::AwaitingBoard { street: Street::Flop });
    assert_eq!(s.derived.street, Street::Flop);
    assert_eq!(s.derived.to_act, None);
    assert!(s.derived.legal.is_empty());
    assert_eq!(s.derived.pots, vec![pot(6, &[0, 1, 5])]);
    assert!(matches!(set_board(&s, &parse_cards("AsKd").unwrap()), Err(RulesError::BadBoard { .. })));
    assert!(matches!(set_board(&s, &parse_cards("AsAs2c").unwrap_or_default()), Err(RulesError::BadBoard { .. })));
    let s = set_board(&s, &parse_cards("AsKd2c").unwrap()).unwrap();
    assert_eq!(s.phase, HandPhase::Betting { street: Street::Flop });
    assert_eq!(s.derived.to_act, Some(Seat(0)), "SB acts first postflop");
    assert_eq!(s.derived.last_full_raise, 2);
    let s = set_hero_cards(&s, parse_hand("QhQd").unwrap()).unwrap();
    assert!(!is_decision_point(&s), "SB is to act; hero is the button (dealt[0])");
    assert!(set_hero_cards(&s, parse_hand("AsQd").unwrap()).is_err(), "board card");
    let s = play(s, Action::Bet { to: 10 }, 600);
    let s = play(s, Action::Fold, 600);
    assert!(is_decision_point(&s), "hero (BTN) faces the bet with two cards on record");
    let s = play(s, Action::Call, 600);
    assert_eq!(s.phase, HandPhase::AwaitingBoard { street: Street::Turn });
    assert!(set_board(&s, &parse_cards("AsKd2c").unwrap()).is_err(), "turn needs four cards");
    assert!(set_board(&s, &parse_cards("AsKd3c7h").unwrap()).is_err(), "the flop on record must be kept");
    let s = set_board(&s, &parse_cards("AsKd2c7h").unwrap()).unwrap();
    let s = play(s, Action::Check, 600);
    let s = play(s, Action::Check, 600);
    let s = set_board(&s, &parse_cards("AsKd2c7h7d").unwrap()).unwrap();
    let s = play(s, Action::Check, 600);
    let s = play(s, Action::Check, 600);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::ShowdownReached });
    assert_eq!(s.derived.pots, vec![pot(26, &[0, 5])]);
    let a = abandon(&s);
    assert_eq!(a.phase, HandPhase::Abandoned);
    assert_eq!(derive(&a).to_act, None);
}
