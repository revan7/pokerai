use std::io::{BufRead, Write};

fn parse_threads(args: &[String]) -> u8 {
    args.iter().position(|a| a == "--threads").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u8>().ok()).filter(|n| *n >= 1).unwrap_or(16)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let threads = parse_threads(&args);
    rayon::ThreadPoolBuilder::new().num_threads(threads as usize).build_global().expect("rayon pool");
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer(&mut out, &solver_worker::ready_message(threads)).expect("ready");
    out.write_all(b"\n").unwrap();
    out.flush().unwrap();
    // Until Task 12 the control loop only drains stdin; EOF exits 0 (§4.5: EOF behaves like shutdown without the ack).
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        if line.is_err() { break; }
    }
    std::process::exit(0);
}
