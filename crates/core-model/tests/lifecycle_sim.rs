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

/// `fixtures/hands/h0001.json`: a straddled six-handed PokerKit hand (blinds 1/2, straddle 4, button
/// `Seat(2)`) that ends on the flop with an uncalled 16 returned to the bettor, two folded seats' dead
/// money merged into the survivors' layer, and `all_in_runout`.
fn h0001_fixture() -> serde_json::Value {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/hands/h0001.json"))
        .expect("fixtures/hands/h0001.json is committed");
    serde_json::from_str(&text).unwrap()
}
fn fixture_seat(v: &serde_json::Value) -> Seat { Seat(u8::try_from(v.as_u64().unwrap()).unwrap()) }
fn fixture_chips(v: &serde_json::Value) -> u32 { u32::try_from(v.as_u64().unwrap()).unwrap() }
/// The `pots` of a fixture step's `after` snapshot; `eligible` is already seat-indexed there.
fn fixture_pots(after: &serde_json::Value) -> Vec<Pot> {
    after["pots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Pot { amount: fixture_chips(&p["amount"]), eligible: p["eligible"].as_array().unwrap().iter().map(fixture_seat).collect() })
        .collect()
}
/// h0001's thirteen actions, in the fixture's `steps` order.
fn h0001_actions() -> Vec<TakenAction> {
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
    ]
}
fn h0001_hand(actions: Vec<TakenAction>) -> HandState {
    let config = HandConfig {
        config_revision: 1, sb_chips: 1, bb_chips: 2, straddle: Some(UtgStraddle { amount_chips: 4 }),
        rake: Rake::TimeCharge, chip_label: "$1".into(),
    };
    table(config, Seat(2), vec![Seat(3), Seat(4), Seat(5), Seat(0), Seat(1), Seat(2)], vec![40, 17, 300, 14, 200, 200], "QcJsTh", actions)
}

/// Pins the closure rule against the committed PokerKit fixture that `tools/gen_fixtures.py`
/// generated. The actions are the fixture's `steps`; the expectations are read out of the fixture file
/// itself, so the generator's normalization and `close()` are compared, not restated.
#[test]
fn street_closure_matches_the_fixture_generators_settlement() {
    let fixture = h0001_fixture();
    let expected_returned: Vec<(Seat, u32)> =
        fixture["returned"].as_array().unwrap().iter().map(|e| (fixture_seat(&e[0]), fixture_chips(&e[1]))).collect();
    let last = fixture["steps"].as_array().unwrap().last().unwrap();
    let expected_pots = fixture_pots(&last["after"]);
    assert_eq!(last["after"]["final"], "all_in_runout", "the fixture this test transcribes must be the all-in runout");
    assert_eq!(expected_returned, vec![(Seat(2), 16)], "guard against a silently regenerated fixture");
    assert_eq!(expected_pots, vec![Pot { amount: 43, eligible: vec![Seat(2), Seat(4)] }]);

    let sim = simulate(&h0001_hand(h0001_actions())).unwrap();
    assert_eq!(sim.returned, expected_returned);
    assert_eq!(sim.pots, expected_pots);
    assert_eq!(sim.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    let d = sim.derived();
    assert_eq!(d.street, Street::Flop);
    assert_eq!(d.stacks_remaining, vec![10, 200, 183, 39, 0, 296], "the fixture's ring-ordered [39,0,296,10,200,183]");
    assert_eq!(d.stacks_remaining.iter().map(|s| u64::from(*s)).sum::<u64>() + u64::from(sim.pots[0].amount), sim.start_total);
}

/// Review R1. A fold changes pot eligibility at once, not at the next closure (spec 4.3): the settled
/// pots keep their chips but drop the folded seat from every eligible set. The already-settled amount,
/// the current street's live commitments and the conservation invariant are untouched.
#[test]
fn a_fold_leaves_the_settled_pots_before_the_street_closes() {
    let actions = vec![
        took(5, Street::Preflop, Action::Call, 2),
        took(0, Street::Preflop, Action::Call, 1),
        took(1, Street::Preflop, Action::Check, 0),
        took(0, Street::Flop, Action::Bet { to: 10 }, 10),
        took(1, Street::Flop, Action::Fold, 0),
    ];
    let sim = simulate(&state("AsKd2c", actions)).unwrap();
    let d = sim.derived();
    assert_eq!(sim.phase, HandPhase::Betting { street: Street::Flop }, "BTN still owes a response: the street is unfinished");
    assert_eq!(d.to_act, Some(Seat(5)));
    assert_eq!(sim.pots, vec![Pot { amount: 6, eligible: vec![Seat(0), Seat(5)] }], "BB left the eligible set the moment it folded");
    assert_eq!(d.pots, sim.pots, "Sim.pots and Derived.pots agree");
    assert_eq!(sim.contributed, [2, 2, 0, 0, 0, 2], "BB's 2 stays in the pot as dead money");
    assert_eq!(d.committed_this_street, vec![10, 0, 0, 0, 0, 0], "SB's live bet is not settled into a pot yet");
    assert_eq!(d.stacks_remaining, vec![188, 198, 0, 0, 0, 198]);
    assert_eq!(d.pot, 16);
    assert_eq!(
        d.stacks_remaining.iter().sum::<u32>() + d.committed_this_street.iter().sum::<u32>() + sim.pots.iter().map(|p| p.amount).sum::<u32>(),
        600,
        "conservation across stacks, live commitments and settled pots"
    );

    // The same moment in the committed fixture: h0001 after seat 5's flop fold, before the last
    // response. The oracle drops seat 5 from the settled 17 and keeps the amount.
    let fixture = h0001_fixture();
    let steps = fixture["steps"].as_array().unwrap();
    let before_last = &steps[steps.len() - 2];
    assert_eq!(before_last["after"]["to_act"], 0, "the step this test transcribes must be seat 5's flop fold");
    let expected = fixture_pots(&before_last["after"]);
    assert_eq!(expected, vec![Pot { amount: 17, eligible: vec![Seat(0), Seat(2), Seat(4)] }], "guard against a regenerated fixture");
    let mut partial = h0001_actions();
    partial.pop();
    let sim = simulate(&h0001_hand(partial)).unwrap();
    assert_eq!(sim.phase, HandPhase::Betting { street: Street::Flop });
    assert_eq!(sim.derived().to_act, Some(Seat(0)));
    assert_eq!(sim.pots, expected);
    assert_eq!(sim.derived().pots, expected);
}

/// Review R2. Once at most one pot-eligible seat still has chips and that seat already covers the
/// outstanding wager, no response is possible and the street is closed (spec 4.3) — a pending Check is
/// not a decision. A lone seat that still owes chips keeps its call/fold decision.
#[test]
fn a_matched_sole_survivor_closes_without_a_synthetic_check() {
    // SB posts its last chip, BB posts 2, BTN calls all-in for 1. Only BB has chips and it owes nothing.
    let closed = table(
        cfg(), Seat(5), vec![Seat(5), Seat(0), Seat(1)], vec![1, 1, 200], "",
        vec![took(5, Street::Preflop, Action::Call, 1)],
    );
    let sim = simulate(&closed).unwrap();
    assert_eq!(sim.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout }, "no seat can respond: the street is closed");
    assert_eq!(sim.returned, vec![(Seat(1), 1)], "BB's unmatched chip is refunded at that closure");
    assert_eq!(sim.pots, vec![Pot { amount: 3, eligible: vec![Seat(0), Seat(1), Seat(5)] }]);
    let d = sim.derived();
    assert_eq!(d.to_act, None);
    assert!(d.legal.is_empty());
    assert_eq!(d.stacks_remaining, vec![0, 199, 0, 0, 0, 0]);
    assert_eq!(d.stacks_remaining.iter().sum::<u32>() + sim.pots.iter().map(|p| p.amount).sum::<u32>(), 202, "conservation");

    // The contrast: BTN jams 50 into BB's 200, so the lone seat with chips still faces 48.
    let owing = table(
        cfg(), Seat(5), vec![Seat(5), Seat(0), Seat(1)], vec![50, 1, 200], "",
        vec![took(5, Street::Preflop, Action::AllIn { to: 50 }, 50)],
    );
    let sim = simulate(&owing).unwrap();
    assert_eq!(sim.phase, HandPhase::Betting { street: Street::Preflop }, "an unpaid wager is a real decision");
    let d = sim.derived();
    assert_eq!(d.to_act, Some(Seat(1)));
    assert_eq!(d.legal, vec![LegalAction::Fold, LegalAction::Call { cost: 48 }]);
    assert_eq!(sim.returned, vec![], "nothing is refunded while the wager is still live");
}

fn preflop_checkdown() -> Vec<TakenAction> {
    vec![
        took(5, Street::Preflop, Action::Call, 2),
        took(0, Street::Preflop, Action::Call, 1),
        took(1, Street::Preflop, Action::Check, 0),
    ]
}
fn street_checkdown(street: Street) -> Vec<TakenAction> {
    vec![took(0, street, Action::Check, 0), took(1, street, Action::Check, 0), took(5, street, Action::Check, 0)]
}

/// Review R3. `simulate` is the gate for an externally supplied `HandState`, so it validates the board
/// before replay (0/3/4/5 cards, ids in range, distinct, never one of hero's cards) and, after replay,
/// requires the recorded board to be exactly the board the replay reached — no partial street and no
/// cards from a street the hand has not opened.
#[test]
fn simulate_rejects_a_board_it_cannot_replay() {
    let dup = simulate(&state("AsAs2c", preflop_checkdown())).unwrap_err();
    assert!(matches!(dup, RulesError::Card(CardParseError::Duplicate(_))), "duplicate flop: {dup}");
    let six = simulate(&state("AsKd2c7h3s4d", preflop_checkdown())).unwrap_err();
    assert!(matches!(six, RulesError::BadBoard { .. }), "six-card board: {six}");
    let partial = simulate(&state("As", preflop_checkdown())).unwrap_err();
    assert!(matches!(partial, RulesError::BadBoard { .. }), "one-card partial flop: {partial}");

    let mut collides = state("AsKd2c", preflop_checkdown());
    let hero = cards("AsQh");
    collides.hero_cards = Some([hero[0], hero[1]]);
    let err = simulate(&collides).unwrap_err();
    assert!(matches!(err, RulesError::BadBoard { .. }), "the flop repeats a card hero holds: {err}");

    let early = simulate(&state("AsKd2c", vec![])).unwrap_err();
    assert!(matches!(early, RulesError::BadBoard { .. }), "a flop on record while preflop is still open: {early}");

    let mut out_of_range = state("", preflop_checkdown());
    out_of_range.board = vec![Card(52), Card(1), Card(2)];
    let err = simulate(&out_of_range).unwrap_err();
    assert!(matches!(err, RulesError::Card(CardParseError::Id(52))), "board card id outside 0..52: {err}");
}

#[test]
fn simulate_accepts_every_board_length_the_replay_reaches() {
    let sim = simulate(&state("", vec![])).unwrap();
    assert_eq!(sim.phase, HandPhase::Betting { street: Street::Preflop }, "0 cards, preflop open");

    let sim = simulate(&state("", preflop_checkdown())).unwrap();
    assert_eq!(sim.phase, HandPhase::AwaitingBoard { street: Street::Flop }, "0 cards, preflop closed");

    let sim = simulate(&state("AsKd2c", preflop_checkdown())).unwrap();
    assert_eq!(sim.phase, HandPhase::Betting { street: Street::Flop }, "3 cards open the flop");

    let mut through_turn = preflop_checkdown();
    through_turn.extend(street_checkdown(Street::Flop));
    let sim = simulate(&state("AsKd2c7h", through_turn.clone())).unwrap();
    assert_eq!(sim.phase, HandPhase::Betting { street: Street::Turn }, "4 cards open the turn");

    let mut through_river = through_turn;
    through_river.extend(street_checkdown(Street::Turn));
    through_river.extend(street_checkdown(Street::River));
    let sim = simulate(&state("AsKd2c7h3s", through_river)).unwrap();
    assert_eq!(sim.phase, HandPhase::Complete { reason: CompleteReason::ShowdownReached }, "5 cards check down to showdown");
    assert_eq!(sim.derived().street, Street::River);
    assert_eq!(sim.pots, vec![Pot { amount: 6, eligible: vec![Seat(0), Seat(1), Seat(5)] }]);
}
