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
fn lock_staging_rejections() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let river = fixture_lines("river_two_combo");
    let lock = fixture_lines("lock_river");
    // a row summing to 0.4 and a row [-0.1, 1.1] are rejected; an all-zero row is the free-combo sentinel
    w.send(&edit(&with_id(&lock[0], "7"), |v| v["locks"][0]["probs"][0] = json!([0.1, 0.3])));
    assert_eq!(ack_of(&w, "7")["status"], "rejected");
    w.send(&edit(&with_id(&lock[0], "8"), |v| v["locks"][0]["probs"][0] = json!([-0.1, 1.1])));
    assert_eq!(ack_of(&w, "8")["status"], "rejected");
    w.send(&with_id(&lock[0], "9"));
    let a = ack_of(&w, "9");
    assert_eq!((a["status"].as_str(), a["replaced"].as_bool()), (Some("staged"), Some(false)));
    w.send(&with_id(&lock[0], "10"));
    assert_eq!(ack_of(&w, "10")["replaced"], true);
    // a staged lock belonging to another spot fails the next solve with lock_mismatch and is discarded
    w.send(&with_id(&river[0], "14"));
    assert_eq!(ack_of(&w, "14")["status"], "accepted");
    let r = result_of(&w, "14", 5 * S);
    assert_eq!((r["status"].as_str(), r["error"]["code"].as_str()), (Some("error"), Some("lock_mismatch")));
    w.send(&with_id(&river[0], "15"));
    assert_eq!(result_of(&w, "15", 5 * S)["status"], "ok");
    w.close_stdin();
    assert_eq!(w.wait_exit(2 * S), Some(0));
}

#[test]
fn cancel_between_iterations() {
    let mut w = Worker::spawn(8);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let flop = fixture_lines("flop_cancel");
    // Solving: ack <= 50 ms, result{cancelled} within one iteration plus one exploitability pass (<= 1.0 s), never a second terminal
    w.send(&flop[0]);
    assert_eq!(ack_of(&w, "43")["status"], "accepted");
    w.recv_until(20 * S, |m| m["type"] == "progress" && m["stage"] == "solving" && m["iterations"].as_u64().unwrap() >= 1).expect("solving progress");
    let t = Instant::now();
    w.send(&flop[1]);
    assert_eq!(ack_of(&w, "44")["status"], "accepted");
    assert!(t.elapsed() <= Duration::from_millis(50), "ack took {:?}", t.elapsed());
    let r = result_of(&w, "43", S);
    assert_eq!(r["status"], "cancelled");
    assert!(w.recv_until(Duration::from_millis(300), |m| m["type"] == "result").is_none());
    w.send(r#"{"type":"cancel","id":"45","target":"43"}"#);
    assert_eq!(ack_of(&w, "45")["status"], "already_finished");
    // Building: cancel sent right after the solve is confirmed within one build step
    w.send(&with_id(&flop[0], "46"));
    w.send(r#"{"type":"cancel","id":"47","target":"46"}"#);
    assert_eq!(ack_of(&w, "46")["status"], "accepted");
    assert_eq!(ack_of(&w, "47")["status"], "accepted");
    assert_eq!(result_of(&w, "46", 2 * S)["status"], "cancelled");
    // Extracting: a cancel after `finalize` yields exactly one terminal (cancelled, or the racing completion)
    w.send(&edit(&with_id(&flop[0], "48"), |v| { v["deadline_ms"] = json!(1500); v["extraction_margin_ms"] = json!(600); v["target_bp"] = json!(1); }));
    w.recv_until(10 * S, |m| m["type"] == "progress" && m["stage"] == "extracting").expect("extracting progress");
    w.send(r#"{"type":"cancel","id":"49","target":"48"}"#);
    let r = result_of(&w, "48", 5 * S);
    assert!(["cancelled", "best_so_far", "ok"].contains(&r["status"].as_str().unwrap()));
    assert!(w.recv_until(Duration::from_millis(500), |m| m["type"] == "result").is_none());
}

/// The brief's test with one fixture correction (report, deviation D1): `lock_river`'s OOP range, the hero
/// facing IP's all-in at node 2, is QQ and 66 (§13.2 `ev_convention_non_root_payoffs`), not AA, so the
/// hero's response is read at a 66 combo. Unlocked, IP bets QQ always and 54o half the time (66 is then
/// indifferent at one bluff in three) and 66 calls a quarter of the time (54o is then indifferent: it
/// wins the pot against 66's folds, 6(1 - c) of 9 OOP combos, and loses its bet against QQ's calls and
/// 66's, 3 + 6c of 9, so c = 1/4). Locked to 54o one time in five, IP bluffs one bet in six: 66's call
/// is worth -50, so 66 folds.
#[test]
fn lock_lifecycle() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let lock = fixture_lines("lock_river");
    let flop = fixture_lines("flop_cancel");
    let qq = proto::combo_index(proto::Card::parse("Qc").unwrap(), proto::Card::parse("Qd").unwrap()) as usize;
    let o54 = proto::combo_index(proto::Card::parse("5c").unwrap(), proto::Card::parse("4d").unwrap()) as usize;
    let sixes = proto::combo_index(proto::Card::parse("6c").unwrap(), proto::Card::parse("6d").unwrap()) as usize;
    // unlocked reference: 66 calls about 25%
    w.send(&edit(&lock[1], |v| { v["id"] = json!("60"); v["spot"] = json!("ffff"); }));
    let free = result_of(&w, "60", 5 * S);
    assert_eq!(free["solution"]["locks_applied"], 0);
    let free_call = free["solution"]["nodes"][2]["probs"][sixes][1].as_f64().unwrap();   // nodes: [] oop, [check] ip, [check, allin] oop
    assert!((free_call - 0.25).abs() < 0.1, "66 calls {free_call} unlocked");
    // lock then the matching solve: locked rows unchanged, locks_applied 1, hero's response differs (66 folds against the 1-in-6 bluff frequency)
    w.send(&lock[0]);
    assert_eq!(ack_of(&w, "47")["status"], "staged");
    w.send(&lock[1]);
    let r = result_of(&w, "51", 5 * S);
    assert_eq!(r["status"], "ok");
    assert_eq!(r["solution"]["locks_applied"], 1);
    let ip = &r["solution"]["nodes"][1];
    assert_eq!(ip["probs"][qq], json!([0.0, 1.0]));
    assert!((ip["probs"][o54][1].as_f64().unwrap() - 0.2).abs() < 1e-3);
    let locked_call = r["solution"]["nodes"][2]["probs"][sixes][1].as_f64().unwrap();
    assert!(locked_call < 0.05, "66 calls {locked_call} against the locked range");
    // a lock with another spot: the next solve with the fixture spot is lock_mismatch and the lock is discarded
    w.send(&edit(&lock[0], |v| { v["id"] = json!("61"); v["spot"] = json!("abcd"); }));
    assert_eq!(ack_of(&w, "61")["status"], "staged");
    w.send(&with_id(&lock[1], "62"));
    assert_eq!(result_of(&w, "62", 5 * S)["error"]["code"], "lock_mismatch");
    w.send(&with_id(&lock[1], "63"));
    assert_eq!(result_of(&w, "63", 5 * S)["solution"]["locks_applied"], 0);
    // lock during Solving is rejected; a lock consumed by a cancelled solve is gone
    w.send(&with_id(&flop[0], "64"));
    assert_eq!(ack_of(&w, "64")["status"], "accepted");
    w.send(&with_id(&lock[0], "65"));
    assert_eq!(ack_of(&w, "65")["reason"], "solve_in_progress");
    w.send(r#"{"type":"cancel","id":"66","target":"64"}"#);
    assert_eq!(result_of(&w, "64", 5 * S)["status"], "cancelled");
    w.send(&with_id(&lock[0], "67"));
    assert_eq!(ack_of(&w, "67")["status"], "staged");
    w.send(&edit(&with_id(&flop[0], "68"), |v| v["spot"] = lock_spot(&lock[0])));
    w.send(r#"{"type":"cancel","id":"69","target":"68"}"#);
    assert_eq!(result_of(&w, "68", 5 * S)["status"], "cancelled");
    w.send(&with_id(&lock[1], "70"));
    assert_eq!(result_of(&w, "70", 5 * S)["solution"]["locks_applied"], 0);
    w.send(r#"{"type":"shutdown","id":"71"}"#);
    assert_eq!(ack_of(&w, "71")["status"], "accepted");
    assert_eq!(w.wait_exit(2 * S), Some(0));
}
fn lock_spot(line: &str) -> Value { serde_json::from_str::<Value>(line).unwrap()["spot"].clone() }
