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

/// Sizes are converted through `f32 -> f64` exactly as the engine computes them; never through the
/// `"33%"` string parser — and every coefficient is validated **before** it is converted (review
/// R2). `MenuSize` validates its own domain on both serde directions, but these `proto` types are
/// public and constructible in Rust, so a direct caller establishes no such invariant: a negative,
/// zero or NaN bet fraction would otherwise reach upstream's float-to-int conversion and
/// minimum-wager clamp and come back out as a plausible `Bet(1)`, and `+inf` as an `AllIn`,
/// silently replacing invalid input with a tree the engine's matching boundary
/// (`crates/engine/src/tree/materialize.rs`, `validate_template`) refuses to produce.
///
/// A *large finite* coefficient stays legal: upstream's saturation and min/max clamp are the
/// sanctioned §4.6 tree rules, and every clamped amount is bounded by the effective stack, so it
/// cannot overflow anything. This is input validation, not clamping — nothing is rewritten here.
fn bet_sizes(side: &SideMenu, street: Street, who: &str) -> Result<BetSizeOptions, String> {
    // `raise` picks the domain: a raise coefficient multiplies the facing wager, so `<= 1.0` names
    // a raise-to that does not exceed it -- which upstream itself refuses at parse time
    // (`third_party/postflop-solver/src/bet_size.rs:164`, "Multiplier must be greater than 1.0").
    let convert = |v: &[MenuSize], label: &str, raise: bool| -> Result<Vec<BetSize>, String> {
        v.iter().map(|m| match m {
            MenuSize::AllIn => Ok(BetSize::AllIn),
            MenuSize::Pot(x) if !x.is_finite() => Err(format!("{who} {label} menu size {x:e} on {street:?} must be finite")),
            MenuSize::Pot(x) if raise && *x <= 1.0 => Err(format!("{who} {label} menu size {x:e} on {street:?} must be greater than 1.0 (a raise multiplies the facing wager)")),
            MenuSize::Pot(x) if !raise && *x <= 0.0 => Err(format!("{who} {label} menu size {x:e} on {street:?} must be positive")),
            MenuSize::Pot(x) => Ok(if raise { BetSize::PrevBetRelative(*x as f64) } else { BetSize::PotRelative(*x as f64) }),
        }).collect()
    };
    Ok(BetSizeOptions { bet: convert(&side.bet, "bet", false)?, raise: convert(&side.raise, "raise", true)? })
}

/// The scalar inputs, validated wide and unclamped before anything narrows or reaches the library
/// (review R2). `EffectiveTree`'s and `SolveRequest`'s serde codecs already enforce these domains
/// on the wire, but a direct caller (the in-process tests, `bench materialize`) builds the public
/// types in Rust and bypasses them. Upstream's own `check_config` tests the rake rate with
/// `< 0.0` / `> 1.0` and the thresholds with `< 0.0`, all of which a NaN passes.
///
/// `rake_cap_mchips` needs no check: `u32 / 1000.0` is finite and non-negative by construction.
fn validate_scalars(t: &EffectiveTree, rake_rate: f32) -> Result<(), String> {
    // Fix round 1 (review I1): half-open at 1, aligned with the shared wire codec's domain
    // (`proto::numeric::domain_rake_rate`) and with `protocol::precheck`'s admission check.
    if !(rake_rate.is_finite() && (0.0..1.0).contains(&rake_rate)) {
        return Err(format!("rake_rate {rake_rate:e} must be finite and in [0, 1)"));
    }
    for (label, x) in [("add_allin_threshold", t.add_allin_threshold), ("force_allin_threshold", t.force_allin_threshold)] {
        if !(x.is_finite() && x >= 0.0) { return Err(format!("{label} {x:e} must be finite and non-negative")); }
    }
    Ok(())
}

/// Refuses a configuration whose all-in thresholds cannot be represented in the library's own
/// integer domain (review R1).
///
/// Upstream adds an all-in threshold to a wager **in `i32`**: `prev_amount + (pot as f64 *
/// add_allin_threshold).round() as i32` (`third_party/postflop-solver/src/action_tree.rs:659`) and
/// `amount + (new_pot as f64 * force_allin_threshold).round() as i32` (`:669`). Rust's
/// float-to-int cast saturates at `i32::MAX`, so a threshold large enough to saturate makes that
/// addition overflow — a panic inside `ActionTree::new` in debug, and in release a wrapped
/// comparison that silently drops an all-in the engine's `i64` mirror of the same rule (spec §4.6,
/// `crates/engine/src/tree/materialize.rs`) does list, i.e. a realized tree that could only ever
/// fail `cross_check`. Neither outcome is recoverable once the library has been called, the
/// threshold must never be clamped, and the engine must never be changed to reproduce wrapped
/// arithmetic; so the configuration is rejected here, before `ActionTree::new`.
///
/// `pot + 2 * eff` bounds every pot upstream can reach in this tree and `eff` bounds every
/// `prev_amount`/`amount` it adds a threshold to, so `round(pot_max * threshold) + eff <= i32::MAX`
/// is sufficient for every node of every tree this configuration can build. Computed in
/// `u64`/`i64`/`f64` throughout — never in the `i32` that is the thing being checked.
fn check_threshold_headroom(label: &str, threshold: f32, pot_max: u64, eff: u32) -> Result<(), String> {
    // `validate_scalars` established that `threshold` is finite and non-negative and the caller
    // that `pot_max < 2^31`, so this product is finite and non-negative.
    let scaled = (pot_max as f64) * (threshold as f64);
    // `pot >= 1` and `pot + 2 * eff < 2^31` give `eff <= 2^30 - 1`, so the headroom stays positive.
    let headroom = i64::from(i32::MAX) - i64::from(eff);
    // Negated comparison so a non-finite product (unreachable, but free to cover) is a rejection.
    if !(scaled.round() <= headroom as f64) {
        return Err(format!(
            "{label} {threshold:e} overflows the pinned solver's i32 threshold addition: with starting pot + 2 * \
             effective stack = {pot_max}, round({pot_max} * {label}) = {scaled:e} must be at most {headroom} \
             (= i32::MAX - effective stack {eff}); reduce the threshold or the stakes"
        ));
    }
    Ok(())
}

pub fn tree_config(t: &EffectiveTree, pot: u32, eff: u32, rake_rate: f32, rake_cap_mchips: u32) -> Result<TreeConfig, String> {
    if t.rules_version != 3 { return Err(format!("rules_version {} unsupported (expected 3)", t.rules_version)); }
    if t.merging_threshold != 0.0 { return Err("merging_threshold must be 0.0".into()); }
    validate_scalars(t, rake_rate)?;
    if pot == 0 || eff == 0 { return Err("starting pot and effective stack must be positive".into()); }
    let pot_max = (pot as u64) + 2 * (eff as u64);
    if pot_max >= (1u64 << 31) { return Err("pot + stacks exceed 2^31".into()); }
    check_threshold_headroom("add_allin_threshold", t.add_allin_threshold, pot_max, eff)?;
    check_threshold_headroom("force_allin_threshold", t.force_allin_threshold, pot_max, eff)?;
    let initial_state = board_state(t.root_street)?;
    let sides = |s: Street| -> Result<[BetSizeOptions; 2], String> {
        if s < t.root_street { return Ok(Default::default()); }       // earlier streets are ignored by the library
        let m = t.menus.get(&s).ok_or_else(|| format!("no menu for {s:?}"))?;
        Ok([bet_sizes(&m.oop, s, "oop")?, bet_sizes(&m.ip, s, "ip")?])
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
    use proto::PlayerMenus;
    use std::collections::BTreeMap;

    /// A river-rooted tree built straight from the public `proto` types, the way a direct caller
    /// that never touches JSON reaches this module (reviews R1 and R2).
    fn river_tree(bet: Vec<MenuSize>, raise: Vec<MenuSize>) -> EffectiveTree {
        let side = SideMenu { bet, raise };
        EffectiveTree {
            rules_version: 3, template_id: "t8_boundary_v1".into(), root_street: Street::River,
            menus: BTreeMap::from([(Street::River, PlayerMenus { oop: side.clone(), ip: side, donk: None })]),
            add_allin_threshold: 1.5, force_allin_threshold: 0.15, merging_threshold: 0.0,
            wager_cap: 1, inserted: vec![], materialized: vec![],
        }
    }
    fn threshold_tree(add: f32, force: f32) -> EffectiveTree {
        let mut t = river_tree(vec![MenuSize::Pot(0.5)], vec![MenuSize::Pot(2.5)]);
        t.add_allin_threshold = add;
        t.force_allin_threshold = force;
        t
    }

    /// R1: upstream adds an all-in threshold to a wager **in `i32`** — `prev_amount + (pot as f64
    /// * add_allin_threshold).round() as i32` (`third_party/postflop-solver/src/action_tree.rs`
    /// line 659) and `amount + (new_pot as f64 * force_allin_threshold).round() as i32` (line 669).
    /// The float-to-int cast saturates, so a saturating threshold overflows that addition: a panic
    /// inside `ActionTree::new` in debug, and in release a wrapped comparison that silently drops
    /// an all-in the engine's `i64` mirror of the same rule (spec §4.6) does list — a tree that
    /// could only ever fail `cross_check`. The adapter must refuse such a configuration before the
    /// library is called, and must refuse it **only** past the representability limit.
    #[test]
    fn thresholds_that_overflow_the_library_i32_addition_are_refused() {
        // pot + 2 * eff = 2_147_483_646, so round(pot_max * 1.0) + eff == 2_147_483_647 == i32::MAX
        let add = threshold_tree(1.0, 0.0);
        tree_config(&add, 2_147_483_644, 1, 0.0, 0).expect("exactly at the i32 limit is representable");
        build(&add, 2_147_483_644, 1, 0.0, 0, &[]).expect("and the library builds it without overflowing");
        // one chip of pot further: the same sum is i32::MAX + 1
        let e = tree_config(&add, 2_147_483_645, 1, 0.0, 0).unwrap_err();
        assert!(e.contains("add_allin_threshold") && e.contains("i32"), "{e}");
        assert!(build(&add, 2_147_483_645, 1, 0.0, 0, &[]).is_err(), "build must refuse it, never panic");

        // the force threshold has the analogous `amount + threshold` exposure (line 669); at the
        // limit the library really does compute that i32::MAX-valued sum, and must not overflow
        let force = threshold_tree(0.0, 1.0);
        tree_config(&force, 2_147_483_644, 1, 0.0, 0).expect("exactly at the i32 limit is representable");
        build(&force, 2_147_483_644, 1, 0.0, 0, &[]).expect("and the library builds it without overflowing");
        let e = tree_config(&force, 2_147_483_645, 1, 0.0, 0).unwrap_err();
        assert!(e.contains("force_allin_threshold") && e.contains("i32"), "{e}");

        // the review's reported counterexample: pot 1e9, stack 5e8, add 1.5 — accepted before the
        // fix, then panicked inside `ActionTree::new` in debug and dropped IP's all-in in release
        let reported = threshold_tree(1.5, 0.0);
        let e = tree_config(&reported, 1_000_000_000, 500_000_000, 0.0, 0).unwrap_err();
        assert!(e.contains("add_allin_threshold"), "{e}");
        assert!(build(&reported, 1_000_000_000, 500_000_000, 0.0, 0, &[]).is_err());

        // ordinary stakes are untouched
        tree_config(&threshold_tree(1.5, 0.15), 100, 500, 0.0, 0).expect("ordinary stakes stay legal");
    }

    /// R2: these `proto` types are public and constructible in Rust, so their serde codecs
    /// establish no invariant for a direct caller. Every number must be validated at this boundary,
    /// before upstream's float-to-int conversion and minimum-wager clamp silently turn invalid
    /// input into a plausible tree (the review probe got `Bet(1)` from `-1.0` and from `NaN`, and
    /// `AllIn(500)` from `+inf`).
    #[test]
    fn invalid_menu_numbers_and_thresholds_are_refused_at_the_boundary() {
        let ok = river_tree(vec![MenuSize::Pot(0.5)], vec![MenuSize::Pot(2.5)]);
        tree_config(&ok, 100, 500, 0.0, 0).expect("the control configuration is valid");

        for bad in [-1.0f32, 0.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let t = river_tree(vec![MenuSize::Pot(bad)], vec![MenuSize::Pot(2.5)]);
            let e = tree_config(&t, 100, 500, 0.0, 0).unwrap_err();
            assert!(e.contains("bet menu size"), "bet fraction {bad:e} should be named: {e}");
            assert!(build(&t, 100, 500, 0.0, 0, &[]).is_err(), "build must refuse bet fraction {bad:e}");
        }
        // a raise multiple must strictly EXCEED 1.0: at or below it names a raise-to that does not
        // exceed the facing wager, which upstream itself refuses (`bet_size.rs:164`)
        for bad in [1.0f32, 0.5, 0.0, -2.0, f32::NAN, f32::INFINITY] {
            let t = river_tree(vec![MenuSize::Pot(0.5)], vec![MenuSize::Pot(bad)]);
            let e = tree_config(&t, 100, 500, 0.0, 0).unwrap_err();
            assert!(e.contains("raise menu size"), "raise multiple {bad:e} should be named: {e}");
        }
        // a LARGE finite coefficient stays legal: upstream's saturation and clamp are the
        // sanctioned §4.6 tree rules, which this boundary must not pre-empt
        let big = river_tree(vec![MenuSize::Pot(1e30)], vec![MenuSize::Pot(1e30)]);
        tree_config(&big, 100, 500, 0.0, 0).expect("a large finite coefficient is valid");
        build(&big, 100, 500, 0.0, 0, &[]).expect("and builds through upstream's saturation and clamp");
        // an `a` entry carries no coefficient to validate
        tree_config(&river_tree(vec![MenuSize::AllIn], vec![MenuSize::AllIn]), 100, 500, 0.0, 0).expect("all-in-only menus are valid");

        for bad in [f32::NAN, f32::INFINITY, -0.5f32] {
            let mut t = ok.clone();
            t.add_allin_threshold = bad;
            let e = tree_config(&t, 100, 500, 0.0, 0).unwrap_err();
            assert!(e.contains("add_allin_threshold"), "add threshold {bad:e}: {e}");
            let mut t = ok.clone();
            t.force_allin_threshold = bad;
            let e = tree_config(&t, 100, 500, 0.0, 0).unwrap_err();
            assert!(e.contains("force_allin_threshold"), "force threshold {bad:e}: {e}");
        }
        // Fix round 1 (review I1): the shared wire domain (`proto::numeric::domain_rake_rate`) is half-open
        // at 1, so this direct-builder path must refuse 1.0 too, never a NaN either.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01f32, 1.0f32, 1.01f32] {
            let e = tree_config(&ok, 100, 500, bad, 0).unwrap_err();
            assert!(e.contains("rake_rate"), "rake_rate {bad:e}: {e}");
        }
        tree_config(&ok, 100, 500, 0.999_999_94, 5_000).expect("a rake rate immediately below 1.0 is valid");
    }

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
