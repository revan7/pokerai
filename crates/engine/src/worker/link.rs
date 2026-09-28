use super::ready::ReadyRefusal;
use proto::worker::{EngineMessage, Ready, WorkerMessage};
use std::time::Duration;

/// Every way the link to the worker can fail. None of them is a panic: a malformed, over-long or non-UTF-8 line
/// is a `Protocol`/`LineTooLong` error the caller answers by restarting the worker (spec 12).
#[derive(Debug, Clone, thiserror::Error)]
pub enum WorkerLinkError {
    /// The worker's stdout closed (`recv`) or it stopped taking requests on stdin (`send`), and its exit was not
    /// confirmed within that call's budget; or there is no live worker (never started, killed, or a restart that
    /// failed). Unlike `Exit`, an unconfirmed end is not final: a later `recv` may confirm it (see `WorkerLink`).
    #[error("worker stdout closed")] Eof,
    /// A line that is not UTF-8, not a `WorkerMessage`, a `ready` that fails validation, or a message out of place.
    #[error("protocol: {0}")] Protocol(String),
    /// A line longer than the limit, counted with its terminator (spec 4.5: result line <= 16 MiB, request line
    /// <= 1 MiB); the number is the line's full length.
    #[error("line too long: {0} bytes")] LineTooLong(usize),
    /// The worker could not be started and validated (bounded: at most two launches, spec 4.5): the binary did not
    /// start, exited, stayed silent or wrote something other than `ready` first. Possibly transient.
    #[error("spawn: {0}")] Spawn(String),
    /// The worker at `exe` started and wrote a `ready` that fails validation (spec 4.5): permanent for that binary, which
    /// reports the same values at every launch, so it is not relaunched (spec 12, "until rebuilt"; follow-up P2.W2).
    #[error("{exe}: ready refused: {refusal}")] ReadyRefused { exe: String, refusal: ReadyRefusal },
    /// The worker process is gone with this confirmed exit code (spec 10.3 `WorkerExit{code}`). Final for the
    /// process: every later `send` reports it, and `recv` reports it after returning every line the worker wrote
    /// before exiting (a `send` may report it first, while such lines are still queued), then keeps reporting it.
    #[error("worker exited with code {code}")] Exit { code: i32 },
}

/// The engine's only view of the worker (§3.1): a process in production, a scripted fake in tests.
pub trait WorkerLink: Send {
    /// Queues one request line for the worker and returns at once. `send` never waits for the worker: its budget for
    /// confirming an exit is zero, one non-blocking poll of the process, so a caller's cancellation bound (§7/§12)
    /// never waits on it.
    /// - `Ok(())`: queued. Delivery is not confirmed here; the reply (or its absence) arrives through `recv`.
    /// - `Err(Protocol | LineTooLong)`: the request cannot be written as a valid line (§4.5); nothing was queued.
    /// - `Err(Exit{code})`: the process has exited, confirmed by that poll or earlier by `recv`.
    /// - `Err(Eof)`: the worker no longer takes requests (its stdin closed) and the poll did not confirm an exit, or
    ///   there is no live worker. Never recorded as a confirmed exit: `recv` confirms it later within its own budget,
    ///   and a caller that cannot wait kills the worker (§12).
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError>;
    /// The next message, within `timeout` for the WHOLE call: one monotonic deadline covers waiting for a line and,
    /// once the worker's stdout has ended, confirming its exit. No part of the call waits past it, so a caller that
    /// passes its remaining cancellation budget (§7/§12: kill after 1.5 s) keeps its bound.
    /// - `Ok(Some(msg))`: the next message; `Ok(None)`: the timeout passed with nothing to report.
    /// - `Err(Protocol | LineTooLong)`: one faulty line (the caller restarts the worker, §12).
    /// - `Err(Exit{code})`: stdout ended after every line before it was returned, and the exit was confirmed within
    ///   this call's budget. Final: every later call answers the same at once.
    /// - `Err(Eof)`: stdout ended but the exit was not confirmed within this call's budget (the process may still be
    ///   exiting, or may have closed its stdout and live on), or there is no live worker. An unconfirmed end is never
    ///   taken for a confirmed exit: a later `recv` tries again within its own budget and answers `Exit{code}` once
    ///   the exit is confirmed, `Eof` until then. A caller that cannot wait for it kills the worker (§12).
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError>;
    /// Kill, reap, respawn and validate `ready` (§12).
    fn restart(&mut self) -> Result<(), WorkerLinkError>;
    fn kill(&mut self);
    fn ready(&self) -> Option<&Ready>;
    fn peak_working_set_bytes(&self) -> u64 { 0 }
    /// The permanent `ready` refusal this link stands for, when it is a degraded engine's (`RefusedWorker`): every
    /// request is then answered with it and nothing is launched (spec 12). `None` for a link that can launch a worker.
    fn refused(&self) -> Option<&ReadyRefusal> { None }
}

/// The link of a degraded engine (spec 12; ruling 29-I4): the worker binary at `exe` refused `ready` at startup
/// (`WorkerLinkError::ReadyRefused`), a refusal that holds until the binary is rebuilt. There is no worker behind it and
/// none is ever launched: every call answers that refusal at once (`send`, `recv`, `restart`), `kill` has nothing to
/// kill, and `refused` names it, so `serve_request` answers every decision with the §12 version mismatch before any
/// solve.
#[derive(Debug, Clone)]
pub struct RefusedWorker {
    exe: String,
    refusal: ReadyRefusal,
}

impl RefusedWorker {
    pub fn new(exe: String, refusal: ReadyRefusal) -> Self { Self { exe, refusal } }

    fn error(&self) -> WorkerLinkError { WorkerLinkError::ReadyRefused { exe: self.exe.clone(), refusal: self.refusal.clone() } }
}

impl WorkerLink for RefusedWorker {
    fn send(&mut self, _msg: &EngineMessage) -> Result<(), WorkerLinkError> { Err(self.error()) }
    fn recv(&mut self, _timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> { Err(self.error()) }
    /// Never relaunches the refused build.
    fn restart(&mut self) -> Result<(), WorkerLinkError> { Err(self.error()) }
    fn kill(&mut self) {}
    fn ready(&self) -> Option<&Ready> { None }
    fn refused(&self) -> Option<&ReadyRefusal> { Some(&self.refusal) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A degraded engine's link answers its refusal to everything, relaunches nothing and has no `ready`.
    #[test]
    fn a_refused_worker_answers_its_refusal_and_launches_nothing() {
        let refusal = ReadyRefusal::ProtoVersion { reported: 2 };
        let mut w = RefusedWorker::new("solver-worker.exe".into(), refusal.clone());
        let is_refusal = |e: WorkerLinkError| matches!(e, WorkerLinkError::ReadyRefused { exe, refusal: r } if exe == "solver-worker.exe" && r == refusal);
        assert!(is_refusal(w.send(&EngineMessage::Shutdown { id: "1".into() }).unwrap_err()));
        assert!(is_refusal(w.recv(Duration::from_secs(3_600)).unwrap_err()));
        assert!(is_refusal(w.restart().unwrap_err()));
        w.kill();
        assert!(w.ready().is_none());
        assert_eq!(w.refused(), Some(&refusal));
        assert_eq!(w.error().to_string(), format!("solver-worker.exe: ready refused: proto_version 2 != {}", proto::worker::PROTO_VERSION));
    }
}
