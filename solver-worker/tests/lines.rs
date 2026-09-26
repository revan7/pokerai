mod common;
use common::Worker;
use serde_json::Value;
use std::time::Duration;

const S: Duration = Duration::from_secs(1);

#[test]
fn oversized_request_line_is_rejected_and_the_worker_survives() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let big = format!(r#"{{"type":"cancel","id":"6","target":"{}"}}"#, "x".repeat(1_100_000));
    w.send(&big);
    let a = w.recv_until(5 * S, |m: &Value| m["type"] == "ack").unwrap();
    assert_eq!(a["status"], "rejected");
    assert!(a["reason"].as_str().unwrap().contains("1 MiB"));
    // the worker is still reading: a following well-formed line is answered, not swallowed
    w.send(r#"{"type":"cancel","id":"7","target":"nope"}"#);
    assert_eq!(w.recv_until(5 * S, |m: &Value| m["id"] == "7").unwrap()["type"], "ack");
}

#[test]
fn eof_exits_zero() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    w.close_stdin();
    assert_eq!(w.wait_exit(2 * S), Some(0));   // §4.5: EOF behaves like shutdown without the ack
}

// ---- Beyond the brief's two tests ----

/// EOF right behind a burst of lines: every line is answered, in order, each answer a complete JSON line,
/// before the process exits 0 (the writer drains everything queued ahead of `Exit`).
#[test]
fn eof_behind_queued_lines_answers_every_line_before_exiting() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    const N: usize = 300;
    for i in 0..N { w.send(&format!(r#"{{"type":"cancel","id":"{i}","target":"nope"}}"#)); }
    w.close_stdin();
    assert_eq!(w.wait_exit(5 * S), Some(0));
    for i in 0..N {
        let a = w.recv(5 * S).unwrap_or_else(|| panic!("no answer to line {i}"));
        assert_eq!((a["type"].as_str(), a["id"].as_str()), (Some("ack"), Some(i.to_string().as_str())), "answer {i}: {a}");
    }
    assert!(w.recv(S).is_none(), "nothing after the last answer");
}
