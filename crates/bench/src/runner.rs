//! `bench run` execution: builds a `SolveRequest` for one suite spot (§4.6/§10.2 materialization,
//! §3.2 dependency direction through `engine::bench_support` — no `core-*` dependency here), drives
//! it against a live `WorkerLink` to a terminal `Result`, and separately measures §7/§13.5 "cancel
//! latency" (send a solve with a long deadline, cancel it once the worker has visibly started
//! solving, and time the ack and the terminal).

use crate::suite::Spot;
use engine::deadline::{extraction_margin_ms, street_budget_ms};
use engine::tree::{materialize_at, Templates};
use engine::worker::link::WorkerLink;
use proto::worker::{EngineMessage, ResultStatus, SolveRequest, Stage, WorkerMessage};
use proto::{Rake, Range1326};
use std::time::{Duration, Instant};

/// One (spot, rep) measurement of a `bench run`. `§13.5` columns are built from a whole suite's rows
/// by `report::Report`; this is the per-attempt record it accumulates.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpotResult {
    pub spot: String,
    pub rep: u32,
    pub cold: bool,
    pub wall_ms: u64,
    pub ack_ms: u64,
    pub status: String,
    pub reached_bp: Option<u16>,
    pub iterations: u32,
    pub memory_bytes: u64,
    pub peak_ws_bytes: u64,
    pub mode: String,
    pub street_violation: bool,
    pub final_violation: bool,
}

/// Range parsing and board blocking go through `engine::bench_support` (§3.2: `bench` never depends
/// on `core-*` directly).
fn range(s: &str, board: &[proto::Card]) -> Range1326 {
    engine::bench_support::prepared_range(s, board).expect("range")
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Builds the wire `SolveRequest` for `spot` at `deadline_ms` (already computed by the caller from
/// the street budget). `spot.history` is an actor-labelled prefix from the street root (0 = oop acts
/// first, alternating) — every spot this plan's suites generate has an empty history, but the field
/// exists on `Spot` for later suites that do not.
pub fn request(spot: &Spot, deadline_ms: u32) -> SolveRequest {
    let t = Templates::get(&spot.template_id).expect("template");
    let prefix: Vec<(usize, proto::Action)> = spot.history.iter().enumerate().map(|(i, a)| (i % 2, a.clone())).collect();
    let b = materialize_at(t, spot.pot, spot.stack_oop.min(spot.stack_ip), &prefix).expect("materialize");
    let (rake_rate, rake_cap_mchips) = match spot.rake {
        Rake::PotRake { rate, cap_mchips, .. } => (rate, cap_mchips),
        Rake::TimeCharge => (0.0, 0),
    };
    SolveRequest {
        id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst).to_string(),
        spot: spot.id.clone(),
        board: spot.board.clone(),
        oop_range: range(&spot.oop_range, &spot.board),
        ip_range: range(&spot.ip_range, &spot.board),
        pot: spot.pot,
        stack_oop: spot.stack_oop,
        stack_ip: spot.stack_ip,
        rake_rate,
        rake_cap_mchips,
        tree: b.tree,
        history: b.history,
        target_bp: spot.target_bp,
        deadline_ms,
        extraction_margin_ms: extraction_margin_ms(spot.root_street),
        memory_limit_bytes: 10 << 30,
        background: false,
    }
}

/// Runs one rep of `spot` to a terminal `Result`, measuring wall time (send to terminal) and ack
/// latency (send to the first `Ack`). `street_violation`/`final_violation` are the §7 wall-clock
/// bounds this suite (river/turn) is scored against; they are never a pass/fail gate inside this
/// function, only a fact recorded on the row for `report::Report` to count.
pub fn run_spot(worker: &mut dyn WorkerLink, spot: &Spot, rep: u32, cold: bool) -> SpotResult {
    let budget = street_budget_ms(spot.root_street, 10);
    let req = request(spot, (budget - 150) as u32);
    let id = req.id.clone();
    let t = Instant::now();
    worker.send(&EngineMessage::Solve(req)).expect("send");
    let (mut ack_ms, mut res) = (0u64, None);
    while res.is_none() {
        match worker.recv(Duration::from_secs(60)).expect("worker alive") {
            Some(WorkerMessage::Ack { id: i, .. }) if i == id => ack_ms = t.elapsed().as_millis() as u64,
            Some(WorkerMessage::Result { id: i, status, solution, error, .. }) if i == id => res = Some((status, solution, error)),
            Some(_) => {}
            None => break,
        }
    }
    let wall_ms = t.elapsed().as_millis() as u64;
    let (status, solution, _) = res.unwrap_or((ResultStatus::Error, None, None));
    let status_s = match status {
        ResultStatus::Ok => "ok",
        ResultStatus::BestSoFar => "best_so_far",
        ResultStatus::Cancelled => "cancelled",
        ResultStatus::Error => "error",
    }
    .to_string();
    // §4.4 accuracy vocabulary in bp: reached exploitability as a fraction of the pot, in basis points.
    let reached_bp = solution.as_ref().map(|s| (10_000.0 * s.exploitability_chips / spot.pot as f32).round() as u16);
    SpotResult {
        spot: spot.id.clone(),
        rep,
        cold,
        wall_ms,
        ack_ms,
        status: status_s,
        reached_bp,
        iterations: solution.as_ref().map(|s| s.iterations).unwrap_or(0),
        memory_bytes: solution.as_ref().map(|s| s.memory_bytes).unwrap_or(0),
        peak_ws_bytes: worker.peak_working_set_bytes(),
        mode: solution.as_ref().map(|s| s.mode.clone()).unwrap_or_default(),
        street_violation: wall_ms > budget,
        final_violation: wall_ms > 15_000,
    }
}

/// §13.5 "cancel latency": send a solve with a long deadline, wait for the FIRST
/// `progress{stage:"solving"}` (a fixed sleep would let a fast river spot finish first and record a
/// misleading 0/0), then cancel and measure the ack and the terminal. `None` when the solve
/// terminated before the cancel could be issued: the report then prints `n/a` rather than a
/// misleading zero.
pub fn cancel_latency(worker: &mut dyn WorkerLink, spot: &Spot) -> Option<(u64, u64)> {
    let req = request(spot, 30_000);
    let id = req.id.clone();
    worker.send(&EngineMessage::Solve(req)).expect("send");
    let mut finished = false;
    loop {
        match worker.recv(Duration::from_secs(30)).expect("worker alive") {
            Some(WorkerMessage::Progress { id: i, stage: Stage::Solving, iterations, .. }) if i == id && iterations >= 1 => break,
            Some(WorkerMessage::Result { id: i, .. }) if i == id => {
                finished = true;
                break;
            }
            Some(_) => {}
            None => {
                finished = true;
                break;
            }
        }
    }
    if finished {
        return None;
    }
    let t = Instant::now();
    let cid = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst).to_string();
    worker.send(&EngineMessage::Cancel { id: cid.clone(), target: id.clone() }).expect("send");
    let (mut ack, mut result) = (0u64, 0u64);
    loop {
        match worker.recv(Duration::from_secs(10)).expect("worker alive") {
            Some(WorkerMessage::Ack { id: i, .. }) if i == cid => ack = t.elapsed().as_millis() as u64,
            Some(WorkerMessage::Result { id: i, .. }) if i == id => {
                result = t.elapsed().as_millis() as u64;
                break;
            }
            Some(_) => {}
            None => break,
        }
    }
    Some((ack, result))
}
