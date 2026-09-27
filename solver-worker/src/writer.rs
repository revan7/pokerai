//! §4.5's single stdout writer. Every protocol message reaches stdout through this thread, one complete
//! JSON object per line, LF-terminated and flushed per message ("the writer never interleaves JSON with
//! other output"); `Exit(code)` is handled in queue order, so it takes effect only after every message
//! queued ahead of it has been written and flushed.
//!
//! Exit codes: `Exit(code)` ends the process with `code` (0 on `shutdown` or stdin EOF, spec §3.5) once
//! everything ahead of it is written and flushed. A writer fault ends it with `EXIT_WRITER_FAULT` instead.
use proto::worker::WorkerMessage;
use std::io::Write;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

pub enum Out { Msg(WorkerMessage), Exit(i32) }

/// The process exit code of a writer fault: a message that does not encode, or a stdout `write` or `flush`
/// that fails (a closed pipe, say), including the final flush before an `Exit(0)`. Protocol output cannot be
/// delivered, so the worker stops at once with this non-zero code and a stderr diagnostic: it consumes no
/// further queued message and never reports success over messages that were lost. The engine sees the
/// non-zero exit (`WorkerExit{code}`, §10.3) and synthesizes the terminal of any accepted job (§4.5).
pub const EXIT_WRITER_FAULT: i32 = 3;

/// One message as the exact bytes of its line: the JSON object and its LF. The whole line is encoded
/// before any of it is written, so a message the checked wire codecs refuse (a non-finite float, say)
/// yields an error and no bytes, never a partial line on stdout.
fn encode(m: &WorkerMessage) -> serde_json::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(m)?;
    line.push(b'\n');
    Ok(line)
}

/// The single stdout writer: one JSON object per line, flushed per message; `Exit` flushes and terminates
/// the process. A writer fault (see `EXIT_WRITER_FAULT`) is reported on stderr and the process exits with
/// that code; a message that does not encode has none of its bytes written (every earlier one is flushed).
pub fn spawn_writer() -> SyncSender<Out> {
    let (tx, rx) = sync_channel::<Out>(256);
    std::thread::Builder::new().name("writer".into()).spawn(move || {
        let code = { let stdout = std::io::stdout(); let mut out = stdout.lock(); drain(&rx, &mut out) };
        std::process::exit(code);
    }).expect("writer thread");
    tx
}

/// The writer loop: writes the queued items in order and returns the process exit code. It returns at the
/// first `Exit`, at the end of the queue (code 0), or at the first fault, taking nothing more from the queue:
/// after a failed `write` (which may already have put a prefix of its line out) nothing else is written, so
/// no later message is ever appended to a torn line. The final flush decides success: if it fails, the code
/// is `EXIT_WRITER_FAULT` whatever the `Exit` asked for.
fn drain(rx: &Receiver<Out>, out: &mut impl Write) -> i32 {
    for item in rx.iter() {
        match item {
            Out::Msg(m) => {
                let line = match encode(&m) { Ok(line) => line, Err(e) => return fault(format_args!("a protocol message does not serialize: {e}")) };
                if let Err(e) = out.write_all(&line).and_then(|()| out.flush()) { return fault(format_args!("stdout failed: {e}")); }
            }
            Out::Exit(code) => return finish(out, code),
        }
    }
    finish(out, 0)
}

/// `code` once a final flush has succeeded, the fault code otherwise.
fn finish(out: &mut impl Write, code: i32) -> i32 {
    match out.flush() { Ok(()) => code, Err(e) => fault(format_args!("stdout failed on the final flush: {e}")) }
}

/// Reports a writer fault on stderr and yields its exit code. The report must not panic, as `eprintln!` does
/// when stderr itself fails: a panicking writer thread would leave the process alive with nobody to exit it.
fn fault(what: std::fmt::Arguments<'_>) -> i32 {
    let _ = writeln!(std::io::stderr(), "writer: {what}; exiting {EXIT_WRITER_FAULT}");
    EXIT_WRITER_FAULT
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::worker::Stage;

    /// One message is one complete line: the JSON object, one LF, no other LF, and it reads back unchanged.
    #[test]
    fn a_message_is_encoded_as_one_complete_line() {
        let msgs = [
            crate::ready_message(4),
            WorkerMessage::Ack { id: "9".into(), status: proto::worker::AckStatus::Rejected, reason: Some("multi\nline reason".into()), replaced: None },
            WorkerMessage::Progress { id: "9".into(), stage: Stage::Solving, iterations: 3, exploitability_chips: Some(0.5), elapsed_ms: 4, memory_bytes: 5 },
        ];
        for m in msgs {
            let line = encode(&m).expect("encodes");
            assert_eq!(line.last(), Some(&b'\n'), "terminated by LF");
            assert_eq!(line.iter().filter(|b| **b == b'\n').count(), 1, "exactly one LF: {}", String::from_utf8_lossy(&line));
            assert_eq!(serde_json::from_slice::<WorkerMessage>(&line[..line.len() - 1]).unwrap(), m);
        }
    }

    /// A message the checked wire codecs refuse produces no bytes at all, so the writer never emits a
    /// partial line: it has nothing to write before it exits non-zero.
    #[test]
    fn a_message_that_does_not_serialize_yields_no_bytes() {
        let bad = WorkerMessage::Progress { id: "9".into(), stage: Stage::Solving, iterations: 3, exploitability_chips: Some(f32::NAN), elapsed_ms: 4, memory_bytes: 5 };
        assert!(encode(&bad).is_err());
    }

    // ---- Fix round 1 (review I1): an output failure is never discarded ----

    use std::io;
    use std::sync::mpsc::TryRecvError;

    /// A stdout that fails like a closed pipe: it accepts `budget` bytes in total, then every `write` fails;
    /// the first `flushes_ok` flushes succeed and every later one fails. It counts the calls made after its
    /// first failure, which a writer that stops at that failure never makes.
    struct Faulty { written: Vec<u8>, budget: usize, flushes_ok: usize, failed: bool, calls_after_failure: usize }
    impl Faulty {
        fn new(budget: usize, flushes_ok: usize) -> Self { Self { written: Vec::new(), budget, flushes_ok, failed: false, calls_after_failure: 0 } }
        fn fail(&mut self) -> io::Error { self.failed = true; io::ErrorKind::BrokenPipe.into() }
    }
    impl Write for Faulty {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.failed { self.calls_after_failure += 1; }
            if self.budget == 0 { return Err(self.fail()); }
            let n = buf.len().min(self.budget);
            self.written.extend_from_slice(&buf[..n]);
            self.budget -= n;
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.failed { self.calls_after_failure += 1; }
            if self.flushes_ok == 0 { return Err(self.fail()); }
            self.flushes_ok -= 1;
            Ok(())
        }
    }

    fn msg(id: &str) -> WorkerMessage { WorkerMessage::Ack { id: id.into(), status: proto::worker::AckStatus::Rejected, reason: Some("r".into()), replaced: None } }
    fn line(id: &str) -> Vec<u8> { encode(&msg(id)).unwrap() }
    fn queue(items: Vec<Out>) -> (SyncSender<Out>, Receiver<Out>) {
        let (tx, rx) = sync_channel::<Out>(16);
        for o in items { tx.send(o).unwrap(); }
        (tx, rx)
    }
    /// What is still queued, unconsumed by the writer.
    fn left(rx: &Receiver<Out>) -> Vec<String> {
        let mut v = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(Out::Msg(WorkerMessage::Ack { id, .. })) => v.push(format!("ack {id}")),
                Ok(Out::Msg(m)) => v.push(format!("{m:?}")),
                Ok(Out::Exit(c)) => v.push(format!("exit {c}")),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return v,
            }
        }
    }

    /// A write that fails part-way through a line (a prefix of it already out): the writer stops there with the
    /// fault code. It makes no further write or flush, so nothing is appended to the torn line, and it takes
    /// nothing more from the queue: the later message and the `Exit(0)` behind it are never consumed.
    #[test]
    fn a_write_that_fails_part_way_stops_the_writer_with_the_fault_code() {
        let (_tx, rx) = queue(vec![Out::Msg(msg("1")), Out::Msg(msg("2")), Out::Msg(msg("3")), Out::Exit(0)]);
        let mut out = Faulty::new(line("1").len() + 5, usize::MAX);
        assert_eq!(drain(&rx, &mut out), EXIT_WRITER_FAULT);
        assert_eq!(out.written, [line("1"), line("2")[..5].to_vec()].concat(), "line 1, then the torn prefix of line 2");
        assert_eq!(out.calls_after_failure, 0, "nothing is attempted after the failure");
        assert_eq!(left(&rx), vec!["ack 3", "exit 0"], "the rest of the queue is left unconsumed");
    }

    /// A flush that fails is an output failure too: the writer stops with the fault code, attempts nothing after
    /// it and consumes nothing more.
    #[test]
    fn a_flush_that_fails_stops_the_writer_with_the_fault_code() {
        let (_tx, rx) = queue(vec![Out::Msg(msg("1")), Out::Msg(msg("2")), Out::Exit(0)]);
        let mut out = Faulty::new(usize::MAX, 0);
        assert_eq!(drain(&rx, &mut out), EXIT_WRITER_FAULT);
        assert_eq!(out.written, line("1"));
        assert_eq!(out.calls_after_failure, 0);
        assert_eq!(left(&rx), vec!["ack 2", "exit 0"]);
    }

    /// The final flush decides the exit code: when it fails, a queued `Exit(0)`, or the end of the queue, ends the
    /// writer with the fault code rather than success.
    #[test]
    fn a_failed_final_flush_overrides_a_successful_exit() {
        let (_tx, rx) = queue(vec![Out::Msg(msg("1")), Out::Exit(0)]);
        let mut out = Faulty::new(usize::MAX, 1);
        assert_eq!(drain(&rx, &mut out), EXIT_WRITER_FAULT, "Exit(0) behind a failing final flush");
        assert_eq!(out.written, line("1"));
        let (tx, rx) = queue(vec![Out::Msg(msg("1"))]);
        drop(tx);
        let mut out = Faulty::new(usize::MAX, 1);
        assert_eq!(drain(&rx, &mut out), EXIT_WRITER_FAULT, "the end of the queue behind a failing final flush");
    }

    /// A healthy stdout: every queued line, in order and whole, then the `Exit` code; nothing queued behind the
    /// `Exit` is consumed.
    #[test]
    fn a_healthy_stdout_gets_every_line_then_the_exit_code() {
        let (_tx, rx) = queue(vec![Out::Msg(msg("1")), Out::Msg(msg("2")), Out::Exit(0), Out::Msg(msg("3"))]);
        let mut out = Faulty::new(usize::MAX, usize::MAX);
        assert_eq!(drain(&rx, &mut out), 0);
        assert_eq!(out.written, [line("1"), line("2")].concat());
        assert_eq!(left(&rx), vec!["ack 3"]);
    }

    /// A message that does not encode ends the writer with the fault code, after the lines before it and with none
    /// of its own bytes written.
    #[test]
    fn a_message_that_does_not_encode_stops_the_writer_with_the_fault_code() {
        let bad = WorkerMessage::Progress { id: "9".into(), stage: Stage::Solving, iterations: 3, exploitability_chips: Some(f32::NAN), elapsed_ms: 4, memory_bytes: 5 };
        let (_tx, rx) = queue(vec![Out::Msg(msg("1")), Out::Msg(bad), Out::Msg(msg("3")), Out::Exit(0)]);
        let mut out = Faulty::new(usize::MAX, usize::MAX);
        assert_eq!(drain(&rx, &mut out), EXIT_WRITER_FAULT);
        assert_eq!(out.written, line("1"));
        assert_eq!(left(&rx), vec!["ack 3", "exit 0"]);
    }
}
