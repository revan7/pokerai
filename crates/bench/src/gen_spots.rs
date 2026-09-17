use crate::suite::{Spot, Suite};
use proto::{Card, Rake, Street};

// R8 addendum A.1 ranges (uniform weights). INTERIM: spec §13.5's baseline set uses chart-replay ranges,
// which arrive with plan 3's bundles (`--source chart`); plan 4 Task 17 freezes the source hashes and Task 20 regenerates all six suites.
// Note: A.1 publishes combo counts 646 / 804 for these two strings; the strings as published expand to 634 / 720
// (recomputed in Task 6). The strings, not A.1's counts, are the definition here and in the fixture generator.
pub const BTN_OPEN: &str = "22+,A2s+,K2s+,Q2s+,J3s+,T6s+,96s+,86s+,75s+,65s,54s,43s,A2o+,K7o+,Q8o+,J8o+,T8o+,98o";
pub const BB_DEFEND: &str = "JJ-22,AJs-A2s,K2s+,Q2s+,J2s+,T2s+,92s+,84s+,74s+,63s+,53s+,43s,32s,AJo-A2o,K5o+,Q7o+,J8o+,T8o+,98o,97o,87o,76o";
pub const CO_CALL_3BET: &str = "QQ-22,AKs-ATs,A5s-A4s,KQs-KTs,QJs-QTs,JTs,J9s,T9s,T8s,98s,87s,76s,65s,54s,AQo-AJo,KQo,KJo";
pub const BTN_3BET: &str = "TT+,AJs+,A5s-A2s,KJs+,QJs,JTs,T9s,76s,65s,54s,AQo+,KQo,KJo";
/// Board classes of addendum A.1 with fixed turn and river cards; chips are 0.1 bb (bb_chips = 10) as in the addendum.
const BOARDS: [(&str, &str); 3] = [("dry", "Kh7d2c4d9s"), ("wet", "Jh9h6c2d5s"), ("paired", "8s8d3cQh4c")];

fn cards(s: &str, n: usize) -> Vec<Card> { s.as_bytes().chunks(2).take(n).map(|c| Card::parse(std::str::from_utf8(c).unwrap()).unwrap()).collect() }

pub fn generate(suite: &str, source: &str) -> Result<Suite, String> {
    // §13.5's baseline set is chart-replay; `r8` is the labelled interim of this plan. `chart` is accepted only
    // once plan 3's bundles are wired in; plan 4 Task 17 freezes source hashes and Task 20 regenerates all six suites before the gate.
    if source != "r8" {
        return Err(format!("source {source:?} is not available in this plan: only \"r8\" (uniform R8 addendum A.1 ranges, interim) is implemented; the \"chart\" replay baseline of spec section 13.5 arrives with plan 3's bundles and is regenerated for all six suites by plan 4 Task 20"));
    }
    let (street, template, n) = match suite {
        "river_std" => (Street::River, "river_std_v1", 5), "river_min" => (Street::River, "river_min_v1", 5),
        "turn_std" => (Street::Turn, "turn_std_v1", 4), "turn_min" => (Street::Turn, "turn_min_v1", 4),
        _ => return Err(format!("suite {suite} is not a river/turn suite of this plan")),
    };
    // SRP BTN-vs-BB at 100bb and 200bb after a 3 bb flop c-bet is called (turn: pot 11.5 bb) and a 55% turn bet called (river: pot 24.1 bb)
    let depths = [(100u32, "srp100"), (200u32, "srp200")];
    let mut spots = Vec::new();
    for (bb, tag) in depths {
        for (class, board) in BOARDS {
            let (pot, stack) = match street { Street::Turn => (115, bb * 10 - 55), _ => (241, bb * 10 - 118) };
            spots.push(Spot { id: format!("{tag}_{class}"), template_id: template.into(), root_street: street, board: cards(board, n),
                oop_range: BB_DEFEND.into(), ip_range: BTN_OPEN.into(), pot, stack_oop: stack, stack_ip: stack,
                rake: Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false }, history: vec![], target_bp: 50, range_source: "r8_uniform".into() });
        }
    }
    Ok(Suite { suite: suite.into(), spots })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn six_spots_per_suite_on_three_boards() {
        for suite in ["river_std", "river_min", "turn_std", "turn_min"] {
            let s = generate(suite, "r8").unwrap();
            assert_eq!(s.spots.len(), 6);
            let boards: std::collections::BTreeSet<String> = s.spots.iter().map(|x| x.board[..3].iter().map(|c| c.to_string()).collect::<String>()).collect();
            assert_eq!(boards.len(), 3);
            assert!(s.spots.iter().all(|x| x.template_id.starts_with(&suite[..suite.find('_').unwrap()])));
            assert!(s.spots.iter().all(|x| x.board.len() == if suite.starts_with("river") { 5 } else { 4 }));
            assert!(s.spots.iter().all(|x| x.range_source == "r8_uniform"), "the interim source must be labelled in every spot");
        }
        assert!(generate("flop_fast", "r8").is_err());
        // §13.5's chart-replay baseline is not available in this plan; the error names it so the gap stays explicit
        let e = generate("river_std", "chart").unwrap_err();
        assert!(e.contains("chart"), "{e}");
    }
}
