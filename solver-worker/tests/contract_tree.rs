//! §13.2 solver-contract tests of the worker's tree handling and of its street exports (plan 2 Task 16; fix round 1
//! per review P2T16R-I1..I5). Every assertion that needs solver progress runs in process, through the job runner's
//! seam, on a fixed solve schedule (`common::run_fixed`: a fixed count of real `solve_step`s), so what is checked
//! never depends on how fast the machine solves. The spawned-worker runs are the wire path: exact where the outcome is
//! machine-independent (the tree cross-check; a stop rule that admits no iteration at any elapsed time), and
//! conditional smoke checks under a liveness bound where it is not (how far a clock-driven §7 solve gets).
mod common;
use common::{fixture_lines, run_fixed, started, Barrier, Harness, Site, Worker, LIVENESS};
use proto::worker::{EngineMessage, ResultStatus, SolveRequest, StreetSolution, WorkerMessage};
use proto::{combo_cards, combo_index, Action, Card, MaterializedNode, Range1326, Street, COMBOS};
use serde_json::{json, Value};
use solver_worker::extract::chip_path_of;
use solver_worker::job::{Checkpoint, JobOutcome, Op};
use solver_worker::solve_loop::LoopSite;
use std::collections::HashSet;
use std::time::{Duration, Instant};

const S: Duration = Duration::from_secs(1);
fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn ready(w: &Worker) { assert_eq!(w.recv(5 * S).unwrap()["type"], "ready"); }
fn cases() -> Vec<Value> { fixture_lines("materialization_cases").iter().map(|l| serde_json::from_str(l).unwrap()).collect() }
fn case(name: &str) -> Value { cases().into_iter().find(|c| c["case"] == name).unwrap() }
fn solve_of(line: &str) -> SolveRequest {
    match serde_json::from_str::<EngineMessage>(line).expect("a solve line") { EngineMessage::Solve(r) => r, other => panic!("not a solve: {other:?}") }
}
fn solution_of(r: &Value) -> StreetSolution { serde_json::from_value(r["solution"].clone()).unwrap_or_else(|e| panic!("no wire solution in {}: {e}", r["id"])) }

/// Every message of one wire exchange, in order, up to and including job `id`'s terminal, within the liveness bound.
fn exchange(w: &Worker, id: &str) -> Vec<Value> {
    let mut got = Vec::new();
    let end = Instant::now() + LIVENESS;
    loop {
        let v = w.recv(end.saturating_duration_since(Instant::now())).unwrap_or_else(|| panic!("no result for {id} after {:?}", briefs(&got)));
        let done = v["type"] == "result" && v["id"] == id;
        got.push(v);
        if done { return got; }
    }
}
/// A message as one comparable line: `ack <id> <status>`, `progress <id> <stage> it=<n> expl=<x> mem=0` (or `mem>0`)
/// or `result <id> <status> <error code or ->`.
fn brief(v: &Value) -> String {
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    match s("type").as_str() {
        "ack" => format!("ack {} {}", s("id"), s("status")),
        "progress" => format!("progress {} {} it={} expl={} {}", s("id"), s("stage"), v["iterations"], v["exploitability_chips"], if v["memory_bytes"] == 0 { "mem=0" } else { "mem>0" }),
        "result" => format!("result {} {} {}", s("id"), s("status"), v["error"]["code"].as_str().unwrap_or("-")),
        other => panic!("unexpected message type {other:?}: {v}"),
    }
}
fn briefs(vs: &[Value]) -> Vec<String> { vs.iter().map(brief).collect() }

// ---- Structure before numbers (review P2T16R-I3) ----

/// The structural preconditions of any numeric comparison, verified against the request's own tree, never against
/// another output: `sol` exports exactly the root street's decision nodes in materialized order, each with its chip
/// path, actor and exact ordered menu; `requested` is the node `req.history` names and `covered_paths` are the node
/// paths; `export` is `street`; every matrix has 1326 rows and every row is exactly as wide as its node's menu.
fn assert_street_structure(what: &str, req: &SolveRequest, sol: &StreetSolution) {
    let m = &req.tree.materialized;
    let street: Vec<&MaterializedNode> = m.iter().filter(|n| n.street == req.tree.root_street).collect();
    assert!(!street.is_empty(), "{what}: the request's tree has no {:?} node", req.tree.root_street);
    assert_eq!((sol.nodes.len(), sol.export.as_str()), (street.len(), "street"), "{what}: every decision node of the street");
    let paths: Vec<Vec<Action>> = street.iter().map(|n| chip_path_of(m, &n.path).unwrap_or_else(|| panic!("{what}: node {:?} has no chip path", n.path))).collect();
    let requested = paths.iter().position(|p| *p == req.history).unwrap_or_else(|| panic!("{what}: history {:?} names no node of the street", req.history));
    assert_eq!(sol.requested as usize, requested, "{what}: the requested node");
    assert_eq!(sol.covered_paths, paths, "{what}: covered paths");
    for (k, (n, e)) in sol.nodes.iter().zip(&street).enumerate() {
        assert_eq!((&n.path, n.actor.as_str(), &n.actions), (&paths[k], e.actor.as_str(), &e.actions), "{what}: node {k}'s identity, actor and menu");
        assert_eq!((n.probs.len(), n.ev_chips.len(), n.available.len()), (COMBOS, COMBOS, COMBOS), "{what}: node {k}'s rows");
        for c in 0..COMBOS {
            assert_eq!((n.probs[c].len(), n.ev_chips[c].len()), (e.actions.len(), e.actions.len()), "{what}: node {k} combo {c}'s row width");
        }
    }
}

/// The largest differences one comparison found, over `entries` probability (and EV) entries.
#[derive(Debug, Default, Clone, Copy)]
struct Deltas { probs: f64, ev: f64, entries: usize }

/// The one comparison of two exports of a street, shared by the metamorphic and the oracle tests (review P2T16R-I3).
/// Structure first: each export is checked against its own request's tree (`assert_street_structure`), then the two
/// against each other (the same nodes, actors, ordered menus, requested node and covered paths). Numbers after, over
/// the expected export's verified dimensions, never the actual's: `combo` maps an expected combo to the actual
/// export's (the identity for the oracle, the suit permutation for the metamorphism); availability must agree there
/// exactly, every probability within `prob_bound` and, when given, every EV within `ev_bound` chips.
fn compare_exports(what: &str, (expected_req, expected): (&SolveRequest, &StreetSolution), (actual_req, actual): (&SolveRequest, &StreetSolution), combo: impl Fn(usize) -> usize, prob_bound: f64, ev_bound: Option<f64>) -> Deltas {
    assert_street_structure(&format!("{what} (expected)"), expected_req, expected);
    assert_street_structure(&format!("{what} (actual)"), actual_req, actual);
    assert_eq!((actual.nodes.len(), actual.requested, &actual.covered_paths, &actual.export), (expected.nodes.len(), expected.requested, &expected.covered_paths, &expected.export), "{what}: the same street export");
    let mut d = Deltas::default();
    for (k, e) in expected.nodes.iter().enumerate() {
        let a = &actual.nodes[k];
        assert_eq!((&a.path, &a.actor, &a.actions), (&e.path, &e.actor, &e.actions), "{what}: node {k}");
        for c in 0..COMBOS {
            let j = combo(c);
            assert_eq!(a.available[j], e.available[c], "{what}: node {k} {:?}, availability of combo {c} (actual combo {j})", e.path);
            for x in 0..e.actions.len() {
                let dp = (f64::from(a.probs[j][x]) - f64::from(e.probs[c][x])).abs();
                assert!(dp <= prob_bound, "{what}: node {k} {:?} combo {c} action {x}: probability {} against {}", e.path, a.probs[j][x], e.probs[c][x]);
                d.probs = d.probs.max(dp);
                if let Some(bound) = ev_bound {
                    let dv = (f64::from(a.ev_chips[j][x]) - f64::from(e.ev_chips[c][x])).abs();
                    assert!(dv <= bound, "{what}: node {k} {:?} combo {c} action {x}: EV {} against {} chips", e.path, a.ev_chips[j][x], e.ev_chips[c][x]);
                    d.ev = d.ev.max(dv);
                }
                d.entries += 1;
            }
        }
    }
    d
}

/// `req` through the production job on a fixed schedule of `steps` real iterations (`common::run_fixed`) on a pool of
/// `threads` threads; the job's operations are the production sequence and the schedule ran exactly `steps` steps.
fn fixed_solution(req: &SolveRequest, threads: usize, steps: u32) -> StreetSolution {
    let mut hooks = Barrier::default();
    let run = run_fixed(req, threads, steps, &mut hooks);
    let sol = match run.outcome { JobOutcome::Ok(s) | JobOutcome::BestSoFar(s) => s, other => panic!("{}: {other:?}", req.id) };
    let sites = hooks.passed();
    use Op::*;
    assert_eq!(started(&sites), [TreeBuild, TreeCheck, GameConfig, MemoryCheck, Allocate, Solve, Finalize, Export, Validate], "{}: the production job", req.id);
    assert_eq!(sites.iter().filter(|s| matches!(s, Site::Loop(LoopSite::Iteration(_)))).count(), steps as usize, "{}", req.id);
    assert_eq!(sol.iterations, steps, "{}", req.id);
    sol
}

// ---- tree_materialization_matches_library (§13.2 (b), the wire cases; (a) is Task 8's in-process equality) ----

#[derive(Debug, Clone, Copy, PartialEq)]
enum Expect { TreeMismatch, NoIteration }

/// The five wire requests: 100 and 101 alter `river_two_combo`'s materialized list (a terminal pot; a missing terminal
/// marker), 102 is `facing_350_full` with the per-street-reset menu at its turn root, 103 and 104 are the unaltered
/// `facing_350_full` tree (104 with the root street's `donk: null` explicit, which §4.6 makes legal). 103 and 104
/// carry an extraction margin above their deadline, so §7's stop rule (`should_stop`: `margin > deadline` alone
/// stops, at any elapsed time) declines the first iteration on any machine: accepted, then `no_iteration` after a
/// successful build and cross-check (review P2T16R-I1), never a status that depends on how fast the solver runs.
fn materialization_requests() -> Vec<(&'static str, String, Expect)> {
    let river = &fixture_lines("river_two_combo")[0];
    let flop = &fixture_lines("flop_cancel")[0];
    let full = case("facing_350_full");
    // `facing_test_v1` is flop-rooted and its flop menu has no donk option, so 104's explicit `null` is an addition
    assert!(full["tree"]["menus"]["flop"].get("donk").is_none(), "{}", full["tree"]["menus"]["flop"]);
    let facing = |id: &str, f: &dyn Fn(&mut Value)| edit(flop, |v| {
        v["id"] = json!(id); v["pot"] = json!(100); v["stack_oop"] = json!(350); v["stack_ip"] = json!(350); v["tree"] = full["tree"].clone();
        f(v);
    });
    let no_room = |v: &mut Value| { v["deadline_ms"] = json!(300); v["extraction_margin_ms"] = json!(600); };
    vec![
        ("100", edit(river, |v| { v["id"] = json!("100"); v["tree"]["materialized"][2]["terminal_pots"][1] = json!(299); }), Expect::TreeMismatch),
        ("101", edit(river, |v| { v["id"] = json!("101"); v["tree"]["materialized"][1]["terminal_pots"][0] = Value::Null; }), Expect::TreeMismatch),
        ("102", facing("102", &|v: &mut Value| {                                    // the per-street reset menu at the turn root
            let turn = v["tree"]["materialized"].as_array().unwrap().iter().position(|n| n["path"] == json!([1, 1])).unwrap();
            v["tree"]["materialized"][turn]["actions"] = json!([{"kind": "check"}, {"kind": "bet", "to": 100}]);
            v["tree"]["materialized"][turn]["terminal_pots"] = json!([null, null]);
        }), Expect::TreeMismatch),
        ("103", facing("103", &no_room), Expect::NoIteration),
        ("104", facing("104", &|v: &mut Value| { v["tree"]["menus"]["flop"]["donk"] = Value::Null; no_room(v); }), Expect::NoIteration),
    ]
}

/// The whole exchange, wire or in process alike: `accepted`, the Building report, then either the cross-check's
/// `tree_mismatch`, or the Building report with the admitted memory estimate (the cross-check, game configuration
/// and memory check passed), the Solving report with no iteration and `no_iteration`.
fn expected_exchange(id: &str, e: Expect) -> Vec<String> {
    let mut x = vec![format!("ack {id} accepted"), format!("progress {id} building it=0 expl=null mem=0")];
    match e {
        Expect::TreeMismatch => x.push(format!("result {id} error tree_mismatch")),
        Expect::NoIteration => x.extend([format!("progress {id} building it=0 expl=null mem>0"), format!("progress {id} solving it=0 expl=null mem>0"), format!("result {id} error no_iteration")]),
    }
    x
}

/// The job's sites, in process: the tree build and the cross-check, where a mismatch ends it; otherwise every
/// Building step completed, the §7 loop entered, and its first poll the last loop site (no solve step).
fn expected_sites(e: Expect) -> Vec<Site> {
    use Checkpoint::*;
    use Op::*;
    let mut s = vec![Site::At(Start), Site::Enter(TreeBuild), Site::Leave(TreeBuild), Site::At(TreeBuilt), Site::Enter(TreeCheck), Site::Leave(TreeCheck)];
    if e == Expect::NoIteration {
        s.extend([Site::At(TreeChecked), Site::Enter(GameConfig), Site::Leave(GameConfig), Site::At(GameConfigured), Site::Enter(MemoryCheck), Site::Leave(MemoryCheck), Site::At(MemoryChecked),
            Site::Enter(Allocate), Site::Leave(Allocate), Site::At(Allocated), Site::Enter(Solve), Site::Loop(LoopSite::Boundary(0)), Site::Leave(Solve), Site::At(SolveEnded)]);
    }
    s
}

#[test]
fn tree_materialization_matches_library() {
    let requests = materialization_requests();
    let terminal = |what: &str, r: &Value| assert_eq!((&r["error"]["retryable"], r.get("solution")), (&json!(false), None), "{what}: {r}");
    // On the wire (§13.2 (b)): every outcome here is machine-independent; the timeouts are liveness bounds only.
    let mut w = Worker::spawn(4);
    ready(&w);
    for (id, line, e) in &requests {
        w.send(line);
        let got = exchange(&w, id);
        assert_eq!(briefs(&got), expected_exchange(id, *e), "wire {id}");
        terminal(&format!("wire {id}"), got.last().unwrap());
    }
    // In process, the same five through the worker's own control and executor, the hooks recording every site: a
    // mismatch ends the job at the cross-check; 103 and 104 complete every Building step and never start a solve step.
    let mut h = Harness::new();
    for (id, line, e) in &requests {
        let from = h.barrier.passed().len();
        let got = h.exchange(line, id);
        assert_eq!(briefs(&got), expected_exchange(id, *e), "in process {id}");
        terminal(&format!("in process {id}"), got.last().unwrap());
        assert_eq!(h.barrier.passed()[from..], expected_sites(*e), "in process {id}: the job's sites");
    }
}

#[test]
fn wager_cap_remove_lines() {
    use postflop_solver::Action as L;
    use solver_worker::{history::history_to_lib, tree_build::build};
    // `cap3_three_wagers` (turn_std_v1, pot 100 / eff 1000): unlike `cap1_two_wagers`, the raw (uncapped)
    // tree never offers an all-in at the node facing OOP's third wager -- verified by rebuilding the same
    // case with `wager_cap` widened to 255 (no removal), which leaves `[Fold, Call, Raise(520)]`, no
    // `AllIn`. So the cap here has only a `Raise` to remove; the fixture's own numbers, not a loosened
    // assertion.
    for (name, expected_after_prefix) in [("cap1_two_wagers", vec![L::Fold, L::Call, L::AllIn(500)]), ("cap3_three_wagers", vec![L::Fold, L::Call])] {
        let c = case(name);
        let tree: proto::EffectiveTree = serde_json::from_value(c["tree"].clone()).unwrap();
        let history: Vec<proto::Action> = serde_json::from_value(c["history"].clone()).unwrap();
        let mut t = build(&tree, c["pot"].as_u64().unwrap() as u32, c["eff"].as_u64().unwrap() as u32, 0.0, 0, &history).unwrap();
        t.apply_history(&history_to_lib(&history)).unwrap();
        assert_eq!(t.available_actions(), &expected_after_prefix[..], "{name}");
        // The observed prefix survives the cap: at the node BEFORE each observed action, that action is still offered.
        // (Applying the whole line first would put the tree at the child AFTER the action, whose menu never contains it.)
        let mut line = history_to_lib(&history);
        while !line.is_empty() {
            let last = line.pop().unwrap();
            t.apply_history(&line).unwrap();
            assert!(t.available_actions().contains(&last), "{name}: {last:?} was removed by the cap at {line:?}");
        }
    }
}

// ---- exact_size_insertion_no_prune ----

/// Ten real iterations, one §7 measurement block: enough for OOP's root strategy to differ across its hands.
const INSERTION_STEPS: u32 = 10;

/// `flop_cancel`'s spot (179 x 264 combos on Qs Jh 2h) on the committed `insert_73` case: `flop_fast_v1` at pot 100,
/// effective stack 500, with OOP's observed 73-chip bet inserted exactly beside the menu's 50 (0.5 pot) and the
/// history at IP's node facing it. Its 4000/600 budget is the wire smoke's; the in-process schedule does not read it.
fn insertion_request(id: &str) -> (String, SolveRequest) {
    let ins = case("insert_73");
    let line = edit(&fixture_lines("flop_cancel")[0], |v| {
        v["id"] = json!(id); v["pot"] = json!(100); v["stack_oop"] = json!(500); v["stack_ip"] = json!(500);
        v["tree"] = ins["tree"].clone(); v["history"] = ins["history"].clone(); v["deadline_ms"] = json!(4000); v["extraction_margin_ms"] = json!(600);
    });
    let req = solve_of(&line);
    (line, req)
}

#[test]
fn exact_size_insertion_no_prune() {
    let (line, req) = insertion_request("110");
    let root_menu = [Action::Check, Action::Bet { to: 50 }, Action::Bet { to: 73 }];
    let after_73 = vec![Action::Bet { to: 73 }];
    // the request's own tree already holds 50 and 73 beside the check (inserted exactly, never merged or pruned)
    assert_eq!(req.tree.materialized.iter().find(|n| n.path.is_empty()).map(|n| n.actions.as_slice()), Some(&root_menu[..]));
    assert_eq!(req.history, after_73);
    let check_export = |what: &str, sol: &StreetSolution| {
        assert_street_structure(what, &req, sol);
        let root = sol.nodes.iter().find(|n| n.path.is_empty()).unwrap_or_else(|| panic!("{what}: no root node"));
        assert_eq!((root.actor.as_str(), root.actions.as_slice()), ("oop", &root_menu[..]), "{what}: the root keeps both sizes");
        let requested = &sol.nodes[sol.requested as usize];
        assert_eq!((&requested.path, requested.actor.as_str()), (&after_73, "ip"), "{what}: the requested node is IP's, facing the 73 bet");
        root.clone()
    };

    // In process, a fixed schedule of real iterations (review P2T16R-I1): OOP's Bet(73) probability varies across its
    // available root combos, so the opponent's posterior after the 73 bet is not uniform over the root range.
    let sol = fixed_solution(&req, 8, INSERTION_STEPS);
    let root = check_export("in process", &sol);
    let p73: Vec<f64> = (0..COMBOS).filter(|&c| root.available[c]).map(|c| f64::from(root.probs[c][2])).collect();
    assert!(!p73.is_empty(), "OOP has available root combos");
    let (min, max) = p73.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    assert!(max - min > 0.05, "Bet(73) is uniform after {INSERTION_STEPS} iterations: {min}..{max}");
    eprintln!("exact_size_insertion_no_prune in process: OOP's Bet(73) probability over {} available root combos spans {min}..{max}", p73.len());

    // Wire smoke: the same request through the spawned worker under its clock-driven §7 loop, whose progress is
    // machine-dependent: a solution (`ok` or `best_so_far`) carries the same structure; otherwise only `no_iteration`.
    let mut w = Worker::spawn(8);
    ready(&w);
    w.send(&line);
    let r = exchange(&w, "110").pop().unwrap();
    match r["status"].as_str() {
        Some("ok" | "best_so_far") => { check_export("wire", &solution_of(&r)); }
        Some("error") => assert_eq!(r["error"]["code"], "no_iteration", "{r}"),
        other => panic!("{other:?}: {r}"),
    }
}

// ---- suit_permutation_metamorphic ----

/// The suit permutation: clubs -> diamonds -> hearts -> spades -> clubs (suit indices c, d, h, s = 0..3, spec 4.1). A
/// 4-cycle moves every suit, so it changes every board card, the monotone board's one suit included.
const SIGMA: [u8; 4] = [1, 2, 3, 0];
fn permute_card(c: Card) -> Card { Card::new(c.rank(), SIGMA[c.suit() as usize]) }
/// The combo that combo `c` becomes under the permutation.
fn permute_combo(c: usize) -> usize { let [a, b] = combo_cards(c as u16); combo_index(permute_card(a), permute_card(b)) as usize }
/// A range carried by the permutation: each permuted combo holds its original's weight.
fn permute_range(r: &Range1326) -> Range1326 { let mut out = Range1326::zero(); for c in 0..COMBOS { out.0[permute_combo(c)] = r.0[c]; } out }

/// One weight per suit (c, d, h, s), all different: no two suits are interchangeable in either range, so the
/// permutation moves range mass between suits (review P2T16R-I2) instead of mapping a range onto itself.
const SUIT_WEIGHT: [f32; 4] = [1.0, 0.8, 0.6, 0.4];
/// OOP: AA, AK, KQ, T9, 55, 43; IP: KK, QQ, A8, 98, 76, 33 (rank indices 2..A = 0..12, higher rank first).
const OOP_CLASSES: [(u8, u8); 6] = [(12, 12), (12, 11), (11, 10), (8, 7), (3, 3), (2, 1)];
const IP_CLASSES: [(u8, u8); 6] = [(11, 11), (10, 10), (12, 6), (7, 6), (5, 4), (1, 1)];
/// A range over rank classes, each combo weighted by the product of its two suits' `SUIT_WEIGHT`s; a combo that
/// shares a card with the board is zero.
fn suit_weighted(board: &[Card], classes: &[(u8, u8)]) -> Range1326 {
    Range1326::from_fn(|i| {
        let [a, b] = combo_cards(i);
        let (hi, lo) = (a.rank().max(b.rank()), a.rank().min(b.rank()));
        if board.contains(&a) || board.contains(&b) || !classes.contains(&(hi, lo)) { 0.0 } else { SUIT_WEIGHT[a.suit() as usize] * SUIT_WEIGHT[b.suit() as usize] }
    })
}

/// A valid flop-root request (review P2T16R-I2): `flop_cancel`'s solve line on the committed `facing_350_full` tree
/// (`facing_test_v1`: flop root, pot 100, effective stack 350, 78 nodes over three streets), from the root, with the
/// given board and ranges. `target_bp` is at its maximum for the wire smoke (the first measurement ends the solve);
/// the in-process schedule does not read it.
fn flop_request(id: &str, board: &[Card], oop: &Range1326, ip: &Range1326) -> (String, SolveRequest) {
    let full = case("facing_350_full");
    let line = edit(&fixture_lines("flop_cancel")[0], |v| {
        v["id"] = json!(id); v["pot"] = json!(100); v["stack_oop"] = json!(350); v["stack_ip"] = json!(350);
        v["tree"] = full["tree"].clone(); v["history"] = json!([]); v["target_bp"] = json!(u16::MAX);
        v["board"] = json!(board.iter().map(|c| c.to_string()).collect::<Vec<_>>());
        v["oop_range"] = serde_json::to_value(oop).unwrap(); v["ip_range"] = serde_json::to_value(ip).unwrap();
    });
    let req = solve_of(&line);
    assert_eq!((req.tree.root_street, req.board.len(), req.history.len()), (Street::Flop, 3, 0), "{id}: a flop-root request");
    (line, req)
}

/// Ten real iterations, one §7 measurement block, on four pinned threads for both solves of a pair.
const METAMORPHIC_STEPS: u32 = 10;
const METAMORPHIC_THREADS: usize = 4;
/// §13.2: identical strategies after inverse mapping, max abs diff <= 1e-4.
const METAMORPHIC_BOUND: f64 = 1e-4;

#[test]
fn suit_permutation_metamorphic() {
    let mut image: Vec<usize> = (0..COMBOS).map(permute_combo).collect();
    image.sort_unstable();
    assert_eq!(image, (0..COMBOS).collect::<Vec<_>>(), "the permutation is a bijection of the 1326 combos");
    let mut w = Worker::spawn(METAMORPHIC_THREADS as u8);
    ready(&w);
    for (k, (what, cards)) in [("rainbow", ["Ks", "8d", "3c"]), ("paired", ["8s", "8d", "3c"]), ("monotone", ["Kh", "8h", "3h"])].into_iter().enumerate() {
        let board: Vec<Card> = cards.iter().map(|s| Card::parse(s).unwrap()).collect();
        let suits: HashSet<u8> = board.iter().map(|c| c.suit()).collect();
        let ranks: HashSet<u8> = board.iter().map(|c| c.rank()).collect();
        assert_eq!((suits.len(), ranks.len()), match what { "rainbow" => (3, 3), "paired" => (3, 2), _ => (1, 3) }, "{what}: {cards:?}");
        let (oop, ip) = (suit_weighted(&board, &OOP_CLASSES), suit_weighted(&board, &IP_CLASSES));
        let pboard: Vec<Card> = board.iter().map(|c| permute_card(*c)).collect();
        let (poop, pip) = (permute_range(&oop), permute_range(&ip));
        // The transformation changes what it is meant to change (review P2T16R-I2): every board card keeps its rank
        // and changes its suit (the monotone board's one suit moves), and both ranges change entry by entry.
        for (a, b) in board.iter().zip(&pboard) { assert!(a.rank() == b.rank() && a.suit() != b.suit(), "{what}: {a} -> {b}"); }
        let moved = |x: &Range1326, y: &Range1326| (0..COMBOS).filter(|&c| x.0[c] != y.0[c]).count();
        let (moved_oop, moved_ip) = (moved(&oop, &poop), moved(&ip, &pip));
        assert!(moved_oop > 0 && moved_ip > 0, "{what}: the permutation leaves a range unchanged ({moved_oop} / {moved_ip} entries moved)");
        let (id_a, id_b) = (format!("12{k}a"), format!("12{k}b"));
        let (line_a, ra) = flop_request(&id_a, &board, &oop, &ip);
        let (line_b, rb) = flop_request(&id_b, &pboard, &poop, &pip);
        assert!(ra.board != rb.board && ra.oop_range != rb.oop_range && ra.ip_range != rb.ip_range, "{what}: the permuted request differs");

        // In process: both solves on the same fixed schedule and the same pinned pool; the strategies are not trivial.
        let (sa, sb) = (fixed_solution(&ra, METAMORPHIC_THREADS, METAMORPHIC_STEPS), fixed_solution(&rb, METAMORPHIC_THREADS, METAMORPHIC_STEPS));
        let spread = sa.nodes.iter().flat_map(|n| (0..COMBOS).filter(|&c| n.available[c]).flat_map(move |c| n.probs[c].iter().map(move |p| (f64::from(*p) - 1.0 / n.actions.len() as f64).abs()))).fold(0.0f64, f64::max);
        assert!(spread > 0.1, "{what}: the solve left every strategy near uniform ({spread})");
        let d = compare_exports(&format!("{what} in process"), (&ra, &sa), (&rb, &sb), permute_combo, METAMORPHIC_BOUND, None);
        eprintln!("suit_permutation_metamorphic {what} in process: {} entries, max |dp| {:e}; {moved_oop} OOP / {moved_ip} IP range entries moved; spread {spread}", d.entries, d.probs);

        // Wire smoke: the pair through the spawned worker (the same pinned thread count) under its clock-driven §7
        // loop. When both return `ok` after the same number of iterations the exports are compared exactly as above;
        // otherwise each outcome must be one §7 allows, and nothing is compared.
        w.send(&line_a);
        let a = exchange(&w, &id_a).pop().unwrap();
        w.send(&line_b);
        let b = exchange(&w, &id_b).pop().unwrap();
        let (wa, wb) = (a["solution"]["iterations"].as_u64(), b["solution"]["iterations"].as_u64());
        if a["status"] == "ok" && b["status"] == "ok" && wa == wb {
            let d = compare_exports(&format!("{what} on the wire"), (&ra, &solution_of(&a)), (&rb, &solution_of(&b)), permute_combo, METAMORPHIC_BOUND, None);
            eprintln!("suit_permutation_metamorphic {what} on the wire: {:?} iterations, max |dp| {:e}", wa, d.probs);
        } else {
            for (r, req) in [(&a, &ra), (&b, &rb)] {
                match r["status"].as_str() {
                    Some("ok" | "best_so_far") => assert_street_structure(&format!("{what} on the wire"), req, &solution_of(r)),
                    Some("error") => assert_eq!(r["error"]["code"], "no_iteration", "{r}"),
                    other => panic!("{what}: {other:?}: {r}"),
                }
            }
        }
    }
}

// ---- pinned_example_fixture ----

/// The committed V1 oracle (P2.T15, `fixtures/solver/basic_0p3.json`): the library's own `solve()` stopped after 120
/// iterations at 0.5146258 chips, below the request's 30-bp target (0.6 chips at pot 200); 20 turn nodes, 56 actions
/// in all, so 74,256 entries in each numeric matrix.
const PINNED_ITERATIONS: u32 = 120;
const PINNED_EXPLOITABILITY: f32 = 0.5146258;
const PINNED_NODES: usize = 20;
const PINNED_ENTRIES: usize = 74_256;
/// §13.2: per-combo strategy and EV within 1e-3 (review P2T16R-I5: the spec's absolute bound, in chips for the EVs,
/// never scaled by the pot). Under the oracle's own schedule the demonstrated arithmetic difference is zero (Task 15:
/// fixed-order summation, byte-identical fixtures at 1, 4, 16 and 24 threads; review P2T16R's probe: every entry of
/// both matrices equal), so no floating-point allowance is added to either bound.
const PINNED_PROB_BOUND: f64 = 1e-3;
const PINNED_EV_BOUND_CHIPS: f64 = 1e-3;
/// Review P2T16R-I4: the reported exploitability against the oracle's, in chips. It is formed from the same EV
/// arithmetic under the same schedule (demonstrated difference zero, as above), so it is held to the same 1e-3-chip
/// bound as the EVs; an iteration-count drift of even one §7 measurement block moves it about 90 times as far (0.606
/// chips after 110 iterations, above the target, against 0.515 after 120; the test asserts the schedule too).
const PINNED_EXPLOITABILITY_BOUND_CHIPS: f64 = 1e-3;

/// The oracle checks on one export: the oracle's metadata; the shared structural and numeric comparison under the
/// pinned bounds; and a finite exploitability within its bound of the oracle's and within the request's target.
fn check_pinned(what: &str, req: &SolveRequest, oracle: &StreetSolution, sol: &StreetSolution, target: f32) -> (Deltas, f64) {
    assert_eq!((sol.iterations, sol.mode.as_str(), sol.memory_bytes, sol.locks_applied), (oracle.iterations, oracle.mode.as_str(), oracle.memory_bytes, oracle.locks_applied), "{what}: metadata");
    let d = compare_exports(what, (req, oracle), (req, sol), |c| c, PINNED_PROB_BOUND, Some(PINNED_EV_BOUND_CHIPS));
    assert_eq!(d.entries, PINNED_ENTRIES, "{what}: every entry of the oracle compared");
    let e = sol.exploitability_chips;
    assert!(e.is_finite() && e >= 0.0, "{what}: exploitability {e}");
    let de = (f64::from(e) - f64::from(oracle.exploitability_chips)).abs();
    assert!(de <= PINNED_EXPLOITABILITY_BOUND_CHIPS, "{what}: exploitability {e} chips against the oracle's {}", oracle.exploitability_chips);
    assert!(e <= target, "{what}: an `ok` export at {e} chips above the {target}-chip target");
    (d, de)
}

#[test]
fn pinned_example_fixture() {
    let line = fixture_lines("basic_turn_std_request").remove(0);
    let req = solve_of(&line);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/solver/basic_0p3.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e} (run: cargo run --release -p solver-worker --example gen_basic_fixture)", path.display()));
    let oracle: StreetSolution = serde_json::from_str(&text).expect("the V1 oracle is a street solution");
    // The job's own target arithmetic (`job::run_with`): 30 bp of the 200 pot, 0.6 chips.
    let target = (f64::from(req.pot) * f64::from(req.target_bp) / 10_000.0) as f32;
    assert_eq!((req.pot, req.target_bp, target), (200, 30, 0.6));
    assert_eq!((oracle.iterations, oracle.exploitability_chips, oracle.nodes.len(), oracle.export.as_str()), (PINNED_ITERATIONS, PINNED_EXPLOITABILITY, PINNED_NODES, "street"));
    assert!(oracle.exploitability_chips <= target, "the oracle meets its own target");

    // In process on the oracle's own schedule (reviews P2T16R-I4/I5): exactly its 120 real iterations, measured every
    // ten as §7 and `solve()` measure. Every earlier measurement is above the target and the last within it, so §7's
    // loop (and `solve()`) would stop exactly there: the run is the oracle's, not a nearby iteration count.
    let mut hooks = Barrier::default();
    let run = run_fixed(&req, 16, oracle.iterations, &mut hooks);
    assert_eq!(run.measured.iter().map(|m| m.0).collect::<Vec<_>>(), (1..=12).map(|k| 10 * k).collect::<Vec<u32>>());
    let (&(_, last), earlier) = run.measured.split_last().unwrap();
    assert!(earlier.iter().all(|&(_, e)| e > target) && last <= target, "the 30-bp target is first met at iteration 120: {:?}", run.measured);
    use Op::*;
    assert_eq!(started(&hooks.passed()), [TreeBuild, TreeCheck, GameConfig, MemoryCheck, Allocate, Solve, Finalize, Export, Validate]);
    let sol = match run.outcome { JobOutcome::Ok(s) => s, other => panic!("{other:?}") };
    let reported = sol.exploitability_chips;
    // ... through the wire codec: the result line the worker writes for it, decoded as the engine decodes it.
    let result = serde_json::to_string(&WorkerMessage::Result { id: req.id.clone(), status: ResultStatus::Ok, elapsed_ms: 0, solution: Some(sol), error: None }).unwrap();
    let sol = match serde_json::from_str::<WorkerMessage>(&result).expect("the result line decodes") { WorkerMessage::Result { solution: Some(s), .. } => s, other => panic!("{other:?}") };
    let (d, de) = check_pinned("in process", &req, &oracle, &sol, target);
    assert_eq!(reported.to_bits(), last.to_bits(), "the last measurement is reported unchanged");
    eprintln!("pinned_example_fixture in process: {} entries per matrix, max |dp| {:e}, max |dEV| {:e} chips, |d exploitability| {de:e} chips; measurements {:?}", d.entries, d.probs, d.ev, run.measured);

    // Wire smoke: the committed request, unchanged, through the spawned worker under its clock-driven §7 loop. An `ok`
    // after the oracle's 120 iterations is the oracle's schedule (a measurement does not change the game), so it is
    // held to the same checks; any other outcome is machine-dependent and must only be one §7 allows.
    let mut w = Worker::spawn(16);
    ready(&w);
    w.send(&line);
    let r = exchange(&w, "basic").pop().unwrap();
    match r["status"].as_str() {
        Some("ok") => {
            let s = solution_of(&r);
            if s.iterations == oracle.iterations {
                let (d, de) = check_pinned("wire", &req, &oracle, &s, target);
                eprintln!("pinned_example_fixture on the wire: max |dp| {:e}, max |dEV| {:e} chips, |d exploitability| {de:e} chips", d.probs, d.ev);
            } else {
                assert_street_structure("wire", &req, &s);
                assert!(s.exploitability_chips <= target, "an `ok` above the target: {}", s.exploitability_chips);
            }
        }
        Some("best_so_far") => {
            let s = solution_of(&r);
            assert_street_structure("wire", &req, &s);
            assert!(s.exploitability_chips > target, "a `best_so_far` within the target: {}", s.exploitability_chips);
        }
        Some("error") => assert_eq!(r["error"]["code"], "no_iteration", "{r}"),
        other => panic!("{other:?}: {r}"),
    }
}
