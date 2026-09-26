//! §4.5's single stdout writer. Every protocol message reaches stdout through this thread, one complete
//! JSON object per line, LF-terminated and flushed per message ("the writer never interleaves JSON with
//! other output"); `Exit(code)` is handled in queue order, so it takes effect only after every message
//! queued ahead of it has been written and flushed.
use proto::worker::WorkerMessage;
use std::io::Write;
use std::sync::mpsc::{sync_channel, SyncSender};

pub enum Out { Msg(WorkerMessage), Exit(i32) }

/// One message as the exact bytes of its line: the JSON object and its LF. The whole line is encoded
/// before any of it is written, so a message the checked wire codecs refuse (a non-finite float, say)
/// yields an error and no bytes, never a partial line on stdout.
fn encode(m: &WorkerMessage) -> serde_json::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(m)?;
    line.push(b'\n');
    Ok(line)
}

/// The single stdout writer: one JSON object per line, flushed per message; `Exit` flushes and terminates
/// the process. A message that does not encode is an internal fault: it is reported on stderr and the
/// process exits 3 with nothing of that message written (every earlier message is already flushed).
pub fn spawn_writer() -> SyncSender<Out> {
    let (tx, rx) = sync_channel::<Out>(256);
    std::thread::Builder::new().name("writer".into()).spawn(move || {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for item in rx {
            match item {
                Out::Msg(m) => match encode(&m) {
                    Ok(line) => { let _ = out.write_all(&line); let _ = out.flush(); }
                    Err(e) => { eprintln!("writer: a protocol message does not serialize: {e}"); std::process::exit(3); }
                },
                Out::Exit(code) => { let _ = out.flush(); std::process::exit(code); }
            }
        }
        std::process::exit(0);
    }).expect("writer thread");
    tx
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
}
