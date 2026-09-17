//! On-disk street-solution cache: canonical structural keys, the bincode+zstd cell store, the
//! replacement/quota policy, the `Cache` handle with its single writer thread, and the background
//! pre-solver queue (spec section 10.4-10.5).

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("invalid cache value: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Codec(#[from] bincode::Error),
}

pub mod entry;
pub mod key;
pub mod label;
pub mod lookup;
pub mod presolver;
pub mod quota;
pub mod storage;

/// Default `cache_quota_bytes` (spec section 10.4): 10 GiB.
pub const CACHE_QUOTA_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Section 10.4 storage root. `engine::Paths.cache` normally supplies it (cross-plan M13/Or5);
/// this fallback exists for tools and tests that have no `Paths`.
pub fn default_cache_root() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("PokerAI").join("cache").join("v3")
}

/// What the writer thread accepts. Every variant is applied by `crate::quota`, and every one is
/// followed by a quota pass, so no command can leave the store over its byte budget.
pub enum WriteCommand {
    /// Store this entry, optionally reporting the outcome once the write *and* its quota pass are
    /// done (see `StoreReceipt`).
    Store(Box<crate::entry::CacheEntry>, Option<std::sync::mpsc::SyncSender<bool>>),
    /// Re-date the entry with this payload digest in this cell: it was just served.
    Touch { key: [u8; 32], payload_digest: Vec<u8>, last_hit: u64 },
    /// Remove this cell outright (used by the pre-solver's reconciliation, plan 4 task 14+).
    Delete([u8; 32]),
    Shutdown,
}

/// The reader thread's command channel type. Task 7 owns the bounded reader thread and the rest
/// of its variants; this task needs only the `Shutdown` the handle's own `shutdown` sends, so
/// `Cache::reader` and `Cache::shutdown` have the shape the plan specifies while `reader` stays
/// `None`. Plan 4 task 6 spells this type `crate::lookup::ReadCommand`; it is declared here
/// because `crates/cache/src/lookup.rs` is outside this task's file list -- task 7 can keep it
/// here or add `pub use crate::ReadCommand;` to `lookup`, which makes that path resolve without
/// moving the definition or changing this field's type.
#[derive(Debug)]
pub enum ReadCommand {
    Shutdown,
}

/// A pending `store_tracked` result. Awaiting it is how the pre-solver (task 16) confirms its
/// solution really is on disk before recording a scenario as done; a live delivery path never
/// waits on one.
pub struct StoreReceipt(Option<std::sync::mpsc::Receiver<bool>>);

impl StoreReceipt {
    /// True only when the writer reported a durable, validated cell on disk. The writer answers
    /// after that store's quota pass has finished, so a `true` also means the store is not about
    /// to be undone by eviction in the same pass, and a caller that then reads the cell sees the
    /// settled state. False covers every other case: a disabled cache, a full or disconnected
    /// writer queue, a rejected or unwritable entry, and a writer that did not answer inside
    /// `budget`.
    pub fn wait(self, budget: std::time::Duration) -> bool {
        self.0.map_or(false, |rx| rx.recv_timeout(budget).unwrap_or(false))
    }
}

/// The cache handle: a root directory plus the endpoints of the two bounded service threads that
/// own all of its I/O (`cache-writer` here, the reader in task 7). Cheap to share; every method
/// takes `&self`, and no mutex is held across any file operation.
pub struct Cache {
    root: std::path::PathBuf,
    writer: Option<std::sync::mpsc::SyncSender<WriteCommand>>,
    reader: Option<std::sync::mpsc::SyncSender<crate::ReadCommand>>, // Task 7
    skipped: std::sync::atomic::AtomicBool,
}

impl Cache {
    /// A handle that stores nothing and serves nothing: every lookup is a miss and every store is
    /// dropped. Used wherever the cache is switched off or could not be opened (section 12: the
    /// recommendation path is unaffected either way).
    pub fn disabled() -> Cache {
        Cache { root: std::path::PathBuf::new(), writer: None, reader: None, skipped: std::sync::atomic::AtomicBool::new(false) }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// Opens the store at `root` and starts its writer thread, which rebuilds the accounting index
    /// from disk (`quota::scan_index`) and clears any temp files a previous run's crash left
    /// behind (`quota::sweep_temporaries`) before it accepts its first command.
    ///
    /// A failure to create the directory yields a disabled cache: every lookup is `Miss`
    /// and every store is dropped, and the recommendation path is unaffected (section 12).
    pub fn open(root: std::path::PathBuf, quota_bytes: u64) -> Cache {
        if std::fs::create_dir_all(&root).is_err() {
            return Cache::disabled();
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<WriteCommand>(8);
        let dir = root.clone();
        if std::thread::Builder::new()
            .name("cache-writer".into())
            .spawn(move || {
                let mut index = crate::quota::scan_index(&dir);
                crate::quota::sweep_temporaries(&dir);
                let mut max_last_hit = index.iter().map(|r| r.last_hit).max().unwrap_or(0);
                while let Ok(command) = rx.recv() {
                    // The receipt is answered at the end of the iteration, after this command's
                    // quota pass, so a waiting caller that then reads the cell sees the settled
                    // state rather than one an eviction is about to change.
                    let mut receipt = None;
                    let mut stored = false;
                    match command {
                        WriteCommand::Shutdown => break,
                        WriteCommand::Delete(key) => {
                            let _ = std::fs::remove_file(crate::storage::entry_path(&dir, key));
                            index.retain(|r| r.key != key);
                        }
                        WriteCommand::Touch { key, payload_digest, last_hit } => {
                            max_last_hit = next_last_hit(max_last_hit, last_hit);
                            let _ = crate::quota::apply_touch(&dir, key, &payload_digest, max_last_hit, &mut index);
                        }
                        WriteCommand::Store(entry, sender) => {
                            max_last_hit = next_last_hit(max_last_hit, entry.last_hit);
                            let mut entry = *entry;
                            entry.last_hit = max_last_hit;
                            receipt = sender;
                            stored = crate::quota::store_entry(&dir, entry, &mut index).is_ok();
                        }
                    }
                    crate::quota::enforce_quota(&dir, quota_bytes, &mut index);
                    if let Some(r) = receipt {
                        let _ = r.try_send(stored);
                    }
                }
            })
            .is_err()
        {
            return Cache::disabled();
        }
        Cache { root, writer: Some(tx), reader: None, skipped: std::sync::atomic::AtomicBool::new(false) }
    }

    fn send(&self, command: WriteCommand) {
        let Some(tx) = self.writer.as_ref() else { return };
        if tx.try_send(command).is_err() && !self.skipped.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("cache write skipped: writer queue full or gone (not reported again this session)");
        }
    }

    /// Queues `entry` for storage. Best-effort and nonblocking: a full or gone writer queue drops
    /// the write (logged once per session) rather than delay whatever is on the delivery path.
    pub fn store(&self, entry: &crate::entry::CacheEntry) {
        self.send(WriteCommand::Store(Box::new(entry.clone()), None));
    }

    /// Used by the pre-solver only (task 16); never on a live delivery path.
    pub fn store_tracked(&self, entry: &crate::entry::CacheEntry) -> StoreReceipt {
        let Some(tx) = self.writer.as_ref() else { return StoreReceipt(None) };
        let (rtx, rrx) = std::sync::mpsc::sync_channel(1);
        if tx.try_send(WriteCommand::Store(Box::new(entry.clone()), Some(rtx))).is_err() {
            return StoreReceipt(None);
        }
        StoreReceipt(Some(rrx))
    }

    /// Records that the entry with this payload digest (`quota::entry_digest`) in cell `key` was
    /// just served, so eviction sees it as fresh. `last_hit` is the caller's clock reading; the
    /// writer persists `max(last_hit, its own maximum + 1)` so a clock reversal cannot make a
    /// fresh hit look like the oldest entry in the store.
    pub fn touch(&self, key: [u8; 32], payload_digest: Vec<u8>, last_hit: u64) {
        self.send(WriteCommand::Touch { key, payload_digest, last_hit });
    }

    /// Asks both service threads to finish. Blocking (unlike every other method here): the
    /// command goes through even if the queue is momentarily full, so shutdown cannot be lost.
    pub fn shutdown(&self) {
        if let Some(tx) = self.writer.as_ref() {
            let _ = tx.send(WriteCommand::Shutdown);
        }
        if let Some(tx) = self.reader.as_ref() {
            let _ = tx.send(crate::ReadCommand::Shutdown);
        }
    }
}

/// The `last_hit` the writer persists for a hit whose caller-supplied timestamp is `proposed`:
/// `max(proposed, previous + 1)`, so the value is strictly monotone across the process's whole
/// run *and* across restarts (the maximum is rebuilt from disk on open) even when the system
/// clock jumps backwards -- a fresh hit can never be recorded as the oldest entry in the store.
/// `saturating_add` keeps that true at the top of the range instead of wrapping to zero.
fn next_last_hit(previous: u64, proposed: u64) -> u64 {
    proposed.max(previous.saturating_add(1))
}
