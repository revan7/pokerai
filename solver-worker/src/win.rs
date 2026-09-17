//! §3.4: the worker runs at BELOW_NORMAL while a `background: true` job is solving, NORMAL otherwise.
//! The child's peak working set is measured by the ENGINE from the parent process
//! (`crates/engine/src/worker/process.rs`), so the worker exposes no memory query of its own.
#[cfg(windows)]
mod imp {
    #[link(name = "kernel32")]
    extern "system" { fn GetCurrentProcess() -> isize; fn SetPriorityClass(h: isize, class: u32) -> i32; }
    const NORMAL_PRIORITY_CLASS: u32 = 0x20;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x4000;
    pub fn set_priority_class(below_normal: bool) {
        unsafe { SetPriorityClass(GetCurrentProcess(), if below_normal { BELOW_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS }); }
    }
}
#[cfg(not(windows))]
mod imp {
    pub fn set_priority_class(_below_normal: bool) {}
}
pub use imp::set_priority_class;
