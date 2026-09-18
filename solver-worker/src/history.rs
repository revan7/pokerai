//! Observed history in the library's own terms: the chip-denominated `proto::Action` line the
//! engine sends (§4.4) mapped onto `postflop_solver::Action`, and the action *indices* that
//! `PostFlopGame::play` takes (§10.3).
use crate::tree_build::to_lib_action;
use postflop_solver::{Action as LibAction, PostFlopGame};
use proto::Action;

pub fn history_to_lib(history: &[Action]) -> Vec<LibAction> { history.iter().map(to_lib_action).collect() }

/// Action indices for `PostFlopGame::apply_history`, validated against `available_actions()` at each step
/// (memory must be allocated: `play` panics otherwise). Leaves the game at the root.
pub fn indices_for(game: &mut PostFlopGame, chip: &[Action]) -> Result<Vec<usize>, String> {
    game.back_to_root();
    let mut idx = Vec::with_capacity(chip.len());
    for a in chip {
        if game.is_terminal_node() || game.is_chance_node() { game.back_to_root(); return Err(format!("path continues past a terminal or chance node at {a:?}")); }
        let la = to_lib_action(a);
        let i = match game.available_actions().iter().position(|x| *x == la) {
            Some(i) => i,
            None => { let at = format!("{:?}", game.history()); game.back_to_root(); return Err(format!("{a:?} not available at {at}")); }
        };
        game.play(i);
        idx.push(i);
    }
    game.back_to_root();
    Ok(idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{board_to_lib, range_to_lib};
    use crate::tree_build::build;
    use postflop_solver::{CardConfig, Range as LibRange};
    use proto::{combo_index, Card, EffectiveTree, MenuSize, PlayerMenus, Range1326, SideMenu, Street};
    use std::collections::BTreeMap;

    const POT: u32 = 100;
    const EFF: u32 = 200;

    /// A turn-rooted tree: small enough to allocate inside a unit test, and it still reaches both
    /// a terminal node (a fold) and a chance node (the river deal) — the two overruns
    /// `indices_for` has to refuse, because `PostFlopGame::play` panics on the first and
    /// reinterprets its argument as a *card id* on the second.
    fn turn_tree() -> EffectiveTree {
        let side = SideMenu { bet: vec![MenuSize::Pot(0.5)], raise: vec![MenuSize::Pot(2.5)] };
        EffectiveTree {
            rules_version: 3, template_id: "t8_history_v1".into(), root_street: Street::Turn,
            menus: BTreeMap::from([
                (Street::Turn, PlayerMenus { oop: side.clone(), ip: side.clone(), donk: None }),
                (Street::River, PlayerMenus { oop: side.clone(), ip: side, donk: Some(vec![]) }),
            ]),
            add_allin_threshold: 0.0, force_allin_threshold: 0.0, merging_threshold: 0.0,
            wager_cap: 1, inserted: vec![], materialized: vec![],
        }
    }

    fn range_of(combos: &[(&str, &str)]) -> LibRange {
        let mut r = Range1326([0.0; 1326]);
        for (a, b) in combos {
            r.0[combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize] = 1.0;
        }
        range_to_lib(&r).unwrap()
    }

    /// An allocated `PostFlopGame`: `play` panics without allocated memory, so the navigator can
    /// only be exercised against a real one. A one-combo range per side keeps it tiny.
    fn game() -> PostFlopGame {
        let board: Vec<Card> = ["Qs", "Jd", "7h", "3c"].iter().map(|s| Card::parse(s).unwrap()).collect();
        let (flop, turn, river) = board_to_lib(&board).unwrap();
        let card_config = CardConfig { range: [range_of(&[("As", "Ah")]), range_of(&[("Ks", "Kh")])], flop, turn, river };
        let tree = build(&turn_tree(), POT, EFF, 0.0, 0, &[]).unwrap();
        let mut g = PostFlopGame::with_config(card_config, tree).expect("turn game config");
        g.allocate_memory(false);
        g
    }

    #[test]
    fn history_maps_to_library_actions_in_order() {
        assert_eq!(history_to_lib(&[Action::Check, Action::Bet { to: 50 }, Action::Fold]), vec![LibAction::Check, LibAction::Bet(50), LibAction::Fold]);
        assert!(history_to_lib(&[]).is_empty());
    }

    #[test]
    fn empty_history_returns_no_indices_and_leaves_the_game_at_root() {
        let mut g = game();
        assert!(indices_for(&mut g, &[]).unwrap().is_empty());
        assert!(g.history().is_empty());
    }

    /// The turn root's OOP menu is `[Check, Bet(50)]`; after OOP checks, IP opens the same menu;
    /// facing IP's bet, OOP has `[Fold, Call]` because `wager_cap` is 1 and no wager was observed.
    #[test]
    fn a_multi_action_history_returns_the_exact_index_sequence() {
        let mut g = game();
        assert_eq!(indices_for(&mut g, &[Action::Check]).unwrap(), vec![0]);
        assert_eq!(indices_for(&mut g, &[Action::Bet { to: 50 }]).unwrap(), vec![1]);
        assert_eq!(indices_for(&mut g, &[Action::Check, Action::Bet { to: 50 }]).unwrap(), vec![0, 1]);
        assert_eq!(indices_for(&mut g, &[Action::Check, Action::Bet { to: 50 }, Action::Call]).unwrap(), vec![0, 1, 1]);
        assert!(g.history().is_empty(), "the game is returned to the root after a success");
    }

    #[test]
    fn navigation_restarts_from_the_root_whatever_position_the_game_was_left_in() {
        let mut g = game();
        g.play(1);                                  // OOP bets: the game is no longer at the root
        assert_eq!(g.history(), &[1]);
        assert_eq!(indices_for(&mut g, &[Action::Check]).unwrap(), vec![0], "indices_for must re-root first");
        assert!(g.history().is_empty());
    }

    #[test]
    fn an_unavailable_action_after_a_valid_prefix_errors_and_returns_to_the_root() {
        let mut g = game();
        let e = indices_for(&mut g, &[Action::Check, Action::Bet { to: 77 }]).unwrap_err();
        assert!(e.contains("Bet { to: 77 }"), "{e}");
        assert!(e.contains("not available at [0]"), "the error must name where the walk stopped: {e}");
        assert!(g.history().is_empty(), "the game is returned to the root after an error");
    }

    #[test]
    fn a_path_past_a_terminal_node_errors_and_returns_to_the_root() {
        let mut g = game();
        // OOP bets, IP folds: the fold is terminal, and `PostFlopGame::play` panics on a terminal
        let e = indices_for(&mut g, &[Action::Bet { to: 50 }, Action::Fold, Action::Check]).unwrap_err();
        assert!(e.contains("past a terminal or chance node") && e.contains("Check"), "{e}");
        assert!(g.history().is_empty());
        assert_eq!(indices_for(&mut g, &[Action::Bet { to: 50 }, Action::Fold]).unwrap(), vec![1, 0], "the same path without the overrun is fine");
    }

    #[test]
    fn a_path_past_a_chance_node_errors_and_returns_to_the_root() {
        let mut g = game();
        // check-check closes the turn: the next node is the river deal, a chance node
        let e = indices_for(&mut g, &[Action::Check, Action::Check, Action::Check]).unwrap_err();
        assert!(e.contains("past a terminal or chance node"), "{e}");
        assert!(g.history().is_empty());
        assert_eq!(indices_for(&mut g, &[Action::Check, Action::Check]).unwrap(), vec![0, 0], "stopping at the chance node is fine");
    }
}
