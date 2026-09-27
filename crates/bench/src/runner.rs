//! `bench run` execution: builds a `SolveRequest` for one suite spot (§4.6/§10.2 materialization,
//! §3.2 dependency direction through `engine::bench_support` — no `core-*` dependency here), drives
//! it against a live `WorkerLink` to a terminal `Result`, and separately measures §7/§13.5 "cancel
//! latency" (send a solve with a long deadline, cancel it once the worker has visibly started
//! solving, and time the ack and the terminal).
//!
//! Time comes from an injected `engine::clock::Clock` (the CLI passes a `SystemClock`, the tests the
//! engine's `FakeClock`), and every wait is bounded by one absolute window on that clock: a message
//! that belongs to nothing being waited for never extends it. What the protocol does not allow, or a
//! reply that never comes, is a `RunError` that reaches the CLI, never a row of zeros (spec 4.5: a
//! rejected solve starts no work and has no terminal; every accepted solve has exactly one terminal).

use crate::suite::Spot;
use engine::clock::Clock;
use engine::deadline::{extraction_margin_ms, final_delivery_ms, street_budget_ms, DELIVERY_MARGIN_MS, PIPE_MARGIN_MS};
use engine::tree::{materialize_at, Templates};
use engine::worker::link::WorkerLink;
use proto::worker::{AckStatus, EngineMessage, ResultStatus, SolveRequest, Stage, WorkerError, WorkerMessage};
use proto::{Rake, Range1326, Street};
use std::time::Duration;

/// §7: the suites run at the default flop budget (only a flop street budget reads it).
pub const FLOP_BUDGET_S: u8 = 10;
/// Absolute window for one measured solve, from its send to its terminal. Wider than every §7 final
/// delivery (15 s river/turn, 5 s + the flop budget), so a late terminal is still measured, and counted
/// as the violation it is, rather than lost.
pub const RUN_WINDOW_MS: u64 = 60_000;
/// The cancel probe's solve deadline: long enough that the probe cancels a solve still running.
pub const CANCEL_PROBE_DEADLINE_MS: u32 = 30_000;
/// Cancel probe: absolute window from the probe solve's send to its first `progress{solving}` after an
/// iteration (or its terminal, when it finishes first).
pub const PROGRESS_WINDOW_MS: u64 = 30_000;
/// Cancel probe: absolute window from the cancel's send to both its ack and the solve's terminal.
pub const CANCEL_WINDOW_MS: u64 = 10_000;

/// One (spot, rep) measurement of a `bench run`. `§13.5` columns are built from a whole suite's rows
/// by `report::Report`; this is the per-attempt record it accumulates.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpotResult {
    /// The spot's human-readable name (`srp100_dry`); the wire request carries its structural identity.
    pub spot: String,
    pub rep: u32,
    pub cold: bool,
    /// Send to terminal, on the injected clock.
    pub wall_ms: u64,
    /// Send to the solve's `ack{accepted}`: always observed (a terminal without it is a protocol failure).
    pub ack_ms: u64,
    pub status: String,
    /// Display only: the reached exploitability in bp of the pot, rounded; never compared with anything.
    pub reached_bp: Option<u16>,
    /// A solution arrived whose raw exploitability meets the raw target (`reached_target`).
    pub at_target: bool,
    pub iterations: u32,
    pub memory_bytes: u64,
    pub peak_ws_bytes: u64,
    pub mode: String,
    /// `street_violation(street, wall_ms, at_target)`.
    pub street_violation: bool,
    /// The terminal arrived after the §7 final delivery.
    pub final_violation: bool,
}

/// Why a measurement could not be taken. Each is a benchmark failure the CLI reports with its exit code;
/// none is ever turned into a measured row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The worker answered a solve with `ack{rejected, reason}`: no work started and no terminal will follow.
    Rejected { request: String, reason: String },
    /// The link refused a request or failed to deliver the next message.
    Link(String),
    /// A reply the protocol does not allow at that point, or a probe solve that ended in `error`.
    Protocol(String),
    /// An expected reply did not arrive within its absolute observation window.
    Timeout(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Rejected { request, reason } => {
                write!(f, "{request} was rejected by the worker (ack rejected, reason: {reason}); a rejected solve starts no work and has no terminal")
            }
            RunError::Link(m) | RunError::Protocol(m) | RunError::Timeout(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for RunError {}

/// What the §13.5 cancel probe observed, when it observed something the protocol allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The cancel was acknowledged `accepted` and the solve's terminal was `cancelled`: both latencies are
    /// measured from the cancel's send. The only outcome with a numeric cancel latency.
    Cancelled { ack_ms: u64, result_ms: u64 },
    /// The solve reached a valid terminal (`ok` or `best_so_far`) before a cancel could take effect: either
    /// before one was sent (`cancel_ack` is `None`), or racing it (spec 4.5: the completion wins and the cancel
    /// is answered `already_finished`, or `accepted` followed by the valid result). There is no cancel latency.
    CompletedBeforeCancel { terminal: ResultStatus, cancel_ack: Option<AckStatus> },
}

/// A result status as its wire tag (spec 4.5).
pub fn result_status_name(s: ResultStatus) -> &'static str {
    match s {
        ResultStatus::Ok => "ok",
        ResultStatus::BestSoFar => "best_so_far",
        ResultStatus::Cancelled => "cancelled",
        ResultStatus::Error => "error",
    }
}

/// An ack status as its wire tag (spec 4.5).
pub fn ack_status_name(s: AckStatus) -> &'static str {
    match s {
        AckStatus::Accepted => "accepted",
        AckStatus::Staged => "staged",
        AckStatus::Rejected => "rejected",
        AckStatus::AlreadyFinished => "already_finished",
        AckStatus::UnknownTarget => "unknown_target",
    }
}

/// Range parsing and board blocking go through `engine::bench_support` (§3.2: `bench` never depends
/// on `core-*` directly).
fn range(s: &str, board: &[proto::Card]) -> Range1326 {
    engine::bench_support::prepared_range(s, board).expect("range")
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_id() -> String {
    NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst).to_string()
}

/// The worker `deadline_ms` of a first attempt on `street` sent at the admission time (§7: the street budget
/// less the delivery and pipe margins).
pub fn worker_deadline_ms(street: Street) -> u32 {
    let d = street_budget_ms(street, FLOP_BUDGET_S)
        .checked_sub(DELIVERY_MARGIN_MS + PIPE_MARGIN_MS)
        .unwrap_or_else(|| panic!("{street:?} has no street budget to solve in"));
    u32::try_from(d).expect("a street budget fits the u32 wire field")
}

/// The budgets a suite on `street` is scored against, and what counts as a violation of them.
pub fn budget_summary(street: Street) -> String {
    format!(
        "street {} ms (worker deadline_ms {}, extraction margin {} ms), final delivery {} ms; a street violation is a terminal after the street budget or, on the river and turn, one without a solution at the raw target",
        street_budget_ms(street, FLOP_BUDGET_S),
        worker_deadline_ms(street),
        extraction_margin_ms(street),
        final_delivery_ms(street, FLOP_BUDGET_S)
    )
}

/// The raw target comparison of spec 4.4/5 step 7, never on rounded basis points: `exploitability_chips / pot <=
/// target_bp / 10_000`, evaluated as `exploitability * 10_000 <= target_bp * pot` in `f64`, where both products are
/// exact. The same comparison as `engine::assemble::coverage_for_solve`.
pub fn reached_target(exploitability_chips: f32, pot: u32, target_bp: u16) -> bool {
    f64::from(exploitability_chips) * 10_000.0 <= f64::from(target_bp) * f64::from(pot)
}

/// Whether a first-attempt terminal on `street` that arrived `wall_ms` after the send violates the street contract:
/// - late: after the §7 street budget, on every street;
/// - on the river and the turn, also a terminal without a solution at the raw target (`at_target == false`: a
///   `best_so_far`, an `ok` above the raw target, an `error` or a `cancelled`), however early: the street budget is
///   the time to *reach the target* (spec 13.5), and spec 13.3's `deadline_best_so_far_labelling` logs a turn
///   `best_so_far` as a violation. The single-raised-pot flop miss is the one designed no-target outcome (spec 7);
///   the flop suites and that exception are plan 4's, so a flop terminal is judged by its lateness only here.
pub fn street_violation(street: Street, wall_ms: u64, at_target: bool) -> bool {
    let late = wall_ms > street_budget_ms(street, FLOP_BUDGET_S);
    let target_required = matches!(street, Street::River | Street::Turn);
    late || (target_required && !at_target)
}

/// Waits for the next message until the absolute `deadline_ms` on `clock`; `Ok(None)` once it has passed. Every
/// receive is bounded by what is left of that one window, so no message, related or not, ever extends it.
fn next_before(worker: &mut dyn WorkerLink, clock: &dyn Clock, deadline_ms: u64) -> Result<Option<WorkerMessage>, RunError> {
    loop {
        let now = clock.now_ms();
        if now >= deadline_ms {
            return Ok(None);
        }
        match worker.recv(Duration::from_millis(deadline_ms - now)) {
            Ok(Some(msg)) => return Ok(Some(msg)),
            Ok(None) => {}
            Err(e) => return Err(RunError::Link(format!("receiving from the worker: {e}"))),
        }
    }
}

fn rejected(what: &str, reason: Option<String>) -> RunError {
    RunError::Rejected { request: what.to_string(), reason: reason.unwrap_or_else(|| "none given".into()) }
}

fn describe(error: Option<WorkerError>) -> String {
    error.map_or_else(|| " (no error object)".into(), |e| format!(": {} ({})", e.code, e.message))
}

/// Builds the wire `SolveRequest` for `spot` at `deadline_ms` (already computed by the caller from
/// the street budget). `spot.history` is an actor-labelled prefix from the street root (0 = oop acts
/// first, alternating) — every spot this plan's suites generate has an empty history, but the field
/// exists on `Spot` for later suites that do not.
///
/// `SolveRequest.spot` is spec 4.5's sha256 hex of the structural identity of the solved game
/// (`engine::bench_support::spot_identity`), never the display name: `srp100_dry` names four different
/// games across the four suites.
pub fn request(spot: &Spot, deadline_ms: u32) -> SolveRequest {
    let t = Templates::get(&spot.template_id).expect("template");
    let prefix: Vec<(usize, proto::Action)> = spot.history.iter().enumerate().map(|(i, a)| (i % 2, a.clone())).collect();
    let b = materialize_at(t, spot.pot, spot.stack_oop.min(spot.stack_ip), &prefix).expect("materialize");
    let (rake_rate, rake_cap_mchips) = match spot.rake {
        Rake::PotRake { rate, cap_mchips, .. } => (rate, cap_mchips),
        Rake::TimeCharge => (0.0, 0),
    };
    let mut req = SolveRequest {
        id: next_id(),
        spot: String::new(),
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
    };
    req.spot = engine::bench_support::spot_identity(&req);
    req
}

/// Runs one rep of `spot` to its terminal `Result` within `RUN_WINDOW_MS` of the send, measuring the wall time
/// (send to terminal) and the ack latency (send to `ack{accepted}`) on `clock`. `street_violation`/`final_violation`
/// are facts recorded on the row for `report::Report` to count, never a pass/fail gate here.
///
/// A rejected solve fails at once with its reason; a terminal before the solve's ack, an unexpected ack status and a
/// missing terminal are failures too: none of them is a measurement.
pub fn run_spot(worker: &mut dyn WorkerLink, clock: &dyn Clock, spot: &Spot, rep: u32, cold: bool) -> Result<SpotResult, RunError> {
    let street = spot.root_street;
    let req = request(spot, worker_deadline_ms(street));
    let (id, pot, target_bp) = (req.id.clone(), req.pot, req.target_bp);
    let what = format!("solve {id} (spot {})", spot.id);
    let t0 = clock.now_ms();
    worker.send(&EngineMessage::Solve(req)).map_err(|e| RunError::Link(format!("sending {what}: {e}")))?;
    let deadline = t0 + RUN_WINDOW_MS;
    let mut ack_ms: Option<u64> = None;
    let (status, solution) = loop {
        match next_before(worker, clock, deadline)? {
            None => return Err(RunError::Timeout(format!("{what}: no terminal result within {RUN_WINDOW_MS} ms of the send"))),
            Some(WorkerMessage::Ack { id: i, status, reason, .. }) if i == id => match (status, ack_ms) {
                (AckStatus::Accepted, None) => ack_ms = Some(clock.now_ms() - t0),
                (AckStatus::Rejected, _) => return Err(rejected(&what, reason)),
                (other, _) => return Err(RunError::Protocol(format!("{what}: unexpected ack {} for a solve", ack_status_name(other)))),
            },
            Some(WorkerMessage::Result { id: i, status, solution, .. }) if i == id => {
                if ack_ms.is_none() {
                    return Err(RunError::Protocol(format!("{what}: its terminal result arrived before the solve was acknowledged")));
                }
                break (status, solution);
            }
            Some(_) => {}
        }
    };
    let wall_ms = clock.now_ms() - t0;
    let ack_ms = ack_ms.expect("a terminal before the ack returned above");
    let at_target = solution.as_ref().is_some_and(|s| reached_target(s.exploitability_chips, pot, target_bp));
    // Display only (§4.4 accuracy vocabulary): the reached exploitability in bp of the pot, saturating at u16::MAX.
    let reached_bp = solution.as_ref().map(|s| {
        let bp = (f64::from(s.exploitability_chips) * 10_000.0 / f64::from(pot)).round();
        if bp <= f64::from(u16::MAX) { bp as u16 } else { u16::MAX }
    });
    Ok(SpotResult {
        spot: spot.id.clone(),
        rep,
        cold,
        wall_ms,
        ack_ms,
        status: result_status_name(status).to_string(),
        reached_bp,
        at_target,
        iterations: solution.as_ref().map(|s| s.iterations).unwrap_or(0),
        memory_bytes: solution.as_ref().map(|s| s.memory_bytes).unwrap_or(0),
        peak_ws_bytes: worker.peak_working_set_bytes(),
        mode: solution.as_ref().map(|s| s.mode.clone()).unwrap_or_default(),
        street_violation: street_violation(street, wall_ms, at_target),
        final_violation: wall_ms > final_delivery_ms(street, FLOP_BUDGET_S),
    })
}

/// A terminal that came before any cancel could take effect: `ok`/`best_so_far` is a completion (no cancel latency
/// exists); an `error` is the probe failing; a `cancelled` with no cancel sent is not something the protocol allows.
fn completed_before_cancel(what: &str, terminal: ResultStatus, error: Option<WorkerError>, cancel_ack: Option<AckStatus>) -> Result<CancelOutcome, RunError> {
    match terminal {
        ResultStatus::Ok | ResultStatus::BestSoFar => Ok(CancelOutcome::CompletedBeforeCancel { terminal, cancel_ack }),
        ResultStatus::Error => Err(RunError::Protocol(format!("{what}: the solve ended in error{}", describe(error)))),
        ResultStatus::Cancelled => Err(RunError::Protocol(format!("{what}: a cancelled terminal although no cancel took effect"))),
    }
}

/// §13.5 "cancel latency": sends a solve with a long deadline, waits (within `PROGRESS_WINDOW_MS` of the send) for its
/// first `progress{solving}` after an iteration, then cancels it and observes, within `CANCEL_WINDOW_MS` of the
/// cancel, both the cancel's ack and the solve's terminal, whichever order they come in. The matching cancel ack is
/// always consumed, so the link is left clean even when the terminal arrives first.
///
/// Observing progress does not prove the solve is still running (spec 4.5: a completion racing a cancel yields the
/// valid result or `cancelled`, never both), so the outcome is classified from what was observed:
/// - `ack{accepted}` and `result{cancelled}`: `Cancelled`, the only numeric latency;
/// - a valid terminal before the cancel, or racing it (`already_finished`, or `accepted` then the valid result):
///   `CompletedBeforeCancel` (n/a);
/// - a missing ack or terminal at the end of a window, a rejected solve, an `error` terminal, or any other
///   combination: a `RunError`, never a latency.
pub fn cancel_latency(worker: &mut dyn WorkerLink, clock: &dyn Clock, spot: &Spot) -> Result<CancelOutcome, RunError> {
    let req = request(spot, CANCEL_PROBE_DEADLINE_MS);
    let id = req.id.clone();
    let what = format!("cancel probe solve {id} (spot {})", spot.id);
    let t_send = clock.now_ms();
    worker.send(&EngineMessage::Solve(req)).map_err(|e| RunError::Link(format!("sending {what}: {e}")))?;
    let progress_deadline = t_send + PROGRESS_WINDOW_MS;
    let mut accepted = false;
    loop {
        match next_before(worker, clock, progress_deadline)? {
            None => {
                return Err(RunError::Timeout(format!(
                    "{what}: neither a progress{{solving}} after an iteration nor a terminal within {PROGRESS_WINDOW_MS} ms of the send"
                )))
            }
            Some(WorkerMessage::Ack { id: i, status, reason, .. }) if i == id => match (status, accepted) {
                (AckStatus::Accepted, false) => accepted = true,
                (AckStatus::Rejected, _) => return Err(rejected(&what, reason)),
                (other, _) => return Err(RunError::Protocol(format!("{what}: unexpected ack {} for a solve", ack_status_name(other)))),
            },
            Some(WorkerMessage::Progress { id: i, stage: Stage::Solving, iterations, .. }) if i == id && iterations >= 1 => {
                if !accepted {
                    return Err(RunError::Protocol(format!("{what}: progress arrived before the solve was acknowledged")));
                }
                break;
            }
            Some(WorkerMessage::Result { id: i, status, error, .. }) if i == id => {
                if !accepted {
                    return Err(RunError::Protocol(format!("{what}: its terminal result arrived before the solve was acknowledged")));
                }
                return completed_before_cancel(&what, status, error, None);
            }
            Some(_) => {}
        }
    }
    let cid = next_id();
    let t_cancel = clock.now_ms();
    worker.send(&EngineMessage::Cancel { id: cid.clone(), target: id.clone() }).map_err(|e| RunError::Link(format!("sending cancel {cid} for {what}: {e}")))?;
    let deadline = t_cancel + CANCEL_WINDOW_MS;
    let mut ack: Option<(AckStatus, Option<String>, u64)> = None;
    let mut terminal: Option<(ResultStatus, Option<WorkerError>, u64)> = None;
    while ack.is_none() || terminal.is_none() {
        match next_before(worker, clock, deadline)? {
            None => break,
            Some(WorkerMessage::Ack { id: i, status, reason, .. }) if i == cid && ack.is_none() => ack = Some((status, reason, clock.now_ms() - t_cancel)),
            Some(WorkerMessage::Result { id: i, status, error, .. }) if i == id && terminal.is_none() => {
                terminal = Some((status, error, clock.now_ms() - t_cancel))
            }
            Some(_) => {}
        }
    }
    match (ack, terminal) {
        (None, None) => Err(RunError::Timeout(format!(
            "{what}: neither the ack of cancel {cid} nor the solve's terminal within {CANCEL_WINDOW_MS} ms of the cancel"
        ))),
        (None, Some((status, _, ms))) => Err(RunError::Timeout(format!(
            "{what}: no ack for cancel {cid} within {CANCEL_WINDOW_MS} ms of the cancel (the solve's terminal {} arrived after {ms} ms)",
            result_status_name(status)
        ))),
        (Some((status, _, ms)), None) => Err(RunError::Timeout(format!(
            "{what}: no terminal for the solve within {CANCEL_WINDOW_MS} ms of cancel {cid} (its ack {} arrived after {ms} ms)",
            ack_status_name(status)
        ))),
        (Some((AckStatus::Accepted, _, ack_ms)), Some((ResultStatus::Cancelled, _, result_ms))) => Ok(CancelOutcome::Cancelled { ack_ms, result_ms }),
        (Some((a @ (AckStatus::Accepted | AckStatus::AlreadyFinished), _, _)), Some((t, error, _))) if t != ResultStatus::Cancelled => {
            completed_before_cancel(&what, t, error, Some(a))
        }
        (Some((a, reason, _)), Some((t, _, _))) => Err(RunError::Protocol(format!(
            "{what}: cancel {cid} was answered {}{} and the solve ended {}: not an outcome the protocol allows",
            ack_status_name(a),
            reason.map_or_else(String::new, |r| format!(" ({r})")),
            result_status_name(t)
        ))),
    }
}

#[cfg(test)]
mod tests {
    //! Protocol and deadline behaviour against the engine's scripted worker and fake clock (Task 19): every
    //! latency below is fake-clock time, so the assertions are exact and never depend on machine speed.
    use super::*;
    use crate::gen_spots;
    use engine::identity::IdentityState;
    use engine::testing::{uniform_solution, FakeClock, FakeReply, FakeState, FakeWorker, IdRef};
    use proto::worker::WorkerError;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    fn spot(suite: &str, id: &str) -> Spot {
        gen_spots::generate(suite, "r8").unwrap().spots.into_iter().find(|s| s.id == id).unwrap()
    }
    fn river() -> Spot { spot("river_std", "srp100_dry") }
    fn turn() -> Spot { spot("turn_std", "srp100_dry") }

    fn fake(script: Vec<FakeReply>) -> (Box<FakeWorker>, Arc<Mutex<FakeState>>, Arc<FakeClock>) {
        let clock = FakeClock::new();
        let (w, state) = FakeWorker::scripted(clock.clone(), Arc::new(Mutex::new(IdentityState::new())), script);
        (w, state, clock)
    }
    /// An ack for the last request sent (resolved when `recv` reads it).
    fn ack(status: AckStatus) -> FakeReply { FakeReply::Ack { id: IdRef::Last, status, reason: None } }
    fn rejected(reason: &str) -> FakeReply { FakeReply::Ack { id: IdRef::Last, status: AckStatus::Rejected, reason: Some(reason.into()) } }
    fn solving(iterations: u32) -> FakeReply {
        FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations, exploitability_chips: Some(1.0), elapsed_ms: 1 }
    }
    fn delay(ms: u64) -> FakeReply { FakeReply::Delay { ms } }
    /// A terminal for the last solve sent; `exploitability` gives it a valid solution over `spot`'s tree.
    fn result(spot: &Spot, status: ResultStatus, exploitability: Option<f32>) -> FakeReply {
        let solution = exploitability.map(|e| uniform_solution(&request(spot, 1).tree, &[], e));
        FakeReply::Result { id: IdRef::Last, status, solution, error: None, elapsed_ms: 1 }
    }
    fn unread(w: &mut FakeWorker) -> Option<WorkerMessage> { w.recv(Duration::ZERO).unwrap() }

    // ---- I4: `SolveRequest.spot` is the structural identity, `SpotResult.spot` the display name ----

    fn is_sha256_hex(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }

    #[test]
    fn the_wire_spot_is_a_repeatable_sha256_hex_identity_independent_of_id_and_deadline() {
        let s = river();
        let (a, b, c) = (request(&s, 1_850), request(&s, 1_850), request(&s, CANCEL_PROBE_DEADLINE_MS));
        assert!(is_sha256_hex(&a.spot), "not 64 lowercase hex characters: {:?}", a.spot);
        assert_ne!(a.id, b.id, "every request carries a fresh id");
        assert_eq!(a.spot, b.spot, "the same spot must hash to the same identity");
        assert_eq!(a.spot, c.spot, "the deadline is a solve parameter, not part of the structural identity");
    }

    #[test]
    fn structurally_different_spots_sharing_a_display_id_get_distinct_identities() {
        let across_suites: HashSet<String> =
            ["river_std", "river_min", "turn_std", "turn_min"].iter().map(|suite| request(&spot(suite, "srp100_dry"), 1_850).spot).collect();
        assert_eq!(across_suites.len(), 4, "srp100_dry in four suites is four different solves: {across_suites:?}");
        // srp100_dry and srp200_dry share the template, board, pot and ranges and differ only in the stacks
        // (882 vs 1882 chips): a different game with a different materialized tree.
        assert_ne!(request(&spot("river_std", "srp100_dry"), 1_850).spot, request(&spot("river_std", "srp200_dry"), 1_850).spot);
    }

    // ---- I2: the street-violation predicate ----

    #[test]
    fn the_raw_target_comparison_never_reads_display_bp() {
        assert!(reached_target(1.0, 241, 50));
        assert!(reached_target(0.5, 100, 50), "exactly at the raw target reaches it");
        assert!(!reached_target(0.5001, 100, 50));
        // 1.21 chips of a 241-chip pot is 50.2 bp, displayed as 50 bp, yet above the raw target of 1.205 chips.
        assert_eq!((10_000.0f64 * 1.21 / 241.0).round(), 50.0);
        assert!(!reached_target(1.21, 241, 50));
    }

    #[test]
    fn a_river_or_turn_row_violates_the_street_budget_when_late_or_off_target() {
        // below budget, above target: the spec 13.3 `deadline_best_so_far_labelling` case
        assert!(street_violation(Street::River, 1_500, false));
        assert!(street_violation(Street::Turn, 5_500, false));
        // at target, timely: no violation (arriving exactly at the budget is not late)
        assert!(!street_violation(Street::River, 1_500, true));
        assert!(!street_violation(Street::River, 2_000, true));
        assert!(!street_violation(Street::Turn, 5_900, true));
        // late, even at target
        assert!(street_violation(Street::River, 2_001, true));
        assert!(street_violation(Street::Turn, 6_001, true));
        // the flop's no-target exception (single-raised-pot miss) belongs to plan 4's suites: only lateness here
        assert!(!street_violation(Street::Flop, 5_000, false));
        assert!(street_violation(Street::Flop, 10_001, true));
    }

    // ---- run_spot: I5 (rejected), I2 (violations through a measured row), bounded waits ----

    #[test]
    fn a_rejected_solve_fails_at_once_with_its_reason_and_never_waits_for_a_terminal() {
        let s = river();
        let (mut w, _state, clock) = fake(vec![rejected("busy"), FakeReply::Hang]);
        let err = run_spot(&mut *w, &*clock, &s, 1, true).unwrap_err();
        match &err {
            RunError::Rejected { reason, .. } => assert_eq!(reason, "busy"),
            other => panic!("expected a rejection, got {other:?}"),
        }
        assert!(err.to_string().contains("busy"), "the reason reaches the CLI message: {err}");
        assert_eq!(clock.now_ms(), 0, "no wait after the rejection");
    }

    #[test]
    fn an_at_target_timely_row_keeps_the_display_name_and_is_no_violation() {
        let s = river();
        let (mut w, state, clock) = fake(vec![ack(AckStatus::Accepted), delay(30), result(&s, ResultStatus::Ok, Some(1.0))]);
        let row = run_spot(&mut *w, &*clock, &s, 1, true).unwrap();
        assert_eq!(row.spot, "srp100_dry");
        let EngineMessage::Solve(sent) = &state.lock().unwrap().sent[0] else { panic!("a solve is sent first") };
        assert!(is_sha256_hex(&sent.spot), "{:?}", sent.spot);
        assert_eq!((row.wall_ms, row.ack_ms, row.status.as_str()), (30, 0, "ok"));
        assert!(row.at_target && !row.street_violation && !row.final_violation, "{row:?}");
        assert_eq!(row.reached_bp, Some(41));
    }

    #[test]
    fn a_below_budget_best_so_far_above_target_is_a_street_violation() {
        let s = river();
        // 4.58 chips of a 241-chip pot: 190 bp against a 50 bp target, delivered at 1.5 s of a 2 s budget
        let (mut w, _state, clock) = fake(vec![ack(AckStatus::Accepted), delay(1_500), result(&s, ResultStatus::BestSoFar, Some(4.58))]);
        let row = run_spot(&mut *w, &*clock, &s, 1, true).unwrap();
        assert_eq!((row.wall_ms, row.status.as_str(), row.reached_bp), (1_500, "best_so_far", Some(190)));
        assert!(!row.at_target && row.street_violation && !row.final_violation, "{row:?}");
    }

    #[test]
    fn a_turn_best_so_far_inside_its_budget_is_a_street_violation() {
        let s = turn();
        let (mut w, _state, clock) = fake(vec![ack(AckStatus::Accepted), delay(5_500), result(&s, ResultStatus::BestSoFar, Some(2.185))]);
        let row = run_spot(&mut *w, &*clock, &s, 1, true).unwrap();
        assert!(!row.at_target && row.street_violation && !row.final_violation, "{row:?}");
    }

    #[test]
    fn an_ok_above_the_raw_target_is_a_street_violation_even_when_its_display_bp_equals_the_target() {
        let s = river();
        let (mut w, _state, clock) = fake(vec![ack(AckStatus::Accepted), delay(40), result(&s, ResultStatus::Ok, Some(1.21))]);
        let row = run_spot(&mut *w, &*clock, &s, 1, true).unwrap();
        assert_eq!(row.reached_bp, Some(50));
        assert!(!row.at_target && row.street_violation, "{row:?}");
    }

    #[test]
    fn a_late_row_is_a_street_violation_and_past_final_delivery_a_final_violation() {
        let s = river();
        let (mut w, _state, clock) = fake(vec![ack(AckStatus::Accepted), delay(2_100), result(&s, ResultStatus::Ok, Some(1.0))]);
        let row = run_spot(&mut *w, &*clock, &s, 1, true).unwrap();
        assert!(row.at_target && row.street_violation && !row.final_violation, "{row:?}");
        let (mut w, _state, clock) = fake(vec![ack(AckStatus::Accepted), delay(15_001), result(&s, ResultStatus::Ok, Some(1.0))]);
        let row = run_spot(&mut *w, &*clock, &s, 1, true).unwrap();
        assert!(row.street_violation && row.final_violation, "{row:?}");
    }

    #[test]
    fn an_accepted_solve_without_a_terminal_fails_at_the_end_of_its_window() {
        let s = river();
        let (mut w, _state, clock) = fake(vec![ack(AckStatus::Accepted), FakeReply::Hang]);
        let err = run_spot(&mut *w, &*clock, &s, 1, true).unwrap_err();
        assert!(matches!(err, RunError::Timeout(_)), "{err:?}");
        assert_eq!(clock.now_ms(), RUN_WINDOW_MS, "one absolute window from the send");
    }

    #[test]
    fn a_terminal_before_the_solve_ack_is_a_protocol_failure() {
        let s = river();
        let (mut w, _state, clock) = fake(vec![result(&s, ResultStatus::Ok, Some(1.0))]);
        let err = run_spot(&mut *w, &*clock, &s, 1, true).unwrap_err();
        assert!(matches!(err, RunError::Protocol(_)), "{err:?}");
    }

    // ---- I1: the cancel probe ----

    #[test]
    fn a_successful_cancellation_measures_the_ack_and_the_terminal_from_the_cancel() {
        let s = river();
        let (mut w, state, clock) = fake(vec![
            ack(AckStatus::Accepted), solving(1), delay(4), ack(AckStatus::Accepted), delay(120), result(&s, ResultStatus::Cancelled, None),
        ]);
        let got = cancel_latency(&mut *w, &*clock, &s).unwrap();
        assert_eq!(got, CancelOutcome::Cancelled { ack_ms: 4, result_ms: 124 });
        assert_eq!(state.lock().unwrap().cancels.len(), 1);
        assert!(unread(&mut w).is_none());
    }

    #[test]
    fn a_terminal_before_an_already_finished_ack_is_completed_before_cancel_and_the_ack_is_consumed() {
        let s = river();
        // the solve finished after its queued progress{solving} and before the worker handled the cancel
        let (mut w, _state, clock) = fake(vec![
            ack(AckStatus::Accepted), solving(1), result(&s, ResultStatus::Ok, Some(1.0)), ack(AckStatus::AlreadyFinished),
        ]);
        let got = cancel_latency(&mut *w, &*clock, &s).unwrap();
        assert_eq!(got, CancelOutcome::CompletedBeforeCancel { terminal: ResultStatus::Ok, cancel_ack: Some(AckStatus::AlreadyFinished) });
        assert!(unread(&mut w).is_none(), "the matching cancel ack must not be left on the link");
    }

    #[test]
    fn a_solve_that_finishes_before_any_progress_is_completed_before_cancel_and_no_cancel_is_sent() {
        let s = river();
        let (mut w, state, clock) = fake(vec![ack(AckStatus::Accepted), result(&s, ResultStatus::Ok, Some(1.0))]);
        let got = cancel_latency(&mut *w, &*clock, &s).unwrap();
        assert_eq!(got, CancelOutcome::CompletedBeforeCancel { terminal: ResultStatus::Ok, cancel_ack: None });
        assert!(state.lock().unwrap().cancels.is_empty());
    }

    #[test]
    fn a_missing_cancel_ack_is_a_failure_at_the_end_of_the_window() {
        let s = river();
        let (mut w, _state, clock) = fake(vec![ack(AckStatus::Accepted), solving(1), delay(5), result(&s, ResultStatus::Cancelled, None), FakeReply::Hang]);
        let err = cancel_latency(&mut *w, &*clock, &s).unwrap_err();
        assert!(matches!(&err, RunError::Timeout(m) if m.contains("no ack")), "{err:?}");
        assert_eq!(clock.now_ms(), CANCEL_WINDOW_MS);
    }

    #[test]
    fn a_missing_terminal_after_the_cancel_ack_is_a_failure_at_the_end_of_the_window() {
        let s = river();
        let (mut w, _state, clock) = fake(vec![ack(AckStatus::Accepted), solving(1), delay(3), ack(AckStatus::Accepted), FakeReply::Hang]);
        let err = cancel_latency(&mut *w, &*clock, &s).unwrap_err();
        assert!(matches!(&err, RunError::Timeout(m) if m.contains("no terminal")), "{err:?}");
        assert_eq!(clock.now_ms(), CANCEL_WINDOW_MS);
    }

    #[test]
    fn a_timeout_before_progress_is_a_failure_not_a_completed_solve() {
        let s = river();
        let (mut w, state, clock) = fake(vec![ack(AckStatus::Accepted), FakeReply::Hang]);
        let err = cancel_latency(&mut *w, &*clock, &s).unwrap_err();
        assert!(matches!(&err, RunError::Timeout(m) if m.contains("progress")), "{err:?}");
        assert_eq!(clock.now_ms(), PROGRESS_WINDOW_MS);
        assert!(state.lock().unwrap().cancels.is_empty());
    }

    #[test]
    fn the_cancel_window_is_absolute_so_unrelated_messages_never_extend_it() {
        let s = river();
        // each gap is under a per-call 10 s timeout, but the replies land 12 s after the cancel
        let (mut w, _state, clock) = fake(vec![
            ack(AckStatus::Accepted), solving(1), delay(6_000), solving(2), delay(6_000), ack(AckStatus::Accepted), result(&s, ResultStatus::Cancelled, None),
        ]);
        let err = cancel_latency(&mut *w, &*clock, &s).unwrap_err();
        assert!(matches!(err, RunError::Timeout(_)), "{err:?}");
        assert_eq!(clock.now_ms(), CANCEL_WINDOW_MS);
    }

    #[test]
    fn a_rejected_probe_solve_fails_at_once_with_its_reason() {
        let s = river();
        let (mut w, _state, clock) = fake(vec![rejected("busy"), FakeReply::Hang]);
        let err = cancel_latency(&mut *w, &*clock, &s).unwrap_err();
        assert!(matches!(&err, RunError::Rejected { reason, .. } if reason == "busy"), "{err:?}");
        assert_eq!(clock.now_ms(), 0);
    }

    #[test]
    fn a_probe_solve_ending_in_error_is_a_failure_carrying_its_code() {
        let s = river();
        let error = WorkerError { code: "tree_too_large".into(), message: "f32 estimate above limit".into(), retryable: false, estimate_bytes: Some(1 << 34) };
        let (mut w, _state, clock) =
            fake(vec![ack(AckStatus::Accepted), FakeReply::Result { id: IdRef::Last, status: ResultStatus::Error, solution: None, error: Some(error), elapsed_ms: 2 }]);
        let err = cancel_latency(&mut *w, &*clock, &s).unwrap_err();
        assert!(matches!(&err, RunError::Protocol(m) if m.contains("tree_too_large")), "{err:?}");
    }
}
