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
