use engine::coverage::{classify, Classification};
use engine::testing::{board, hand, play};
use proto::{Action, ApproxReason, Card, Seat, UnsupportedReason};

fn label(c: &Classification) -> String {
    match c {
        Classification::NoDecision { .. } => "no_decision".into(),
        Classification::Preflop => "preflop".into(),
        Classification::Multiway { pot_eligible } => format!("multiway:{pot_eligible}"),
        Classification::Unsupported(UnsupportedReason::UnsupportedHistory { .. }) => "unsupported_history".into(),
        Classification::Unsupported(r) => format!("unsupported:{r:?}"),
        Classification::HuStreet { reasons, facing_allin, .. } => {
            let mut s = "hu_street".to_string();
            for r in reasons { if let ApproxReason::MultiwayStreetRoot { folded_this_street, dead_this_street } = r { s += &format!(":MultiwayStreetRoot{{{folded_this_street},{dead_this_street}}}"); } }
            if *facing_allin { s += ":facing_allin"; }
            s
        }
    }
}
fn c(s: &str) -> Card { Card::parse(s).unwrap() }
const B: Seat = Seat(0); const SB: Seat = Seat(1); const BB: Seat = Seat(2); const UTG: Seat = Seat(3); const HJ: Seat = Seat(4); const CO: Seat = Seat(5);
fn six(stacks: [u32; 6]) -> Vec<(Seat, u32)> { (0..6).map(|i| (Seat(i as u8), stacks[i])).collect() }
fn fold3(s: proto::HandState) -> proto::HandState { play(&s, &[Action::Fold, Action::Fold, Action::Fold]) }   // UTG, HJ, CO
fn r(to: u32) -> Action { Action::Raise { to } }
fn b(to: u32) -> Action { Action::Bet { to } }

#[test]
fn coverage_classification_golden() {
    let aa = Some([c("Ah"), c("Ad")]);
    let mut cases: Vec<(&str, String)> = Vec::new();
    // HU flop: three folds, BTN raises to 30, SB folds, BB (hero) calls
    let hu = board(&play(&fold3(hand(&six([1000; 6]), B, BB, aa)), &[r(30), Action::Fold, Action::Call]), "Kh 7d 2c");
    cases.push(("hu_flop", label(&classify(&hu))));
    // 3-way flop: CO raises, BTN calls, SB folds, BB calls
    let three = board(&play(&hand(&six([1000; 6]), B, BB, aa), &[Action::Fold, Action::Fold, r(30), Action::Call, Action::Fold, Action::Call]), "Kh 7d 2c");
    cases.push(("three_way_flop", label(&classify(&three))));
    // third player all-in: BTN (100 chips) shoves preflop, SB folds, BB calls, CO calls; BB to act on the flop with BTN all-in
    let mut st = six([1000; 6]); st[0].1 = 100;
    let allin3 = board(&play(&hand(&st, B, BB, aa), &[Action::Fold, Action::Fold, r(30), Action::AllIn { to: 100 }, Action::Fold, Action::Call, Action::Call]), "Kh 7d 2c");
    cases.push(("third_player_allin", label(&classify(&allin3))));
    // two preflop folds then HU flop: UTG folds, HJ raises, CO/BTN/SB fold, BB calls
    let hu2 = board(&play(&hand(&six([1000; 6]), B, BB, aa), &[Action::Fold, r(30), Action::Fold, Action::Fold, Action::Fold, Action::Call]), "Kh 7d 2c");
    cases.push(("two_folds_then_hu_flop", label(&classify(&hu2))));
    // §10.2 projection cases: three limpers (BTN, SB, BB) see the flop with 1,000 behind each; postflop order A = SB, B = BB, C = BTN
    let limped = |hero: Seat| board(&play(&hand(&six([1010; 6]), B, hero, aa), &[Action::Fold, Action::Fold, Action::Fold, Action::Call, Action::Call, Action::Check]), "Kh 7d 2c");
    cases.push(("projection_a_admitted_dead_0", label(&classify(&play(&limped(SB), &[b(50), Action::Fold, r(150)])))));
    cases.push(("projection_b_admitted_dead_50", label(&classify(&play(&limped(B), &[b(50), Action::Call, r(150), r(250), Action::Fold])))));
    cases.push(("projection_c_not_reproducible", label(&classify(&play(&limped(BB), &[b(50), Action::Call, r(150), Action::Fold])))));
    // opponent all-in on the flop; hero all-in; hero cards unknown
    cases.push(("opponent_allin", label(&classify(&play(&hu, &[Action::Check, Action::AllIn { to: 970 }])))));
    cases.push(("hero_allin", label(&classify(&play(&hu, &[Action::AllIn { to: 970 }])))));
    cases.push(("hero_cards_unknown", label(&classify(&board(&play(&fold3(hand(&six([1000; 6]), B, BB, None)), &[r(30), Action::Fold, Action::Call]), "Kh 7d 2c")))));
    // two dealt seats: FormatUnsupported at begin_hand
    let (_, hc) = engine::testing::cfg_1_2();
    let two = core_model::begin_hand(&hc, core_model::BeginHand { hand_id: 1, button: B, hero: SB, hero_cards: aa, dealt: vec![B, SB], stacks_start: vec![1000, 1000] });
    cases.push(("two_dealt_seats", match two { Err(e) => format!("format_unsupported:{}", e.to_string().contains("two dealt seats")), Ok(_) => "accepted".into() }));
    let _ = (UTG, HJ, CO);
    let expected: Vec<(String, String)> = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/coverage_classification_golden.json")).unwrap()).unwrap();
    assert_eq!(cases.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Vec<_>>(), expected);
}

/// `Derived`'s per-seat vectors are indexed by `Seat.0` (proto), not by a seat's position in `dealt`: on a gapped
/// three-handed table (seats 1, 3, 5; button 5, so SB = 1, BB = 3) the two lookups disagree, and hero facing a flop
/// shove must be a HU decision facing an all-in whichever opponent shoved.
#[test]
fn derived_vectors_are_read_by_seat_id_on_a_gapped_table() {
    let aa = Some([c("Ah"), c("Ad")]);
    let (sb, bb, btn) = (Seat(1), Seat(3), Seat(5));
    let facing = |cls: &Classification| match cls { Classification::HuStreet { opponent, .. } => Some(*opponent), _ => None };
    // SB (100 chips) completes, BB checks; SB shoves the flop into hero.
    let sb_short = play(&board(&play(&hand(&[(sb, 100), (bb, 1000), (btn, 1000)], btn, bb, aa), &[Action::Fold, Action::Call, Action::Check]), "Kh 7d 2c"), &[Action::AllIn { to: 90 }]);
    let cls = classify(&sb_short);
    assert_eq!((label(&cls), facing(&cls)), ("hu_street:facing_allin".to_string(), Some(sb)), "{cls:?}");
    // BTN (100 chips) limps, SB folds, BB checks; hero checks the flop and BTN shoves.
    let btn_short = play(&board(&play(&hand(&[(sb, 1000), (bb, 1000), (btn, 100)], btn, bb, aa), &[Action::Call, Action::Fold, Action::Check]), "Kh 7d 2c"), &[Action::Check, Action::AllIn { to: 90 }]);
    let cls = classify(&btn_short);
    assert_eq!((label(&cls), facing(&cls)), ("hu_street:facing_allin".to_string(), Some(btn)), "{cls:?}");
    assert_eq!(engine::coverage::seat_index(&btn_short, btn), 5);
}
