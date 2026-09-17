use core_model::lifecycle::simulate;
use core_model::RulesError;
use proto::*;

fn cfg() -> HandConfig {
    HandConfig { config_revision: 1, sb_chips: 1, bb_chips: 2, straddle: None, rake: Rake::TimeCharge, chip_label: "$1".into() }
}
fn cards(text: &str) -> Vec<Card> { text.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap().parse().unwrap()).collect() }
fn took(seat: u8, street: Street, action: Action, paid: u32) -> TakenAction {
    TakenAction { seat: Seat(seat), street, action, paid }
}
/// Button 5, dealt [BTN(5), SB(0), BB(1)], 200 chips each, blinds 1/2. `simulate` never reads
/// `phase` or `derived`, so a literal state with the defaults is a legitimate input here.
fn state(board: &str, actions: Vec<TakenAction>) -> HandState {
    HandState {
        hand_id: 1, hand_revision: 0, config: cfg(), phase: HandPhase::Betting { street: Street::Preflop },
        button: Seat(5), hero: Seat(5), hero_cards: None,
        dealt: vec![Seat(5), Seat(0), Seat(1)], stacks_start: vec![200, 200, 200],
        board: cards(board), actions, derived: Derived::default(),
    }
}

/// The same literal state with the table, the stacks and the config left to the caller; `stacks` is
/// in `dealt` order, as `HandState.stacks_start` is.
fn table(config: HandConfig, button: Seat, dealt: Vec<Seat>, stacks: Vec<u32>, board: &str, actions: Vec<TakenAction>) -> HandState {
    HandState {
        hand_id: 1, hand_revision: 0, config, phase: HandPhase::Betting { street: Street::Preflop },
        button, hero: dealt[0], hero_cards: None,
        dealt, stacks_start: stacks,
        board: cards(board), actions, derived: Derived::default(),
    }
}

#[test]
fn simulate_opens_the_preflop_round_from_the_posts() {
    let sim = simulate(&state("", vec![])).unwrap();
    let d = sim.derived();
    assert_eq!(sim.phase, HandPhase::Betting { street: Street::Preflop });
    assert_eq!(d.to_act, Some(Seat(5)), "BTN acts first three-handed");
    assert_eq!(d.pot, 3);
    assert_eq!(d.committed_this_street, vec![1, 2, 0, 0, 0, 0], "SB and BB posted; vectors are indexed by Seat.0");
    assert_eq!(d.stacks_remaining, vec![199, 198, 0, 0, 0, 200]);
    assert_eq!(d.folded, vec![false, false, true, true, true, false], "undealt seats are folded");
    assert_eq!(d.all_in, vec![false; 6]);
    assert_eq!(d.last_full_raise, 2);
    assert_eq!(d.legal, vec![LegalAction::Fold, LegalAction::Call { cost: 2 }, LegalAction::Raise { min_to: 4, max_to: 200 }, LegalAction::AllIn { to: 200 }]);
    assert_eq!(d.stacks_remaining.iter().sum::<u32>() + d.committed_this_street.iter().sum::<u32>(), 600, "conservation at the open");
}

#[test]
fn simulate_closes_streets_settles_and_sets_the_phase() {
    let actions = vec![
        took(5, Street::Preflop, Action::Call, 2),
        took(0, Street::Preflop, Action::Call, 1),
        took(1, Street::Preflop, Action::Check, 0),
        took(0, Street::Flop, Action::Bet { to: 10 }, 10),
        took(1, Street::Flop, Action::Fold, 0),
        took(5, Street::Flop, Action::Call, 10),
    ];
    let sim = simulate(&state("AsKd2c", actions)).unwrap();
    let d = sim.derived();
    assert_eq!(sim.phase, HandPhase::AwaitingBoard { street: Street::Turn });
    assert_eq!(d.street, Street::Turn, "AwaitingBoard names the next street");
    assert_eq!(d.to_act, None);
    assert!(d.legal.is_empty());
    assert_eq!(sim.contributed, [12, 2, 0, 0, 0, 12]);
    assert_eq!(sim.returned, vec![], "both streets closed on a match");
    assert_eq!(sim.pots, vec![Pot { amount: 26, eligible: vec![Seat(0), Seat(5)] }], "BB's dead 2 merges into the survivors' layer");
    assert_eq!(d.stacks_remaining, vec![188, 198, 0, 0, 0, 188]);
    assert_eq!(d.stacks_remaining.iter().sum::<u32>() + sim.pots.iter().map(|p| p.amount).sum::<u32>(), 600);
}

#[test]
fn simulate_rejects_states_it_did_not_build() {
    let wrong_paid = vec![took(5, Street::Preflop, Action::Call, 3)];
    assert!(matches!(simulate(&state("", wrong_paid)), Err(RulesError::IllegalAction { .. })), "the recorded paid amount must replay");
    let wrong_street = vec![took(5, Street::Flop, Action::Call, 2)];
    assert!(matches!(simulate(&state("", wrong_street)), Err(RulesError::IllegalAction { .. })), "an action on a street the hand has not reached");
    let out_of_turn = vec![took(0, Street::Preflop, Action::Call, 1)];
    assert!(matches!(simulate(&state("", out_of_turn)), Err(RulesError::IllegalAction { .. })), "seat 5 is to act, not seat 0");
    let no_board = vec![
        took(5, Street::Preflop, Action::Call, 2),
        took(0, Street::Preflop, Action::Call, 1),
        took(1, Street::Preflop, Action::Check, 0),
        took(0, Street::Flop, Action::Bet { to: 10 }, 10),
    ];
    assert!(matches!(simulate(&state("", no_board)), Err(RulesError::NotBetting)), "no flop on record: the flop action cannot replay");
}

/// `settlement.rs` documents `sum(starting stacks) <= u32::MAX` as an admission-time precondition that
/// settlement itself does not re-check; `simulate` is the admission point, so it rejects a table that
/// breaks it with a typed error rather than letting a later `expect` abort. The other rejections here
/// exist for the same reason: `simulate` is the safe entry point for an externally supplied
/// `HandState`, so a table it cannot replay is an `Err`, never a panic out of `Round::open`.
#[test]
fn simulate_admits_only_tables_settlement_can_survive() {
    let three = || vec![Seat(5), Seat(0), Seat(1)];
    let over = table(cfg(), Seat(5), three(), vec![u32::MAX, 1, 1], "", vec![]);
    assert!(
        matches!(simulate(&over), Err(RulesError::InvalidConfig { .. })),
        "starting stacks summing past u32::MAX break settlement's aggregate precondition"
    );
    // The boundary itself is admitted: the sum is computed in u64 and compared, never wrapped.
    let exact = table(cfg(), Seat(5), three(), vec![u32::MAX - 2, 1, 1], "", vec![]);
    let sim = simulate(&exact).expect("a total of exactly u32::MAX is admissible");
    assert_eq!(sim.start_total, u64::from(u32::MAX));
    assert_eq!(sim.derived().stacks_remaining.iter().map(|s| u64::from(*s)).sum::<u64>() + 2, u64::from(u32::MAX), "both short blinds are all-in");

    let short = table(cfg(), Seat(5), three(), vec![200, 200], "", vec![]);
    assert!(matches!(simulate(&short), Err(RulesError::InvalidConfig { .. })), "one starting stack per dealt seat");
    let broke = table(cfg(), Seat(5), three(), vec![200, 0, 200], "", vec![]);
    assert!(matches!(simulate(&broke), Err(RulesError::InvalidConfig { .. })), "a dealt seat with no chips cannot open a street");
    let heads_up = table(cfg(), Seat(5), vec![Seat(5), Seat(0)], vec![200, 200], "", vec![]);
    assert!(matches!(simulate(&heads_up), Err(RulesError::FormatUnsupported { .. })), "two dealt seats are unsupported, not a panic");
}

/// `Sim.returned` carries one entry per street closure that refunded an uncalled portion, appended in
/// closure order — the same list, in the same order, that `tools/gen_fixtures.py` accumulates in its
/// `returned` field. A hand has at most one such closure: a refund means every other unfolded seat is
/// all-in for less, so the closure that refunds also ends the hand (`AllInRunout` or `FoldedOut`).
/// The two hands below therefore pin the order by which closure contributed the single entry.
#[test]
fn returned_records_one_entry_per_refunding_closure_in_closure_order() {
    // The preflop closure refunds: BTN jams 200 into two 50-chip stacks.
    let preflop = table(
        cfg(), Seat(5), vec![Seat(5), Seat(0), Seat(1)], vec![200, 50, 50], "",
        vec![
            took(5, Street::Preflop, Action::AllIn { to: 200 }, 200),
            took(0, Street::Preflop, Action::Call, 49),
            took(1, Street::Preflop, Action::Call, 48),
        ],
    );
    let sim = simulate(&preflop).unwrap();
    assert_eq!(sim.returned, vec![(Seat(5), 150)], "one entry, from the preflop closure");
    assert_eq!(sim.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(sim.pots, vec![Pot { amount: 150, eligible: vec![Seat(0), Seat(1), Seat(5)] }]);
    assert_eq!(sim.derived().stacks_remaining, vec![0, 0, 0, 0, 0, 150]);
    assert_eq!(sim.derived().street, Street::Preflop, "Complete names the last betting street");

    // The preflop closure matches (no entry); the flop closure refunds 72 of SB's 100-chip bet.
    let flop = table(
        cfg(), Seat(5), vec![Seat(5), Seat(0), Seat(1)], vec![200, 200, 30], "AsKd2c",
        vec![
            took(5, Street::Preflop, Action::Call, 2),
            took(0, Street::Preflop, Action::Call, 1),
            took(1, Street::Preflop, Action::Check, 0),
            took(0, Street::Flop, Action::Bet { to: 100 }, 100),
            took(1, Street::Flop, Action::Call, 28),
            took(5, Street::Flop, Action::Fold, 0),
        ],
    );
    let sim = simulate(&flop).unwrap();
    assert_eq!(sim.returned, vec![(Seat(0), 72)], "the refund-free preflop closure appended nothing");
    assert_eq!(sim.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(sim.pots, vec![Pot { amount: 62, eligible: vec![Seat(0), Seat(1)] }]);
    assert_eq!(sim.derived().stacks_remaining, vec![170, 0, 0, 0, 0, 198]);
    assert_eq!(sim.derived().street, Street::Flop);
}

/// Street closure is refund first, then layering (spec 4.3). Layering the uncalled chips first would
/// give the jammer a private side pot instead of returning them, so the two orders are distinguishable
/// and this pins the spec's one.
#[test]
fn street_closure_refunds_before_it_layers() {
    let jam = table(
        cfg(), Seat(5), vec![Seat(5), Seat(0), Seat(1)], vec![200, 50, 50], "",
        vec![
            took(5, Street::Preflop, Action::AllIn { to: 200 }, 200),
            took(0, Street::Preflop, Action::Call, 49),
            took(1, Street::Preflop, Action::Call, 48),
        ],
    );
    let sim = simulate(&jam).unwrap();
    assert_eq!(
        sim.pots,
        vec![Pot { amount: 150, eligible: vec![Seat(0), Seat(1), Seat(5)] }],
        "refund first: one 150 pot, not a 150 main plus a 150 side pot eligible to BTN alone"
    );
    assert_eq!(sim.contributed, [50, 50, 0, 0, 0, 50], "the refunded chips never reached a contribution level");
}

/// Pins the closure rule against the committed PokerKit fixture that `tools/gen_fixtures.py`
/// generated: `fixtures/hands/h0001.json`, a straddled six-handed hand that ends on the flop with an
/// uncalled 16 returned to the bettor, two folded seats' dead money merged into the survivors' layer,
/// and `all_in_runout`. The actions below are the fixture's `steps`; the expectations are read out of
/// the fixture file itself, so the generator's normalization and `close()` are compared, not restated.
#[test]
fn street_closure_matches_the_fixture_generators_settlement() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/hands/h0001.json"))
        .expect("fixtures/hands/h0001.json is committed");
    let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
    let seat = |v: &serde_json::Value| Seat(u8::try_from(v.as_u64().unwrap()).unwrap());
    let chips = |v: &serde_json::Value| u32::try_from(v.as_u64().unwrap()).unwrap();
    let expected_returned: Vec<(Seat, u32)> =
        fixture["returned"].as_array().unwrap().iter().map(|e| (seat(&e[0]), chips(&e[1]))).collect();
    let last = fixture["steps"].as_array().unwrap().last().unwrap();
    let expected_pots: Vec<Pot> = last["after"]["pots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Pot { amount: chips(&p["amount"]), eligible: p["eligible"].as_array().unwrap().iter().map(seat).collect() })
        .collect();
    assert_eq!(last["after"]["final"], "all_in_runout", "the fixture this test transcribes must be the all-in runout");
    assert_eq!(expected_returned, vec![(Seat(2), 16)], "guard against a silently regenerated fixture");
    assert_eq!(expected_pots, vec![Pot { amount: 43, eligible: vec![Seat(2), Seat(4)] }]);

    let config = HandConfig {
        config_revision: 1, sb_chips: 1, bb_chips: 2, straddle: Some(UtgStraddle { amount_chips: 4 }),
        rake: Rake::TimeCharge, chip_label: "$1".into(),
    };
    let hand = table(
        config,
        Seat(2),
        vec![Seat(3), Seat(4), Seat(5), Seat(0), Seat(1), Seat(2)],
        vec![40, 17, 300, 14, 200, 200],
        "QcJsTh",
        vec![
            took(0, Street::Preflop, Action::Call, 4),
            took(1, Street::Preflop, Action::Fold, 0),
            took(2, Street::Preflop, Action::Call, 4),
            took(3, Street::Preflop, Action::Fold, 0),
            took(4, Street::Preflop, Action::Call, 2),
            took(5, Street::Preflop, Action::Check, 0),
            took(4, Street::Flop, Action::Check, 0),
            took(5, Street::Flop, Action::Check, 0),
            took(0, Street::Flop, Action::Check, 0),
            took(2, Street::Flop, Action::Bet { to: 29 }, 29),
            took(4, Street::Flop, Action::Call, 13),
            took(5, Street::Flop, Action::Fold, 0),
            took(0, Street::Flop, Action::Fold, 0),
        ],
    );
    let sim = simulate(&hand).unwrap();
    assert_eq!(sim.returned, expected_returned);
    assert_eq!(sim.pots, expected_pots);
    assert_eq!(sim.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    let d = sim.derived();
    assert_eq!(d.street, Street::Flop);
    assert_eq!(d.stacks_remaining, vec![10, 200, 183, 39, 0, 296], "the fixture's ring-ordered [39,0,296,10,200,183]");
    assert_eq!(d.stacks_remaining.iter().map(|s| u64::from(*s)).sum::<u64>() + u64::from(sim.pots[0].amount), sim.start_total);
}
