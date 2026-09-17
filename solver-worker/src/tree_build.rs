//! §10.3 tree mapping and the §4.6 cross-check: the engine materializes, the worker mirrors and compares.
use postflop_solver::{Action as LibAction, ActionTree, BetSize, BetSizeOptions, BoardState, DonkSizeOptions, TreeConfig};
use proto::{Action, EffectiveTree, MaterializedNode, MenuSize, SideMenu, Street};

pub fn to_lib_action(a: &Action) -> LibAction {
    match a { Action::Fold => LibAction::Fold, Action::Check => LibAction::Check, Action::Call => LibAction::Call,
        Action::Bet { to } => LibAction::Bet(*to as i32), Action::Raise { to } => LibAction::Raise(*to as i32), Action::AllIn { to } => LibAction::AllIn(*to as i32) }
}
pub fn from_lib_action(a: LibAction) -> Option<Action> {
    Some(match a { LibAction::Fold => Action::Fold, LibAction::Check => Action::Check, LibAction::Call => Action::Call,
        LibAction::Bet(x) => Action::Bet { to: x as u32 }, LibAction::Raise(x) => Action::Raise { to: x as u32 }, LibAction::AllIn(x) => Action::AllIn { to: x as u32 }, _ => return None })
}
pub fn board_state(s: Street) -> Result<BoardState, String> {
    match s { Street::Flop => Ok(BoardState::Flop), Street::Turn => Ok(BoardState::Turn), Street::River => Ok(BoardState::River), Street::Preflop => Err("preflop is not a solve root".into()) }
}
fn next_street(s: Street) -> Street { match s { Street::Flop => Street::Turn, _ => Street::River } }
fn actor_name(a: usize) -> String { if a == 0 { "oop".into() } else { "ip".into() } }

/// Sizes are converted through `f32 -> f64` exactly as the engine computes them; never through the `"33%"` string parser.
fn bet_sizes(side: &SideMenu) -> BetSizeOptions {
    let bet = side.bet.iter().map(|m| match m { MenuSize::Pot(r) => BetSize::PotRelative(*r as f64), MenuSize::AllIn => BetSize::AllIn }).collect();
    let raise = side.raise.iter().map(|m| match m { MenuSize::Pot(x) => BetSize::PrevBetRelative(*x as f64), MenuSize::AllIn => BetSize::AllIn }).collect();
    BetSizeOptions { bet, raise }
}

pub fn tree_config(t: &EffectiveTree, pot: u32, eff: u32, rake_rate: f32, rake_cap_mchips: u32) -> Result<TreeConfig, String> {
    if t.rules_version != 3 { return Err(format!("rules_version {} unsupported (expected 3)", t.rules_version)); }
    if t.merging_threshold != 0.0 { return Err("merging_threshold must be 0.0".into()); }
    if pot == 0 || eff == 0 { return Err("starting pot and effective stack must be positive".into()); }
    if (pot as u64) + 2 * (eff as u64) >= (1u64 << 31) { return Err("pot + stacks exceed 2^31".into()); }
    let initial_state = board_state(t.root_street)?;
    let sides = |s: Street| -> Result<[BetSizeOptions; 2], String> {
        if s < t.root_street { return Ok(Default::default()); }       // earlier streets are ignored by the library
        let m = t.menus.get(&s).ok_or_else(|| format!("no menu for {s:?}"))?;
        Ok([bet_sizes(&m.oop), bet_sizes(&m.ip)])
    };
    for s in [Street::Turn, Street::River] {
        if s > t.root_street {
            match t.menus.get(&s).and_then(|m| m.donk.as_ref()) { Some(d) if d.is_empty() => {}, Some(_) => return Err(format!("{s:?} donk sizes must be empty")), None => return Err(format!("{s:?} donk option must be the explicit empty list (never None)")) }
        }
    }
    Ok(TreeConfig {
        initial_state, starting_pot: pot as i32, effective_stack: eff as i32,
        rake_rate: rake_rate as f64, rake_cap: rake_cap_mchips as f64 / 1000.0,
        flop_bet_sizes: sides(Street::Flop)?, turn_bet_sizes: sides(Street::Turn)?, river_bet_sizes: sides(Street::River)?,
        turn_donk_sizes: Some(DonkSizeOptions { donk: vec![] }), river_donk_sizes: Some(DonkSizeOptions { donk: vec![] }),
        add_allin_threshold: t.add_allin_threshold as f64, force_allin_threshold: t.force_allin_threshold as f64, merging_threshold: 0.0,
    })
}

/// Template tree, then every inserted observed size with `add_line`, then the wager cap with `remove_line`.
pub fn build(t: &EffectiveTree, pot: u32, eff: u32, rake_rate: f32, rake_cap_mchips: u32, history: &[Action]) -> Result<ActionTree, String> {
    let mut tree = ActionTree::new(tree_config(t, pot, eff, rake_rate, rake_cap_mchips)?)?;
    for (chip, _actor, action) in &t.inserted {
        let mut line: Vec<LibAction> = chip.iter().map(to_lib_action).collect();
        line.push(to_lib_action(action));
        tree.add_line(&line).map_err(|e| format!("insert {action:?} at {chip:?}: {e}"))?;
    }
    let lib_history: Vec<LibAction> = history.iter().map(to_lib_action).collect();
    apply_wager_cap(&mut tree, t.wager_cap, &lib_history)?;
    tree.apply_history(&lib_history).map_err(|e| format!("history not representable: {e}"))?;
    tree.back_to_root();
    Ok(tree)
}

/// §4.6 wager cap: at every node where the non-all-in wagers of the street (prefix included) reached `cap`,
/// every non-all-in bet and raise is removed, except an observed prefix action.
pub fn apply_wager_cap(tree: &mut ActionTree, cap: u8, history: &[LibAction]) -> Result<(), String> {
    let mut to_remove: Vec<Vec<LibAction>> = Vec::new();
    walk_cap(tree, &mut Vec::new(), 0, usize::from(cap), history, &mut to_remove)?;
    for line in to_remove { tree.remove_line(&line)?; }
    tree.back_to_root();
    Ok(())
}
/// `wagers` is a `usize` for the same reason the engine's mirror counts in one (a byte counter
/// overflows on a long observed prefix, which would silently reopen capped raises).
fn walk_cap(tree: &mut ActionTree, line: &mut Vec<LibAction>, wagers: usize, cap: usize, history: &[LibAction], out: &mut Vec<Vec<LibAction>>) -> Result<(), String> {
    tree.apply_history(line)?;
    if tree.is_terminal_node() { return Ok(()); }
    let wagers = if tree.is_chance_node() { 0 } else { wagers };      // a street transition resets the count
    for a in tree.available_actions().to_vec() {
        let observed = line.len() < history.len() && history[..line.len()] == line[..] && history[line.len()] == a;
        let is_wager = matches!(a, LibAction::Bet(_) | LibAction::Raise(_));
        line.push(a);
        if wagers >= cap && is_wager && !observed { out.push(line.clone()); } else { walk_cap(tree, line, wagers + is_wager as usize, cap, history, out)?; }
        line.pop();
    }
    Ok(())
}

/// The library tree as a betting skeleton (§2): every action node of every street, with the actor derived from the
/// path, street from chance crossings, and terminal pots from `total_bet_amount()`.
pub fn enumerate(tree: &mut ActionTree, root_street: Street, starting_pot: u32) -> Result<Vec<MaterializedNode>, String> {
    let mut out = Vec::new();
    walk_enum(tree, &mut Vec::new(), &mut Vec::new(), root_street, 0, starting_pot as i64, &mut out)?;
    tree.back_to_root();
    Ok(out)
}
fn walk_enum(tree: &mut ActionTree, line: &mut Vec<LibAction>, path: &mut Vec<u8>, street: Street, actor: usize, p: i64, out: &mut Vec<MaterializedNode>) -> Result<(), String> {
    tree.apply_history(line)?;
    let actions = tree.available_actions().to_vec();
    let mut node = MaterializedNode { path: path.clone(), street, actor: actor_name(actor), actions: Vec::new(), terminal_pots: Vec::new() };
    let mut children = Vec::new();
    for a in &actions {
        line.push(*a);
        tree.apply_history(line)?;
        let terminal = tree.is_terminal_node();
        let pot = if terminal { let t = tree.total_bet_amount(); Some((p + 2 * t[0].min(t[1]) as i64) as u32) } else { None };
        let (s, act) = if terminal { (street, actor) } else if tree.is_chance_node() { (next_street(street), 0) } else { (street, if *a == LibAction::Check { 1 } else { actor ^ 1 }) };
        node.actions.push(from_lib_action(*a).ok_or("chance action in a decision menu")?);
        node.terminal_pots.push(pot);
        children.push((*a, terminal, s, act));
        line.pop();
    }
    out.push(node);
    for (i, (a, terminal, s, act)) in children.into_iter().enumerate() {
        if terminal { continue; }
        // An ordinal that does not fit `u8` would alias another child's path, exactly as
        // `proto::resolve_chip_path` and the engine's materializer refuse rather than narrow.
        let idx = u8::try_from(i).map_err(|_| format!("menu index {i} is not a representable ordinal"))?;
        line.push(a); path.push(idx);
        walk_enum(tree, line, path, s, act, p, out)?;
        line.pop(); path.pop();
    }
    Ok(())
}

/// Any difference is `tree_mismatch` (§4.6): never a silently different tree.
pub fn cross_check(lib: &[MaterializedNode], expected: &[MaterializedNode]) -> Result<(), String> {
    if lib.len() != expected.len() { return Err(format!("node count {} (library) != {} (engine)", lib.len(), expected.len())); }
    for (a, b) in lib.iter().zip(expected) {
        if a != b { return Err(format!("first difference at path {:?}: library {:?} {:?} {:?} {:?} vs engine {:?} {:?} {:?} {:?}", a.path, a.street, a.actor, a.actions, a.terminal_pots, b.street, b.actor, b.actions, b.terminal_pots)); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history;
    use crate::testutil::{cases, solve_request};
    use postflop_solver::Action as L;

    #[test]
    fn river_fixture_tree_matches_library() {
        let req = solve_request("river_two_combo", 0);
        let mut tree = build(&req.tree, req.pot, req.stack_oop.min(req.stack_ip), req.rake_rate, req.rake_cap_mchips, &req.history).unwrap();
        assert_eq!(tree.available_actions(), &[L::Check]);
        tree.apply_history(&[L::Check]).unwrap();
        assert_eq!(tree.available_actions(), &[L::Check, L::AllIn(100)]);
        let lib = enumerate(&mut tree, req.tree.root_street, req.pot).unwrap();
        cross_check(&lib, &req.tree.materialized).unwrap();
        assert_eq!(lib.len(), 3);
    }

    #[test]
    fn every_materialization_case_matches_library() {
        let all = cases();
        assert_eq!(all.len(), 47);
        assert_eq!(all.iter().filter(|c| c.template_id == "flop_full_v1").count(), 5);
        for c in &all {
            let mut tree = build(&c.tree, c.pot, c.eff, 0.0, 0, &c.history).unwrap_or_else(|e| panic!("{}: {e}", c.case));
            let lib = enumerate(&mut tree, c.tree.root_street, c.pot).unwrap();
            cross_check(&lib, &c.tree.materialized).unwrap_or_else(|e| panic!("{}: {e}", c.case));
            // the decision node is reachable through the library with the observed history
            tree.apply_history(&history::history_to_lib(&c.history)).unwrap_or_else(|e| panic!("{}: {e}", c.case));
        }
        let cap1 = all.iter().find(|c| c.case == "cap1_two_wagers").unwrap();
        let mut t = build(&cap1.tree, cap1.pot, cap1.eff, 0.0, 0, &cap1.history).unwrap();
        t.apply_history(&history::history_to_lib(&cap1.history)).unwrap();
        assert_eq!(t.available_actions(), &[L::Fold, L::Call, L::AllIn(500)]);
        t.apply_history(&[L::Bet(40)]).unwrap();
        assert_eq!(t.available_actions(), &[L::Fold, L::Call, L::Raise(120), L::AllIn(500)]);   // the observed prefix survives the cap
        let ins = all.iter().find(|c| c.case == "insert_73").unwrap();
        let mut t = build(&ins.tree, ins.pot, ins.eff, 0.0, 0, &ins.history).unwrap();
        assert_eq!(t.available_actions(), &[L::Check, L::Bet(50), L::Bet(73)]);
        t.apply_history(&[L::Bet(73)]).unwrap();
        assert!(t.available_actions().contains(&L::Raise(183)));
    }

    /// The §4.6 cross-check is only as strong as the ordinal arithmetic it shares with the wire:
    /// the library-enumerated tree must resolve each case's observed history to the very
    /// `decision_path` the engine published (`proto::resolve_chip_path`, §2), and each prefix step
    /// must have produced exactly one history action.
    #[test]
    fn observed_history_resolves_to_the_published_decision_path() {
        for c in cases() {
            assert_eq!(c.prefix.len(), c.history.len(), "{}: one history action per prefix step", c.case);
            let mut tree = build(&c.tree, c.pot, c.eff, 0.0, 0, &c.history).unwrap();
            let lib = enumerate(&mut tree, c.tree.root_street, c.pot).unwrap();
            assert_eq!(proto::resolve_chip_path(&lib, &c.history), Some(c.decision_path.clone()), "{}", c.case);
        }
    }

    #[test]
    fn altered_materialized_entry_is_a_mismatch() {
        let c = cases().into_iter().find(|c| c.case == "facing_350_full").unwrap();
        let mut tree = build(&c.tree, c.pot, c.eff, 0.0, 0, &c.history).unwrap();
        let lib = enumerate(&mut tree, c.tree.root_street, c.pot).unwrap();
        let mut altered = c.tree.materialized.clone();
        // missing terminal marker. The brief's literal `altered[1].terminal_pots[0] = None` is a
        // no-op on this fixture -- node 1 is the flop IP node after OOP's check, whose markers are
        // already `[None, None]` -- so the FIRST marker that is actually `Some` is the one dropped.
        let (i, j) = altered.iter().enumerate()
            .find_map(|(i, n)| n.terminal_pots.iter().position(Option::is_some).map(|j| (i, j)))
            .expect("the fixture tree carries at least one terminal marker");
        altered[i].terminal_pots[j] = None;
        assert!(cross_check(&lib, &altered).is_err());
        let mut reset = c.tree.materialized.clone();             // a per-street reset menu at the turn root
        let turn = reset.iter().position(|n| n.path == [1, 1]).unwrap();
        reset[turn].actions = vec![Action::Check, Action::Bet { to: 100 }];
        reset[turn].terminal_pots = vec![None, None];
        assert!(cross_check(&lib, &reset).is_err());
        // A LATER street's None is structurally invalid input for a direct caller that bypasses `precheck`.
        // It is not a tree_mismatch: nothing is enumerated and `cross_check` is never reached.
        let mut none_donk = c.tree.clone();
        none_donk.menus.get_mut(&Street::Turn).unwrap().donk = None;
        let e = tree_config(&none_donk, c.pot, c.eff, 0.0, 0).unwrap_err();
        assert!(e.contains("donk option must be the explicit empty list"), "{e}");
    }

    #[test]
    fn root_street_none_donk_is_legal() {
        // `facing_test_v1` is flop-rooted, so the flop menu's donk is None by construction (§4.6) and is accepted;
        // upstream ignores donk sizes at the root because `prev_action` is None there.
        let c = cases().into_iter().find(|c| c.case == "facing_350_full").unwrap();
        assert_eq!(c.tree.menus[&c.tree.root_street].donk, None);
        tree_config(&c.tree, c.pot, c.eff, 0.0, 0).expect("root-street None donk is legal");
        let mut tree = build(&c.tree, c.pot, c.eff, 0.0, 0, &c.history).unwrap();
        let lib = enumerate(&mut tree, c.tree.root_street, c.pot).unwrap();
        cross_check(&lib, &c.tree.materialized).unwrap();
    }
}
