//! §10.3: the worker runs inside a Windows job object with a 16 GiB process memory limit, so a runaway allocation
//! kills the worker, not the machine; and with kill-on-close, so the worker dies with the engine (§4.5).
//!
//! Win32 calls (kernel32), confined to this module:
//! - `CreateJobObjectW(NULL, NULL)`: an anonymous job. NULL security attributes make the handle non-inheritable,
//!   so no child (the worker included) ever holds a handle to the job: the engine's is the only one.
//! - `SetInformationJobObject(job, JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION)` with
//!   `LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and
//!   `ProcessMemoryLimit = 16 GiB` (a commit above it fails inside the worker, which aborts: a typed
//!   `WorkerExit{code}` in the engine).
//! - `AssignProcessToJobObject(job, child)`.
//! - `CloseHandle(job)` when the `JobHandle` drops. That is the last handle to the job, so the OS terminates every
//!   process still in it. The OS closes a process's handles however it ends, so the same happens when the engine
//!   exits, crashes or is killed: the worker cannot outlive it.
//!
//! The one window: std's `Command` cannot start a child suspended, so the child runs for the instant between its
//! creation and `AssignProcessToJobObject`. The worker allocates no solve memory before its first `solve`, which the
//! engine sends only after `ready`, i.e. after the assignment; and if the engine died inside that instant, the worker
//! still exits on its own at stdin EOF (§4.5). `ProcessWorker` treats a failed assignment as a failed launch (fail closed).
use std::process::Child;

/// §10.3's `JOB_OBJECT_LIMIT_PROCESS_MEMORY` value (spec decision).
pub const PROCESS_MEMORY_LIMIT_BYTES: u64 = 16 << 30;

/// The engine's (only) handle to the worker's job. Dropping it closes the job, which kills whatever is still in it.
pub struct JobHandle {
    #[cfg(windows)]
    raw: isize,
    #[cfg(not(windows))]
    _none: (),
}

#[cfg(windows)]
mod imp {
    use super::{JobHandle, PROCESS_MEMORY_LIMIT_BYTES};
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    /// `JOBOBJECT_BASIC_LIMIT_INFORMATION` (winnt.h), x64 layout: 64 bytes.
    #[repr(C)]
    pub(super) struct BasicLimit {
        pub(super) per_process_user_time_limit: i64,
        pub(super) per_job_user_time_limit: i64,
        pub(super) limit_flags: u32,
        pub(super) minimum_working_set_size: usize,
        pub(super) maximum_working_set_size: usize,
        pub(super) active_process_limit: u32,
        pub(super) affinity: usize,
        pub(super) priority_class: u32,
        pub(super) scheduling_class: u32,
    }
    /// `IO_COUNTERS` (winnt.h): 48 bytes.
    #[repr(C)]
    pub(super) struct IoCounters { pub(super) counts: [u64; 6] }
    /// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` (winnt.h), x64 layout: 144 bytes.
    #[repr(C)]
    pub(super) struct ExtendedLimit {
        pub(super) basic: BasicLimit,
        pub(super) io: IoCounters,
        pub(super) process_memory_limit: usize,
        pub(super) job_memory_limit: usize,
        pub(super) peak_process_memory_used: usize,
        pub(super) peak_job_memory_used: usize,
    }
    #[cfg(target_pointer_width = "64")]
    const _: () = assert!(std::mem::size_of::<BasicLimit>() == 64 && std::mem::size_of::<ExtendedLimit>() == 144);

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const u8, name: *const u16) -> isize;
        fn SetInformationJobObject(job: isize, class: i32, info: *const u8, len: u32) -> i32;
        fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }
    pub(super) const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
    pub(super) const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    /// `JOBOBJECTINFOCLASS::JobObjectExtendedLimitInformation`.
    pub(super) const EXTENDED_LIMIT_INFORMATION: i32 = 9;

    fn last_error(call: &str) -> String { format!("{call} failed: {}", std::io::Error::last_os_error()) }

    pub fn assign(child: &Child) -> Result<JobHandle, String> {
        let limit = usize::try_from(PROCESS_MEMORY_LIMIT_BYTES).map_err(|_| "the 16 GiB limit does not fit a usize on this target".to_string())?;
        // SAFETY: both arguments may be NULL (default security, not inheritable; anonymous job).
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw == 0 { return Err(last_error("CreateJobObjectW")); }
        // From here the handle is owned: every early return below closes it through `JobHandle`'s `Drop`.
        let job = JobHandle { raw };
        // SAFETY: an all-zero JOBOBJECT_EXTENDED_LIMIT_INFORMATION is valid (no limits); the two fields set below are plain integers.
        let mut info: ExtendedLimit = unsafe { std::mem::zeroed() };
        info.basic.limit_flags = JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        info.process_memory_limit = limit;
        let len = std::mem::size_of::<ExtendedLimit>() as u32;
        // SAFETY: `info` is a live, correctly laid out JOBOBJECT_EXTENDED_LIMIT_INFORMATION of `len` bytes for this class.
        if unsafe { SetInformationJobObject(job.raw, EXTENDED_LIMIT_INFORMATION, &info as *const ExtendedLimit as *const u8, len) } == 0 {
            return Err(last_error("SetInformationJobObject"));
        }
        // SAFETY: `child`'s process handle is open for as long as `child` lives (CreateProcess grants PROCESS_ALL_ACCESS).
        if unsafe { AssignProcessToJobObject(job.raw, child.as_raw_handle() as isize) } == 0 {
            return Err(last_error("AssignProcessToJobObject"));
        }
        Ok(job)
    }

    impl Drop for JobHandle {
        fn drop(&mut self) {
            // SAFETY: `raw` is the job handle this value owns, closed exactly once, here.
            unsafe { CloseHandle(self.raw); }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::JobHandle;
    use std::process::Child;
    /// No job objects off Windows (the project targets Windows 11 only, §1): the worker still exits at stdin EOF.
    pub fn assign(_child: &Child) -> Result<JobHandle, String> { Ok(JobHandle { _none: () }) }
}

/// Places `child` in a new job with the 16 GiB per-process memory limit and kill-on-close. The returned handle
/// owns the job: drop it to kill whatever is still in the job.
pub fn assign(child: &Child) -> Result<JobHandle, String> { imp::assign(child) }

#[cfg(all(test, windows))]
impl JobHandle {
    /// Whether `child` is in THIS job (not merely in some job: the test runner may itself run inside one).
    pub(crate) fn contains(&self, child: &Child) -> bool {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        extern "system" { fn IsProcessInJob(process: isize, job: isize, result: *mut i32) -> i32; }
        let mut result = 0i32;
        // SAFETY: both handles are open; `result` is a valid out pointer.
        let ok = unsafe { IsProcessInJob(child.as_raw_handle() as isize, self.raw, &mut result) };
        assert!(ok != 0, "IsProcessInJob failed: {}", std::io::Error::last_os_error());
        result != 0
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::{ExtendedLimit, EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY};
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::process::{Command, Stdio};

    #[link(name = "kernel32")]
    extern "system" {
        fn QueryInformationJobObject(job: isize, class: i32, info: *mut u8, len: u32, returned: *mut u32) -> i32;
        fn WaitForSingleObject(handle: isize, ms: u32) -> u32;
    }
    const WAIT_OBJECT_0: u32 = 0;

    /// A process that lives until it is killed or its stdin closes: `cmd` reading commands from a pipe we hold open.
    fn waiting_child() -> Child {
        Command::new("cmd").args(["/d", "/q"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("spawn cmd")
    }

    /// The job carries exactly §10.3's limits, read back from the OS, and holds the child.
    #[test]
    fn the_job_holds_the_child_with_the_16_gib_limit_and_kill_on_close() {
        let mut child = waiting_child();
        let job = assign(&child).expect("assign");
        assert!(job.contains(&child));
        // SAFETY: zeroed is a valid buffer for the OS to fill; its size is passed.
        let mut info: ExtendedLimit = unsafe { std::mem::zeroed() };
        let mut returned = 0u32;
        let ok = unsafe { QueryInformationJobObject(job.raw, EXTENDED_LIMIT_INFORMATION, &mut info as *mut ExtendedLimit as *mut u8, std::mem::size_of::<ExtendedLimit>() as u32, &mut returned) };
        assert!(ok != 0, "QueryInformationJobObject failed: {}", std::io::Error::last_os_error());
        assert_eq!(returned as usize, std::mem::size_of::<ExtendedLimit>());
        assert_eq!(info.basic.limit_flags, JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE);
        assert_eq!(info.process_memory_limit as u64, PROCESS_MEMORY_LIMIT_BYTES);
        assert_eq!(PROCESS_MEMORY_LIMIT_BYTES, 16 * 1024 * 1024 * 1024);
        assert_eq!(info.job_memory_limit, 0, "the limit is per process, not per job");
        drop(job);
        let _ = child.wait();
    }

    /// Closing the engine's handle to the job ends the worker: this is what the OS does to every handle of an engine
    /// that exits, crashes or is killed, so the worker cannot outlive it (§4.5). The child is still holding its stdin
    /// open, so nothing but the job ends it; 10 s is a liveness bound, the termination is immediate.
    #[test]
    fn closing_the_job_kills_the_child() {
        let mut child = waiting_child();
        let job = assign(&child).expect("assign");
        assert!(child.try_wait().unwrap().is_none(), "the child is alive before the job closes");
        drop(job);
        // SAFETY: the child's process handle is open while `child` lives.
        let waited = unsafe { WaitForSingleObject(child.as_raw_handle() as isize, 10_000) };
        assert_eq!(waited, WAIT_OBJECT_0, "the child did not end when its job closed");
        assert!(child.try_wait().unwrap().is_some());
    }
}
