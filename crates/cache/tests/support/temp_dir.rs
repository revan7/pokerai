//! The one per-invocation temp-directory helper the cache's filesystem tests use (plan 4 task 5
//! review R5; task 6 review R7). Every test crate that needs it -- `crates/cache/tests/quota.rs`,
//! `crates/cache/tests/storage.rs`, and the unit tests in `crates/cache/src/storage.rs` -- includes
//! this same file with `#[path]`, so there is exactly one definition to maintain. It is
//! deliberately not declared in `support/mod.rs`: the test crates that never touch the filesystem
//! would then compile an unused type.
//!
//! A directory is named by label, process id and a process-wide counter, so two guards in one
//! test binary -- even on different threads -- never share a name. It is created with an
//! *exclusive* `std::fs::create_dir` (never `create_dir_all`, which would silently adopt whatever
//! is already there) and removed on `Drop`, which also runs while a failing assertion unwinds, so
//! a failed test cleans up after itself. A directory already present at a fresh name can only be a
//! leftover of a dead process that had this process id (the counter makes every name unique
//! within this process), so `fresh` removes that stale leftover first and then still creates the
//! directory exclusively: a test never inherits a previous run's files, and never fails because a
//! previous run leaked them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static UNIQUE: AtomicU64 = AtomicU64::new(0);

pub struct TempDir(PathBuf);

impl TempDir {
    /// A fresh directory under the system temp dir, unique to this process and this call.
    pub fn new(label: &str) -> Self {
        let id = UNIQUE.fetch_add(1, Ordering::Relaxed);
        Self::fresh(std::env::temp_dir().join(format!("pokerai-cache-{label}-{}-{id}", std::process::id())))
    }

    /// Takes ownership of `dir` as a fresh, empty directory: a stale leftover at that path is
    /// removed first, then the directory is created exclusively.
    ///
    /// # Panics
    /// If a stale leftover cannot be removed, or the exclusive creation fails.
    pub fn fresh(dir: PathBuf) -> Self {
        if std::fs::symlink_metadata(&dir).is_ok() {
            std::fs::remove_dir_all(&dir).unwrap_or_else(|e| panic!("TempDir must clear the stale leftover at {dir:?}: {e}"));
        }
        std::fs::create_dir(&dir).unwrap_or_else(|e| panic!("TempDir must get a fresh, exclusively-created directory at {dir:?}: {e}"));
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
