use proto::worker::{EngineMessage, Ready, WorkerMessage};
use std::time::Duration;

/// Every way the link to the worker can fail. None of them is a panic: a malformed, over-long or non-UTF-8 line
/// is a `Protocol`/`LineTooLong` error the caller answers by restarting the worker (spec 12).
#[derive(Debug, Clone, thiserror::Error)]
pub enum WorkerLinkError {
    /// The worker's stdout closed and no exit code could be confirmed (or there is no live worker: never
    /// started, killed, or a restart that failed).
    #[error("worker stdout closed")] Eof,
    /// A line that is not UTF-8, not a `WorkerMessage`, a `ready` that fails validation, or a message out of place.
    #[error("protocol: {0}")] Protocol(String),
    /// A line longer than the limit, counted with its terminator (spec 4.5: result line <= 16 MiB, request line
    /// <= 1 MiB); the number is the line's full length.
    #[error("line too long: {0} bytes")] LineTooLong(usize),
    /// The worker could not be started and validated (bounded: at most two launches, spec 4.5).
    #[error("spawn: {0}")] Spawn(String),
    /// The worker process is gone with this confirmed exit code (spec 10.3 `WorkerExit{code}`).
    #[error("worker exited with code {code}")] Exit { code: i32 },
}

/// The engine's only view of the worker (§3.1): a process in production, a scripted fake in tests.
pub trait WorkerLink: Send {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError>;
    /// `Ok(None)` on timeout; `Err(Eof | Exit)` once the process is gone.
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError>;
    /// Kill, reap, respawn and validate `ready` (§12).
    fn restart(&mut self) -> Result<(), WorkerLinkError>;
    fn kill(&mut self);
    fn ready(&self) -> Option<&Ready>;
    fn peak_working_set_bytes(&self) -> u64 { 0 }
}
