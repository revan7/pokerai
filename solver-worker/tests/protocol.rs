mod common;
use common::{fixture_lines, Worker};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn with_id(line: &str, id: &str) -> String { edit(line, |v| v["id"] = json!(id)) }
fn ack_of(w: &Worker, id: &str) -> Value { w.recv_until(Duration::from_secs(5), |m| m["type"] == "ack" && m["id"] == id).unwrap_or_else(|| panic!("no ack for {id}")) }
fn result_of(w: &Worker, id: &str, t: Duration) -> Value { w.recv_until(t, |m| m["type"] == "result" && m["id"] == id).unwrap_or_else(|| panic!("no result for {id}")) }
const S: Duration = Duration::from_secs(1);

#[test]
fn protocol_rejections() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let river = fixture_lines("river_two_combo");
    let flop = fixture_lines("flop_cancel");
    w.send(r#"{"type":"bogus","id":"1"}"#);
    assert_eq!(ack_of(&w, "1")["status"], "rejected");
    w.send(r#"{"type":"cancel","id":"2","target":"x","extra":1}"#);
    assert_eq!(ack_of(&w, "2")["status"], "rejected");
    w.send(&edit(&with_id(&river[0], "3"), |v| { v["oop_range"].as_array_mut().unwrap().pop(); }));
    assert_eq!(ack_of(&w, "3")["status"], "rejected");
    // `NaN` is not valid JSON, so the id can only come from `lenient_id`'s byte scan
    w.send(&with_id(&river[0], "4").replacen("\"pot\":100", "\"pot\":NaN", 1));
    assert_eq!(ack_of(&w, "4")["status"], "rejected");
    // §13.2: a `None` donk option on a street strictly AFTER the root is structurally invalid input.
    // It is `ack{rejected, reason}` with NO work — never `result{error{tree_mismatch}}`, which is reserved
    // for a realized tree that differs from `tree.materialized` (Task 16).
    w.send(&edit(&with_id(&flop[0], "5"), |v| v["tree"]["menus"]["turn"]["donk"] = Value::Null));
    let a = ack_of(&w, "5");
    assert_eq!(a["status"], "rejected");
    assert!(a["reason"].as_str().unwrap().contains("donk"));
    assert!(w.recv_until(S, |m| m["type"] == "result" && m["id"] == "5").is_none(), "a rejected request does no work");
    // The root street's own `None` is legal: `flop_cancel` is flop-rooted and its flop menu carries `donk: null`.
    w.send(&edit(&with_id(&flop[0], "6"), |v| v["tree"]["menus"]["flop"]["donk"] = Value::Null));
    assert_eq!(ack_of(&w, "6")["status"], "accepted");
    w.send(r#"{"type":"cancel","id":"7","target":"6"}"#);
    assert_eq!(result_of(&w, "6", 5 * S)["status"], "cancelled");
    // busy: a long flop solve, then a river solve is rejected "busy"; a duplicate id of the live job is "duplicate"
    w.send(&with_id(&flop[0], "11"));
    assert_eq!(ack_of(&w, "11")["status"], "accepted");
    w.send(&with_id(&river[0], "12"));
    let a = ack_of(&w, "12");
    assert_eq!((a["status"].as_str(), a["reason"].as_str()), (Some("rejected"), Some("busy")));
    w.send(&with_id(&flop[0], "11"));
    assert_eq!(ack_of(&w, "11")["reason"], "duplicate");
    w.send(r#"{"type":"cancel","id":"13","target":"11"}"#);
    assert_eq!(ack_of(&w, "13")["status"], "accepted");
    assert_eq!(result_of(&w, "11", 5 * S)["status"], "cancelled");
    // a finished id is still remembered: re-sending it is a duplicate, not a fresh admission
    w.send(&with_id(&flop[0], "11"));
    assert_eq!(ack_of(&w, "11")["reason"], "duplicate");
    // a valid river solve: progress during building carries exploitability_chips null
    w.send(&with_id(&river[0], "15"));
    let p = w.recv_until(5 * S, |m| m["type"] == "progress" && m["id"] == "15").unwrap();
    assert!(p["stage"] == "building" && p["exploitability_chips"].is_null());
    assert_eq!(result_of(&w, "15", 5 * S)["status"], "ok");
    w.send(r#"{"type":"cancel","id":"16","target":"nope"}"#);
    assert_eq!(ack_of(&w, "16")["status"], "unknown_target");        // still alive after every rejection
    w.send(r#"{"type":"shutdown","id":"17"}"#);
    assert_eq!(ack_of(&w, "17")["status"], "accepted");
    assert_eq!(w.wait_exit(2 * S), Some(0));
}

/// Fix round 1 (review I1): the wire codec's domain (`proto::numeric::domain_rake_rate`) is already
/// half-open at 1, so a `rake_rate` of exactly 1.0 never reaches `precheck` at all -- the deserializer
/// itself refuses the line as an invalid message, before an `EngineMessage` even exists. This pins that
/// wire-level rejection (never a silent admission) alongside the direct-handler rejection this same fix
/// adds at the `precheck` boundary (`admission_rejects_a_rake_rate_of_exactly_one`, protocol.rs).
#[test]
fn rake_rate_of_exactly_one_is_rejected_at_the_wire() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let river = fixture_lines("river_two_combo");
    w.send(&edit(&with_id(&river[0], "50"), |v| v["rake_rate"] = json!(1.0)));
    let a = ack_of(&w, "50");
    assert_eq!(a["status"], "rejected");
    assert!(a["reason"].as_str().unwrap().contains("rake_rate"), "{a}");
    assert!(w.recv_until(S, |m| m["id"] == "50" && m["type"] != "ack").is_none(), "a rejected request does no work");
    w.send(r#"{"type":"shutdown","id":"51"}"#);
    assert_eq!(ack_of(&w, "51")["status"], "accepted");
    assert_eq!(w.wait_exit(2 * S), Some(0));
}

// ---- Beyond the brief's test: the spec's river wire example, and a stop with a live job ----

/// §4.5's river wire example replayed from `river_two_combo.jsonl` (solve 41, cancel 42, shutdown 48), reading every
/// message in order: the solve's ack is the first answer (ahead of all its progress, the first of which is
/// `building` with a null measurement), the job has exactly one terminal, a cancel of the finished id answers
/// `already_finished`, and shutdown's ack is the last message before exit 0. Each ack is pinned whole.
#[test]
fn the_river_wire_example_replays_in_order() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let lines = fixture_lines("river_two_combo");
    assert_eq!(lines.len(), 3, "solve 41, cancel 42, shutdown 48");
    w.send(&lines[0]);
    assert_eq!(w.recv(5 * S).unwrap(), json!({"type": "ack", "id": "41", "status": "accepted"}), "the ack comes first");
    let first = w.recv(5 * S).unwrap();
    assert_eq!((first["type"].as_str(), first["id"].as_str(), first["stage"].as_str()), (Some("progress"), Some("41"), Some("building")));
    assert!(first["exploitability_chips"].is_null());
    let result = loop {
        let m = w.recv(5 * S).expect("the job's terminal");
        assert_eq!(m["id"], "41", "only job 41 is talking: {m}");
        if m["type"] == "result" { break m; }
        assert_eq!(m["type"], "progress", "{m}");
    };
    assert_eq!(result["status"], "ok");
    w.send(&lines[1]);
    assert_eq!(w.recv(5 * S).unwrap(), json!({"type": "ack", "id": "42", "status": "already_finished"}));
    w.send(&lines[2]);
    assert_eq!(w.recv(5 * S).unwrap(), json!({"type": "ack", "id": "48", "status": "accepted"}));
    assert_eq!(w.wait_exit(2 * S), Some(0));
    assert!(w.recv(S).is_none(), "nothing after the shutdown ack");
}

/// §4.5: `shutdown` in any state is acked, a running job receives `result{cancelled}` at its next checkpoint, and the
/// process then exits 0 within 2 s; stdin EOF behaves the same without the ack. The stop is sent once the flop job is
/// solving (its full solve takes far longer than the test). Everything after the stop is read to the end of stdout:
/// the ack (shutdown only) ahead of the job's single terminal, `cancelled`, and nothing after it. Measured stop-to-exit
/// on this job: about 0.3 s at 4 threads, well inside the 1.9 s at which the stop watchdog would exit regardless.
#[test]
fn a_stop_during_a_live_solve_cancels_it_then_exits_zero() {
    let flop = fixture_lines("flop_cancel");
    for by_eof in [false, true] {
        let mut w = Worker::spawn(4);
        assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
        w.send(&flop[0]);
        assert_eq!(w.recv(5 * S).unwrap(), json!({"type": "ack", "id": "43", "status": "accepted"}));
        w.recv_until(10 * S, |m| m["type"] == "progress" && m["stage"] == "solving").expect("the job reaches Solving");
        let stopped = Instant::now();
        if by_eof { w.close_stdin(); } else { w.send(r#"{"type":"shutdown","id":"48"}"#); }
        assert_eq!(w.wait_exit(2 * S), Some(0), "exit 0 within 2 s of the stop (by_eof: {by_eof})");
        let took = stopped.elapsed();
        let mut after = Vec::new();
        while let Some(m) = w.recv(S) { after.push(m); }
        let answers: Vec<&Value> = after.iter().filter(|m| m["type"] != "progress").collect();
        let mut want = vec![json!({"type": "result", "id": "43", "status": "cancelled", "elapsed_ms": answers.last().map(|r| r["elapsed_ms"].clone()).unwrap_or(Value::Null)})];
        if !by_eof { want.insert(0, json!({"type": "ack", "id": "48", "status": "accepted"})); }
        assert_eq!(answers, want.iter().collect::<Vec<_>>(), "by_eof: {by_eof}, after {took:?}");
        assert!(after.iter().all(|m| m["type"] != "progress" || m["id"] == "43"));
        assert_eq!(after.last().unwrap()["type"], "result", "the terminal is the last message");
    }
}
