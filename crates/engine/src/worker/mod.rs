//! The engine's side of the worker process (spec 3.1, 4.5, 10.3, 12): the `WorkerLink` trait every
//! caller talks to, the production `ProcessWorker`, `ready` validation, and the Windows job object the
//! worker runs in. Every Win32 call is confined to `process.rs` and `job_object.rs`.
pub mod job_object;
pub mod link;
pub mod process;
pub mod ready;
pub use link::{RefusedWorker, WorkerLink, WorkerLinkError};
pub use process::ProcessWorker;
