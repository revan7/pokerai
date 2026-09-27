mod common;
use common::Worker;
use serde_json::{json, Value};
use solver_worker::writer::EXIT_WRITER_FAULT;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

const S: Duration = Duration::from_secs(1);

/// The §4.5 answer to the well-formed line these tests send, a `cancel` of the never-seen target "nope": the
/// complete ack, every field pinned (review M1 of Task 12), so a stray `reason` or `replaced` fails too.
fn unknown_target_ack(id: &str) -> Value { json!({"type": "ack", "id": id, "status": "unknown_target"}) }

#[test]
fn oversized_request_line_is_rejected_and_the_worker_survives() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let big = format!(r#"{{"type":"cancel","id":"6","target":"{}"}}"#, "x".repeat(1_100_000));
    w.send(&big);
    let a = w.recv_until(5 * S, |m: &Value| m["type"] == "ack").unwrap();
    // the complete ack (review M1): the line is discarded unparsed, so its id is "unknown"
    assert_eq!(a, json!({"type": "ack", "id": "unknown", "status": "rejected", "reason": "line exceeds 1 MiB"}));
    // the worker is still reading: a following well-formed line is answered, not swallowed
    w.send(r#"{"type":"cancel","id":"7","target":"nope"}"#);
    assert_eq!(w.recv_until(5 * S, |m: &Value| m["id"] == "7").unwrap(), unknown_target_ack("7"));
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
        assert_eq!(a, unknown_target_ack(&i.to_string()), "answer {i}");
    }
    assert!(w.recv(S).is_none(), "nothing after the last answer");
}

// ---- Fix round 1: byte-level stdin and a failing stdout (`common::Worker` sends text and always reads stdout) ----

/// The worker binary started directly, with piped stdin and the given stdout and stderr.
fn spawn_raw(stdout: Stdio, stderr: Stdio) -> Child {
    Command::new(env!("CARGO_BIN_EXE_solver-worker")).args(["--threads", "4"])
        .stdin(Stdio::piped()).stdout(stdout).stderr(stderr).spawn().expect("spawn solver-worker")
}

/// The exit code once the process has ended, or None at the deadline (a liveness guard, not a race decider).
fn exit_code(child: &mut Child, timeout: Duration) -> Option<i32> {
    let end = Instant::now() + timeout;
    loop {
        if let Ok(Some(st)) = child.try_wait() { return st.code(); }
        if Instant::now() >= end { let _ = child.kill(); return None; }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Review I1: once stdout fails, the worker does not carry on as if its messages were delivered. Here the engine's
/// end of the stdout pipe is closed before the worker answers, so the answer cannot be written: the worker must end
/// with the writer-fault exit code and a stderr diagnostic (the engine then sees a failed worker exit, §10.3), never
/// read on and exit 0 at EOF as if the answer had gone out.
#[test]
fn a_stdout_that_fails_ends_the_worker_with_the_writer_fault_code() {
    let mut child = spawn_raw(Stdio::piped(), Stdio::piped());
    drop(child.stdout.take());   // no reader is left: every stdout write from here on fails
    let mut stdin = child.stdin.take().unwrap();
    // The ack to this line cannot be delivered. The write may itself fail when the worker has already ended on its
    // undeliverable `ready`; either way nothing reaches the engine.
    let _ = stdin.write_all(br#"{"type":"cancel","id":"1","target":"nope"}"#).and_then(|()| stdin.write_all(b"\n")).and_then(|()| stdin.flush());
    drop(stdin);   // EOF: a worker that ignored the failure would exit 0 here
    assert_eq!(exit_code(&mut child, 5 * S), Some(EXIT_WRITER_FAULT));
    let mut err = String::new();
    child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert!(err.contains("writer:"), "a stderr diagnostic names the failure: {err:?}");
}

/// Review I2: a request line that is not UTF-8 is rejected as exactly that, with the complete ack
/// `{id: "unknown", rejected, "line is not valid UTF-8"}`, never decoded lossily and answered as a different
/// request (here it would be answered with id "\u{FFFD}"). The worker reads on: the next line is answered, and
/// a final unterminated line that is not UTF-8 is rejected the same way before the EOF exit.
#[test]
fn a_request_line_that_is_not_utf8_is_rejected_as_such_and_the_worker_survives() {
    let mut child = spawn_raw(Stdio::piped(), Stdio::inherit());
    let (tx, lines) = channel::<String>();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || for l in BufReader::new(stdout).lines() { if tx.send(l.expect("stdout is UTF-8")).is_err() { break; } });
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"{\"type\":\"cancel\",\"id\":\"\xff\",\"target\":\"nope\"}\n").unwrap();
    stdin.write_all(br#"{"type":"cancel","id":"9","target":"nope"}"#).unwrap();
    stdin.write_all(b"\n{\"type\":\"cancel\",\"id\":\"\xc3\",\"target\":\"nope\"}").unwrap();
    drop(stdin);
    let next = || -> Value { serde_json::from_str(&lines.recv_timeout(5 * S).expect("an answer")).unwrap() };
    assert_eq!(next()["type"], "ready");
    let not_utf8 = json!({"type": "ack", "id": "unknown", "status": "rejected", "reason": "line is not valid UTF-8"});
    assert_eq!(next(), not_utf8);
    assert_eq!(next(), unknown_target_ack("9"));
    assert_eq!(next(), not_utf8);
    assert_eq!(exit_code(&mut child, 5 * S), Some(0));
    assert!(lines.recv_timeout(S).is_err(), "nothing after the last answer");
}
