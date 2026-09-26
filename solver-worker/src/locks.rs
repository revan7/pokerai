//! §4.5 lock matrices: validated at the protocol boundary (`validate`, before any tree is known) and
//! applied with `lock_current_strategy` after allocation and before the first iteration (`apply`, §10.3).
use crate::cards::lib_hand_to_combo;
use crate::history::indices_for;
use crate::tree_build::from_lib_action;
use postflop_solver::{Game, PostFlopGame};
use proto::worker::NodeLock;
use proto::{index_materialized, resolve_chip_path_indexed, Action, MaterializedNode, COMBOS};

/// §4.5: a locked row sums to 1 within this. The same `f32` constant, widened the same way, as
/// `proto::worker::validate_locks`, so a lock the engine accepts is never refused here at the boundary.
const ROW_TOLERANCE: f32 = 1e-3;

/// §4.5: exactly 1326 rows of one non-zero width; every entry finite and in `[0, 1]`; each row either
/// all zero (the free-combo sentinel, the only permitted all-zero row) or summing to 1 +- 1e-3 (summed
/// in f64, wide before any comparison); actor `oop` or `ip`; and no two locks naming the same node,
/// since only one of them could take effect and `locks_applied` would overstate what was locked.
pub fn validate(locks: &[NodeLock]) -> Result<(), String> {
    for (k, l) in locks.iter().enumerate() {
        check_matrix(l).map_err(|e| format!("lock {k}: {e}"))?;
        if let Some(j) = locks[..k].iter().position(|o| o.path == l.path) {
            return Err(format!("lock {k}: path {:?} duplicates lock {j}", l.path));
        }
    }
    Ok(())
}

/// One lock's matrix rules; returns its width (the number of action columns).
fn check_matrix(l: &NodeLock) -> Result<usize, String> {
    if l.probs.len() != COMBOS { return Err(format!("{} rows, expected {COMBOS}", l.probs.len())); }
    if l.actor != "oop" && l.actor != "ip" { return Err(format!("actor {:?} is neither \"oop\" nor \"ip\"", l.actor)); }
    let width = l.probs[0].len();
    if width == 0 { return Err("empty rows".into()); }
    for (c, row) in l.probs.iter().enumerate() {
        if row.len() != width { return Err(format!("row {c} has {} entries, expected {width}", row.len())); }
        let mut sum = 0.0f64;
        for p in row {
            if !(p.is_finite() && (0.0..=1.0).contains(p)) { return Err(format!("row {c} has an entry {p} outside [0, 1]")); }
            sum += f64::from(*p);
        }
        if sum != 0.0 && (sum - 1.0).abs() > f64::from(ROW_TOLERANCE) {
            return Err(format!("row {c} sums to {sum}, expected 1 (or all zero, the free-combo sentinel)"));
        }
    }
    Ok(width)
}

/// Applies one lock (§10.3). The node its chip path names must be a decision node of the root street
/// whose actor and menu width match the lock; the combo-major rows `[1326][action]` are written onto
/// the library's action-major `[action][hand]` layout over `private_cards(player)`. An all-zero row
/// leaves its combo free; a row for a combo outside the player's compact hand list (not in the range,
/// or on the board) has no hand to lock and is not read.
///
/// Precondition: memory allocated and no iteration run yet — the job applies locks between
/// `allocate_memory` and the solve loop. An unallocated or already solved game is refused rather than
/// reaching the library's panics. Every refusal locks nothing; the game is back at the root on every return.
pub fn apply(game: &mut PostFlopGame, lock: &NodeLock, materialized: &[MaterializedNode]) -> Result<(), String> {
    let at = |e: String| format!("lock at {:?}: {e}", lock.path);
    let width = check_matrix(lock).map_err(at)?;
    let index = index_materialized(materialized);
    let ordinal = resolve_chip_path_indexed(&index, &lock.path).ok_or_else(|| at("the path is not a decision node of the materialized tree".into()))?;
    let (Some(node), Some(root)) = (index.get(ordinal.as_slice()), index.get(&[][..])) else {
        return Err(at("the resolved node is not materialized".into()));
    };
    if node.street != root.street {
        return Err(at(format!("the node is on the {:?} street; only nodes of the root street ({:?}) can be locked", node.street, root.street)));
    }
    if node.actor != lock.actor { return Err(at(format!("actor {} differs from the node's actor {}", lock.actor, node.actor))); }
    if node.actions.len() != width { return Err(at(format!("{width} columns for a node with {} actions", node.actions.len()))); }
    if game.is_solved() { return Err(at("the game is already solved; locks are applied before the first iteration".into())); }
    if !game.is_ready() { return Err(at("the game's memory is not allocated; locks are applied after allocation".into())); }
    let idx = indices_for(game, &lock.path).map_err(at)?;
    game.apply_history(&idx);
    let out = lock_here(game, lock, node).map_err(at);
    game.back_to_root();
    out
}

fn lock_here(game: &mut PostFlopGame, lock: &NodeLock, node: &MaterializedNode) -> Result<(), String> {
    if game.is_terminal_node() || game.is_chance_node() { return Err("not a decision node of the library tree".into()); }
    let player = game.current_player();
    let actor = if player == 0 { "oop" } else { "ip" };
    if actor != node.actor { return Err(format!("the library's actor {actor} differs from the skeleton's {}", node.actor)); }
    let lib_actions: Option<Vec<Action>> = game.available_actions().into_iter().map(from_lib_action).collect();
    if lib_actions.as_deref() != Some(node.actions.as_slice()) {
        return Err(format!("the library's menu {lib_actions:?} differs from the skeleton's {:?}", node.actions));
    }
    let slice = {
        let hands = game.private_cards(player);
        let n = hands.len();
        let mut slice = vec![0.0f32; n * node.actions.len()];
        for (h, hand) in hands.iter().enumerate() {
            for (a, p) in lock.probs[usize::from(lib_hand_to_combo(*hand))].iter().enumerate() { slice[a * n + h] = *p; }
        }
        slice
    };
    game.lock_current_strategy(&slice);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_lines;
    use proto::worker::EngineMessage;

    fn staged() -> Vec<NodeLock> {
        match serde_json::from_str::<EngineMessage>(&fixture_lines("lock_river")[0]).unwrap() {
            EngineMessage::Lock { locks, .. } => locks,
            other => panic!("line 0 of lock_river is not a lock: {other:?}"),
        }
    }

    #[test]
    fn lock_matrix_rules_of_section_4_5() {
        let ok = staged();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].probs.len(), 1326);
        validate(&ok).unwrap();                                    // all-zero rows are the free-combo sentinel
        let mut wrong_sum = ok.clone(); wrong_sum[0].probs[0] = vec![0.1, 0.3];
        assert!(validate(&wrong_sum).unwrap_err().contains("sums to"));
        let mut out_of_range = ok.clone(); out_of_range[0].probs[0] = vec![-0.1, 1.1];
        assert!(validate(&out_of_range).unwrap_err().contains("outside"));
        let mut ragged = ok.clone(); ragged[0].probs[5] = vec![1.0];
        assert!(validate(&ragged).unwrap_err().contains("entries"));
        let mut short = ok.clone(); short[0].probs.pop();
        assert!(validate(&short).unwrap_err().contains("1326"));
        let mut actor = ok.clone(); actor[0].actor = "hero".into();
        assert!(validate(&actor).unwrap_err().contains("actor"));
    }

    use crate::extract::extract_node;
    use crate::extract::test_games::{allocated, configured, solved, turn_request};
    use crate::testutil::solve_request;
    use postflop_solver::solve;
    use proto::{combo_index, Card};

    const CHECK: Action = Action::Check;
    const JAM: Action = Action::AllIn { to: 100 };

    fn combo(a: &str, b: &str) -> usize { combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize }
    /// Line 1 of `lock_river` is the solve the staged lock belongs to (same `spot`).
    fn lock_request() -> proto::worker::SolveRequest { solve_request("lock_river", 1) }

    /// Rows are summed in f64 before the 1e-3 tolerance (standing ruling: wide before narrow); the
    /// tolerance is two-sided and agrees with the engine's `proto::worker::validate_locks` on every
    /// probe; two locks naming the same node are refused, since only one of them could take effect and
    /// `locks_applied` would overstate what was locked.
    #[test]
    fn row_tolerance_and_duplicate_nodes() {
        let ok = staged();
        let m = &lock_request().tree.materialized;
        let mut near = ok.clone();
        for (row, accepted) in [(vec![0.8, 0.2009], true), (vec![0.8, 0.1991], true), (vec![0.8, 0.2011], false), (vec![0.8, 0.1989], false), (vec![0.0, 0.0], true)] {
            near[0].probs[75] = row.clone();
            assert_eq!(validate(&near).is_ok(), accepted, "{row:?}: {:?}", validate(&near));
            assert_eq!(proto::worker::validate_locks(&near, m).is_ok(), accepted, "the engine's validator agrees on {row:?}");
        }
        near[0].probs[75] = vec![0.8, 0.2011];
        assert!(validate(&near).unwrap_err().contains("sums to"));
        let mut nan = ok.clone();
        nan[0].probs[75] = vec![f32::NAN, 1.0];
        assert!(validate(&nan).unwrap_err().contains("outside"));
        let mut empty = ok.clone();
        for row in empty[0].probs.iter_mut() { row.clear(); }
        assert!(validate(&empty).unwrap_err().contains("empty"));

        let mut twice = ok.clone();
        twice.push(ok[0].clone());
        assert!(validate(&twice).unwrap_err().contains("duplicates lock 0"), "{:?}", validate(&twice));
        let mut two_nodes = ok.clone();
        two_nodes.push(NodeLock { path: vec![CHECK, JAM], actor: "oop".into(), probs: vec![vec![0.0, 0.0]; 1326] });
        validate(&two_nodes).unwrap();
        validate(&[]).unwrap();
    }

    /// §10.3: the combo-major rows land on the library's own hand order at the locked node; an all-zero
    /// row leaves that combo free (the library's `-1.0` marker); the game is back at the root.
    #[test]
    fn apply_locks_named_rows_and_leaves_free_combos_free() {
        let req = lock_request();
        let mut lock = staged().remove(0);
        let freed = [combo("5c", "4d"), combo("5d", "4c"), combo("5h", "4s")];
        for c in freed { assert!(lock.probs[c].iter().any(|p| *p > 0.0), "{c} is locked in the fixture"); lock.probs[c] = vec![0.0, 0.0]; }
        let mut game = allocated(&req);
        apply(&mut game, &lock, &req.tree.materialized).unwrap();
        assert!(game.history().is_empty(), "apply returns the game to the root");

        game.play(0);                                               // OOP's only root action: check
        assert_eq!(game.current_player(), 1);
        let locking = game.current_locking_strategy().expect("IP's node is locked");
        let hands = game.private_cards(1).to_vec();
        let n = hands.len();
        assert_eq!(n, 15, "QQ (3) and 54o (12)");
        for (h, hand) in hands.iter().enumerate() {
            let c = lib_hand_to_combo(*hand) as usize;
            let got = [locking[h], locking[n + h]];
            if freed.contains(&c) {
                assert_eq!(got, [-1.0, -1.0], "combo {c} stays free");
            } else {
                assert!((got[0] - lock.probs[c][0]).abs() < 1e-6 && (got[1] - lock.probs[c][1]).abs() < 1e-6, "combo {c}: {got:?} vs {:?}", lock.probs[c]);
            }
        }
    }

    /// §13.2 `ev_convention_non_root_payoffs`, in process: IP's river node locked to bet QQ 100% and
    /// 54o 20%. Locked rows come back unchanged; at OOP's facing node fold is exactly 0 and the call is
    /// `equity * 300 - 100` against the locked range (QQ mass 3, 54o mass 0.6): +200 for QQ, -50 for 66,
    /// each within 1e-3; and OOP's response follows those signs.
    #[test]
    fn locked_river_reproduces_the_section_13_2_call_values() {
        let req = lock_request();
        let lock = staged().remove(0);
        let mut game = allocated(&req);
        apply(&mut game, &lock, &req.tree.materialized).unwrap();
        solve(&mut game, 500, 0.0, false);
        let m = &req.tree.materialized;

        let ip = extract_node(&mut game, m.iter().find(|n| n.path == [0]).unwrap(), &[CHECK]).unwrap();
        for c in 0..1326 {
            assert_eq!(ip.available[c], lock.probs[c].iter().any(|p| *p > 0.0), "combo {c}");
            if ip.available[c] {
                for a in 0..2 { assert!((ip.probs[c][a] - lock.probs[c][a]).abs() < 1e-6, "locked row {c} changed: {:?} vs {:?}", ip.probs[c], lock.probs[c]); }
            }
        }

        let oop = extract_node(&mut game, m.iter().find(|n| n.path == [0, 1]).unwrap(), &[CHECK, JAM]).unwrap();
        assert_eq!(oop.actions, vec![Action::Fold, Action::Call]);
        let queens = [combo("Qc", "Qd"), combo("Qc", "Qh"), combo("Qd", "Qh")];
        let sixes = [combo("6c", "6d"), combo("6c", "6h"), combo("6c", "6s"), combo("6d", "6h"), combo("6d", "6s"), combo("6h", "6s")];
        for (combos, call) in [(&queens[..], 200.0f32), (&sixes[..], -50.0)] {
            for &c in combos {
                assert!(oop.available[c], "combo {c}");
                assert_eq!(oop.ev_chips[c][0].to_bits(), 0.0f32.to_bits(), "fold EV of combo {c} is exactly +0.0");
                assert!((oop.ev_chips[c][1] - call).abs() <= 1e-3, "combo {c}: call EV {} expected {call}", oop.ev_chips[c][1]);
                let calls = oop.probs[c][1];
                assert!(if call > 0.0 { calls > 0.9 } else { calls < 0.1 }, "combo {c} calls {calls} with call EV {call}");
            }
        }
    }

    /// Every refusal is an `Err` (the job's `lock_mismatch`), never a library panic, and locks nothing:
    /// a path that is not a decision node, a different actor, a column count that differs from the
    /// node's menu, an invalid matrix, a node beyond the root street, and a game that is not allocated
    /// or already solved.
    #[test]
    fn apply_refuses_what_it_cannot_lock() {
        let req = lock_request();
        let m = &req.tree.materialized;
        let ok = staged().remove(0);
        let mut game = allocated(&req);
        let mut refuse = |lock: &NodeLock, needle: &str| {
            let e = apply(&mut game, lock, m).unwrap_err();
            assert!(e.contains(needle), "{needle:?} in {e}");
            assert!(game.history().is_empty(), "back at the root after {e}");
        };
        let mut off_tree = ok.clone();
        off_tree.path = vec![Action::Bet { to: 50 }];
        refuse(&off_tree, "not a decision node");
        let mut terminal = ok.clone();
        terminal.path = vec![CHECK, CHECK];
        refuse(&terminal, "not a decision node");
        let mut actor = ok.clone();
        actor.actor = "oop".into();
        refuse(&actor, "actor");
        let mut wide = ok.clone();
        for row in wide.probs.iter_mut() { row.push(0.0); }
        refuse(&wide, "columns");
        let mut short = ok.clone();
        short.probs.pop();
        refuse(&short, "1326");
        let mut sum = ok.clone();
        sum.probs[75] = vec![0.1, 0.3];
        refuse(&sum, "sums to");
        game.play(0);
        assert!(game.current_locking_strategy().is_none(), "no refusal locked anything");
        game.back_to_root();

        let e = apply(&mut configured(&req), &ok, m).unwrap_err();
        assert!(e.contains("allocat"), "{e}");
        let (mut done, _) = solved(&req, 10, 0.0);
        let e = apply(&mut done, &ok, m).unwrap_err();
        assert!(e.contains("solved"), "{e}");

        // a river node of a turn-rooted tree: its path crosses a chance node the lock cannot name
        let turn = turn_request();
        let river = turn.tree.materialized.iter().find(|n| n.street == proto::Street::River).expect("the turn tree reaches the river");
        let path = crate::extract::chip_path_of(&turn.tree.materialized, &river.path).unwrap();
        let later = NodeLock { path, actor: river.actor.clone(), probs: vec![vec![0.0; river.actions.len()]; 1326] };
        let e = apply(&mut allocated(&turn), &later, &turn.tree.materialized).unwrap_err();
        assert!(e.contains("street"), "{e}");
    }
}
