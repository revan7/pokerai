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
    /// Remove this cell outright (used by the pre-solver's reconciliation, plan 4 task 14+). A
    /// deletion the filesystem refuses keeps the cell accounted until a later pass removes it.
    Delete([u8; 32]),
    Shutdown,
}

/// A pending `store_tracked` result. Awaiting it is how the pre-solver (task 16) confirms its
/// solution really is on disk before recording a scenario as done; a live delivery path never
/// waits on one.
pub struct StoreReceipt(Option<std::sync::mpsc::Receiver<bool>>);

impl StoreReceipt {
    /// The store's outcome *after* its own quota pass (review R2): true only when the entry was
    /// validated, published, and is still on disk once that pass has run -- so a caller that then
    /// reads the cell sees it. False covers every other case: a disabled cache, a full or
    /// disconnected writer queue, a rejected or unwritable entry, a `last_hit` above
    /// `quota::LAST_HIT_MAX`, a *dominated* insertion (`quota::retain_two` kept the cell's
    /// existing representatives instead of it, so this entry was not stored -- the cell is left as
    /// it was), a store the same quota pass evicted again (a quota smaller than its cell), and a
    /// writer that did not answer inside `budget`.
    pub fn wait(self, budget: std::time::Duration) -> bool {
        self.0.map_or(false, |rx| rx.recv_timeout(budget).unwrap_or(false))
    }
}

/// The cache handle: a root directory plus the endpoints of the two bounded service threads that
/// own all of its I/O (`cache-writer` and `cache-reader`). Cheap to share; every method takes
/// `&self`, and no mutex is held across any file operation.
///
/// Thread ownership (plan 4 task 7): the handle owns the `cache-reader` thread and joins it on
/// `Drop`, after closing its queue -- the reads still queued are bounded (at most the queue's four
/// requests plus the one in hand, three cells each), so the join is too. The writer ends on its own
/// once this handle, its only sender, is gone (task 6); `shutdown` stops both early.
pub struct Cache {
    root: std::path::PathBuf,
    writer: Option<std::sync::mpsc::SyncSender<WriteCommand>>,
    reader: Option<std::sync::mpsc::SyncSender<crate::lookup::ReadCommand>>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
    /// The token of the next `ReadCommand::Cells`; starts at 1 and only ever increases.
    next_token: std::sync::atomic::AtomicU64,
    /// Why an `open` yielded a handle that cannot serve (`availability_warning`).
    warning: Option<String>,
    skipped: std::sync::atomic::AtomicBool,
}

impl Cache {
    /// A handle that stores nothing and serves nothing: every lookup is a miss and every store is
    /// dropped. Used wherever the cache is switched off or could not be opened (section 12: the
    /// recommendation path is unaffected either way).
    pub fn disabled() -> Cache {
        Cache {
            root: std::path::PathBuf::new(),
            writer: None,
            reader: None,
            reader_thread: None,
            next_token: std::sync::atomic::AtomicU64::new(1),
            warning: None,
            skipped: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The disabled handle `open` falls back to, carrying the reason for the startup banner.
    fn unavailable(warning: String) -> Cache {
        let mut cache = Cache::disabled();
        cache.warning = Some(warning);
        cache
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// Section 12: why this handle, returned by `open`, cannot serve lookups or store entries --
    /// the root directory could not be created, or a service thread did not start -- as one line
    /// for the startup banners (`engine::StartupReport::banners`). `None` for a working cache and
    /// for a deliberately `disabled` one.
    pub fn availability_warning(&self) -> Option<String> {
        self.warning.clone()
    }

    /// Opens the store at `root` and starts its writer thread, which rebuilds the accounting index
    /// from disk (`quota::scan_index`) and clears any temp files a previous run's crash left
    /// behind (`quota::sweep_temporaries`) before it accepts its first command, then its reader
    /// thread (`start_reader`).
    ///
    /// A failure to create the directory yields a disabled cache: every lookup is `Miss`
    /// and every store is dropped, and the recommendation path is unaffected (section 12). Its
    /// `availability_warning` says why.
    pub fn open(root: std::path::PathBuf, quota_bytes: u64) -> Cache {
        if let Err(error) = std::fs::create_dir_all(&root) {
            return Cache::unavailable(format!(
                "the flop cache at {} could not be opened ({error}); every cache lookup misses and nothing is stored this session",
                root.display()
            ));
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<WriteCommand>(8);
        let dir = root.clone();
        if std::thread::Builder::new()
            .name("cache-writer".into())
            .spawn(move || {
                let mut index = crate::quota::scan_index(&dir);
                crate::quota::sweep_temporaries(&dir);
                let mut max_last_hit = index.max_last_hit();
                while let Ok(command) = rx.recv() {
                    // The receipt is answered at the end of the iteration, after this command's
                    // quota pass, with whether the stored entry is *still* indexed -- i.e. still on
                    // disk -- once that pass has run (review R2).
                    let mut receipt = None;
                    let mut stored = None;
                    match command {
                        WriteCommand::Shutdown => break,
                        WriteCommand::Delete(key) => {
                            crate::quota::delete_cell(&dir, key, &mut index);
                        }
                        WriteCommand::Touch { key, payload_digest, last_hit } => {
                            // An unusable timestamp re-dates nothing and leaves the counter alone.
                            if let Some(hit) = next_last_hit(max_last_hit, last_hit) {
                                max_last_hit = hit;
                                let _ = crate::quota::apply_touch(&dir, key, &payload_digest, hit, &mut index);
                            }
                        }
                        WriteCommand::Store(entry, sender) => {
                            receipt = sender;
                            // An unusable timestamp refuses the store and leaves the counter alone.
                            if let Some(hit) = next_last_hit(max_last_hit, entry.last_hit) {
                                max_last_hit = hit;
                                let mut entry = *entry;
                                entry.last_hit = hit;
                                stored = crate::quota::store_entry(&dir, entry, &mut index).ok().flatten();
                            }
                        }
                    }
                    crate::quota::enforce_quota(&dir, quota_bytes, &mut index);
                    if let Some(r) = receipt {
                        let _ = r.try_send(stored.is_some_and(|entry| index.contains(&entry)));
                    }
                }
            })
            .is_err()
        {
            return Cache::unavailable(format!(
                "the flop cache at {} is unavailable (its writer thread did not start); every cache lookup misses and nothing is stored this session",
                root.display()
            ));
        }
        let mut cache = Cache::disabled();
        cache.root = root;
        cache.writer = Some(tx);
        cache.start_reader();
        cache
    }

    /// Starts the one `cache-reader` thread that owns every cell read (installed by `open` once the
    /// writer runs). Each `ReadCommand::Cells` reads its distinct cell keys through
    /// `storage::read_cell` -- which deletes only a file that fails `decode`/`validate_entry` -- and
    /// answers without blocking; a reply whose waiter has gone is dropped. Does nothing on a handle
    /// without a writer (a disabled cache never reads) or one whose reader already runs: a reader is
    /// never respawned. A thread that does not start leaves every lookup a miss, with a warning.
    pub fn start_reader(&mut self) {
        if self.writer.is_none() || self.reader.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<crate::lookup::ReadCommand>(4);
        let dir = self.root().to_path_buf();
        let spawned = std::thread::Builder::new().name("cache-reader".into()).spawn(move || {
            while let Ok(command) = rx.recv() {
                match command {
                    crate::lookup::ReadCommand::Shutdown => break,
                    crate::lookup::ReadCommand::Cells { token, keys, reply } => {
                        let mut seen = std::collections::BTreeSet::new();
                        let cells = keys
                            .iter()
                            .filter(|k| seen.insert(**k))
                            .filter_map(|k| crate::storage::read_cell(&crate::storage::entry_path(&dir, *k)))
                            .collect::<Vec<_>>();
                        let _ = reply.try_send((token, cells));
                    }
                }
            }
        });
        match spawned {
            Ok(handle) => {
                self.reader = Some(tx);
                self.reader_thread = Some(handle);
            }
            Err(error) => {
                self.warning = Some(format!(
                    "the flop cache at {} cannot serve lookups (its reader thread did not start: {error}); every cache lookup misses this session",
                    self.root.display()
                ));
            }
        }
    }

    /// Spec 10.4 lookup, bounded: posts one read of the query's `b - 1, b, b + 1` cells to the
    /// reader without blocking and waits at most `min(q.budget, 500 ms)` for its reply. A full
    /// queue, a closed channel, a timeout or a reply carrying another request's token is an
    /// immediate `Miss`; so is a disabled cache. The reply is then filtered and ranked
    /// (`lookup::select`), rebuilt in the query's chips and suits (`lookup::reconstruct`), and
    /// labelled from the entry's inherited reasons, the query's own reasons and its raw accuracy
    /// (`label::label`). A served hit is touched so eviction sees it as fresh. An ordinary query
    /// mismatch is a `Miss` and never deletes anything; only `read_cell` deletes, and only a file
    /// that fails `decode`/`validate_entry`. The caller runs this on the request's own
    /// `fast-path` work, never on `watchdog` or `engine-main`.
    pub fn lookup(&self, q: &crate::lookup::CacheQuery) -> crate::lookup::Lookup {
        use crate::label::Label;
        use crate::lookup::{Lookup, ReadCommand};
        let Some(reader) = self.reader.as_ref() else { return Lookup::Miss };
        let token = self.next_token.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let b = q.key.spr_bucket;
        let keys = [q.key.at_bucket(b.saturating_sub(1)).digest(), q.key.digest(), q.key.at_bucket(b.saturating_add(1)).digest()];
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        if reader.try_send(ReadCommand::Cells { token, keys, reply: tx }).is_err() {
            return Lookup::Miss;
        }
        let budget = q.budget.min(std::time::Duration::from_millis(500));
        let Ok((replied, cells)) = rx.recv_timeout(budget) else { return Lookup::Miss };
        if replied != token {
            return Lookup::Miss;
        }
        let Some((entry, comparison)) = crate::lookup::select(cells, q) else { return Lookup::Miss };
        let Some(mut hit) = crate::lookup::reconstruct(&entry, q) else { return Lookup::Miss };
        let label = crate::label::label(&entry, q.source.spr, &comparison, q.target_bp, &q.reasons);
        hit.coverage = label.coverage();
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        self.touch(entry.key.digest(), crate::lookup::payload_digest(&entry), now_ms);
        match label {
            Label::Exact => Lookup::Exact { hit },
            Label::Approximate { reasons } => Lookup::Approximate { hit, reasons },
            Label::Provisional { reasons } => Lookup::Provisional { hit, reasons },
        }
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
    /// fresh hit look like the oldest entry in the store. A reading above `quota::LAST_HIT_MAX`
    /// is refused: the touch re-dates nothing.
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
            let _ = tx.send(crate::lookup::ReadCommand::Shutdown);
        }
    }
}

impl Drop for Cache {
    /// Closes the reader's queue (this handle holds its only sender) and joins `cache-reader`,
    /// which finishes the bounded reads already queued and ends; a reply nobody awaits any more is
    /// dropped. The writer is not joined here: it ends once its own queue closes with this handle.
    fn drop(&mut self) {
        self.reader = None;
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

/// The `last_hit` the writer persists for a hit whose caller-supplied timestamp is `proposed`:
/// `max(proposed, previous + 1)`, so the value is strictly monotone across the process's whole
/// run *and* across restarts (the maximum is rebuilt from disk on open) even when the system
/// clock jumps backwards -- a fresh hit can never tie with, let alone be recorded as older than,
/// any entry already in the store (review R5).
///
/// `None` when `proposed` is above `quota::LAST_HIT_MAX`, *or* when the monotonic bump
/// (`previous + 1`) alone would land above it: either way the result would be unusable, so it is
/// refused, never clamped, and the caller leaves the counter where it was (fix round 2 N1 -- the
/// re-review found that the old code let `proposed.max(bumped)` exceed `LAST_HIT_MAX` whenever
/// `previous` was already at the ceiling, even though `proposed` alone was usable; the writer then
/// persisted that value, and its own next restart deleted the cell as unusable). Validating the
/// bump against the same ceiling as `proposed` closes that gap: the writer can never itself
/// produce a value `quota::scan_index` would consider corrupt. The `+ 1` is checked, never
/// saturating.
///
/// # Panics
/// If `previous` is `u64::MAX` (always-on, standing ruling (b)) -- true arithmetic overflow,
/// distinct from the (far lower) `LAST_HIT_MAX` ceiling this function also enforces. The writer's
/// counter cannot get there: it starts at most `LAST_HIT_MAX` (`quota::scan_index` never indexes a
/// row above it) and this function refuses to raise it any higher, so reaching `u64::MAX` is
/// unreachable in practice, not just distant.
fn next_last_hit(previous: u64, proposed: u64) -> Option<u64> {
    if proposed > crate::quota::LAST_HIT_MAX {
        return None;
    }
    let Some(bumped) = previous.checked_add(1) else {
        panic!("last_hit counter exhausted at {previous}: the writer's strictly increasing counter has no successor");
    };
    if bumped > crate::quota::LAST_HIT_MAX {
        return None;
    }
    Some(proposed.max(bumped))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review R5, tightened by fix round 2 N1: `max(proposed, previous + 1)` for every usable
    /// proposal, strictly increasing up to and including `LAST_HIT_MAX` -- but never past it. A
    /// proposal above `LAST_HIT_MAX` is refused (`None`) exactly as before; *now* a bump that would
    /// land above `LAST_HIT_MAX` is refused the same way, even though `previous` and `proposed` are
    /// each individually usable, so the writer can never itself persist an over-ceiling value.
    #[test]
    fn next_last_hit_is_strictly_monotone_and_refuses_unusable_timestamps() {
        let ceiling = crate::quota::LAST_HIT_MAX;
        assert_eq!(next_last_hit(0, 5), Some(5), "a caller clock ahead of the counter is honored");
        assert_eq!(next_last_hit(10, 5), Some(11), "a caller clock behind the counter cannot make a hit older");
        assert_eq!(next_last_hit(ceiling - 1, 1), Some(ceiling), "the bump from one below the ceiling lands exactly on it");
        assert_eq!(next_last_hit(ceiling - 1, ceiling), Some(ceiling));
        assert_eq!(next_last_hit(7, ceiling), Some(ceiling), "LAST_HIT_MAX itself is usable");
        assert_eq!(next_last_hit(7, ceiling + 1), None, "a timestamp above LAST_HIT_MAX is refused");
        assert_eq!(next_last_hit(7, u64::MAX), None);
        assert_eq!(next_last_hit(ceiling, 1), None, "a bump that would land one past LAST_HIT_MAX is refused, not persisted above it (fix round 2 N1)");
        assert_eq!(next_last_hit(ceiling, ceiling), None, "even a proposal that is itself usable is refused once the bump alone would exceed the ceiling");
        assert_eq!(next_last_hit(u64::MAX - 1, 0), None, "checked arithmetic reaches u64::MAX without overflow, but that is still far past LAST_HIT_MAX and is refused");
    }

    /// Review R5: the counter's `+ 1` is checked, never saturating -- at `u64::MAX` it is an
    /// always-on assertion, not a silent tie with the previous hit.
    #[test]
    #[should_panic(expected = "last_hit counter exhausted")]
    fn next_last_hit_never_saturates() {
        let _ = next_last_hit(u64::MAX, 0);
    }

    /// Review R4: `Cache::reader` carries the canonical `crate::lookup::ReadCommand`. This is
    /// exactly what task 7's `start_reader` does -- build `sync_channel::<ReadCommand>(4)` and
    /// assign the sender to `self.reader` -- and `shutdown` must reach that reader.
    #[test]
    fn the_reader_channel_carries_lookup_read_command() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<crate::lookup::ReadCommand>(4);
        let mut cache = Cache::disabled();
        cache.reader = Some(tx);
        cache.shutdown();
        assert!(matches!(rx.try_recv(), Ok(crate::lookup::ReadCommand::Shutdown)), "shutdown must reach the reader as lookup::ReadCommand::Shutdown");
    }

    /// Section 12 (plan 4 task 7): an `open` that cannot create its root is the disabled handle
    /// with a startup banner saying why; a working cache and a deliberately disabled one have none.
    #[test]
    fn availability_warning_names_an_unopenable_root_only() {
        let dir = std::env::temp_dir().join(format!("pokerai-cache-warning-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("not-a-directory");
        std::fs::write(&blocker, b"a file where the cache root should be").unwrap();
        let failed = Cache::open(blocker.join("v3"), CACHE_QUOTA_BYTES);
        let warning = failed.availability_warning().expect("an unopenable root is announced");
        assert!(warning.contains("could not be opened") && warning.contains("not-a-directory"), "{warning}");
        assert!(failed.reader.is_none() && failed.writer.is_none() && failed.root.as_os_str().is_empty());
        let working = Cache::open(dir.join("cache"), CACHE_QUOTA_BYTES);
        assert_eq!(working.availability_warning(), None);
        assert!(working.reader.is_some() && working.reader_thread.is_some(), "open starts the reader");
        assert_eq!(Cache::disabled().availability_warning(), None);
        drop(working);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reader is started once, by `open`, and never by a disabled handle (which would read
    /// cell paths relative to the working directory) or a second time (a reader is never respawned).
    #[test]
    fn start_reader_never_starts_a_second_reader_or_one_without_a_store() {
        let mut disabled = Cache::disabled();
        disabled.start_reader();
        assert!(disabled.reader.is_none() && disabled.reader_thread.is_none());
        let dir = std::env::temp_dir().join(format!("pokerai-cache-reader-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cache = Cache::open(dir.clone(), CACHE_QUOTA_BYTES);
        let first = cache.reader_thread.as_ref().map(|h| h.thread().id());
        cache.start_reader();
        assert_eq!(cache.reader_thread.as_ref().map(|h| h.thread().id()), first, "the running reader is kept");
        assert_eq!(cache.reader_thread.as_ref().and_then(|h| h.thread().name().map(str::to_owned)), Some("cache-reader".to_owned()));
        drop(cache);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dropping the handle closes the reader's queue, and the reader ends (`Drop` joins it; here
    /// the handle is taken first so the test can watch the thread end on its own).
    #[test]
    fn the_reader_ends_once_its_handle_is_dropped() {
        let dir = std::env::temp_dir().join(format!("pokerai-cache-reader-join-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cache = Cache::open(dir.clone(), CACHE_QUOTA_BYTES);
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = cache.reader_thread.take().expect("open starts the reader");
        // Hand the join to a watcher so the test observes the end of the thread itself.
        let watcher = std::thread::spawn(move || {
            let _ = reader.join();
            let _ = tx.send(());
        });
        drop(cache);
        rx.recv_timeout(std::time::Duration::from_secs(30)).expect("the reader ends once its handle is dropped");
        watcher.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
