//! V1 (spec §13.0, §13.2 `pinned_example_fixture`; plan 2 Task 15): the pinned regression fixture. The library's own
//! `solve()` on the vendored `examples/basic.rs` ranges and board (turn root `Td 9d 6h Qc`), under `turn_std_v1` at pot
//! 200 / effective stack 900 (the committed `basic_turn_std` materialization case), to 30 bp (0.6 chips), exported
//! through the worker's own adapter (`extract::street_solution`) exactly as the worker would export it. Writes
//!
//! - `fixtures/solver/basic_0p3.json`: the solution (`proto::worker::StreetSolution`), and
//! - `fixtures/worker/basic_turn_std_request.jsonl`: the `solve` request line that reproduces it through the worker.
//!
//! Both are committed data, regenerated only by this program (`cargo run --release -p solver-worker --example
//! gen_basic_fixture`); a regeneration must compare equal byte for byte (serde's fixed field order, LF, one trailing
//! newline, no clock or thread count in the output). The library's arithmetic is independent of the rayon thread
//! count (its parallel loops write disjoint per-child slices that are then summed in a fixed order), which the
//! report of Task 15 verifies by regenerating at 1 thread, 4 threads and the default.
use postflop_solver::{compute_exploitability, finalize, solve, solve_step, CardConfig, PostFlopGame, Range};
use proto::worker::{validate_solution, EngineMessage, SolveRequest, StreetSolution};
use proto::{Card, EffectiveTree, Range1326};
use solver_worker::solve_loop::meets_target;
use solver_worker::{cards, extract, memory, protocol, tree_build};
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// `examples/basic.rs` at the pinned commit, verbatim.
const OOP_RANGE: &str = "66+,A8s+,A5s-A4s,AJo+,K9s+,KQo,QTs+,JTs,96s+,85s+,75s+,65s,54s";
const IP_RANGE: &str = "QQ-22,AQs-A2s,ATo+,K5s+,KJo+,Q8s+,J8s+,T7s+,96s+,86s+,75s+,64s+,53s+";
const BOARD: [&str; 4] = ["Td", "9d", "6h", "Qc"];
/// `solve()`'s iteration budget. Reaching it before the target is an error, never a fixture.
const MAX_ITERATIONS: u32 = 1000;

fn to_range1326(r: &Range) -> Range1326 {
    let mut out = Range1326([0.0; 1326]);
    let (hands, weights) = r.get_hands_weights(0);
    for (h, w) in hands.iter().zip(weights) { out.0[usize::from(cards::lib_hand_to_combo(*h))] = w; }
    out
}

/// The request's game, built the way `job::run_with` builds it: tree (with its cross-check against the materialized
/// skeleton, so the worker will accept the committed request), card config, memory admission, allocation.
fn build_game(req: &SolveRequest) -> (PostFlopGame, memory::Admission) {
    protocol::precheck(req).expect("the request passes the worker's precheck");
    let eff = req.stack_oop.min(req.stack_ip);
    let mut tree = tree_build::build(&req.tree, req.pot, eff, req.rake_rate, req.rake_cap_mchips, &req.history).expect("tree builds");
    let lib = tree_build::enumerate(&mut tree, req.tree.root_street, req.pot).expect("tree enumerates");
    tree_build::cross_check(&lib, &req.tree.materialized).expect("library tree equals the materialized skeleton");
    let (flop, turn, river) = cards::board_to_lib(&req.board).expect("board");
    let range = [cards::range_to_lib(&req.oop_range).expect("oop range"), cards::range_to_lib(&req.ip_range).expect("ip range")];
    let mut game = PostFlopGame::with_config(CardConfig { range, flop, turn, river }, tree).expect("game config");
    let (f32_bytes, i16_bytes) = game.memory_usage();
    let adm = memory::admit(f32_bytes, i16_bytes, req.memory_limit_bytes).expect("memory admission");
    assert_eq!(adm.mode, "f32", "the pinned fixture is an f32 solve");
    game.allocate_memory(adm.compressed);
    (game, adm)
}

/// The number of iterations `solve()` ran, which it does not return: its loop replayed on a second, identically built
/// game (the same `solve_step` calls, the same measurement cadence and stop rule, then `finalize`). The caller requires
/// the replay to end on the same exploitability bits and to export the same solution, so the count is the count of
/// the `solve()` that produced the fixture.
fn replay_solve(game: &mut PostFlopGame, max_iterations: u32, target: f32) -> (u32, f32) {
    let mut exploitability = compute_exploitability(game);
    let mut iterations = 0;
    for t in 0..max_iterations {
        if exploitability <= target { break; }
        solve_step(game, t);
        iterations = t + 1;
        if (t + 1) % 10 == 0 || t + 1 == max_iterations { exploitability = compute_exploitability(game); }
    }
    finalize(game);
    (iterations, exploitability)
}

/// The `f32` stopping threshold that makes `solve()`'s own comparison (`exploitability <= target`, in `f32`) the
/// worker's (follow-up P2.W1): the largest `f32` that meets the raw target by the §7 loop's predicate
/// (`solve_loop::meets_target`, spec 4.4), so every `f32` measurement is at or below it exactly when it meets the raw
/// target. The quotient narrowed to `f32` can lie above the raw target (0.6f32 for 30 bp of 200 chips); the `f32` below
/// it then is the edge.
fn target_edge(pot: u32, target_bp: u16) -> f32 {
    let rounded = (f64::from(pot) * f64::from(target_bp) / 10_000.0) as f32;
    let edge = if meets_target(rounded, pot, target_bp) { rounded } else { f32::from_bits(rounded.to_bits() - 1) };
    assert!(meets_target(edge, pot, target_bp) && !meets_target(f32::from_bits(edge.to_bits() + 1), pot, target_bp), "{target_bp} bp of {pot}: edge {edge:e}");
    edge
}

fn export(game: &mut PostFlopGame, req: &SolveRequest, meta: extract::SolutionMeta) -> StreetSolution {
    extract::street_solution(game, req, meta, &AtomicBool::new(false)).expect("street export").expect("never cancelled")
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let cases = std::fs::read_to_string(root.join("fixtures/worker/materialization_cases.jsonl")).expect("fixtures/worker/materialization_cases.jsonl");
    let case: serde_json::Value = cases.lines().map(|l| serde_json::from_str::<serde_json::Value>(l).expect("case line"))
        .find(|c| c["case"] == "basic_turn_std").expect("basic_turn_std case");
    assert_eq!((case["pot"].as_u64(), case["eff"].as_u64(), case["history"].as_array().map(Vec::len)), (Some(200), Some(900), Some(0)), "basic_turn_std is turn_std_v1 at 200 / 900 from the turn root");
    let tree: EffectiveTree = serde_json::from_value(case["tree"].clone()).expect("case tree");
    let oop: Range = OOP_RANGE.parse().expect("oop range string");
    let ip: Range = IP_RANGE.parse().expect("ip range string");
    let board: Vec<Card> = BOARD.iter().map(|s| Card::parse(s).expect("board card")).collect();
    let (mut oop_v, mut ip_v) = (to_range1326(&oop), to_range1326(&ip));
    for i in 0..1326 {
        let [a, b] = proto::combo_cards(i as u16);
        if board.contains(&a) || board.contains(&b) { oop_v.0[i] = 0.0; ip_v.0[i] = 0.0; }
    }
    let req = SolveRequest { id: "basic".into(), spot: "b".repeat(64), board, oop_range: oop_v, ip_range: ip_v, pot: 200, stack_oop: 900, stack_ip: 900,
        rake_rate: 0.0, rake_cap_mchips: 0, tree, history: vec![], target_bp: 30, deadline_ms: 20000, extraction_margin_ms: 200, memory_limit_bytes: 10 << 30, background: false };
    // The worker's own target comparison (`solve_loop::meets_target`): 30 bp of the 200 pot = 0.6 chips, met by every
    // `f32` up to 0.59999996f32 and missed by 0.6f32 (0.60000002384185791015625).
    let target = target_edge(req.pot, req.target_bp);

    // V1: the library's own `solve()` (it finalizes).
    let (mut game, adm) = build_game(&req);
    let raw = solve(&mut game, MAX_ITERATIONS, target, false);
    let (mut replay, _) = build_game(&req);
    let (iterations, replay_raw) = replay_solve(&mut replay, MAX_ITERATIONS, target);
    assert_eq!(raw.to_bits(), replay_raw.to_bits(), "the step-wise replay must end where solve() ended: {raw} vs {replay_raw}");
    assert!(meets_target(raw, req.pot, req.target_bp), "solve() stopped at {raw} chips after {iterations} iterations, missing the {}-bp target of the {}-chip pot", req.target_bp, req.pot);
    // The worker's reporting policy (constraints: exploitability noise floor) applies to the fixture as to any result.
    let reported = extract::report_exploitability(raw, req.pot).expect("reportable exploitability");
    if let Some(line) = &reported.log_line { eprintln!("{line}"); }
    let meta = extract::SolutionMeta { exploitability_chips: reported.chips, iterations, memory_bytes: adm.estimate_bytes, mode: adm.mode, locks_applied: 0 };
    let sol = export(&mut game, &req, meta);
    assert_eq!(sol, export(&mut replay, &req, meta), "the replay's export differs from solve()'s");
    validate_solution(&sol, &req.tree.materialized).expect("the fixture passes the worker's self-validation");
    let street_nodes = req.tree.materialized.iter().filter(|n| n.street == req.tree.root_street).count();
    assert_eq!((sol.export.as_str(), sol.nodes.len()), ("street", street_nodes), "the fixture is the whole street, never truncated");

    std::fs::create_dir_all(root.join("fixtures/solver")).expect("fixtures/solver");
    std::fs::write(root.join("fixtures/solver/basic_0p3.json"), format!("{}\n", serde_json::to_string(&sol).expect("solution serializes"))).expect("write basic_0p3.json");
    std::fs::write(root.join("fixtures/worker/basic_turn_std_request.jsonl"), format!("{}\n", serde_json::to_string(&EngineMessage::Solve(req)).expect("request serializes"))).expect("write basic_turn_std_request.jsonl");
    println!("exploitability {:.4} chips (raw {raw:e}), {iterations} iterations, {} nodes, {} bytes {}", reported.chips, sol.nodes.len(), sol.memory_bytes, sol.mode);
}
