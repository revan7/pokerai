#![allow(dead_code)]
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub struct Worker { child: Child, stdin: Option<ChildStdin>, lines: Receiver<String> }

impl Worker {
    pub fn spawn(threads: u8) -> Worker {
        let mut child = Command::new(env!("CARGO_BIN_EXE_solver-worker"))
            .args(["--threads", &threads.to_string()])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .spawn().expect("spawn solver-worker");
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || { for line in BufReader::new(stdout).lines() { match line { Ok(l) => { if tx.send(l).is_err() { break; } } Err(_) => break } } });
        Worker { stdin: child.stdin.take(), child, lines: rx }
    }
    pub fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(line.trim_end().as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }
    pub fn recv(&self, timeout: Duration) -> Option<Value> {
        match self.lines.recv_timeout(timeout) { Ok(l) => Some(serde_json::from_str(&l).unwrap_or_else(|e| panic!("non-JSON stdout line {l:?}: {e}"))), Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None }
    }
    /// Skips messages until `pred` holds; returns None at the deadline.
    pub fn recv_until(&self, timeout: Duration, pred: impl Fn(&Value) -> bool) -> Option<Value> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() { return None; }
            let v = self.recv(left)?;
            if pred(&v) { return Some(v); }
        }
    }
    pub fn close_stdin(&mut self) { self.stdin.take(); }
    pub fn wait_exit(&mut self, timeout: Duration) -> Option<i32> {
        let end = Instant::now() + timeout;
        loop {
            if let Ok(Some(st)) = self.child.try_wait() { return st.code(); }
            if Instant::now() >= end { return None; }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn kill(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}
impl Drop for Worker { fn drop(&mut self) { if self.child.try_wait().ok().flatten().is_none() { self.kill(); } } }

pub fn fixture_lines(name: &str) -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/worker").join(format!("{name}.jsonl"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e} (run tools/gen_worker_fixtures.py)", path.display())).lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}
