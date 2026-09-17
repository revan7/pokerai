use core_model::state::BeginHand; // explicit: `proto::BeginHand` is the DTO of the same name
use core_model::*;
use proto::*;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Fixture { id: String, config: FixCfg, button: u8, hero: u8, dealt: Vec<u8>, stacks_start: Vec<u32>, hole_cards: Vec<String>, steps: Vec<Step>, returned: Vec<(u8, u32)> }
#[derive(Deserialize)]
struct FixCfg { sb_chips: u32, bb_chips: u32, straddle_chips: Option<u32> }
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Step { Action { seat: u8, street: String, action: Action, after: Snap }, Board { cards: Vec<String>, after: Snap } }
#[derive(Deserialize)]
struct Snap { phase: String, street: String, to_act: Option<u8>, pot_total: u32, committed: Vec<u32>, stacks: Vec<u32>, folded: Vec<bool>, all_in: Vec<bool>, pots: Vec<FixPot>, legal: Option<Legal>, #[serde(rename = "final")] final_reason: Option<String> }
#[derive(Deserialize)]
struct FixPot { amount: u32, eligible: Vec<u8> }
#[derive(Deserialize)]
struct Legal { fold: bool, check_or_call: Option<Cost>, raise: Option<MinMax> }
#[derive(Deserialize)]
struct Cost { cost: u32 }
#[derive(Deserialize)]
struct MinMax { min_to: u32, max_to: u32 }

fn street(name: &str) -> Street {
    match name { "preflop" => Street::Preflop, "flop" => Street::Flop, "turn" => Street::Turn, "river" => Street::River, other => panic!("street {other}") }
}

fn legal_triple(legal: &[LegalAction]) -> (bool, Option<u32>, Option<(u32, u32)>) {
    let fold = legal.contains(&LegalAction::Fold);
    let cc = legal.iter().find_map(|l| match l { LegalAction::Check => Some(0), LegalAction::Call { cost } => Some(*cost), _ => None });
    let mut raise = legal.iter().find_map(|l| match l { LegalAction::Bet { min_to, max_to } | LegalAction::Raise { min_to, max_to } => Some((*min_to, *max_to)), _ => None });
    if raise.is_none() { raise = legal.iter().find_map(|l| match l { LegalAction::AllIn { to } => Some((*to, *to)), _ => None }); }
    (fold, cc, raise)
}

fn check_snapshot(id: &str, k: usize, state: &HandState, snap: &Snap) {
    let d = &state.derived;
    let ctx = format!("{id} step {k}");
    let phase = match snap.phase.as_str() {
        "betting" => HandPhase::Betting { street: street(&snap.street) },
        "awaiting_board" => HandPhase::AwaitingBoard { street: street(&snap.street) },
        "complete" => HandPhase::Complete { reason: match snap.final_reason.as_deref() { Some("folded_out") => CompleteReason::FoldedOut, Some("all_in_runout") => CompleteReason::AllInRunout, Some("showdown_reached") => CompleteReason::ShowdownReached, other => panic!("{ctx}: final {other:?}") } },
        other => panic!("{ctx}: phase {other}"),
    };
    assert_eq!(state.phase, phase, "{ctx}: phase");
    assert_eq!(d.street, street(&snap.street), "{ctx}: street");
    assert_eq!(d.to_act, snap.to_act.map(Seat), "{ctx}: to_act");
    assert_eq!(d.pot, snap.pot_total, "{ctx}: pot");
    for (idx, seat) in state.dealt.iter().enumerate() {
        let s = seat.0 as usize;
        assert_eq!(d.committed_this_street[s], snap.committed[idx], "{ctx}: committed seat {s}");
        assert_eq!(d.stacks_remaining[s], snap.stacks[idx], "{ctx}: stack seat {s}");
        assert_eq!(d.folded[s], snap.folded[idx], "{ctx}: folded seat {s}");
        assert_eq!(d.all_in[s], snap.all_in[idx], "{ctx}: all_in seat {s}");
    }
    let pots: Vec<Pot> = snap.pots.iter().map(|p| Pot { amount: p.amount, eligible: p.eligible.iter().map(|s| Seat(*s)).collect() }).collect();
    assert_eq!(d.pots, pots, "{ctx}: pots");
    match &snap.legal {
        None => assert!(d.legal.is_empty(), "{ctx}: legal should be empty"),
        Some(l) => {
            let expected = (l.fold, l.check_or_call.as_ref().map(|c| c.cost), l.raise.as_ref().map(|r| (r.min_to, r.max_to)));
            assert_eq!(legal_triple(&d.legal), expected, "{ctx}: legal {:?}", d.legal);
        }
    }
}

#[test]
fn state_machine_pokerkit_fixtures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hands");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).expect("fixtures/hands exists (Task 17)").map(|e| e.unwrap().path()).filter(|p| p.extension().map_or(false, |e| e == "json")).collect();
    files.sort();
    assert_eq!(files.len(), 200, "200 PokerKit hands");
    for file in files {
        let fx: Fixture = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let cfg = HandConfig { config_revision: 1, sb_chips: fx.config.sb_chips, bb_chips: fx.config.bb_chips, straddle: fx.config.straddle_chips.map(|a| UtgStraddle { amount_chips: a }), rake: Rake::TimeCharge, chip_label: "$1".into() };
        let hero_idx = fx.dealt.iter().position(|s| *s == fx.hero).unwrap();
        let mut state = begin_hand(&cfg, BeginHand { hand_id: 1, button: Seat(fx.button), hero: Seat(fx.hero), dealt: fx.dealt.iter().map(|s| Seat(*s)).collect(), stacks_start: fx.stacks_start.clone(), hero_cards: Some(parse_hand(&fx.hole_cards[hero_idx]).unwrap()) }).unwrap();
        let mut board: Vec<Card> = vec![];
        for (k, step) in fx.steps.iter().enumerate() {
            match step {
                Step::Action { seat, street: st, action, after } => {
                    assert_eq!(state.derived.to_act, Some(Seat(*seat)), "{} step {k}: actor", fx.id);
                    state = apply_action(&state, *action).unwrap_or_else(|e| panic!("{} step {k}: {action:?} rejected: {e}", fx.id));
                    let taken = state.actions.last().unwrap();
                    assert_eq!((taken.seat, taken.street, taken.action), (Seat(*seat), street(st), *action), "{} step {k}: recorded action", fx.id);
                    check_snapshot(&fx.id, k, &state, after);
                }
                Step::Board { cards, after } => {
                    for c in cards { board.push(c.parse().unwrap()); }
                    state = set_board(&state, &board).unwrap_or_else(|e| panic!("{} step {k}: board rejected: {e}", fx.id));
                    check_snapshot(&fx.id, k, &state, after);
                }
            }
        }
        assert!(matches!(state.phase, HandPhase::Complete { .. }), "{}: every fixture ends complete", fx.id);
        let returned: Vec<(Seat, u32)> = fx.returned.iter().map(|(s, a)| (Seat(*s), *a)).collect();
        assert_eq!(settle_pots(&state).returned, returned, "{}: refunds", fx.id);
    }
}
