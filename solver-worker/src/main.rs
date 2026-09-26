use solver_worker::protocol::{executor_loop, handle_eof, handle_line, read_line, Incoming, Proto, Shared};
use solver_worker::writer::{spawn_writer, Out};
use std::sync::{mpsc::channel, Arc, Mutex};

fn parse_threads(args: &[String]) -> u8 {
    args.iter().position(|a| a == "--threads").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u8>().ok()).filter(|n| *n >= 1).unwrap_or(16)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let threads = parse_threads(&args);
    rayon::ThreadPoolBuilder::new().num_threads(threads as usize).build_global().expect("rayon pool");
    let out = spawn_writer();
    let (jobs_tx, jobs_rx) = channel();
    let shared = Arc::new(Shared { proto: Mutex::new(Proto::new()), out: out.clone(), jobs: jobs_tx });
    let _ = out.send(Out::Msg(solver_worker::ready_message(threads)));
    let exec = shared.clone();
    std::thread::Builder::new().name("executor".into()).spawn(move || executor_loop(exec, jobs_rx)).expect("executor thread");
    // control: this thread reads stdin in every state
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    loop {
        match read_line(&mut reader) {
            Ok(Incoming::Line(l)) => handle_line(&shared, &l),
            Ok(Incoming::TooLong) => { let _ = out.send(Out::Msg(proto::worker::WorkerMessage::Ack { id: "unknown".into(), status: proto::worker::AckStatus::Rejected, reason: Some("line exceeds 1 MiB".into()), replaced: None })); }
            Ok(Incoming::Eof) | Err(_) => { handle_eof(&shared); break; }
        }
    }
    // The writer exits the process (Out::Exit) once the live job, if any, has written its terminal result.
    // `park` may return spuriously, and returning from `main` would end the process before the writer drains.
    loop { std::thread::park(); }
}
