//! Task 18: the engine's worker link (spec sections 3.1, 3.7, 4.5, 10.3, 12 and 13.2's
//! `river_check_only_terminal_oracle`). The process tests spawn the real `solver-worker` binary; the
//! stand-in-process cases (refused `ready`, exit codes, startup timeout, stderr ring) are unit tests of
//! `engine::worker::process`, and the job object's are unit tests of `engine::worker::job_object`.
use engine::worker::link::{WorkerLink, WorkerLinkError};
use engine::worker::process::ProcessWorker;
use engine::worker::ready::{cpu_lacks_avx2, validate_ready};
use proto::worker::{AckStatus, EngineMessage, Ready, WorkerMessage, ADAPTER_VERSION, PROTO_VERSION, SOLVER_COMMIT};
use std::path::PathBuf;
use std::time::Duration;

/// The engine never links the worker (§3.2), so the binary is discovered, not built by this crate.
/// Order: `POKERAI_WORKER`, then the release build in the workspace target directory, then the debug one, then the
/// same two next to this test binary (a redirected `CARGO_TARGET_DIR`).
/// `None` means "not built in this run" and the caller returns early with a printed reason, so
/// `cargo test --workspace --release` is green whether or not the V1-selected worker build has run.
fn worker_exe() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("POKERAI_WORKER") { let p = PathBuf::from(p); if p.exists() { return Some(p); } }
    let name = if cfg!(windows) { "solver-worker.exe" } else { "solver-worker" };
    // CARGO_MANIFEST_DIR is crates/engine; the workspace target dir is ../../target
    let mut targets = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target")];
    // this test binary is <target>/<profile>/deps/worker_link-<hash>.exe
    if let Some(t) = std::env::current_exe().ok().and_then(|e| e.parent()?.parent()?.parent().map(PathBuf::from)) { targets.push(t); }
    for target in targets {
        for profile in ["release", "debug"] {
            let p = target.join(profile).join(name);
            if p.exists() { return Some(p); }
        }
    }
    None
}
/// Returns the path or prints why the test is skipped. Used as `let Some(exe) = require_exe() else { return; };`.
fn require_exe() -> Option<PathBuf> {
    match worker_exe() {
        Some(p) => Some(p),
        None => { eprintln!("skipping: solver-worker binary not found; build it with the V1-selected toolchain (docs/bench/worker-toolchain.json) and set POKERAI_WORKER"); None }
    }
}

#[test]
fn ready_validation_rules() {
    let ok = Ready { proto_version: PROTO_VERSION, solver_commit: SOLVER_COMMIT.into(), adapter_version: ADAPTER_VERSION, threads: 16, build_features: vec!["avx2".into()], cpu_features: vec!["avx2".into()], capabilities: vec!["solve".into()] };
    assert!(validate_ready(&ok, 16).is_ok());
    assert!(validate_ready(&ok, 8).unwrap_err().contains("threads"));
    let mut no_avx = ok.clone(); no_avx.build_features = vec![];
    assert_eq!(validate_ready(&no_avx, 16).unwrap_err(), "worker built without AVX2");
    let mut v = ok.clone(); v.proto_version = 2;
    assert!(validate_ready(&v, 16).unwrap_err().contains("proto_version"));
    let mut c = ok.clone(); c.solver_commit = "deadbeef".into();
    assert!(validate_ready(&c, 16).unwrap_err().contains("commit"));
    // Every §4.5 rule is tested (standing ruling): the adapter version too.
    let mut a = ok.clone(); a.adapter_version = ADAPTER_VERSION + 1;
    assert!(validate_ready(&a, 16).unwrap_err().contains("adapter_version"));
    // The commit is compared exactly: a padded string is not the pinned commit.
    let mut padded = ok.clone(); padded.solver_commit = format!("{SOLVER_COMMIT}\n");
    assert!(validate_ready(&padded, 16).unwrap_err().contains("commit"));
    // `avx2` is matched exactly among the BUILD features; neighbours and a CPU feature do not stand in for it.
    let mut near = ok.clone(); near.build_features = vec!["avx".into(), "AVX2".into(), "avx512f".into()];
    assert_eq!(validate_ready(&near, 16).unwrap_err(), "worker built without AVX2");
    // CPU features never refuse a worker (§3.7): they only feed the startup banner.
    let mut no_cpu = ok.clone(); no_cpu.cpu_features = vec!["fma".into()];
    assert!(validate_ready(&no_cpu, 16).is_ok());
    assert!(cpu_lacks_avx2(&no_cpu));
    assert!(!cpu_lacks_avx2(&ok));
}

#[test]
fn process_worker_spawns_validates_ready_and_restarts() {
    let Some(exe) = require_exe() else { return; };
    let mut w = ProcessWorker::spawn(&exe, 4).unwrap();
    assert_eq!(w.ready().unwrap().threads, 4);
    w.send(&EngineMessage::Cancel { id: "1".into(), target: "none".into() }).unwrap();
    match w.recv(Duration::from_secs(2)).unwrap() { Some(WorkerMessage::Ack { id, .. }) => assert_eq!(id, "1"), other => panic!("{other:?}") }
    w.kill();
    assert!(matches!(w.recv(Duration::from_millis(500)), Err(WorkerLinkError::Exit { .. }) | Err(WorkerLinkError::Eof)));
    w.restart().unwrap();
    assert_eq!(w.ready().unwrap().threads, 4);
    w.send(&EngineMessage::Shutdown { id: "2".into() }).unwrap();
    assert!(matches!(w.recv(Duration::from_secs(2)).unwrap(), Some(WorkerMessage::Ack { .. })));
}

#[test]
fn river_check_only_terminal_oracle() {
    // §13.2 (spec S8: this test lives here because its oracle is `core-eval`, which `solver-worker` may not depend on).
    // Check-check only, pot 100, stacks 100, no all-in: EV(check) per combo equals equity_actual_combo * 100 within 1e-3
    // at both nodes (OOP's root and IP's node) of both solutions; swapping seats leaves every value unchanged, checked
    // in both directions.
    use core_eval::{equity, exact_cost, EquityMode, EquityRequest, EquityStatus, PlayerRange};
    use proto::{combo_cards, combo_index, Action, Card, EffectiveTree, MaterializedNode, PlayerMenus, Range1326, Seat, SideMenu, Street};
    use proto::worker::SolveRequest;
    let Some(exe) = require_exe() else { return; };
    let board: Vec<Card> = ["Qs", "Jd", "7h", "3c", "2d"].iter().map(|s| Card::parse(s).unwrap()).collect();
    let mut oop = Range1326([0.0; 1326]); let mut ip = Range1326([0.0; 1326]);
    for (a, b) in [("Ac", "Ad"), ("Ah", "As"), ("6c", "6d"), ("Kc", "Kd")] { oop.0[combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize] = 1.0; }
    for (a, b, w) in [("Qc", "Qd", 1.0), ("5c", "4d", 0.25), ("5h", "4s", 0.25), ("Tc", "9c", 0.5)] { ip.0[combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize] = w; }
    let menus = PlayerMenus { oop: SideMenu { bet: vec![], raise: vec![] }, ip: SideMenu { bet: vec![], raise: vec![] }, donk: None };
    let tree = EffectiveTree { rules_version: 3, template_id: "check_only_test".into(), root_street: Street::River, menus: [(Street::River, menus)].into_iter().collect(),
        add_allin_threshold: 0.0, force_allin_threshold: 0.0, merging_threshold: 0.0, wager_cap: 1, inserted: vec![],
        materialized: vec![MaterializedNode { path: vec![], street: Street::River, actor: "oop".into(), actions: vec![Action::Check], terminal_pots: vec![None] },
                           MaterializedNode { path: vec![0], street: Street::River, actor: "ip".into(), actions: vec![Action::Check], terminal_pots: vec![Some(100)] }] };
    let mut w = ProcessWorker::spawn(&exe, 4).unwrap();
    let mut run = |id: &str, oop: &Range1326, ip: &Range1326| -> proto::worker::StreetSolution {
        w.send(&EngineMessage::Solve(SolveRequest { id: id.into(), spot: "c".repeat(64), board: board.clone(), oop_range: oop.clone(), ip_range: ip.clone(), pot: 100, stack_oop: 100, stack_ip: 100,
            rake_rate: 0.0, rake_cap_mchips: 0, tree: tree.clone(), history: vec![], target_bp: 1, deadline_ms: 2000, extraction_margin_ms: 200, memory_limit_bytes: 10 << 30, background: false })).unwrap();
        // A rejection or a silent worker fails the test instead of looping on it (5 s is a liveness bound: the
        // check-only river solves in milliseconds).
        loop {
            match w.recv(Duration::from_secs(5)).unwrap() {
                Some(WorkerMessage::Result { solution, status, .. }) => { assert_eq!(status, proto::worker::ResultStatus::Ok); return solution.unwrap(); }
                Some(WorkerMessage::Ack { status: AckStatus::Rejected, reason, .. }) => panic!("solve {id} rejected: {reason:?}"),
                Some(_) => continue,
                None => panic!("no result for solve {id} within 5 s"),
            }
        }
    };
    // `core-eval`'s shape (plan 1 Task 24): players are seat-tagged ranges, the result carries per-seat shares.
    // A fixed hero combo is a range with one supported combo; hero is seat 0 and the villain seat 1 in every query.
    let hero_equity = |hero_combo: usize, villain: &Range1326| -> f32 {
        let mut fixed = Range1326([0.0; 1326]); fixed.0[hero_combo] = 1.0;
        let mut opp = villain.clone();
        let [x, y] = combo_cards(hero_combo as u16);
        for j in 0..1326 { let [p, q] = combo_cards(j as u16); if [p, q].iter().any(|c| *c == x || *c == y) { opp.0[j] = 0.0; } }
        let req = EquityRequest::single_pot(board.clone(), vec![PlayerRange { seat: Seat(0), range: fixed }, PlayerRange { seat: Seat(1), range: opp }], EquityMode::Exact);
        assert!(exact_cost(&req) <= 20_000_000, "the §7 rule admits exact enumeration for this request");
        let res = equity(&req, Duration::from_secs(5), &std::sync::atomic::AtomicBool::new(false));
        assert_eq!(res.status, EquityStatus::Ready);
        res.shares.iter().find(|s| s.pot_index == 0 && s.seat == Seat(0)).expect("hero share").value
    };
    let a = run("1", &oop, &ip);
    let b = run("2", &ip, &oop);
    // Node 0 is OOP's root, node 1 IP's node after OOP's check (the only line); each carries its actor's combos.
    for (run_name, sol) in [("a", &a), ("b", &b)] {
        assert_eq!((sol.nodes[0].path.as_slice(), sol.nodes[0].actor.as_str()), (&[][..], "oop"), "{run_name}: node 0");
        assert_eq!((sol.nodes[1].path.as_slice(), sol.nodes[1].actor.as_str()), (&[Action::Check][..], "ip"), "{run_name}: node 1");
    }
    // Both nodes of both solutions (§13.2: "at the OOP root and at IP's node"): EV(check) of every supported combo of
    // the node's actor equals its equity against the opposing range, times the pot. In `b` the ranges swapped seats,
    // so `b`'s OOP node holds the `ip` range's combos and `b`'s IP node the `oop` range's.
    for (name, node, hero, villain) in [("a.nodes[0] (oop range at OOP)", &a.nodes[0], &oop, &ip), ("a.nodes[1] (ip range at IP)", &a.nodes[1], &ip, &oop),
                                        ("b.nodes[0] (ip range at OOP)", &b.nodes[0], &ip, &oop), ("b.nodes[1] (oop range at IP)", &b.nodes[1], &oop, &ip)] {
        for i in 0..1326 {
            if hero.0[i] == 0.0 { continue; }
            let eq = hero_equity(i, villain);
            assert!((node.ev_chips[i][0] - eq * 100.0).abs() <= 1e-3, "{name} combo {i}: ev {} vs equity*100 {}", node.ev_chips[i][0], eq * 100.0);
        }
    }
    // Swapping seats swapped the roles, not the values: each range's combos carry the same EV in either seat, both ways.
    for i in 0..1326 {
        if ip.0[i] > 0.0 {
            let (x, y) = (a.nodes[1].ev_chips[i][0], b.nodes[0].ev_chips[i][0]);
            assert!((x - y).abs() <= 1e-3, "ip combo {i}: a.nodes[1] {x} vs b.nodes[0] {y}");
        }
        if oop.0[i] > 0.0 {
            let (x, y) = (a.nodes[0].ev_chips[i][0], b.nodes[1].ev_chips[i][0]);
            assert!((x - y).abs() <= 1e-3, "oop combo {i}: a.nodes[0] {x} vs b.nodes[1] {y}");
        }
    }
}

// ---- Beyond the brief's three: the standing rulings on the link (spawn bounded and reported, kill idempotent,
// exit codes confirmed, a second `ready` refused). ----

/// A binary that cannot be started is a `Spawn` error after exactly one retry (§4.5), never a hang or a panic.
#[test]
fn spawn_of_a_missing_binary_is_a_bounded_spawn_error() {
    let missing = std::env::temp_dir().join(format!("pokerai-no-such-worker-{}.exe", std::process::id()));
    assert!(!missing.exists());
    match ProcessWorker::spawn(&missing, 4) {
        Err(WorkerLinkError::Spawn(msg)) => assert!(msg.contains("after 2 attempts"), "{msg}"),
        Err(other) => panic!("expected a spawn error, got {other:?}"),
        Ok(_) => panic!("a missing binary spawned"),
    }
}

/// `kill` twice is `kill` once; a killed link answers `Eof`, forgets its `ready`, and restarts.
#[test]
fn kill_is_idempotent_and_a_killed_link_restarts() {
    let Some(exe) = require_exe() else { return; };
    let mut w = ProcessWorker::spawn(&exe, 2).unwrap();
    assert!(w.peak_working_set_bytes() > 0, "a live worker has a working set");
    w.kill();
    w.kill();
    assert!(w.ready().is_none());
    assert_eq!(w.peak_working_set_bytes(), 0);
    assert!(matches!(w.send(&EngineMessage::Shutdown { id: "1".into() }), Err(WorkerLinkError::Eof)));
    assert!(matches!(w.recv(Duration::from_millis(10)), Err(WorkerLinkError::Eof)));
    w.restart().unwrap();
    assert_eq!(w.ready().unwrap().threads, 2);
    assert_eq!(w.restarts(), 1);
    w.kill();
}

/// `shutdown` is acked, then the worker exits 0 (§4.5) and the link reports the confirmed exit code, not a bare EOF,
/// once every line before it has been read; afterwards it keeps answering with that exit.
#[test]
fn shutdown_ends_in_a_confirmed_exit_code_zero() {
    let Some(exe) = require_exe() else { return; };
    let mut w = ProcessWorker::spawn(&exe, 2).unwrap();
    w.send(&EngineMessage::Shutdown { id: "9".into() }).unwrap();
    match w.recv(Duration::from_secs(2)).unwrap() { Some(WorkerMessage::Ack { id, status: AckStatus::Accepted, .. }) => assert_eq!(id, "9"), other => panic!("{other:?}") }
    // 5 s is a liveness bound over §4.5's "exits 0 within 2 s"
    match w.recv(Duration::from_secs(5)) { Err(WorkerLinkError::Exit { code: 0 }) => {}, other => panic!("expected exit 0, got {other:?}") }
    assert!(matches!(w.recv(Duration::from_millis(10)), Err(WorkerLinkError::Exit { code: 0 })));
    assert!(matches!(w.send(&EngineMessage::Shutdown { id: "10".into() }), Err(WorkerLinkError::Exit { code: 0 })));
}
