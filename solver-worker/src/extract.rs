//! §4.5 / §10.3 extraction: every decision node of the solved street as combo-major wire matrices,
//! under the §4.5 export limits.
use crate::cards::lib_hand_to_combo;
use crate::history::indices_for;
use crate::tree_build::from_lib_action;
use postflop_solver::{Card as LibCard, Game, PostFlopGame};
use proto::worker::{NodeStrategy, ResultStatus, SolveRequest, StreetSolution, WorkerMessage};
use proto::{index_materialized, Action, MaterializedIndex, MaterializedNode, COMBOS};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

/// §4.5 limits, owned by `proto::worker` (plan 1 Task 7). Re-exported so `job.rs` and `protocol.rs`
/// read the same numbers `validate_solution` enforces; never redefined here.
pub use proto::worker::{MAX_EXPORTED_NODES, RESULT_LINE_MAX};

/// The chip path (§2) of the decision node at ordinal path `ordinal`: the exact inverse of
/// `proto::resolve_chip_path`. `None` unless every edge leaves a materialized node through a
/// continuation (a `None` terminal marker) and the node at `ordinal` is itself materialized: an
/// ordinal outside a menu, a terminal child or an absent node never yields a chip path.
pub fn chip_path_of(materialized: &[MaterializedNode], ordinal: &[u8]) -> Option<Vec<Action>> {
    chip_path_indexed(&index_materialized(materialized), ordinal)
}

/// [`chip_path_of`] against an index built once by the caller. The street export resolves one path
/// per node (up to `MAX_EXPORTED_NODES` of them) against the same tree, so it hoists the index rather
/// than scanning the materialized list per prefix, as `proto::worker::validate_solution` does (review S2).
fn chip_path_indexed(index: &MaterializedIndex<'_>, ordinal: &[u8]) -> Option<Vec<Action>> {
    let mut chip = Vec::with_capacity(ordinal.len());
    for k in 0..ordinal.len() {
        let node = index.get(&ordinal[..k])?;
        let i = usize::from(ordinal[k]);
        let action = *node.actions.get(i)?;
        if node.terminal_pots.get(i)?.is_some() { return None; }
        chip.push(action);
    }
    index.get(ordinal)?;
    Some(chip)
}

/// One decision node (§10.3): navigate, `cache_normalized_weights`, `strategy`,
/// `expected_values_detail(current_player())`, then transpose the library's action-major arrays over
/// its compact hand list to combo-major `[1326][action]`. `available` = reach > 0 (the player's
/// `weights`); combos outside the compact list (not in the range, or on the board) are absent and
/// stay all-zero, as do combos with zero reach.
///
/// EV is the library's actor-owned value unchanged (`normalize_ev` is the identity, §10.3) except the
/// fold column, which is emitted as exactly `+0.0` (§2: chips already in the pot are sunk;
/// `proto::worker::validate_solution` compares its bits) once the library's own value there has been
/// checked to be zero: a non-zero fold value means the navigation reached some other node, which is an
/// error, never something to overwrite. A non-finite value or a probability outside `[0, 1]` is an
/// error too, never clamped. The game is back at the root on every return.
pub fn extract_node(game: &mut PostFlopGame, node: &MaterializedNode, chip_path: &[Action]) -> Result<NodeStrategy, String> {
    // `expected_values_detail` panics before `finalize`: refuse instead.
    if !game.is_solved() { return Err(format!("{chip_path:?}: extraction needs a solved game (`finalize` has not run)")); }
    let idx = indices_for(game, chip_path)?;
    game.apply_history(&idx);
    let out = extract_here(game, node, chip_path);
    game.back_to_root();
    out
}

fn extract_here(game: &mut PostFlopGame, node: &MaterializedNode, chip_path: &[Action]) -> Result<NodeStrategy, String> {
    if game.is_terminal_node() || game.is_chance_node() { return Err(format!("{chip_path:?} is not a decision node")); }
    let player = game.current_player();
    let actor = if player == 0 { "oop" } else { "ip" };
    if actor != node.actor { return Err(format!("actor {actor} at {chip_path:?} differs from the skeleton's {}", node.actor)); }
    let lib_actions: Option<Vec<Action>> = game.available_actions().into_iter().map(from_lib_action).collect();
    if lib_actions.as_deref() != Some(node.actions.as_slice()) {
        return Err(format!("menu {lib_actions:?} at {chip_path:?} differs from the skeleton's {:?}", node.actions));
    }
    game.cache_normalized_weights();
    let m = combo_major(game.private_cards(player), game.weights(player), &game.strategy(), &game.expected_values_detail(player), &node.actions)
        .map_err(|e| format!("{e} at {chip_path:?}"))?;
    Ok(NodeStrategy { path: chip_path.to_vec(), actor: node.actor.clone(), actions: node.actions.clone(), probs: m.probs, ev_chips: m.ev_chips, available: m.available })
}

/// The three combo-major `[1326]` matrices of one node.
#[derive(Debug)]
struct Matrices { probs: Vec<Vec<f32>>, ev_chips: Vec<Vec<f32>>, available: Vec<bool> }

/// The §10.3 matrix contract as a pure function of the library's arrays at one node: `hands` is the
/// actor's compact hand list, `reach` its `weights`, and `strat` / `evs` the action-major
/// `[action][hand]` arrays of `strategy()` and `expected_values_detail(actor)`. Library hand ->
/// named combo -> `ComboIndex`; transposed to `[combo][action]`; zero-filled and unavailable
/// wherever reach is zero or the combo is outside the list. The fold column is checked to be zero and
/// emitted as `+0.0` (§2); nothing is clamped or overwritten.
fn combo_major(hands: &[(LibCard, LibCard)], reach: &[f32], strat: &[f32], evs: &[f32], actions: &[Action]) -> Result<Matrices, String> {
    let (n, na) = (hands.len(), actions.len());
    if strat.len() != n * na || evs.len() != n * na || reach.len() != n {
        return Err(format!("matrix shape: strategy {}, EV {}, reach {} for {n} hands x {na} actions", strat.len(), evs.len(), reach.len()));
    }
    let mut m = Matrices { probs: vec![vec![0.0f32; na]; COMBOS], ev_chips: vec![vec![0.0f32; na]; COMBOS], available: vec![false; COMBOS] };
    for (h, hand) in hands.iter().enumerate() {
        let r = reach[h];
        if !r.is_finite() { return Err(format!("non-finite reach {r} for library hand {hand:?}")); }
        if r <= 0.0 { continue; }
        let c = usize::from(lib_hand_to_combo(*hand));
        m.available[c] = true;
        for (a, action) in actions.iter().enumerate() {
            let (p, v) = (strat[a * n + h], evs[a * n + h]);
            if !(p.is_finite() && (0.0..=1.0).contains(&p)) { return Err(format!("probability {p} of {action:?} for combo {c} is outside [0, 1]")); }
            if !v.is_finite() { return Err(format!("non-finite EV {v} of {action:?} for combo {c}")); }
            m.probs[c][a] = p;
            m.ev_chips[c][a] = if *action == Action::Fold {
                // The library writes a literal 0.0 for a fold row, so a non-zero value means the
                // navigation reached some other node: an error, never overwritten.
                if v != 0.0 { return Err(format!("the library's fold EV {v} for combo {c} is not 0 (section 2: chips in the pot are sunk)")); }
                0.0
            } else {
                v
            };
        }
    }
    Ok(m)
}

#[derive(Debug, Clone, Copy)]
pub struct SolutionMeta { pub exploitability_chips: f32, pub iterations: u32, pub memory_bytes: u64, pub mode: &'static str, pub locks_applied: u16 }

/// The §4.5 export limits `street_solution` applies (`MAX_EXPORTED_NODES`, `RESULT_LINE_MAX`); a
/// parameter only so the tests reach both truncation paths without a 100,000-node or 16 MiB export.
#[derive(Debug, Clone, Copy)]
struct Limits { max_nodes: usize, max_line: usize }

fn assemble(nodes: Vec<NodeStrategy>, requested: u32, export: &str, m: &SolutionMeta) -> StreetSolution {
    let covered_paths = nodes.iter().map(|n| n.path.clone()).collect();
    StreetSolution { nodes, requested, exploitability_chips: m.exploitability_chips, iterations: m.iterations, memory_bytes: m.memory_bytes, mode: m.mode.into(), locks_applied: m.locks_applied, export: export.into(), covered_paths }
}

/// Counts serialized bytes without buffering a line that may run to many MiB.
struct ByteCount(usize);
impl io::Write for ByteCount {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> { self.0 += buf.len(); Ok(buf.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

/// Bytes of the `result` line that would carry `sol`, terminating newline included. The export is
/// sized before its `status` and `elapsed_ms` are known, so both are taken at their longest
/// (`best_so_far`, a ten-digit `u32::MAX`): the line actually written is never longer than measured.
/// The solution is moved in and handed back rather than cloned; a solution that does not serialize
/// (a non-finite or out-of-domain value) is an error, never a size.
fn result_line_len(sol: StreetSolution, id: &str) -> (Result<usize, String>, StreetSolution) {
    let msg = WorkerMessage::Result { id: id.to_string(), status: ResultStatus::BestSoFar, elapsed_ms: u32::MAX, solution: Some(sol), error: None };
    let mut count = ByteCount(0);
    let len = serde_json::to_writer(&mut count, &msg).map(|()| count.0 + 1).map_err(|e| format!("the solution does not serialize: {e}"));
    match msg {
        WorkerMessage::Result { solution: Some(sol), .. } => (len, sol),
        _ => unreachable!("built above as a result carrying the solution"),
    }
}

/// Every decision node of the root street (§10.3), in materialized order, with `requested` the index
/// of the node `req.history` reaches; under the §4.5 limits an export over `MAX_EXPORTED_NODES` nodes
/// or over `RESULT_LINE_MAX` bytes carries the requested node only (`export: "truncated"`,
/// `covered_paths` naming exactly it), and a requested node that does not fit on its own is an error.
/// `Ok(None)` when `cancel` is observed between nodes or after the last one (§4.5: `Extracting`
/// answers a cancel after the node being extracted). The game is left at the root.
pub fn street_solution(game: &mut PostFlopGame, req: &SolveRequest, meta: SolutionMeta, cancel: &AtomicBool) -> Result<Option<StreetSolution>, String> {
    export(game, req, meta, cancel, Limits { max_nodes: MAX_EXPORTED_NODES, max_line: RESULT_LINE_MAX })
}

fn export(game: &mut PostFlopGame, req: &SolveRequest, meta: SolutionMeta, cancel: &AtomicBool, limits: Limits) -> Result<Option<StreetSolution>, String> {
    let index = index_materialized(&req.tree.materialized);
    let street: Vec<&MaterializedNode> = req.tree.materialized.iter().filter(|n| n.street == req.tree.root_street).collect();
    let paths = street.iter()
        .map(|n| chip_path_indexed(&index, &n.path).ok_or_else(|| format!("materialized node {:?} has no chip path", n.path)))
        .collect::<Result<Vec<_>, _>>()?;
    let requested = paths.iter().position(|p| *p == req.history)
        .ok_or_else(|| format!("history {:?} is not a decision node of the {:?} street", req.history, req.tree.root_street))?;
    let over_count = street.len() > limits.max_nodes;
    let order: Vec<usize> = if over_count { vec![requested] } else { (0..street.len()).collect() };
    let mut nodes = Vec::with_capacity(order.len());
    for &i in &order {
        if cancel.load(Ordering::SeqCst) { return Ok(None); }
        nodes.push(extract_node(game, street[i], &paths[i])?);
    }
    if cancel.load(Ordering::SeqCst) { return Ok(None); }
    let (requested_index, export) = if over_count { (0, "truncated") } else { (requested, "street") };
    let requested_index = u32::try_from(requested_index).map_err(|_| format!("requested index {requested_index} does not fit u32"))?;
    let too_long = |len: usize| format!("the requested node alone needs a {len}-byte result line, over the {}-byte limit (section 4.5)", limits.max_line);

    let (len, sol) = result_line_len(assemble(nodes, requested_index, export, &meta), &req.id);
    let len = len?;
    if len <= limits.max_line { return Ok(Some(sol)); }
    if sol.nodes.len() == 1 { return Err(too_long(len)); }
    let only = sol.nodes.into_iter().nth(requested).ok_or_else(|| format!("requested node {requested} missing from the export"))?;
    let (len, sol) = result_line_len(assemble(vec![only], 0, "truncated", &meta), &req.id);
    let len = len?;
    if len <= limits.max_line { Ok(Some(sol)) } else { Err(too_long(len)) }
}

/// Games built straight from a `SolveRequest` for the in-process tests of `extract` and `locks`: the
/// request's own tree (`tree_build::build`), board and ranges, exactly as the job builds them.
#[cfg(test)]
pub(crate) mod test_games {
    use crate::cards::{board_to_lib, range_to_lib};
    use crate::testutil::{cases, solve_request};
    use crate::tree_build::build;
    use postflop_solver::{solve, CardConfig, PostFlopGame};
    use proto::worker::SolveRequest;
    use proto::{combo_index, Card, Range1326};

    /// Configured but not allocated: `play` and `lock_current_strategy` would panic on it.
    pub(crate) fn configured(req: &SolveRequest) -> PostFlopGame {
        let tree = build(&req.tree, req.pot, req.stack_oop.min(req.stack_ip), req.rake_rate, req.rake_cap_mchips, &req.history).expect("fixture tree builds");
        let (flop, turn, river) = board_to_lib(&req.board).expect("fixture board");
        let range = [range_to_lib(&req.oop_range).expect("oop range"), range_to_lib(&req.ip_range).expect("ip range")];
        PostFlopGame::with_config(CardConfig { range, flop, turn, river }, tree).expect("fixture game config")
    }

    /// Allocated uncompressed (f32) and unsolved: the state locks are applied in (§10.3).
    pub(crate) fn allocated(req: &SolveRequest) -> PostFlopGame {
        let mut g = configured(req);
        g.allocate_memory(false);
        g
    }

    /// Allocated, solved and finalized (`postflop_solver::solve` finalizes); returns the exploitability.
    pub(crate) fn solved(req: &SolveRequest, iterations: u32, target_chips: f32) -> (PostFlopGame, f32) {
        let mut g = allocated(req);
        let expl = solve(&mut g, iterations, target_chips, false);
        (g, expl)
    }

    pub(crate) fn range_of(combos: &[(&str, &str)]) -> Range1326 {
        let mut r = Range1326([0.0; 1326]);
        for (a, b) in combos { r.0[combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize] = 1.0; }
        r
    }

    /// A turn-rooted request over the committed `basic_turn_std` materialization case (turn_std_v1 at
    /// pot 200, effective stack 900): 20 turn decision nodes and a river beyond them. Two combos a side
    /// keep the solve instantaneous.
    pub(crate) fn turn_request() -> SolveRequest {
        let case = cases().into_iter().find(|c| c.case == "basic_turn_std").expect("basic_turn_std case");
        let mut req = solve_request("river_two_combo", 0);
        req.id = "turn".into();
        req.board = ["Td", "9d", "6h", "Qc"].iter().map(|s| Card::parse(s).unwrap()).collect();
        req.oop_range = range_of(&[("As", "Ah"), ("8s", "7s")]);
        req.ip_range = range_of(&[("Ks", "Kh"), ("Js", "Jh")]);
        req.pot = case.pot;
        req.stack_oop = case.eff;
        req.stack_ip = case.eff;
        req.tree = case.tree;
        req.history = case.history;
        req
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::cases;
    #[test]
    fn ordinal_paths_become_chip_paths() {
        let c = cases().into_iter().find(|c| c.case == "insert_73").unwrap();
        let m = &c.tree.materialized;
        assert_eq!(chip_path_of(m, &[]), Some(vec![]));
        assert_eq!(chip_path_of(m, &[2]), Some(vec![Action::Bet { to: 73 }]));
        assert_eq!(chip_path_of(m, &[1]), Some(vec![Action::Bet { to: 50 }]));
        assert_eq!(chip_path_of(m, &[9]), None);              // ordinal outside the menu
        let r = cases().into_iter().find(|c| c.case == "river_std_v1_100_100").unwrap();
        let root = r.tree.materialized.iter().find(|n| n.path.is_empty()).unwrap();
        assert_eq!(chip_path_of(&r.tree.materialized, &[0]), Some(vec![root.actions[0].clone()]));
    }

    use super::test_games::{allocated, solved, turn_request};
    use crate::testutil::solve_request;
    use proto::worker::validate_solution;
    use proto::{combo_index, resolve_chip_path, Card, Street};

    const CHECK: Action = Action::Check;
    const JAM: Action = Action::AllIn { to: 100 };

    fn combo(a: &str, b: &str) -> usize { combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize }
    /// Fixed metadata: the extraction must copy it through untouched, whatever the solve reached.
    fn meta() -> SolutionMeta { SolutionMeta { exploitability_chips: 0.09, iterations: 7, memory_bytes: 4096, mode: "f32", locks_applied: 0 } }
    fn no_cancel() -> AtomicBool { AtomicBool::new(false) }

    /// §2: `chip_path_of` is the exact inverse of `proto::resolve_chip_path` on every node of all 47
    /// committed cases, and names nothing that is not a materialized decision node: a terminal child
    /// and an ordinal past the menu are `None`, never a chip path to a node that does not exist.
    #[test]
    fn chip_paths_invert_the_wire_resolution_on_every_case() {
        let all = cases();
        assert_eq!(all.len(), 47);
        for c in &all {
            let m = &c.tree.materialized;
            for n in m {
                let chip = chip_path_of(m, &n.path).unwrap_or_else(|| panic!("{}: node {:?} has no chip path", c.case, n.path));
                assert_eq!(resolve_chip_path(m, &chip), Some(n.path.clone()), "{}: {:?}", c.case, n.path);
                for (i, marker) in n.terminal_pots.iter().enumerate() {
                    let mut child = n.path.clone();
                    child.push(u8::try_from(i).unwrap());
                    assert_eq!(chip_path_of(m, &child).is_some(), marker.is_none(), "{}: child {child:?} (terminal marker {marker:?})", c.case);
                }
                let mut past = n.path.clone();
                past.push(u8::try_from(n.actions.len()).unwrap());
                assert_eq!(chip_path_of(m, &past), None, "{}: {past:?} is past the menu", c.case);
            }
        }
    }

    /// The malformed-tree cases `proto::resolve_chip_path` also refuses (its review R3): a node
    /// materialized behind a terminal edge is unreachable, a continuation edge whose child is missing
    /// names no node, and without a root nothing resolves.
    #[test]
    fn chip_paths_need_continuation_edges_and_materialized_nodes() {
        let m = solve_request("river_two_combo", 0).tree.materialized;
        assert_eq!(chip_path_of(&m, &[0, 1]), Some(vec![CHECK, JAM]), "control");
        let mut stray = m.clone();
        stray.push(MaterializedNode { path: vec![0, 0], street: Street::River, actor: "oop".into(), actions: vec![CHECK], terminal_pots: vec![Some(100)] });
        assert_eq!(chip_path_of(&stray, &[0, 0]), None, "IP's check at [0] is terminal (marker Some(100))");
        assert_eq!(resolve_chip_path(&stray, &[CHECK, CHECK]), None, "the wire resolution agrees");
        let missing: Vec<MaterializedNode> = m.iter().filter(|n| n.path != [0, 1]).cloned().collect();
        assert_eq!(chip_path_of(&missing, &[0, 1]), None, "a continuation edge into a node that is not materialized");
        assert_eq!(chip_path_of(&m[1..], &[0]), None, "no root node, no path");
    }

    /// The §10.3 matrix contract as a pure function over synthetic library arrays (the
    /// `combo_matrix_two_named` shape of §13.2), so that its guards are reachable: the action-major
    /// `[action][hand]` input lands at `[combo][action]`; zero reach and every combo outside the hand
    /// list are absent and all-zero; a `-0.0` fold value is emitted as `+0.0`; a non-zero fold value, a
    /// non-finite EV or reach, a probability outside `[0, 1]` and a shape mismatch are errors, never a
    /// clamp or an overwrite.
    #[test]
    fn combo_major_transposes_masks_and_guards() {
        use crate::cards::to_lib;
        let lib = |a: &str, b: &str| (to_lib(Card::parse(a).unwrap()), to_lib(Card::parse(b).unwrap()));
        let hands = [lib("Ks", "As"), lib("7d", "7h"), lib("2c", "3c")];
        let actions = [Action::Fold, Action::Call, Action::Raise { to: 300 }];
        let reach = [1.0f32, 0.5, 0.0];
        //                fold (h0 h1 h2)    call              raise
        let strat = [0.1f32, 0.0, 0.3, 0.6, 1.0, 0.3, 0.3, 0.0, 0.4];
        let evs = [-0.0f32, 0.0, 0.0, 12.5, -40.0, 7.0, 30.0, -1.0, 2.0];
        let m = combo_major(&hands, &reach, &strat, &evs, &actions).unwrap();
        let (aks, sevens, deuces) = (combo("As", "Ks"), combo("7h", "7d"), combo("3c", "2c"));
        assert_eq!(m.probs[aks], vec![0.1, 0.6, 0.3]);
        assert_eq!(m.probs[sevens], vec![0.0, 1.0, 0.0]);
        assert_eq!(m.ev_chips[aks][1..], [12.5, 30.0]);
        assert_eq!(m.ev_chips[aks][0].to_bits(), 0.0f32.to_bits(), "a -0.0 fold value is emitted as +0.0");
        assert_eq!(m.ev_chips[sevens], vec![0.0, -40.0, -1.0]);
        assert!(m.available[aks] && m.available[sevens]);
        for c in (0..1326).filter(|c| *c != aks && *c != sevens) {
            assert!(!m.available[c] && m.probs[c].iter().chain(&m.ev_chips[c]).all(|x| x.to_bits() == 0), "combo {c} (deuces {deuces}) is absent");
        }
        let refuse = |reach: &[f32], strat: &[f32], evs: &[f32], needle: &str| {
            let e = combo_major(&hands, reach, strat, evs, &actions).unwrap_err();
            assert!(e.contains(needle), "{needle:?} in {e}");
        };
        let mut fold = evs;
        fold[0] = 1.5;
        refuse(&reach, &strat, &fold, "fold");
        let mut nan = evs;
        nan[3] = f32::NAN;
        refuse(&reach, &strat, &nan, "non-finite EV");
        let mut over = strat;
        over[3] = 1.5;
        refuse(&reach, &over, &evs, "outside [0, 1]");
        let mut negative = strat;
        negative[6] = -0.25;
        refuse(&reach, &negative, &evs, "outside [0, 1]");
        refuse(&[1.0, f32::NAN, 0.0], &strat, &evs, "reach");
        refuse(&reach, &strat[..8], &evs, "shape");
        refuse(&reach[..2], &strat, &evs, "shape");
    }

    /// `available` is reach > 0 (§4.5), not range membership: with OOP's turn root locked to bet 66
    /// with AsAh and check with 8s7s, OOP's node after bet/raise holds AsAh only and its node after
    /// check/bet holds 8s7s only; the unreached combo's rows are all zero.
    #[test]
    fn availability_follows_reach_not_range_membership() {
        let req = turn_request();
        let (aces, eight_seven) = (combo("As", "Ah"), combo("8s", "7s"));
        let mut probs = vec![vec![0.0f32; 3]; 1326];
        probs[aces] = vec![0.0, 1.0, 0.0];
        probs[eight_seven] = vec![1.0, 0.0, 0.0];
        let lock = proto::worker::NodeLock { path: vec![], actor: "oop".into(), probs };
        let mut game = allocated(&req);
        crate::locks::apply(&mut game, &lock, &req.tree.materialized).unwrap();
        postflop_solver::solve(&mut game, 20, 0.0, false);
        let sol = street_solution(&mut game, &req, meta(), &no_cancel()).unwrap().unwrap();
        validate_solution(&sol, &req.tree.materialized).unwrap();
        let at = |path: &[Action]| sol.nodes.iter().find(|n| n.path == path).unwrap_or_else(|| panic!("{path:?} is exported"));
        let root = at(&[]);
        assert!(root.available[aces] && root.available[eight_seven]);
        let after_bet = at(&[Action::Bet { to: 66 }, Action::Raise { to: 165 }]);
        assert!(after_bet.available[aces] && !after_bet.available[eight_seven]);
        assert!(after_bet.probs[eight_seven].iter().chain(&after_bet.ev_chips[eight_seven]).all(|x| *x == 0.0));
        let after_check = at(&[CHECK, Action::Bet { to: 66 }]);
        assert!(!after_check.available[aces] && after_check.available[eight_seven]);
    }

    /// The §4.5 river wire example (`river_two_combo`, the §13.2 polarized-versus-bluffcatcher spot):
    /// every decision node of the street in materialized order, the requested node the one `history`
    /// reaches, the metadata copied through, and matrices `validate_solution` accepts.
    #[test]
    fn river_street_export_is_complete_and_valid() {
        let req = solve_request("river_two_combo", 0);
        let (mut game, expl) = solved(&req, 2000, 0.05);
        assert!(expl <= 0.1, "exploitability {expl}");
        let sol = street_solution(&mut game, &req, meta(), &no_cancel()).unwrap().expect("not cancelled");
        assert!(game.history().is_empty(), "extraction leaves the game at the root");
        assert_eq!((sol.nodes.len(), sol.requested, sol.export.as_str()), (3, 1, "street"));
        assert_eq!(sol.covered_paths, vec![vec![], vec![CHECK], vec![CHECK, JAM]]);
        assert_eq!((sol.exploitability_chips, sol.iterations, sol.memory_bytes, sol.mode.as_str(), sol.locks_applied), (0.09, 7, 4096, "f32", 0));
        assert_eq!(validate_solution(&sol, &req.tree.materialized).unwrap(), vec![vec![], vec![0], vec![0, 1]]);
        let actors: Vec<&str> = sol.nodes.iter().map(|n| n.actor.as_str()).collect();
        assert_eq!(actors, ["oop", "ip", "oop"]);
        // available = reach > 0: nobody's reach changes on this line (OOP's root has one action)
        for c in 0..1326 {
            assert_eq!(sol.nodes[0].available[c], req.oop_range.0[c] > 0.0, "root combo {c}");
            assert_eq!(sol.nodes[1].available[c], req.ip_range.0[c] > 0.0, "ip combo {c}");
            assert_eq!(sol.nodes[2].available[c], req.oop_range.0[c] > 0.0, "facing combo {c}");
        }
        // the analytic equilibrium (§13.2): IP bets QQ always and 54o half the time, OOP calls half the time
        let ip = &sol.nodes[1];
        for qq in [combo("Qc", "Qd"), combo("Qc", "Qh"), combo("Qd", "Qh")] { assert!(ip.probs[qq][1] > 0.97, "QQ bets: {:?}", ip.probs[qq]); }
        let bluff = ip.probs[combo("5c", "4d")][1];
        assert!((bluff - 0.5).abs() <= 0.03, "54o bluffs {bluff}");
        let call = sol.nodes[2].probs[combo("Ac", "Ad")][1];
        assert!((call - 0.5).abs() <= 0.03, "AA calls {call}");
        // `equity * pot` at a check (§13.2): QQ always wins the 100-chip pot, 54o never does
        assert!((ip.ev_chips[combo("Qc", "Qd")][0] - 100.0).abs() <= 1e-3, "QQ check EV {}", ip.ev_chips[combo("Qc", "Qd")][0]);
        assert!(ip.ev_chips[combo("5c", "4d")][0].abs() <= 1e-3, "54o check EV {}", ip.ev_chips[combo("5c", "4d")][0]);
    }

    /// §10.3 matrix contract: the library's action-major arrays over its compact hand list, read
    /// independently here, land at `[combo][action]` bit for bit; the fold column is exactly +0.0
    /// (§2: chips already in the pot are sunk) and every other EV is the library's own, unshifted.
    #[test]
    fn matrices_are_the_library_arrays_transposed_to_named_combos() {
        let req = solve_request("river_two_combo", 0);
        let (mut game, _) = solved(&req, 300, 0.0);
        let sol = street_solution(&mut game, &req, meta(), &no_cancel()).unwrap().unwrap();
        for (k, indices) in [(1usize, vec![0usize]), (2, vec![0, 1])] {
            game.apply_history(&indices);
            game.cache_normalized_weights();
            let player = game.current_player();
            let hands = game.private_cards(player).to_vec();
            let (strat, evs) = (game.strategy(), game.expected_values_detail(player));
            let node = &sol.nodes[k];
            let n = hands.len();
            for (h, hand) in hands.iter().enumerate() {
                let c = crate::cards::lib_hand_to_combo(*hand) as usize;
                for (a, action) in node.actions.iter().enumerate() {
                    assert_eq!(node.probs[c][a].to_bits(), strat[a * n + h].to_bits(), "node {k} combo {c} action {a}");
                    if *action == Action::Fold {
                        assert_eq!(node.ev_chips[c][a].to_bits(), 0.0f32.to_bits(), "fold EV is exactly +0.0");
                    } else {
                        assert_eq!(node.ev_chips[c][a].to_bits(), evs[a * n + h].to_bits(), "node {k} combo {c} action {a}");
                    }
                }
            }
            game.back_to_root();
        }
        assert_eq!(sol.nodes[2].actions, vec![Action::Fold, Action::Call]);
    }

    /// §4.5 `cancel` in `Extracting`: answered at the next node boundary, as `Ok(None)`, with the game
    /// back at the root.
    #[test]
    fn a_cancel_between_nodes_yields_none() {
        let req = solve_request("river_two_combo", 0);
        let (mut game, _) = solved(&req, 50, 0.0);
        assert!(street_solution(&mut game, &req, meta(), &AtomicBool::new(true)).unwrap().is_none());
        assert!(game.history().is_empty());
        // the checkpoint comes before each node: with the flag already set, not even a game that could
        // not be extracted (never finalized) is touched
        let mut unsolved = allocated(&req);
        assert!(street_solution(&mut unsolved, &req, meta(), &AtomicBool::new(true)).unwrap().is_none());
    }

    /// §4.5 limits: an export over the node limit or the result-line limit carries the requested node
    /// only, `export: "truncated"` and `covered_paths` naming exactly that node; the byte limit is
    /// inclusive; a requested node that does not fit on its own is an error, never a silent drop.
    #[test]
    fn over_limit_exports_carry_the_requested_node_only() {
        let req = solve_request("river_two_combo", 0);
        let (mut game, _) = solved(&req, 50, 0.0);
        let full = street_solution(&mut game, &req, meta(), &no_cancel()).unwrap().unwrap();
        let (full_len, full) = result_line_len(full, &req.id);
        let full_len = full_len.unwrap();

        let by_count = export(&mut game, &req, meta(), &no_cancel(), Limits { max_nodes: 2, max_line: RESULT_LINE_MAX }).unwrap().unwrap();
        assert_eq!((by_count.nodes.len(), by_count.requested, by_count.export.as_str()), (1, 0, "truncated"));
        assert_eq!(by_count.covered_paths, vec![req.history.clone()]);
        assert_eq!(by_count.nodes[0], full.nodes[1]);
        assert_eq!(validate_solution(&by_count, &req.tree.materialized).unwrap(), vec![vec![0]]);

        let at = export(&mut game, &req, meta(), &no_cancel(), Limits { max_nodes: MAX_EXPORTED_NODES, max_line: full_len }).unwrap().unwrap();
        assert_eq!(at, full, "a line exactly at the limit is exported whole");
        let by_bytes = export(&mut game, &req, meta(), &no_cancel(), Limits { max_nodes: MAX_EXPORTED_NODES, max_line: full_len - 1 }).unwrap().unwrap();
        assert_eq!(by_bytes, by_count, "one byte over: the requested node only");

        let (one_len, _) = result_line_len(by_count, &req.id);
        let tight = Limits { max_nodes: MAX_EXPORTED_NODES, max_line: one_len.unwrap() - 1 };
        let e = export(&mut game, &req, meta(), &no_cancel(), tight).unwrap_err();
        assert!(e.contains("requested node"), "{e}");
        assert!(game.history().is_empty());
    }

    /// The line is sized before `status` and `elapsed_ms` are known, so it is measured at its longest
    /// (`best_so_far`, a ten-digit `elapsed_ms`) plus the terminating newline; measuring hands the
    /// solution back untouched, and a solution that cannot be serialized is an error.
    #[test]
    fn line_length_is_measured_at_the_longest_result_line() {
        let req = solve_request("river_two_combo", 0);
        let (mut game, _) = solved(&req, 50, 0.0);
        let sol = street_solution(&mut game, &req, meta(), &no_cancel()).unwrap().unwrap();
        let (len, back) = result_line_len(sol.clone(), &req.id);
        assert_eq!(back, sol);
        let line = |status, elapsed_ms| serde_json::to_vec(&WorkerMessage::Result { id: req.id.clone(), status, elapsed_ms, solution: Some(sol.clone()), error: None }).unwrap().len();
        let (longest, shortest) = (line(ResultStatus::BestSoFar, u32::MAX), line(ResultStatus::Ok, 0));
        assert_eq!(len.unwrap(), longest + 1);
        assert_eq!(longest - shortest, "best_so_far".len() - "ok".len() + "4294967295".len() - "0".len());

        let mut nan = meta();
        nan.exploitability_chips = f32::NAN;
        let e = street_solution(&mut game, &req, nan, &no_cancel()).unwrap_err();
        assert!(e.contains("serialize"), "{e}");
    }

    /// `extract_node` reads only the node the skeleton names: an unsolved game, a different actor, a
    /// different menu, a terminal and an unavailable action are errors (never a library panic), and the
    /// game is back at the root after each.
    #[test]
    fn extract_node_refuses_anything_the_skeleton_does_not_name() {
        let req = solve_request("river_two_combo", 0);
        let ip = req.tree.materialized.iter().find(|n| n.path == [0]).unwrap();

        let mut unsolved = allocated(&req);
        let e = extract_node(&mut unsolved, ip, &[CHECK]).unwrap_err();
        assert!(e.contains("finalize"), "{e}");

        let (mut game, _) = solved(&req, 50, 0.0);
        let good = extract_node(&mut game, ip, &[CHECK]).unwrap();
        assert_eq!((good.path.as_slice(), good.actor.as_str()), (&[CHECK][..], "ip"));
        let mut wrong_actor = ip.clone();
        wrong_actor.actor = "oop".into();
        let mut wrong_menu = ip.clone();
        wrong_menu.actions = vec![CHECK, Action::Bet { to: 50 }];
        for (node, path, needle) in [
            (&wrong_actor, vec![CHECK], "actor"),
            (&wrong_menu, vec![CHECK], "menu"),
            (ip, vec![CHECK, CHECK], "not a decision node"),       // check-check ends the river
            (ip, vec![Action::Bet { to: 50 }], "not available"),  // OOP cannot bet in this tree
        ] {
            let e = extract_node(&mut game, node, &path).unwrap_err();
            assert!(e.contains(needle), "{needle:?} in {e}");
            assert!(game.history().is_empty(), "back at the root after {e}");
        }
    }

    /// A turn-rooted solve exports the turn's decision nodes and none of the river's, and every facing
    /// node's fold column is exactly +0.0 for every available combo.
    #[test]
    fn a_turn_export_carries_the_turn_nodes_only() {
        let req = turn_request();
        let (mut game, _) = solved(&req, 20, 0.0);
        let sol = street_solution(&mut game, &req, meta(), &no_cancel()).unwrap().unwrap();
        let turn: Vec<Vec<u8>> = req.tree.materialized.iter().filter(|n| n.street == Street::Turn).map(|n| n.path.clone()).collect();
        assert_eq!(turn.len(), 20);
        assert!(req.tree.materialized.iter().any(|n| n.street == Street::River), "the tree reaches the river");
        assert_eq!((sol.nodes.len(), sol.requested, sol.export.as_str()), (20, 0, "street"));
        assert_eq!(validate_solution(&sol, &req.tree.materialized).unwrap(), turn);
        let mut folds = 0;
        for node in &sol.nodes {
            for (a, action) in node.actions.iter().enumerate().filter(|(_, x)| **x == Action::Fold) {
                for c in (0..1326).filter(|&c| node.available[c]) {
                    assert_eq!(node.ev_chips[c][a].to_bits(), 0.0f32.to_bits(), "{:?} {action:?} combo {c}", node.path);
                    folds += 1;
                }
            }
        }
        assert!(folds > 0, "the turn tree has facing nodes with a fold");
    }
}
