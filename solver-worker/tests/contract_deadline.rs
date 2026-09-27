//! §4.5 / §4.6 worker deadline and memory contracts through the spawned worker binary
//! (`common::Worker`): the §7 stop-point mapping to `best_so_far` / `no_iteration`, and the §10.3
//! memory admission gate. Every case below is made deterministic by a machine-independent fact
//! rather than by racing the solver's real speed against a clock -- see each test's doc comment.
mod common;
use common::{fixture_lines, Worker};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const S: Duration = Duration::from_secs(1);
fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn ready(w: &Worker) { assert_eq!(w.recv(5 * S).unwrap()["type"], "ready"); }

/// Two spots at a 1 bp target, each with a generous per-iteration budget (deadline minus the
/// extraction margin) next to how expensive one iteration is on that tree, so the loop is certain
/// to run at least one measurement either way -- the status is never `error`/`no_iteration` -- while
/// which of `ok` / `best_so_far` actually happens depends on real solve speed and is asserted either
/// way (never a placement sleep, per the P2.T14 lesson):
///
/// - `basic_turn_std_request` (167 x 250 combos) at deadline 1000 ms, margin 200 ms, target 1 bp:
///   an 800 ms solving budget against a spot measured at 0.17 s to a *much looser* 50 bp target
///   (P2.T11's job tests) leaves many iterations' room for at least one measurement, so the loop
///   cannot stop at `no_iteration`; whether the tighter 1 bp target is actually reached in 800 ms
///   is left to the real solve, so both outcomes are accepted and only the exploitability implied
///   by each is checked (<= target for `ok`, > target and finite for `best_so_far`).
/// - `flop_best_so_far` (179 x 264 combos, the R8 FLOP-FAST size class) at deadline 2000 ms, margin
///   600 ms, target 1 bp: R8's own bench puts even a 50 bp target at 4-7 s on this size class, so a
///   1 bp target inside the 1400 ms solving budget is far out of reach -- `best_so_far` is the only
///   possible status, asserted exactly, not "either way".
///
/// Either way the elapsed time is bounded by its own `deadline_ms` (the worker's own contract, not a
/// margin the test invents) and the returned solution passes `proto::worker::validate_solution`.
#[test]
fn deadline_best_so_far_bounded() {
    let mut w = Worker::spawn(16);
    ready(&w);
    let req = &fixture_lines("basic_turn_std_request")[0];
    w.send(&edit(req, |v| {
        v["id"] = json!("130");
        v["deadline_ms"] = json!(1000);
        v["extraction_margin_ms"] = json!(200);
        v["target_bp"] = json!(1);
    }));
    let r = w.recv_until(5 * S, |m| m["type"] == "result" && m["id"] == "130").unwrap();
    assert!(r["elapsed_ms"].as_u64().unwrap() <= 1000, "elapsed {}", r["elapsed_ms"]);
    let expl = r["solution"]["exploitability_chips"].as_f64().unwrap();
    match r["status"].as_str().unwrap() {
        "ok" => assert!(expl <= 0.02),
        "best_so_far" => assert!(expl > 0.02 && expl.is_finite()),
        other => panic!("{other}"),
    }
    let tree: proto::EffectiveTree = serde_json::from_value(serde_json::from_str::<Value>(req).unwrap()["tree"].clone()).unwrap();
    let sol: proto::worker::StreetSolution = serde_json::from_value(r["solution"].clone()).unwrap();
    proto::worker::validate_solution(&sol, &tree.materialized).unwrap();

    // Second case, §13.0's `flop_best_so_far` fixture: a flop spot at target 1 bp that cannot reach
    // target inside 2 s (R8), so `best_so_far` carries a measured exploitability and a validated
    // street export.
    let flop = &fixture_lines("flop_best_so_far")[0];
    w.send(&edit(flop, |v| {
        v["id"] = json!("133");
        v["deadline_ms"] = json!(2000);
        v["extraction_margin_ms"] = json!(600);
        v["target_bp"] = json!(1);
    }));
    let r = w.recv_until(15 * S, |m| m["type"] == "result" && m["id"] == "133").unwrap();
    assert_eq!(r["status"], "best_so_far", "{r}");
    assert!(r["elapsed_ms"].as_u64().unwrap() <= 2000, "elapsed {}", r["elapsed_ms"]);
    let expl = r["solution"]["exploitability_chips"].as_f64().unwrap();
    assert!(expl.is_finite() && expl > 0.0);
    let tree: proto::EffectiveTree = serde_json::from_value(serde_json::from_str::<Value>(flop).unwrap()["tree"].clone()).unwrap();
    let sol: proto::worker::StreetSolution = serde_json::from_value(r["solution"].clone()).unwrap();
    proto::worker::validate_solution(&sol, &tree.materialized).unwrap();
}

/// §7's stop rule (`solve_loop::should_stop`) is `elapsed + 1.5 * max_iter + margin > deadline`
/// (plus an exploitability-pass term); at `elapsed = 0` this reduces to `margin > deadline` alone
/// admitting nothing. Here `extraction_margin_ms` (600) exceeds `deadline_ms` (300) by itself, so
/// the very first admission check refuses the first (non-interruptible) iteration -- deterministically,
/// for any machine, since no per-iteration cost measurement is even needed to reach that conclusion
/// (`solve_loop::tests::declines_all_work_when_even_the_first_iteration_cannot_fit` covers the same
/// arithmetic against a fake clock). The job therefore reports `no_iteration` with no solution, and
/// the wall-clock bound below is a liveness allowance (tree build, cross-check, game configuration and
/// memory admission still run before the loop refuses), not a timing assertion the test relies on for
/// determinism.
#[test]
fn deadline_no_iteration() {
    let mut w = Worker::spawn(8);
    ready(&w);
    let flop = &fixture_lines("flop_cancel")[0];
    let t = Instant::now();
    w.send(&edit(flop, |v| {
        v["id"] = json!("131");
        v["deadline_ms"] = json!(300);
        v["extraction_margin_ms"] = json!(600);
    }));
    let r = w.recv_until(5 * S, |m| m["type"] == "result" && m["id"] == "131").unwrap();
    assert_eq!(
        (r["status"].as_str(), r["error"]["code"].as_str(), r["error"]["retryable"].as_bool()),
        (Some("error"), Some("no_iteration"), Some(false))
    );
    assert!(r.get("solution").map(|s| s.is_null()).unwrap_or(true));
    assert!(t.elapsed() <= Duration::from_millis(300 + 2000), "took {:?}", t.elapsed());
}

/// §10.3 memory admission (`memory::admit`) runs in `Building`, before any allocation or solving: a
/// 64 MiB limit is refused against the flop tree's own memory estimate (the R8 FLOP-FAST size class,
/// on the order of hundreds of MB uncompressed) independent of how fast this machine solves -- the
/// estimate is arithmetic over the tree shape, not a measurement of solve progress. The job therefore
/// answers `tree_too_large` with `estimate_bytes` above the limit in milliseconds (tree build, cross
/// check and the memory check only), and the liveness bound below is generous headroom, not a race.
#[test]
fn memory_admission() {
    let mut w = Worker::spawn(4);
    ready(&w);
    let flop = &fixture_lines("flop_cancel")[0];
    let t = Instant::now();
    w.send(&edit(flop, |v| {
        v["id"] = json!("132");
        v["memory_limit_bytes"] = json!(64 * 1024 * 1024);
    }));
    let r = w.recv_until(5 * S, |m| m["type"] == "result" && m["id"] == "132").unwrap();
    assert_eq!(
        (r["status"].as_str(), r["error"]["code"].as_str(), r["error"]["retryable"].as_bool()),
        (Some("error"), Some("tree_too_large"), Some(false))
    );
    assert!(r["error"]["estimate_bytes"].as_u64().unwrap() > 64 * 1024 * 1024);
    assert!(t.elapsed() <= Duration::from_secs(2));
}
