use proto::worker::{EngineMessage, Ready, WorkerMessage};
use std::time::Duration;

/// Every way the link to the worker can fail. None of them is a panic: a malformed, over-long or non-UTF-8 line
/// is a `Protocol`/`LineTooLong` error the caller answers by restarting the worker (spec 12).
#[derive(Debug, Clone, thiserror::Error)]
pub enum WorkerLinkError {
    /// The worker's stdout closed and its exit was not confirmed within the receiving call's budget, or there is no
    /// live worker (never started, killed, or a restart that failed). Unlike `Exit`, an unconfirmed end is not final:
    /// a later `recv` may confirm it (see `WorkerLink::recv`).
    #[error("worker stdout closed")] Eof,
    /// A line that is not UTF-8, not a `WorkerMessage`, a `ready` that fails validation, or a message out of place.
    #[error("protocol: {0}")] Protocol(String),
    /// A line longer than the limit, counted with its terminator (spec 4.5: result line <= 16 MiB, request line
    /// <= 1 MiB); the number is the line's full length.
    #[error("line too long: {0} bytes")] LineTooLong(usize),
    /// The worker could not be started and validated (bounded: at most two launches, spec 4.5).
    #[error("spawn: {0}")] Spawn(String),
    /// The worker process is gone with this confirmed exit code (spec 10.3 `WorkerExit{code}`). Final: once a link
    /// has reported it, it keeps reporting it.
    #[error("worker exited with code {code}")] Exit { code: i32 },
}

/// The engine's only view of the worker (§3.1): a process in production, a scripted fake in tests.
pub trait WorkerLink: Send {
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
}
