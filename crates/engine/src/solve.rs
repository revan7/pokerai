//! The solve client (spec 5 step 7, 7, 12): one live solve on the worker, with absolute deadlines, identity checked on
//! every reply, stale ids discarded and the result validated before it is accepted, made resilient by the heartbeat,
//! the cancel-then-kill of a superseded request, the one `_min` retry and the §12 error-code policy (Task 23).
//!
//! Deadlines. The request's deadlines are absolute on the engine's clock (`Deadlines`, Task 20). The worker receives
//! the relative `deadline_ms` computed at send time from what is left until the street deadline (`worker_deadline_ms`);
//! a request with no room for one iteration is never sent. Every receive is given only what is left until the earlier of
//! the attempt's hang bound (`deadline_ms` + the delivery and pipe margins + `RESULT_GRACE_MS` after the send) and the
//! watchdog's fire (`final delivery - 100 ms`), so the client never waits past either: the link's `recv` keeps that bound
//! for the whole call, exit confirmation included (ruling 18-I1).
//!
//! The watchdog. The client does not arm the watchdog (`serve_request` does, Task 28): it runs beside it. It shares the
//! request's furthest stage through `EngineCore::stage` (the watchdog's `Armed::stage`), advancing it as the worker
//! reports its stages, so a watchdog `Final` names the stage reached; it stops waiting at the watchdog's fire, when the
//! watchdog delivers the request's one `Final` independently (§7); and it judges the street deadline as the watchdog's
//! `StreetDeadline` does (ruling 20-I1): from the engine-clock time the first attempt's terminal arrived, never from when
//! the attempt returned. That arrival is published to the request's shared `StreetDeadline` (`SolvePlan::street_deadline`,
//! the watchdog's `Armed::street_deadline`) the moment the terminal is received, before it is validated or anything is
//! recovered, and returned in `SolveOutcome::first_terminal_ms` (ruling 22-I4).
//!
//! Replies. Identity is checked before anything is built, immediately before the request is sent, and on every reply:
//! once the decision is no longer active the attempt ends `Superseded` without sending, forwarding, validating or
//! accepting anything (ruling 22-I2). Expiry is judged the same way, at the engine-clock time the client observes each
//! reply rather than by the bound its receive was given, since a suspend or a stalled thread can hand a reply over later
//! (ruling 22-I1): at or after the watchdog's fire the reply is neither forwarded nor accepted (`DeadlinePassed`), and at
//! or after the attempt's hang bound the attempt ends as the hang it would have been had the client resumed at the top
//! of its loop. A validated success is exposed only if the decision is still active and the fire has not come by then.
//! A reply carrying another id (a superseded request's, a cancel's) is discarded without side effects: an `ack` for
//! another id, accepted or rejected, frees nothing. A reply for this solve that breaks the protocol is a protocol error,
//! answered by restarting the worker (§12).
//!
//! Resilience (§7, §12). While the worker reports `Solving`, no `progress` for `HEARTBEAT_MS` ends the attempt as a hung
//! worker, judged like the hang bound (at the top of the loop and on every reply observed). A decision superseded after
//! its request was sent is cancelled (`cancel_or_kill`): the worker has `CANCEL_KILL_MS` to confirm with the job's
//! `result{cancelled}`, else it is killed and restarted; one superseded before the send, or after its terminal arrived,
//! has nothing to cancel. A failed attempt is classified by `classify`: the worker is restarted (kill, reap, respawn) when
//! it exited, broke the protocol, hung or missed the heartbeat, and its new `ready` is validated again; then, only after
//! the first attempt, only when the failure allows it and a `_min` template exists, one retry on that template is sent
//! if §7's admission passes, with only what is left until final delivery. A retry never rewinds the reported stage, and
//! its terminal is never published as the first attempt's. A request that finds no live worker (a restart that failed
//! earlier) relaunches it once before anything is sent. At the watchdog's fire the client stops and cleans nothing up:
//! the watchdog delivers the `Final`, and the kill and restart of a still-busy worker after it are `serve_request`'s.

use crate::bench_support::spot_identity;
use crate::core::EngineCore;
use crate::deadline::{retry_admitted, Deadlines, DELIVERY_MARGIN_MS, PIPE_MARGIN_MS};
use crate::tree::{build_tree_full, TemplateSelection, TreeBuild};
use crate::watchdog::{SharedSink, StreetDeadline};
use crate::worker::link::WorkerLinkError;
use crate::worker::ready::validate_ready;
use proto::worker::{validate_solution, AckStatus, EngineMessage, ResultStatus, SolveRequest, Stage, StreetSolution, WorkerError, WorkerMessage, FAILURE_CODES};
use proto::{DecisionIdentity, EffectiveTree, OrdinalPath, Rake, RecommendationEvent, SolveInput, UnsupportedReason};
use std::sync::Arc;
use std::time::Duration;

/// §12: no `progress` for this long during `Solving` is a hung worker (Task 23).
pub const HEARTBEAT_MS: u64 = 5_000;
/// §7/§12: a `cancel` not confirmed by `result{cancelled}` within this long is answered by a kill (Task 23).
pub const CANCEL_KILL_MS: u64 = 1_500;
/// How long after its `deadline_ms` and the delivery and pipe margins the worker's terminal `result` may still arrive
/// before the attempt counts as hung.
const RESULT_GRACE_MS: u64 = 500;

/// One live solve: the decision it answers, its absolute deadlines and the request's shared street deadline, the
/// template and its `_min` retry (§10.1), the rake, hero's role at the decision node, and whether the job is a
/// pre-solver `background` job (plan 4 sends `true`).
#[derive(Clone)]
pub struct SolvePlan {
    pub identity: DecisionIdentity,
    pub deadlines: Deadlines,
    /// The request's street deadline, shared with its watchdog (`watchdog::Armed::street_deadline`, ruling 20-I1): the
    /// client publishes the first attempt's terminal arrival to it at receipt (ruling 22-I4). It is the street deadline
    /// of `deadlines` (`run_solve` asserts it). A job no watchdog watches (plan 4's `background`) still gets one.
    pub street_deadline: Arc<StreetDeadline>,
    pub template_id: String,
    pub retry_template_id: Option<String>,
    pub rake: Rake,
    pub hero_actor: String,
    pub background: bool,
}

/// `StreetDeadline` has no `Debug`; the plan shows its deadline and what has been published to it.
impl std::fmt::Debug for SolvePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let street = format!("StreetDeadline {{ deadline_ms: {}, terminal_arrival_ms: {:?} }}", self.street_deadline.deadline_ms(), self.street_deadline.terminal_arrival_ms());
        f.debug_struct("SolvePlan").field("identity", &self.identity).field("deadlines", &self.deadlines).field("street_deadline", &format_args!("{street}"))
            .field("template_id", &self.template_id).field("retry_template_id", &self.retry_template_id).field("rake", &self.rake)
            .field("hero_actor", &self.hero_actor).field("background", &self.background).finish()
    }
}

/// How a solve ended. `Ok` and `BestSoFar` both carry a validated solution: `Ok` iff its raw exploitability meets the
/// raw target, `BestSoFar` for a worker `best_so_far` (a deadline stop) short of it (see `terminal_for`).
#[derive(Debug, Clone, PartialEq)]
pub enum Terminal { Ok, BestSoFar, Failed(UnsupportedReason) }

#[derive(Debug, Clone)]
pub struct SolveOutcome {
    pub terminal: Terminal,
    /// The validated solution, present iff the terminal is `Ok` or `BestSoFar`.
    pub solution: Option<StreetSolution>,
    /// The ordinal path of every node of `solution`, in its order.
    pub ordinal_paths: Vec<OrdinalPath>,
    /// The decision node's ordinal path (empty when there is no solution).
    pub decision_path: OrdinalPath,
    pub tree: EffectiveTree,
    /// Engine-clock milliseconds from the call to its return.
    pub elapsed_ms: u32,
    pub template_used: String,
    /// §7: the first attempt's terminal did not arrive by the street deadline (judged from its arrival time).
    pub street_violation: bool,
    /// The engine-clock time the first attempt's terminal `result` arrived, as published to the plan's street deadline
    /// at receipt (ruling 22-I4); `None` when no terminal of the first attempt was received. Never a retry's.
    pub first_terminal_ms: Option<u64>,
    /// The worker restarts this solve made, failed ones included: the relaunch of a missing worker before the request,
    /// the restart after a failed attempt, and the kill-and-restart answering an unconfirmed cancel.
    pub restarts: u8,
    /// Display only: the raw exploitability in basis points of the solved pot, rounded (§4.4).
    pub reached_bp: Option<u16>,
}

/// Spec 4.5 `solve.spot`: the lowercase sha256 hex of the structural identity of the game `req` solves, the string a
/// staged lock is matched against (ruling 22-S). It is `bench_support::spot_identity`, the one implementation in the
/// workspace: the tree signature, the chip scale (pot and both stacks), the rake, the board and both scaled range
/// hashes, and none of the solve parameters (id, deadline, target, history). The brief's `spot_hash(tree, pot, board,
/// ranges)` could not see the stacks or the rake, so two games that differ only there would have shared one identity.
pub fn spot_hash(req: &SolveRequest) -> String { spot_identity(req) }

fn stage_name(s: Stage) -> &'static str { match s { Stage::Building => "building", Stage::Solving => "solving", Stage::Extracting => "extracting" } }
fn ack_name(s: AckStatus) -> &'static str {
    match s { AckStatus::Accepted => "accepted", AckStatus::Staged => "staged", AckStatus::Rejected => "rejected", AckStatus::AlreadyFinished => "already_finished", AckStatus::UnknownTarget => "unknown_target" }
}
fn status_name(s: ResultStatus) -> &'static str {
    match s { ResultStatus::Ok => "ok", ResultStatus::BestSoFar => "best_so_far", ResultStatus::Cancelled => "cancelled", ResultStatus::Error => "error" }
}
fn engine_error(m: impl Into<String>, retryable: bool) -> UnsupportedReason { UnsupportedReason::EngineError { message: m.into(), retryable } }
/// The reason of a solve whose decision is no longer active (§4.4, §12).
fn superseded() -> UnsupportedReason { engine_error("superseded by a newer request", false) }

/// The reason of a solve whose worker could not be restarted after the attempt failed with `reason` (§12): both causes,
/// named. No live worker is left, which is retryable as `ready_for_requests` has it: the next request relaunches it.
fn restart_failed(reason: UnsupportedReason, e: &WorkerLinkError) -> UnsupportedReason {
    let failure = match reason { UnsupportedReason::EngineError { message, .. } => message, other => format!("{other:?}") };
    engine_error(format!("{failure}; restarting the worker failed: {e}"), true)
}

/// §7 retry admission's `p95(_min template)`: until plan 4's bench matrix exists (plan 4 Task 21), the street budget,
/// taken from the request's own deadlines (`street_deadline_ms - t0_ms`, which `Deadlines::for_request` makes
/// `deadline::street_budget_ms(street, flop_budget_s)`, so a raised flop budget counts as the §7 budget it is).
fn min_template_p95_ms(d: &Deadlines) -> u64 {
    d.street_deadline_ms.checked_sub(d.t0_ms)
        .unwrap_or_else(|| panic!("street deadline {} ms precedes the request's admission at t0 {} ms", d.street_deadline_ms, d.t0_ms))
}

/// Milliseconds from `from` to `to` on the engine clock. Both are readings of one monotonic clock taken in that order,
/// and every solve ends by the watchdog's fire (at most 35 s after `t0`), so the span fits a `u32`: anything else is an
/// engine bug, asserted rather than wrapped.
fn ms_between(from: u64, to: u64) -> u32 {
    let span = to.checked_sub(from).unwrap_or_else(|| panic!("engine clock reading {to} ms precedes {from} ms"));
    u32::try_from(span).unwrap_or_else(|_| panic!("a solve span of {span} ms does not fit u32"))
}

/// Ruling 26-Q4: the raw comparison of spec 4.4 and 5 step 7, `exploitability_chips / pot <= target_bp / 10_000`,
/// evaluated exactly as `expl * 10_000 <= target_bp * pot` in `f64` (an `f32` significand times 10^4 and a `u16` times a
/// `u32` are both exact there): the predicate `assemble::coverage_for_solve` uses for `Exact`.
fn meets_target(exploitability_chips: f32, pot: u32, target_bp: u16) -> bool {
    f64::from(exploitability_chips) * 10_000.0 <= f64::from(target_bp) * f64::from(pot)
}

/// Display only (§4.4): the raw exploitability in basis points of `pot`, rounded; above `u16::MAX` bp (6.5535 times
/// the pot) shown as `u16::MAX`, as `assemble::coverage_for_solve` shows it. No comparison ever reads it.
fn reached_bp(exploitability_chips: f32, pot: u32) -> u16 {
    let bp = (f64::from(exploitability_chips) * 10_000.0 / f64::from(pot)).round();
    if bp <= f64::from(u16::MAX) { bp as u16 } else { u16::MAX }
}

/// The terminal of a validated `ok` or `best_so_far` solution (rulings 26-Q4 and 22-I3). Target compliance is the raw
/// comparison (`meets_target`): `Ok` iff the raw target is met, whatever the worker's status. `BestSoFar`, which
/// assembly labels `DeadlineBestSoFar`, is reserved for a genuine deadline stop, a worker `best_so_far` (§7: the
/// measured solution at the stop point), that misses it: only reasons actually incurred (§2). A worker `ok` that misses
/// it breaks the worker's contract (`ok` means the target was met, §7): the worker stops at its f32-rounded threshold
/// `(pot * target_bp / 10_000) as f32`, which can lie half an ulp above the raw target, and an `ok` there had no
/// deadline stop. It is a non-retryable worker-contract `EngineError` naming the measurement, the raw target and the
/// worker's threshold, never `Exact` and never `DeadlineBestSoFar`. (The worker's own comparison is follow-up P2.W1.)
fn terminal_for(status: ResultStatus, exploitability_chips: f32, pot: u32, target_bp: u16) -> Result<Terminal, UnsupportedReason> {
    match (status, meets_target(exploitability_chips, pot, target_bp)) {
        (ResultStatus::Ok | ResultStatus::BestSoFar, true) => Ok(Terminal::Ok),
        (ResultStatus::BestSoFar, false) => Ok(Terminal::BestSoFar),
        (ResultStatus::Ok, false) => {
            let raw_target = f64::from(target_bp) * f64::from(pot) / 10_000.0;
            // The worker's threshold, computed exactly as the worker computes it (`solver-worker` job, `target_chips`).
            let worker_threshold = (f64::from(pot) * f64::from(target_bp) / 10_000.0) as f32;
            Err(engine_error(format!("worker contract: `ok` at {} chips misses the raw target {raw_target} chips ({target_bp} bp of the {pot}-chip pot); \
                the worker's f32-rounded threshold is {} chips", f64::from(exploitability_chips), f64::from(worker_threshold)), false))
        }
        (s @ (ResultStatus::Cancelled | ResultStatus::Error), _) => unreachable!("terminal_for: a {} result carries no solution (`result_shape`)", status_name(s)),
    }
}

/// Where an attempt stands when `ended_at` judges it.
#[derive(Debug, Clone, Copy)]
enum Watch {
    /// Immediately before the send: nothing has reached the worker, and there is no hang bound yet.
    BeforeSend,
    /// Waiting for this solve's terminal: the attempt's hang bound and, while the worker reports `Solving`, the time the
    /// heartbeat is due (its last `progress` + `HEARTBEAT_MS`).
    Waiting { hang_bound_ms: u64, heartbeat_due_ms: Option<u64> },
    /// This solve's terminal has arrived (within the hang bound, which no longer applies): before a validated success
    /// is exposed.
    AfterTerminal,
}

/// Whether the attempt can no longer act at `now_ms`, and how it ends, checked in this order at the top of the receive
/// loop, on every reply the moment the client observes it (ruling 22-I1), before the request is sent and before a
/// validated success is exposed: the decision is no longer active (ruling 22-I2), and only while waiting may the worker
/// still be running its job; the watchdog's fire has come, and it delivers the `Final` (§7); while waiting, the
/// attempt's hang bound has come, or the heartbeat is due (§12). `None` while the attempt may go on.
fn ended_at(core: &EngineCore, plan: &SolvePlan, now_ms: u64, watch: Watch) -> Option<AttemptEnd> {
    if !core.identity_active(&plan.identity) {
        return Some(AttemptEnd::Superseded { running: matches!(watch, Watch::Waiting { .. }) });
    }
    if now_ms >= plan.deadlines.watchdog_fire_ms() {
        return Some(AttemptEnd::DeadlinePassed);
    }
    match watch {
        Watch::Waiting { hang_bound_ms, .. } if now_ms >= hang_bound_ms => Some(AttemptEnd::Hang),
        Watch::Waiting { heartbeat_due_ms: Some(due_ms), .. } if now_ms >= due_ms => Some(AttemptEnd::Heartbeat),
        Watch::BeforeSend | Watch::Waiting { .. } | Watch::AfterTerminal => None,
    }
}

/// §7's street verdict as `watchdog::StreetDeadline::violated` judges it (ruling 20-I1), from the engine-clock time
/// the first attempt's terminal `result` arrived: late iff it arrived after `deadline_ms` (one arriving at the deadline
/// is on time). With no terminal, the deadline is violated once the clock has reached it, here at `now_ms`, when the
/// attempt returns.
fn street_violated(deadline_ms: u64, first_terminal_ms: Option<u64>, now_ms: u64) -> bool {
    match first_terminal_ms {
        Some(at_ms) => at_ms > deadline_ms,
        None => now_ms >= deadline_ms,
    }
}

/// §4.5/§12: the worker's `ready`, validated before any request. The protocol version, the pinned solver commit, the
/// adapter version and AVX2 in `build_features` are checked here before every request (a mismatch answers every
/// request with `EngineError("worker/proto version mismatch")` until the worker is rebuilt); `threads == requested` was
/// checked by the link at launch against its own launch argument (`ProcessWorker`), which the engine core does not hold.
/// No live worker (killed, or a restart that failed) is a retryable `EngineError`.
fn ready_for_requests(core: &EngineCore) -> Result<(), UnsupportedReason> {
    let ready = core.worker.ready().ok_or_else(|| engine_error("worker not ready: no live worker", true))?;
    validate_ready(ready, ready.threads).map_err(|e| engine_error(format!("worker/proto version mismatch: {e}"), false))
}

/// How one attempt ended.
pub(crate) enum AttemptEnd {
    /// This solve's terminal `result`, well-formed (`result_shape`); `run_attempt` returns its arrival time with it.
    Result { status: ResultStatus, solution: Option<StreetSolution>, error: Option<WorkerError> },
    /// The link could not write the request as a request line (§4.5): nothing was queued and the worker is unaffected.
    Unsent(String),
    /// The worker process exited with this confirmed code (§10.3 `WorkerExit{code}`).
    Exit(i32),
    /// The worker stopped taking requests or its stdout ended, and its exit was not confirmed within the call's budget
    /// (or there is no live worker). Never given an exit code (ruling 18: an unconfirmed end is never a confirmed exit).
    Ended,
    /// A faulty line, or a reply for this solve that breaks the protocol.
    Protocol(String),
    /// No terminal `result` by the attempt's hang bound (a reply observed at or after it included, ruling 22-I1).
    Hang,
    /// §12 heartbeat: no `progress` for `HEARTBEAT_MS` while the worker reported `Solving` (a reply observed at or after
    /// that point included, as for the hang bound).
    Heartbeat,
    /// `ack{rejected}` for this solve: the worker started no work.
    Rejected(String),
    /// The decision is no longer active. `running`: the request had reached the worker and its terminal had not been
    /// taken, so the worker may still be running the job, which `run_solve` cancels (`cancel_or_kill`). Before the send,
    /// or once the terminal arrived, there is nothing to cancel.
    Superseded { running: bool },
    /// The watchdog's fire time was reached (a reply observed at or after it included, ruling 22-I1; before the send:
    /// nothing was sent); the watchdog delivers the request's `Final` (§7).
    DeadlinePassed,
}

/// The `solve` for one attempt: the tree `b` at the root's chips, both ranges, the rake, the relative `deadline_ms` and
/// the street's extraction margin, the engine's memory limit, the request's `target_bp` and `background` flag.
pub(crate) fn request(core: &mut EngineCore, input: &SolveInput, plan: &SolvePlan, b: &TreeBuild, deadline_ms: u32) -> SolveRequest {
    // `materialize` refuses a zero pot, so every tree that exists was built at a positive one (the divisor of every
    // basis-point and percentage figure of this attempt).
    assert!(b.pot > 0, "solve request for a tree built at a zero pot");
    let (rake_rate, rake_cap_mchips) = match plan.rake { Rake::PotRake { rate, cap_mchips, .. } => (rate, cap_mchips), Rake::TimeCharge => (0.0, 0) };
    let mut req = SolveRequest { id: core.next_id(), spot: String::new(), board: input.root.board.clone(),
        oop_range: input.ranges[0].clone(), ip_range: input.ranges[1].clone(), pot: b.pot, stack_oop: input.root.stack_oop_root, stack_ip: input.root.stack_ip_root,
        rake_rate, rake_cap_mchips, tree: b.tree.clone(), history: b.history.clone(), target_bp: input.target_bp, deadline_ms,
        extraction_margin_ms: plan.deadlines.extraction_margin_ms, memory_limit_bytes: core.memory_limit_bytes, background: plan.background };
    // The identity reads every field above but `id` and `spot` itself (see `spot_hash`).
    req.spot = spot_hash(&req);
    req
}

/// §4.5: a `result` carries a solution iff its status is `ok` or `best_so_far`, and an error iff it is `error`, with
/// one of the listed failure codes.
fn result_shape(status: ResultStatus, solution: &Option<StreetSolution>, error: &Option<WorkerError>) -> Result<(), String> {
    match (status, solution.is_some(), error) {
        (ResultStatus::Ok | ResultStatus::BestSoFar, true, None) | (ResultStatus::Cancelled, false, None) => Ok(()),
        (ResultStatus::Error, false, Some(e)) if FAILURE_CODES.contains(&e.code.as_str()) => Ok(()),
        (ResultStatus::Error, false, Some(e)) => Err(format!("result error code {:?} is not a failure code of section 4.5", e.code)),
        (s, has_solution, e) => Err(format!("result {} {} a solution and {} an error", status_name(s),
            if has_solution { "with" } else { "without" }, if e.is_some() { "with" } else { "without" })),
    }
}

/// `progress.exploitability_chips` as a percentage of the solved pot (§7 display). The worker never reports a negative
/// exploitability (its noise floor maps tiny negatives to 0), so a negative one is a protocol error, never forwarded
/// (the event's codec refuses it); a signed zero is shown as `+0.0`.
fn exploitability_pct(chips: f32, pot: u32) -> Result<f32, String> {
    if chips < 0.0 { return Err(format!("progress exploitability {chips} chips is negative")); }
    let pct = (100.0 * f64::from(chips) / f64::from(pot)) as f32;
    if !pct.is_finite() { return Err(format!("progress exploitability {chips} chips of a {pot}-chip pot is not a finite percentage")); }
    Ok(if pct == 0.0 { 0.0 } else { pct })
}

/// The cancel-then-kill of §7/§12, for a solve `target` superseded while the worker may still be running it (never one
/// superseded before its send: nothing reached the worker). Sends `cancel` for it under a fresh request id and waits at
/// most `CANCEL_KILL_MS` from the send for the job's `result{cancelled}`, the only confirmation (§12); anything else read
/// meanwhile is discarded, a late reply of the job included. Unconfirmed by then, the worker is killed and restarted. A
/// link failure (the cancel cannot be sent, the worker exits or breaks the protocol in the window) is answered by a
/// restart at once. Returns whether the worker was restarted; a restart that fails leaves no live worker, which the next
/// request relaunches (`run_solve`), and is never a panic.
pub(crate) fn cancel_or_kill(core: &mut EngineCore, target: &str) -> bool {
    let id = core.next_id();
    if core.worker.send(&EngineMessage::Cancel { id, target: target.to_string() }).is_err() {
        let _ = core.worker.restart();
        return true;
    }
    let sent = core.clock.now_ms();
    let until = sent.checked_add(CANCEL_KILL_MS).unwrap_or_else(|| panic!("the kill bound of a cancel sent at {sent} ms overflows u64"));
    loop {
        let now = core.clock.now_ms();
        if now >= until {
            core.worker.kill();
            let _ = core.worker.restart();
            return true;
        }
        // What is left of the window, never more: `recv` keeps it for the whole call.
        match core.worker.recv(Duration::from_millis(until - now)) {
            Ok(Some(WorkerMessage::Result { id, status: ResultStatus::Cancelled, .. })) if id == target => return false,
            Ok(_) => {}
            Err(_) => {
                let _ = core.worker.restart();
                return true;
            }
        }
    }
}

/// One send/receive cycle, with the engine-clock time its terminal `result` arrived, if one was received. While the
/// worker reports `Solving`, the heartbeat bounds the wait too (§12); recovery (a cancel, a restart, a retry) is
/// `run_solve`'s, after the attempt has ended.
///
/// `first`: the request's street deadline when this is the first attempt. The terminal's arrival is published to it at
/// receipt, before the result is checked, validated or anything is recovered (ruling 22-I4). A retry passes `None`: its
/// terminal never replaces the first attempt's (§7 judges the first attempt only).
pub(crate) fn run_attempt(core: &mut EngineCore, plan: &SolvePlan, sink: &SharedSink, req: &SolveRequest, first: Option<&StreetDeadline>) -> (AttemptEnd, Option<u64>) {
    // Immediately before the send: a decision superseded, or a request expired, while it was prepared starts no work.
    if let Some(end) = ended_at(core, plan, core.clock.now_ms(), Watch::BeforeSend) { return (end, None); }
    if let Err(e) = core.worker.send(&EngineMessage::Solve(req.clone())) {
        let end = match e {
            WorkerLinkError::Protocol(m) => AttemptEnd::Unsent(m),
            WorkerLinkError::LineTooLong(n) => AttemptEnd::Unsent(format!("a {n}-byte request line is over the limit")),
            WorkerLinkError::Exit { code } => AttemptEnd::Exit(code),
            // `send` never launches a worker: no live worker is its `Eof`.
            WorkerLinkError::Eof | WorkerLinkError::Spawn(_) => AttemptEnd::Ended,
        };
        return (end, None);
    }
    let sent = core.clock.now_ms();
    let expected_by = sent
        .checked_add(u64::from(req.deadline_ms) + DELIVERY_MARGIN_MS + PIPE_MARGIN_MS + RESULT_GRACE_MS)
        .unwrap_or_else(|| panic!("the hang bound of a solve sent at {sent} ms overflows u64"));
    let fire_ms = plan.deadlines.watchdog_fire_ms();
    // While the worker reports `Solving`: when the heartbeat is due, its last `progress` + `HEARTBEAT_MS` (§12).
    let mut heartbeat_due_ms: Option<u64> = None;
    loop {
        let now = core.clock.now_ms();
        let watch = Watch::Waiting { hang_bound_ms: expected_by, heartbeat_due_ms };
        if let Some(end) = ended_at(core, plan, now, watch) { return (end, None); }
        // What is left until the earliest bound, never more: `recv` keeps it for the whole call.
        let bound_ms = expected_by.min(fire_ms).min(heartbeat_due_ms.unwrap_or(u64::MAX));
        let msg = match core.worker.recv(Duration::from_millis(bound_ms - now)) {
            Ok(Some(msg)) => msg,
            Ok(None) => continue,
            Err(WorkerLinkError::Exit { code }) => return (AttemptEnd::Exit(code), None),
            Err(WorkerLinkError::Eof | WorkerLinkError::Spawn(_)) => return (AttemptEnd::Ended, None),
            Err(e @ (WorkerLinkError::Protocol(_) | WorkerLinkError::LineTooLong(_))) => return (AttemptEnd::Protocol(e.to_string()), None),
        };
        // Identity and expiry on every reply, at the engine-clock time the client observes it (ruling 22-I1): the
        // receive's bound limits the wait, not when the client runs again (a suspend, a stalled thread). A mutation that
        // landed while the reply was in flight (or in the same receive), or a reply seen at or after the watchdog's fire,
        // the hang bound or the heartbeat's due time, ends the attempt before anything of the reply is forwarded,
        // validated or accepted, as the top of the loop would have a moment later (§4.4, §7, §12).
        let at_ms = core.clock.now_ms();
        if let Some(end) = ended_at(core, plan, at_ms, Watch::Waiting { hang_bound_ms: expected_by, heartbeat_due_ms }) { return (end, None); }
        match msg {
            WorkerMessage::Ack { id, status, reason, .. } if id == req.id => match status {
                AckStatus::Accepted => {}
                AckStatus::Rejected => return (AttemptEnd::Rejected(reason.unwrap_or_else(|| "no reason given".into())), None),
                // `staged`, `already_finished` and `unknown_target` answer a lock or a cancel, never a solve.
                other => return (AttemptEnd::Protocol(format!("ack {} for solve {}", ack_name(other), req.id)), None),
            },
            WorkerMessage::Progress { id, stage, iterations, exploitability_chips, .. } if id == req.id => {
                let exploitability_pct = match exploitability_chips.map(|c| exploitability_pct(c, req.pot)).transpose() {
                    Ok(p) => p,
                    Err(m) => return (AttemptEnd::Protocol(m), None),
                };
                // Each progress resets the heartbeat, which runs only while the worker reports `Solving` (§12).
                heartbeat_due_ms = (stage == Stage::Solving).then(|| at_ms.checked_add(HEARTBEAT_MS)
                    .unwrap_or_else(|| panic!("the heartbeat due after a progress at {at_ms} ms overflows u64")));
                core.set_stage(stage_name(stage));
                sink.lock().unwrap().emit(RecommendationEvent::Progress { identity: plan.identity.clone(), stage: stage_name(stage).into(), iterations,
                    exploitability_pct, elapsed_ms: ms_between(plan.deadlines.t0_ms, at_ms) });
            }
            WorkerMessage::Result { id, status, solution, error, .. } if id == req.id => {
                // This solve's terminal arrived at `at_ms`: published before anything else is made of it (ruling 22-I4).
                if let Some(street) = first { street.terminal_arrived(at_ms); }
                if let Err(m) = result_shape(status, &solution, &error) { return (AttemptEnd::Protocol(m), Some(at_ms)); }
                return (AttemptEnd::Result { status, solution, error }, Some(at_ms));
            }
            WorkerMessage::Ready(_) => return (AttemptEnd::Protocol("a second ready (ready is written once, section 4.5)".into()), None),
            // Another request's reply (a superseded request's, a cancel's): discarded without side effects. It is not
            // forwarded, validated or counted, and an ack of it frees nothing.
            WorkerMessage::Ack { .. } | WorkerMessage::Progress { .. } | WorkerMessage::Result { .. } => {}
        }
    }
}

/// §4.5 output validation: the whole solution against the materialized tree (`validate_solution`, strict), then the
/// requested node against the decision node and hero's role (§5 step 7). Returns every node's ordinal path.
pub(crate) fn validate(b: &TreeBuild, plan: &SolvePlan, sol: &StreetSolution) -> Result<Vec<OrdinalPath>, String> {
    let paths = validate_solution(sol, &b.tree.materialized)?;
    let r = sol.requested as usize;
    if paths.get(r) != Some(&b.decision_path) { return Err(format!("requested node {:?} is not the decision node {:?}", paths.get(r), b.decision_path)); }
    if sol.nodes[r].actor != plan.hero_actor { return Err(format!("requested node actor {} is not hero's {}", sol.nodes[r].actor, plan.hero_actor)); }
    Ok(paths)
}

/// A validated solution with its terminal (`terminal_for`: `Ok` or `BestSoFar`). `Terminal::Ok` always means the raw
/// target was met, so assembly never sees an `ok` above target (ruling 26-Q4).
pub(crate) fn succeeded(core: &EngineCore, t_start: u64, b: &TreeBuild, template: &str, sol: StreetSolution, paths: Vec<OrdinalPath>, terminal: Terminal, street_violation: bool, restarts: u8, first_terminal_ms: Option<u64>) -> SolveOutcome {
    assert!(matches!(terminal, Terminal::Ok | Terminal::BestSoFar), "succeeded: a {terminal:?} terminal carries no solution");
    let reached_bp = Some(reached_bp(sol.exploitability_chips, b.pot));
    SolveOutcome { terminal, decision_path: b.decision_path.clone(), tree: b.tree.clone(), solution: Some(sol), ordinal_paths: paths,
        elapsed_ms: ms_between(t_start, core.clock.now_ms()), template_used: template.to_string(), street_violation, first_terminal_ms, restarts, reached_bp }
}

pub(crate) fn failed(core: &EngineCore, t_start: u64, reason: UnsupportedReason, input: &SolveInput, template: &str, restarts: u8, street_violation: bool, first_terminal_ms: Option<u64>) -> SolveOutcome {
    SolveOutcome { terminal: Terminal::Failed(reason), solution: None, ordinal_paths: vec![], decision_path: vec![], tree: input.tree.clone(),
        elapsed_ms: ms_between(t_start, core.clock.now_ms()), template_used: template.to_string(), street_violation, first_terminal_ms, restarts, reached_bp: None }
}

/// Maps a non-success attempt end to its §12 reason, whether a `_min` retry is allowed and whether the worker must be
/// restarted. `stage` is the furthest stage the request reached (`EngineCore::stage`), reported when the deadline
/// passed. A superseded or expired attempt is never retried here and never restarts the worker: a running job of a
/// superseded decision is cancelled (`cancel_or_kill`), and the cleanup after the watchdog's fire is `serve_request`'s.
pub(crate) fn classify(end: AttemptEnd, stage: &str) -> (UnsupportedReason, bool, bool) {
    match end {
        AttemptEnd::Result { status: ResultStatus::Cancelled, .. } => (engine_error("worker cancelled the job", true), true, false),
        AttemptEnd::Result { error: Some(e), .. } => match e.code.as_str() {
            "tree_mismatch" | "invalid_request" | "lock_mismatch" => (engine_error(format!("{}: {}", e.code, e.message), false), false, false),
            "no_iteration" => (UnsupportedReason::DeadlineExceeded { stage: "solving".into() }, true, false),
            "tree_too_large" | "out_of_memory" => (UnsupportedReason::TreeTooLarge { estimate_bytes: e.estimate_bytes.unwrap_or(0) }, true, false),
            _ => (engine_error(format!("{}: {}", e.code, e.message), e.retryable), true, false),
        },
        AttemptEnd::Result { status, .. } => unreachable!("classify: a {} result with a solution is a success, and `result_shape` refuses any other", status_name(status)),
        AttemptEnd::Unsent(m) => (engine_error(format!("request not sent: {m}"), false), true, false),
        AttemptEnd::Exit(code) => (engine_error(format!("WorkerExit{{code: {code}}}"), true), true, true),
        AttemptEnd::Ended => (engine_error("the worker stopped taking requests or closed its stdout, and its exit was not confirmed", true), true, true),
        AttemptEnd::Protocol(m) => (engine_error(format!("protocol error: {m}"), true), true, true),
        AttemptEnd::Hang => (engine_error("no terminal result by the worker deadline", true), true, true),
        AttemptEnd::Heartbeat => (engine_error(format!("no progress for {HEARTBEAT_MS} ms while solving (heartbeat)"), true), true, true),
        AttemptEnd::Rejected(r) => (engine_error(format!("solve rejected: {r}"), true), false, false),
        AttemptEnd::Superseded { .. } => (superseded(), false, false),
        AttemptEnd::DeadlinePassed => (UnsupportedReason::DeadlineExceeded { stage: stage.into() }, false, false),
    }
}

/// §5 step 7 / §7 / §12: one live solve with absolute deadlines, the heartbeat, the cancel-then-kill of a superseded
/// request and at most one `_min` retry under §7's admission.
///
/// A solve that cannot start starts no work: a decision already superseded (checked before anything is built, ruling
/// 22-I2), a tree that does not build, no live worker and a relaunch that fails, a worker whose `ready` is refused, or a
/// street deadline that leaves no room for one iteration returns `Failed` without a request. A missing worker (a restart
/// that failed earlier, a kill) is relaunched once, and its `ready` validated, before anything is sent.
///
/// The first attempt (`attempt_no` 0) has until the street deadline, and its terminal's arrival is published to the
/// plan's street deadline at receipt (ruling 22-I4). A validated success ends the solve; so does a superseded decision
/// (its running job cancelled, `cancel_or_kill`) or the watchdog's fire (nothing cleaned up: the watchdog delivers the
/// `Final`, and the cleanup after it is `serve_request`'s). Any other failure restarts the worker when `classify` says so,
/// validating its new `ready`; then one retry (`attempt_no` 1) is sent on the `_min` template, only after the first
/// attempt, when the failure allows it, the decision is still active and §7's admission passes, with what is left until
/// final delivery. The retry's failure is final. The street verdict is judged when `run_solve` returns, from the first
/// attempt's terminal arrival alone (ruling 20-I1).
pub fn run_solve(core: &mut EngineCore, input: &SolveInput, plan: &SolvePlan, sink: &SharedSink) -> SolveOutcome {
    let t_start = core.clock.now_ms();
    assert!(t_start >= plan.deadlines.t0_ms, "run_solve at {t_start} ms, before its request's admission at t0 {} ms", plan.deadlines.t0_ms);
    let street_deadline_ms = plan.deadlines.street_deadline_ms;
    assert!(plan.street_deadline.deadline_ms() == street_deadline_ms,
        "the plan's shared street deadline is at {} ms, its deadlines' at {street_deadline_ms} ms", plan.street_deadline.deadline_ms());
    // §12: a solve is never retried on the template it failed on.
    assert!(plan.retry_template_id.as_deref() != Some(plan.template_id.as_str()), "the retry template {:?} is the solve's own", plan.template_id);
    // Every failure, with the street verdict judged at the return from the first attempt's terminal arrival.
    let fail = |core: &EngineCore, reason: UnsupportedReason, template: &str, restarts: u8, first_terminal_ms: Option<u64>| {
        let violated = street_violated(street_deadline_ms, first_terminal_ms, core.clock.now_ms());
        failed(core, t_start, reason, input, template, restarts, violated, first_terminal_ms)
    };
    let mut template = plan.template_id.clone();
    let mut restarts = 0u8;
    // Before any construction: a decision superseded before its solve starts starts no work (ruling 22-I2).
    if !core.identity_active(&plan.identity) { return fail(core, superseded(), &template, restarts, None); }
    let mut b = match build_tree_full(&input.root, &TemplateSelection::from_history(&template, &input.root.history)) {
        Ok(b) => b,
        Err(r) => return fail(core, r, &template, restarts, None),
    };
    // No live worker (a restart that failed earlier, a kill): one relaunch before anything is sent (review P2T22R).
    if core.worker.ready().is_none() {
        restarts += 1;
        if let Err(e) = core.worker.restart() {
            return fail(core, engine_error(format!("worker not ready: no live worker, and relaunching it failed: {e}"), true), &template, restarts, None);
        }
    }
    if let Err(reason) = ready_for_requests(core) { return fail(core, reason, &template, restarts, None); }
    // The first attempt's terminal arrival (ruling 22-I4): a retry's never replaces it.
    let mut first_terminal_ms = None;
    for attempt_no in 0..2u8 {
        // §7: the first attempt has until the street deadline, the retry until final delivery.
        let until_ms = if attempt_no == 0 { street_deadline_ms } else { plan.deadlines.final_delivery_ms };
        let Some(deadline_ms) = plan.deadlines.worker_deadline_ms(core.clock.now_ms(), until_ms) else {
            let stage = core.stage();
            return fail(core, UnsupportedReason::DeadlineExceeded { stage }, &template, restarts, first_terminal_ms);
        };
        // Advances only: a retry that starts at Building never rewinds the stage the first attempt reached.
        core.set_stage("building");
        let req = request(core, input, plan, &b, deadline_ms);
        // Only the first attempt publishes its terminal's arrival to the request's street deadline.
        let (end, terminal_ms) = run_attempt(core, plan, sink, &req, (attempt_no == 0).then_some(plan.street_deadline.as_ref()));
        if attempt_no == 0 { first_terminal_ms = terminal_ms; }
        if let AttemptEnd::Result { status: status @ (ResultStatus::Ok | ResultStatus::BestSoFar), solution: Some(sol), .. } = end {
            let violated = street_violated(street_deadline_ms, first_terminal_ms, core.clock.now_ms());
            let refused = |core: &EngineCore, reason: UnsupportedReason| failed(core, t_start, reason, input, &template, restarts, violated, first_terminal_ms);
            let paths = match validate(&b, plan, &sol) { Ok(paths) => paths, Err(e) => return refused(core, engine_error(format!("invalid solution: {e}"), false)) };
            let terminal = match terminal_for(status, sol.exploitability_chips, b.pot, input.target_bp) { Ok(terminal) => terminal, Err(reason) => return refused(core, reason) };
            // Identity and expiry again before the success is exposed: the checks after the receive held when the
            // result was observed, and validation can take the client past the watchdog's fire (ruling 22-I1).
            if let Some(end) = ended_at(core, plan, core.clock.now_ms(), Watch::AfterTerminal) {
                let (reason, _retry_allowed, _restart) = classify(end, &core.stage());
                return refused(core, reason);
            }
            return succeeded(core, t_start, &b, &template, sol, paths, terminal, violated, restarts, first_terminal_ms);
        }
        match end {
            // A job the worker may still be running is cancelled (§7/§12); before the send nothing was sent, and once
            // its terminal arrived there is nothing left to cancel.
            AttemptEnd::Superseded { running } => {
                if running && cancel_or_kill(core, &req.id) { restarts += 1; }
                return fail(core, superseded(), &template, restarts, first_terminal_ms);
            }
            // The watchdog delivers the `Final` (§7); the cleanup after it is `serve_request`'s, not the client's.
            AttemptEnd::DeadlinePassed => {
                let stage = core.stage();
                return fail(core, UnsupportedReason::DeadlineExceeded { stage }, &template, restarts, first_terminal_ms);
            }
            _ => {}
        }
        let (reason, retry_allowed, restart) = classify(end, &core.stage());
        if restart {
            restarts += 1;
            if let Err(e) = core.worker.restart() { return fail(core, restart_failed(reason, &e), &template, restarts, first_terminal_ms); }
            // §12: the restarted worker's `ready` is validated again before anything more is sent to it.
            if let Err(refused) = ready_for_requests(core) { return fail(core, refused, &template, restarts, first_terminal_ms); }
        }
        // §12: one retry, after the first attempt only, on the `_min` template, if the failure allows one.
        let retry = match plan.retry_template_id.as_deref() {
            Some(retry) if attempt_no == 0 && retry_allowed => retry,
            _ => return fail(core, reason, &template, restarts, first_terminal_ms),
        };
        // §7: admitted only if what is left covers the `_min` p95 and every margin.
        let p95_ms = min_template_p95_ms(&plan.deadlines);
        if !retry_admitted(core.clock.now_ms(), plan.deadlines.final_delivery_ms, p95_ms, plan.deadlines.extraction_margin_ms) {
            return fail(core, reason, &template, restarts, first_terminal_ms);
        }
        // A decision superseded meanwhile (during a restart, say) starts no retry: nothing is built or sent (ruling 22-I2).
        if !core.identity_active(&plan.identity) { return fail(core, superseded(), &template, restarts, first_terminal_ms); }
        template = retry.to_string();
        b = match build_tree_full(&input.root, &TemplateSelection::from_history(&template, &input.root.history)) {
            Ok(b) => b,
            Err(r) => return fail(core, r, &template, restarts, first_terminal_ms),
        };
    }
    unreachable!("run_solve: the retry (attempt 1) always returns")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Q4 predicate is exact at the boundary and agrees with `assemble::coverage_for_solve`'s `Exact` test.
    #[test]
    fn the_raw_target_comparison_is_exact_at_the_boundary() {
        assert!(meets_target(0.5, 100, 50));
        assert!(!meets_target(f32::from_bits(0.5f32.to_bits() + 1), 100, 50));
        // 0.3f32 lies above the raw 0.3 chips (30 bp of 100): the worker's f32 threshold admits it, the raw one does not
        assert!(!meets_target(0.3, 100, 30));
        assert!(meets_target(f32::from_bits(0.3f32.to_bits() - 1), 100, 30));
        for (expl, pot, bp) in [(0.5f32, 100u32, 50u16), (0.3, 100, 30), (0.325, 65, 50), (1.9, 100, 50), (0.0, 1, 0)] {
            let exact = crate::assemble::coverage_for_solve(expl, pot, bp, false, vec![]) == proto::Coverage::Exact;
            assert_eq!(meets_target(expl, pot, bp), exact, "{expl} chips of {pot} at {bp} bp");
        }
        assert_eq!((reached_bp(0.3, 100), reached_bp(1.9, 100), reached_bp(1_000.0, 1)), (30, 190, u16::MAX));
    }

    /// Ruling 22-I3: `Ok` iff the raw target is met, whatever the status; `BestSoFar` only for a `best_so_far` short of
    /// it; an `ok` short of it (0.3f32 of 100 chips at 30 bp: the worker's own threshold admits it) is a non-retryable
    /// worker-contract error naming the raw target and the worker's f32-rounded threshold.
    #[test]
    fn the_terminal_follows_the_raw_target_and_the_worker_contract() {
        assert_eq!(terminal_for(ResultStatus::Ok, 0.5, 100, 50), Ok(Terminal::Ok));
        assert_eq!(terminal_for(ResultStatus::BestSoFar, 0.5, 100, 50), Ok(Terminal::Ok));
        assert_eq!(terminal_for(ResultStatus::BestSoFar, 0.3, 100, 30), Ok(Terminal::BestSoFar));
        let Err(UnsupportedReason::EngineError { message, retryable: false }) = terminal_for(ResultStatus::Ok, 0.3, 100, 30) else { panic!("an ok short of the raw target") };
        assert_eq!(message, "worker contract: `ok` at 0.30000001192092896 chips misses the raw target 0.3 chips (30 bp of the 100-chip pot); \
            the worker's f32-rounded threshold is 0.30000001192092896 chips");
    }

    /// The street verdict: on time at the deadline, late 1 ms after; without a terminal, violated once reached.
    #[test]
    fn the_street_verdict_matches_the_watchdogs() {
        assert!(!street_violated(2_000, Some(2_000), 9_000));
        assert!(street_violated(2_000, Some(2_001), 2_001));
        assert!(!street_violated(2_000, None, 1_999));
        assert!(street_violated(2_000, None, 2_000));
    }

    /// §4.5 result shapes: a solution iff ok/best_so_far, an error iff error with a listed code, nothing for cancelled.
    #[test]
    fn result_shapes_follow_section_4_5() {
        let e = |code: &str| Some(WorkerError { code: code.into(), message: String::new(), retryable: false, estimate_bytes: None });
        assert!(result_shape(ResultStatus::Cancelled, &None, &None).is_ok());
        assert!(result_shape(ResultStatus::Error, &None, &e("no_iteration")).is_ok());
        assert!(result_shape(ResultStatus::Error, &None, &e("boom")).unwrap_err().contains("\"boom\""));
        assert_eq!(result_shape(ResultStatus::Error, &None, &None).unwrap_err(), "result error without a solution and without an error");
        assert_eq!(result_shape(ResultStatus::Cancelled, &None, &e("internal")).unwrap_err(), "result cancelled without a solution and with an error");
    }

    #[test]
    fn exploitability_percentages_are_checked() {
        assert_eq!(exploitability_pct(0.8, 100), Ok(0.8));
        assert_eq!(exploitability_pct(-0.0, 100).map(f32::to_bits), Ok(0.0f32.to_bits()));
        assert!(exploitability_pct(-0.5, 100).unwrap_err().contains("negative"));
        assert!(exploitability_pct(f32::MAX, 1).unwrap_err().contains("not a finite percentage"));
    }
}
