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

// Test builds only: the in-crate unit tests below share the integration tests' fixtures verbatim.
// `tests/support/mod.rs` names this crate `cache::`, as the integration-test crates see it, so the
// crate is given that name for itself here (plan 4 task 7 fix round 1).
#[cfg(test)]
extern crate self as cache;
#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;
#[cfg(test)]
#[path = "../tests/support/temp_dir.rs"]
mod test_dir;

/// Test-only seams for the lookup and reader lifecycle (plan 4 task 7 fix round 1): a one-shot
/// hook registry and a trace of the points reached, both keyed by the cache root, so tests running
/// in parallel never trip one another's. Absent from every normal build, and unreachable from
/// `crates/cache/tests/` (a separate crate built against the non-test library), following
/// `storage::inject_before_rename`'s precedent.
#[cfg(test)]
pub(crate) mod seams {
    use std::path::{Path, PathBuf};
    use std::sync::{Condvar, Mutex, MutexGuard};

    /// A point a hook can be armed at, and the trace records.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Point {
        /// The reader is about to ask its gate before this stage.
        Gate(crate::lookup::Stage),
        /// The reader abandoned a request at a gate, for this reason.
        Abandoned(crate::lookup::MissReason),
        /// The waiter is about to receive its reply.
        Receive,
        /// The reader sent a prepared hit.
        ReplyHit,
        /// The reader thread is ending.
        Exit,
    }

    type Hook = Box<dyn FnOnce() + Send>;
    static HOOKS: Mutex<Vec<(PathBuf, Point, Hook)>> = Mutex::new(Vec::new());
    static TRACE: Mutex<Vec<(PathBuf, Point)>> = Mutex::new(Vec::new());
    static TRACED: Condvar = Condvar::new();

    fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Arms a one-shot `hook` for the next time `point` is reached under `dir`.
    pub(crate) fn arm(dir: &Path, point: Point, hook: impl FnOnce() + Send + 'static) {
        guard(&HOOKS).push((dir.to_path_buf(), point, Box::new(hook)));
    }

    /// Records `point` under `dir`, then runs (and disarms) a hook armed there, with no lock held.
    pub(crate) fn fire(dir: &Path, point: Point) {
        guard(&TRACE).push((dir.to_path_buf(), point));
        TRACED.notify_all();
        let hook = {
            let mut hooks = guard(&HOOKS);
            hooks.iter().position(|(d, p, _)| d == dir && *p == point).map(|i| hooks.remove(i).2)
        };
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Every point reached under `dir` so far, in order.
    pub(crate) fn trace(dir: &Path) -> Vec<Point> {
        guard(&TRACE).iter().filter(|(d, _)| d == dir).map(|(_, p)| *p).collect()
    }

    /// Waits up to `timeout` for `point` to be reached under `dir`.
    pub(crate) fn wait_for(dir: &Path, point: Point, timeout: std::time::Duration) -> bool {
        let reached = |t: &mut Vec<(PathBuf, Point)>| t.iter().any(|(d, p)| d == dir && *p == point);
        let (mut trace, _) = TRACED.wait_timeout_while(guard(&TRACE), timeout, |t| !reached(t)).unwrap_or_else(|poisoned| poisoned.into_inner());
        reached(&mut trace)
    }
}

pub mod entry;
pub mod key;
pub mod label;
pub mod lookup;
pub mod presolver;
pub mod quota;
pub mod storage;

/// How long `Cache::shutdown` and the handle's `Drop` wait for the `cache-reader` thread to stop
/// before they give up on it (plan 4 task 7 fix round 1, review P4T7-I5).
pub const READER_STOP_BOUND: std::time::Duration = std::time::Duration::from_secs(2);

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
/// Thread ownership (plan 4 task 7, fix round 1 review P4T7-I5): the handle owns the
/// `cache-reader` thread. `shutdown` and `Drop` stop it through an out-of-band stop flag (the
/// request in hand is abandoned at its next check, the queued ones are discarded unanswered) and
/// join it within `READER_STOP_BOUND`. The one residual: std cannot cancel a blocking OS file read,
/// so a reader still inside one when the bound expires is detached with a logged diagnostic and
/// ends as soon as that read returns (see `stop_reader`). The writer ends on its own once this
/// handle, its only sender, is gone (task 6); `shutdown` stops it early.
pub struct Cache {
    root: std::path::PathBuf,
    writer: Option<std::sync::mpsc::SyncSender<WriteCommand>>,
    reader: Option<std::sync::mpsc::SyncSender<crate::lookup::ReadCommand>>,
    /// Taken (and joined, or detached past its bound) by the first `shutdown` or by `Drop`.
    reader_thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Shared with the reader: its out-of-band stop flag and its completion signal.
    reader_state: std::sync::Arc<ReaderState>,
    /// The token of the next `ReadCommand::Serve`; starts at 1 and only ever increases.
    next_token: std::sync::atomic::AtomicU64,
    /// Why an `open` yielded a handle that cannot serve (`availability_warning`).
    warning: Option<String>,
    /// The byte budget the writer enforces (`open`'s `quota_bytes`; 0 for a disabled handle).
    quota_bytes: u64,
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
            reader_thread: std::sync::Mutex::new(None),
            reader_state: std::sync::Arc::new(ReaderState::default()),
            next_token: std::sync::atomic::AtomicU64::new(1),
            warning: None,
            quota_bytes: 0,
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
        cache.quota_bytes = quota_bytes;
        cache.start_reader();
        cache
    }

    /// Section 12 (fix round 1, review P4T7-I1): this handle's state as one line for
    /// `engine::StartupReport::cache_state`. It is `open: <root> (quota <n> bytes)` for a working
    /// cache and `disabled: <why>` for one `open` could not open. It is `degraded: <why>` when the
    /// writer runs but the reader did not start, and `disabled: switched off` for a deliberately
    /// `disabled` handle.
    pub fn summary(&self) -> String {
        match (&self.warning, self.writer.is_some()) {
            (None, true) => format!("open: {} (quota {} bytes)", self.root.display(), self.quota_bytes),
            (Some(why), true) => format!("degraded: {why}"),
            (Some(why), false) => format!("disabled: {why}"),
            (None, false) => "disabled: switched off".into(),
        }
    }

    /// Starts the one `cache-reader` thread that owns every cell read and all of a lookup's
    /// preparation (installed by `open` once the writer runs). Each `ReadCommand::Serve` is served
    /// by `serve_lookup` and answered without blocking; a reply whose waiter has gone is dropped.
    /// Does nothing on a handle without a writer (a disabled cache never reads) or one whose reader
    /// already runs: a reader is never respawned. A thread that does not start leaves every lookup
    /// a miss, with a warning.
    pub fn start_reader(&mut self) {
        if self.writer.is_none() || self.reader.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<crate::lookup::ReadCommand>(4);
        let dir = self.root().to_path_buf();
        let state = std::sync::Arc::clone(&self.reader_state);
        let spawned = std::thread::Builder::new().name("cache-reader".into()).spawn(move || {
            let _ends = EndsReader(std::sync::Arc::clone(&state));
            while let Ok(command) = rx.recv() {
                // Stopped: this command and every one still queued are discarded unanswered.
                if state.stop.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                match command {
                    crate::lookup::ReadCommand::Shutdown => break,
                    crate::lookup::ReadCommand::Serve { token, keys, deadline, query, reply } => {
                        let prepared = serve_lookup(&dir, &state.stop, &keys, deadline, &query);
                        #[cfg(test)]
                        let hit = prepared.touch.is_some();
                        let _ = reply.try_send((token, prepared));
                        #[cfg(test)]
                        if hit {
                            seams::fire(&dir, seams::Point::ReplyHit);
                        }
                    }
                }
            }
            #[cfg(test)]
            seams::fire(&dir, seams::Point::Exit);
        });
        match spawned {
            Ok(handle) => {
                self.reader = Some(tx);
                *self.reader_thread.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(handle);
            }
            Err(error) => {
                self.warning = Some(format!(
                    "the flop cache at {} cannot serve lookups (its reader thread did not start: {error}); every cache lookup misses this session",
                    self.root.display()
                ));
            }
        }
    }

    /// Spec 10.4 lookup, bounded by one absolute deadline (fix round 1, review P4T7-I3):
    /// `min(q.budget, lookup::LOOKUP_BOUND)` from entry covers the whole operation.
    ///
    /// A zero budget is a `BudgetExhausted` miss decided first, before a token is taken, a key
    /// hashed or anything posted (review P4T7-I2). Otherwise one `ReadCommand::Serve` is posted to
    /// the reader without blocking (`QueueFull` / `ReaderUnavailable` when that fails). The reader
    /// does every expensive stage -- reads, `select`, `reconstruct` (with `validate_solution` and
    /// the menu-legality check), the label and the touch digest -- checking the deadline and its
    /// stop flag before each and abandoning late work (`serve_lookup`). The waiter only receives,
    /// for the time remaining until the deadline. A reply that is late, or whose token is not the
    /// request's, is discarded: no hit is returned and nothing is touched. An accepted hit is
    /// touched (a nonblocking writer command) so eviction sees it as fresh.
    ///
    /// Every miss carries its `MissReason`. An ordinary query mismatch never deletes anything;
    /// only `read_cell` deletes, and only a file that fails `decode`/`validate_entry`. The caller
    /// runs this on the request's own `fast-path` work, never on `watchdog` or `engine-main`.
    pub fn lookup(&self, q: &crate::lookup::CacheQuery) -> crate::lookup::Lookup {
        use crate::lookup::{Lookup, MissReason, ReadCommand};
        let miss = |reason| Lookup::Miss { reason };
        if q.budget.is_zero() {
            return miss(MissReason::BudgetExhausted);
        }
        let deadline = std::time::Instant::now() + q.budget.min(crate::lookup::LOOKUP_BOUND);
        let Some(reader) = self.reader.as_ref() else { return miss(MissReason::ReaderUnavailable) };
        if self.reader_state.stop.load(std::sync::atomic::Ordering::SeqCst) {
            return miss(MissReason::ReaderUnavailable);
        }
        let token = self.next_token.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let b = q.key.spr_bucket;
        let keys = [q.key.at_bucket(b.saturating_sub(1)).digest(), q.key.digest(), q.key.at_bucket(b.saturating_add(1)).digest()];
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        match reader.try_send(ReadCommand::Serve { token, keys, deadline, query: Box::new(q.clone()), reply: tx }) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => return miss(MissReason::QueueFull),
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return miss(MissReason::ReaderUnavailable),
        }
        #[cfg(test)]
        seams::fire(&self.root, seams::Point::Receive);
        let (replied, prepared) = match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(reply) => reply,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return miss(MissReason::BudgetExhausted),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return miss(MissReason::ReaderUnavailable),
        };
        if std::time::Instant::now() >= deadline {
            return miss(MissReason::BudgetExhausted);
        }
        if replied != token {
            return miss(MissReason::ReaderUnavailable);
        }
        if let Some((key, payload_digest)) = prepared.touch {
            let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
            self.touch(key, payload_digest, now_ms);
        }
        prepared.outcome
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

    /// Asks both service threads to finish. The writer's `Shutdown` is a blocking send (task 6):
    /// it goes through even if the queue is momentarily full, so it cannot be lost. The reader is
    /// stopped out of band and waited for at most `READER_STOP_BOUND` (`stop_reader`). Every lookup
    /// after this is a `ReaderUnavailable` miss.
    pub fn shutdown(&self) {
        if let Some(tx) = self.writer.as_ref() {
            let _ = tx.send(WriteCommand::Shutdown);
        }
        self.stop_reader();
    }

    /// Stops `cache-reader` with a bound (fix round 1, review P4T7-I5). Raises the out-of-band stop
    /// flag, which the reader checks between commands, between its cell reads and before every
    /// preparation stage, so it abandons the request in hand and discards every queued one
    /// unanswered (their waiters see `ReaderUnavailable`); wakes an idle reader with a nonblocking
    /// `Shutdown`; then waits up to `READER_STOP_BOUND` for the reader's completion signal and joins
    /// it. The first caller takes the handle; later calls return at once.
    ///
    /// Residual (ruling on P4T7-I5): std cannot cancel a blocking OS file read. A reader still
    /// inside one when the bound expires is detached, with a logged diagnostic, rather than stall
    /// the engine's shutdown. It ends by itself as soon as that read returns (the stop flag is
    /// seen at its next check) and serves nothing more. Only then is the thread not joined.
    fn stop_reader(&self) {
        self.reader_state.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(tx) = self.reader.as_ref() {
            let _ = tx.try_send(crate::lookup::ReadCommand::Shutdown);
        }
        let handle = self.reader_thread.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        let Some(handle) = handle else { return };
        if self.reader_state.wait_ended(READER_STOP_BOUND) {
            let _ = handle.join();
        } else {
            eprintln!(
                "cache-reader did not stop within {READER_STOP_BOUND:?} (a file read in progress cannot be cancelled); detached: it ends once that read returns, and serves nothing more"
            );
        }
    }
}

/// What the `Cache` handle shares with its `cache-reader` thread: the out-of-band stop flag the
/// reader checks between commands, between its cell reads and before every preparation stage,
/// and the completion signal the thread raises as it ends (`EndsReader`), which lets a stop wait
/// for it with a bound (`Cache::stop_reader`).
#[derive(Default)]
struct ReaderState {
    stop: std::sync::atomic::AtomicBool,
    ended: std::sync::Mutex<bool>,
    ended_changed: std::sync::Condvar,
}

impl ReaderState {
    fn ended(&self) -> std::sync::MutexGuard<'_, bool> {
        self.ended.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Waits up to `bound` for the reader to end; true once it has.
    fn wait_ended(&self, bound: std::time::Duration) -> bool {
        let (ended, _) = self.ended_changed.wait_timeout_while(self.ended(), bound, |ended| !*ended).unwrap_or_else(|poisoned| poisoned.into_inner());
        *ended
    }
}

/// Held by the reader thread for its whole life: raises `ReaderState::ended` when the thread ends,
/// by returning or by unwinding.
struct EndsReader(std::sync::Arc<ReaderState>);

impl Drop for EndsReader {
    fn drop(&mut self) {
        *self.0.ended() = true;
        self.0.ended_changed.notify_all();
    }
}

/// The reader's side of one lookup: reads the distinct cells among `keys`, then
/// `lookup::prepare`s the outcome, asking one gate before every stage (`lookup::Stage`): a set
/// stop flag abandons the work as `ReaderUnavailable`, a passed `deadline` as `BudgetExhausted`.
/// Each read is bounded by `storage::read_cell` (one cell, at most two entries), so between gates
/// the reader does a bounded amount of work; a gate that refuses ends the request there, and no
/// later stage runs. A hit passes one last gate (`Stage::Reply`) before it is handed back.
fn serve_lookup(dir: &std::path::Path, stop: &std::sync::atomic::AtomicBool, keys: &[[u8; 32]; 3], deadline: std::time::Instant, q: &crate::lookup::CacheQuery) -> crate::lookup::Prepared {
    use crate::lookup::{MissReason, Prepared, Stage};
    let mut gate = |stage: Stage| -> Result<(), MissReason> {
        #[cfg(test)]
        seams::fire(dir, seams::Point::Gate(stage));
        #[cfg(not(test))]
        let _ = stage; // the stage names the test seam only; every gate asks the same questions
        let refused = if stop.load(std::sync::atomic::Ordering::SeqCst) {
            Some(MissReason::ReaderUnavailable)
        } else if std::time::Instant::now() >= deadline {
            Some(MissReason::BudgetExhausted)
        } else {
            None
        };
        match refused {
            None => Ok(()),
            Some(reason) => {
                #[cfg(test)]
                seams::fire(dir, seams::Point::Abandoned(reason));
                Err(reason)
            }
        }
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut cells = Vec::new();
    for key in keys {
        if !seen.insert(*key) {
            continue;
        }
        if let Err(reason) = gate(Stage::Read) {
            return Prepared::miss(reason);
        }
        if let Some(cell) = crate::storage::read_cell(&crate::storage::entry_path(dir, *key)) {
            cells.push(cell);
        }
    }
    let prepared = crate::lookup::prepare(cells, q, &mut gate);
    if prepared.touch.is_some() {
        if let Err(reason) = gate(Stage::Reply) {
            return Prepared::miss(reason);
        }
    }
    prepared
}

impl Drop for Cache {
    /// Stops `cache-reader` with the same bound as `shutdown` (`stop_reader`): the stop flag is
    /// raised and the reader's queue closed (this handle holds its only sender), so a busy reader
    /// abandons its request and discards the queued ones, and an idle one wakes and ends; then it is
    /// joined, or -- past `READER_STOP_BOUND`, inside an uncancellable file read -- detached with a
    /// logged diagnostic. The writer is not joined here: it ends once its own queue closes with
    /// this handle (task 6).
    fn drop(&mut self) {
        self.reader_state.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.reader = None;
        self.stop_reader();
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

    /// The fixture's root-node query: `test_support::entry()`'s own key, source, tree and suits,
    /// for `oop` at the root, whose legal menu is exactly the tree's `Check` / `AllIn{500}`.
    fn root_query(e: &crate::entry::CacheEntry) -> crate::lookup::CacheQuery {
        crate::lookup::CacheQuery {
            key: e.key.clone(),
            source: e.source.clone(),
            tree: e.tree.clone(),
            requested: vec![],
            actor: "oop".into(),
            legal: vec![proto::LegalAction::Check, proto::LegalAction::AllIn { to: 500 }],
            target_bp: 50,
            reasons: vec![],
            inverse_perm: core_iso::SuitPerm::IDENTITY,
            budget: std::time::Duration::from_secs(5),
        }
    }

    /// Fix round 1 (review P4T7-I2, ruling 7-D7): a zero budget is a `BudgetExhausted` miss decided
    /// before anything else -- no token is taken, no key hashed and nothing posted -- so a spent
    /// request never occupies the one reader. Observed on a private channel standing in for it.
    #[test]
    fn a_zero_budget_posts_nothing_and_consumes_no_token() {
        use crate::lookup::{Lookup, MissReason};
        let (tx, rx) = std::sync::mpsc::sync_channel::<crate::lookup::ReadCommand>(4);
        let mut cache = Cache::disabled();
        cache.reader = Some(tx);
        let mut q = root_query(&test_support::entry());
        q.budget = std::time::Duration::ZERO;
        assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::BudgetExhausted });
        assert!(matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)), "nothing may be posted for a spent budget");
        assert_eq!(cache.next_token.load(std::sync::atomic::Ordering::Relaxed), 1, "no token may be consumed");
    }

    /// The request after a spent one is posted normally, with the first token. Nobody answers on
    /// this private channel, so it times out: a deterministic `BudgetExhausted` (review P4T7-I6).
    #[test]
    fn the_next_request_is_posted_and_an_unanswered_one_times_out_as_budget_exhausted() {
        use crate::lookup::{Lookup, MissReason};
        let (tx, rx) = std::sync::mpsc::sync_channel::<crate::lookup::ReadCommand>(4);
        let mut cache = Cache::disabled();
        cache.reader = Some(tx);
        let mut q = root_query(&test_support::entry());
        q.budget = std::time::Duration::ZERO;
        assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::BudgetExhausted });
        q.budget = std::time::Duration::from_millis(30);
        let started = std::time::Instant::now();
        assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::BudgetExhausted }, "an unanswered request times out");
        let returned = std::time::Instant::now();
        assert!(returned - started >= std::time::Duration::from_millis(30), "it waited for its budget");
        let posted = rx.try_recv().expect("the second request was posted");
        let crate::lookup::ReadCommand::Serve { token, deadline, .. } = posted else { panic!("a serve command") };
        assert_eq!(token, 1, "the first token goes to the first request actually posted");
        assert!(started + std::time::Duration::from_millis(30) <= deadline && deadline <= returned, "one absolute deadline, fixed at entry, that the wait honoured");
        assert!(rx.try_recv().is_err(), "exactly one command was posted");
    }

    /// A failure budget for the writer thread, never a synchronization delay.
    const WRITER_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);

    /// Opens a real cache at `dir` holding `test_support::entry()`, confirmed durable.
    fn opened_with_fixture(dir: &test_dir::TempDir) -> (Cache, crate::entry::CacheEntry) {
        let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
        let e = test_support::entry();
        assert!(cache.store_tracked(&e).wait(WRITER_BUDGET), "the writer must store the fixture");
        (cache, e)
    }

    /// The `last_hit` of `e` as it is on disk now.
    fn last_hit_on_disk(dir: &test_dir::TempDir, e: &crate::entry::CacheEntry) -> u64 {
        crate::storage::read_cell(&crate::storage::entry_path(dir.path(), e.key.digest())).expect("the cell is on disk").entries[0].last_hit
    }

    /// A writer barrier: a refused store is answered only after every command queued before it,
    /// so any touch a lookup sent has been applied (or refused) once this returns.
    fn writer_barrier(cache: &Cache) {
        let mut refused = test_support::entry();
        refused.key.schema_version = 2;
        assert!(!cache.store_tracked(&refused).wait(WRITER_BUDGET), "the barrier store must be refused");
    }

    /// Fix round 1 (review P4T7-I3): one absolute deadline, `min(budget, 500 ms)` from entry,
    /// covers the whole lookup. A preparation stage that stalls past it (a deterministic sleeping
    /// hook at the stage's gate) does not hold the waiter: it gets `BudgetExhausted` at its
    /// deadline. The reader abandons the late work at its next gate (the following stage is never
    /// reached, no hit is sent) and nothing is touched.
    #[test]
    fn a_stage_that_stalls_past_the_deadline_is_a_budget_miss_the_reader_abandons_untouched() {
        use crate::lookup::{Lookup, MissReason, Stage};
        use seams::Point;
        for (name, stalled, next) in [("select", Stage::Select, Stage::Reconstruct), ("reconstruct", Stage::Reconstruct, Stage::Label), ("label", Stage::Label, Stage::Reply)] {
            let dir = test_dir::TempDir::new(&format!("stall-{name}"));
            let (cache, e) = opened_with_fixture(&dir);
            let before = last_hit_on_disk(&dir, &e);
            seams::arm(dir.path(), Point::Gate(stalled), || std::thread::sleep(std::time::Duration::from_millis(300)));
            let mut q = root_query(&e);
            q.budget = std::time::Duration::from_millis(100);
            let started = std::time::Instant::now();
            assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::BudgetExhausted }, "{name}: a stalled stage is a budget miss");
            assert!(started.elapsed() < std::time::Duration::from_millis(280), "{name}: the waiter never waits out the stall ({:?})", started.elapsed());
            assert!(
                seams::wait_for(dir.path(), Point::Abandoned(MissReason::BudgetExhausted), std::time::Duration::from_secs(10)),
                "{name}: the reader abandons the late work at its next gate"
            );
            writer_barrier(&cache);
            let trace = seams::trace(dir.path());
            assert!(!trace.contains(&Point::Gate(next)) && !trace.contains(&Point::ReplyHit), "{name}: no stage after the stall ran: {trace:?}");
            assert_eq!(last_hit_on_disk(&dir, &e), before, "{name}: nothing was touched");
            drop(cache);
        }
    }

    /// Fix round 1 (review P4T7-I3): time spent before the wait counts against the one deadline --
    /// the waiter receives only for the time remaining, never a fresh budget. The waiter is held
    /// 300 ms (a hook at `Receive`) of a 400 ms budget while the reader stalls in selection, so it
    /// must give up about 100 ms later, not 400 ms later.
    #[test]
    fn the_wait_is_for_the_time_remaining_until_the_deadline() {
        use crate::lookup::{Lookup, MissReason, Stage};
        use seams::Point;
        let dir = test_dir::TempDir::new("remaining-time");
        let (cache, e) = opened_with_fixture(&dir);
        seams::arm(dir.path(), Point::Gate(Stage::Select), || std::thread::sleep(std::time::Duration::from_millis(1500)));
        seams::arm(dir.path(), Point::Receive, || std::thread::sleep(std::time::Duration::from_millis(300)));
        let mut q = root_query(&e);
        q.budget = std::time::Duration::from_millis(400);
        let started = std::time::Instant::now();
        assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::BudgetExhausted });
        let took = started.elapsed();
        assert!(took >= std::time::Duration::from_millis(400), "the whole budget was available ({took:?})");
        assert!(took < std::time::Duration::from_millis(600), "the wait was the time remaining, not a fresh budget ({took:?})");
        drop(cache);
    }

    /// Fix round 1 (review P4T7-I3): a prepared hit that reaches the waiter after its deadline is
    /// discarded -- `BudgetExhausted`, no hit, no touch. The waiter is held (a hook at `Receive`)
    /// until the reader has sent the hit, then past the deadline.
    #[test]
    fn a_prepared_hit_that_arrives_after_the_deadline_is_discarded_and_never_touched() {
        use crate::lookup::{Lookup, MissReason};
        use seams::Point;
        let dir = test_dir::TempDir::new("late-reply");
        let (cache, e) = opened_with_fixture(&dir);
        let before = last_hit_on_disk(&dir, &e);
        let watched = dir.path().to_path_buf();
        seams::arm(dir.path(), Point::Receive, move || {
            assert!(seams::wait_for(&watched, Point::ReplyHit, std::time::Duration::from_secs(10)), "the reader must send the prepared hit in time");
            std::thread::sleep(std::time::Duration::from_millis(300));
        });
        let mut q = root_query(&e);
        q.budget = std::time::Duration::from_millis(250);
        assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::BudgetExhausted }, "a late hit is discarded");
        writer_barrier(&cache);
        assert_eq!(last_hit_on_disk(&dir, &e), before, "a discarded hit is never touched");
        q.budget = std::time::Duration::from_secs(5);
        assert!(matches!(cache.lookup(&q), Lookup::Exact { .. }), "the next request is served");
        drop(cache);
    }

    /// Arms a hook that holds the reader inside its next cell read -- standing in for a blocking OS
    /// file read, which nothing can cancel -- until the returned sender is used or dropped.
    fn stall_the_next_read(dir: &test_dir::TempDir) -> std::sync::mpsc::Sender<()> {
        let (release, released) = std::sync::mpsc::channel::<()>();
        seams::arm(dir.path(), seams::Point::Gate(crate::lookup::Stage::Read), move || {
            let _ = released.recv();
        });
        release
    }

    /// Stalls the reader inside its first read and queues two more lookups behind it; each of the
    /// three waiters gets its `BudgetExhausted` at its own deadline.
    fn stall_the_reader_with_queued_work(dir: &test_dir::TempDir, cache: &Cache, e: &crate::entry::CacheEntry) -> std::sync::mpsc::Sender<()> {
        use crate::lookup::{Lookup, MissReason};
        let release = stall_the_next_read(dir);
        let mut q = root_query(e);
        q.budget = std::time::Duration::from_millis(50);
        assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::BudgetExhausted });
        assert!(seams::wait_for(dir.path(), seams::Point::Gate(crate::lookup::Stage::Read), std::time::Duration::from_secs(10)), "the reader is inside the stalled read");
        for _ in 0..2 {
            assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::BudgetExhausted }, "queued behind the stalled read");
        }
        release
    }

    fn reads(dir: &test_dir::TempDir) -> usize {
        seams::trace(dir.path()).iter().filter(|p| **p == seams::Point::Gate(crate::lookup::Stage::Read)).count()
    }

    /// Fix round 1 (review P4T7-I5), on the actual `Drop` path: with the reader held inside a read
    /// that outlasts `READER_STOP_BOUND`, dropping the handle returns once that bound has passed --
    /// it neither hangs nor returns before waiting it out -- and the reader, detached with a logged
    /// diagnostic, ends as soon as its read returns, discarding the queued requests unread. The
    /// read is released by a helper thread well after the bound, so a `Drop` that waits for it
    /// would show up as a long drop rather than a hung test.
    #[test]
    fn dropping_the_cache_during_a_read_stalled_past_the_bound_returns_at_the_bound_and_discards_queued_work() {
        let dir = test_dir::TempDir::new("stalled-read-detached");
        let (cache, e) = opened_with_fixture(&dir);
        let release = stall_the_reader_with_queued_work(&dir, &cache, &e);
        let late_release = std::thread::spawn(move || {
            std::thread::sleep(READER_STOP_BOUND + std::time::Duration::from_millis(1500));
            let _ = release.send(());
        });
        let started = std::time::Instant::now();
        drop(cache);
        let took = started.elapsed();
        assert!(took >= READER_STOP_BOUND, "Drop waits its whole bound for a busy reader ({took:?})");
        assert!(took < READER_STOP_BOUND + std::time::Duration::from_millis(1000), "Drop returns at its bound, never after the stalled read ({took:?})");
        late_release.join().unwrap();
        assert!(seams::wait_for(dir.path(), seams::Point::Exit, std::time::Duration::from_secs(10)), "the detached reader ends once its read returns");
        assert_eq!(reads(&dir), 1, "the queued requests were discarded, never read");
        assert!(seams::trace(dir.path()).contains(&seams::Point::Abandoned(crate::lookup::MissReason::ReaderUnavailable)), "the stalled request was abandoned at its next gate");
    }

    /// A stalled read that returns inside the bound: `Drop` stops and joins the reader (it has
    /// ended when the drop returns) and the queued requests were discarded, never read.
    #[test]
    fn dropping_the_cache_joins_a_reader_whose_stalled_read_returns_within_the_bound() {
        let dir = test_dir::TempDir::new("stalled-read-joined");
        let (cache, e) = opened_with_fixture(&dir);
        let release = stall_the_reader_with_queued_work(&dir, &cache, &e);
        let early_release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let _ = release.send(());
        });
        let started = std::time::Instant::now();
        drop(cache);
        let took = started.elapsed();
        early_release.join().unwrap();
        assert!(took < READER_STOP_BOUND, "joined inside the bound ({took:?})");
        assert!(seams::trace(dir.path()).contains(&seams::Point::Exit), "Drop returned only once the reader had ended");
        assert_eq!(reads(&dir), 1, "the queued requests were discarded, never read");
    }

    /// `shutdown` is bounded the same way (it is `&self`, so the handle stays usable): with the
    /// reader stalled past the bound it returns at the bound, and every later lookup is a
    /// `ReaderUnavailable` miss without posting anything.
    #[test]
    fn shutdown_during_a_stalled_read_returns_at_the_bound_and_later_lookups_miss() {
        use crate::lookup::{Lookup, MissReason};
        let dir = test_dir::TempDir::new("stalled-read-shutdown");
        let (cache, e) = opened_with_fixture(&dir);
        let release = stall_the_reader_with_queued_work(&dir, &cache, &e);
        let late_release = std::thread::spawn(move || {
            std::thread::sleep(READER_STOP_BOUND + std::time::Duration::from_millis(1500));
            let _ = release.send(());
        });
        let started = std::time::Instant::now();
        cache.shutdown();
        let took = started.elapsed();
        assert!(took >= READER_STOP_BOUND && took < READER_STOP_BOUND + std::time::Duration::from_millis(1000), "shutdown returns at its bound ({took:?})");
        let tokens = cache.next_token.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(cache.lookup(&root_query(&e)), Lookup::Miss { reason: MissReason::ReaderUnavailable });
        assert_eq!(cache.next_token.load(std::sync::atomic::Ordering::Relaxed), tokens, "a stopped reader is asked nothing");
        late_release.join().unwrap();
        assert!(seams::wait_for(dir.path(), seams::Point::Exit, std::time::Duration::from_secs(10)));
        assert_eq!(reads(&dir), 1, "the queued requests were discarded, never read");
        let started = std::time::Instant::now();
        drop(cache);
        assert!(started.elapsed() < std::time::Duration::from_millis(500), "nothing is left to wait for");
    }

    /// Review P4T7-I6: a full queue and a gone reader are reported as such, without blocking.
    #[test]
    fn a_full_queue_and_a_gone_reader_have_their_own_miss_reasons() {
        use crate::lookup::{Lookup, MissReason};
        let q = root_query(&test_support::entry());
        let (tx, rx) = std::sync::mpsc::sync_channel::<crate::lookup::ReadCommand>(0);
        let mut cache = Cache::disabled();
        cache.reader = Some(tx);
        assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::QueueFull }, "no room and nobody receiving");
        drop(rx);
        assert_eq!(cache.lookup(&q), Lookup::Miss { reason: MissReason::ReaderUnavailable });
        assert_eq!(Cache::disabled().lookup(&q), Lookup::Miss { reason: MissReason::ReaderUnavailable });
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
        assert!(working.reader.is_some() && working.reader_thread.lock().unwrap().is_some(), "open starts the reader");
        assert_eq!(Cache::disabled().availability_warning(), None);
        // Fix round 1 (review P4T7-I1): the one-line state `StartupReport.cache_state` carries.
        assert_eq!(working.summary(), format!("open: {} (quota {CACHE_QUOTA_BYTES} bytes)", dir.join("cache").display()));
        assert_eq!(failed.summary(), format!("disabled: {warning}"));
        assert_eq!(Cache::disabled().summary(), "disabled: switched off");
        drop(working);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reader is started once, by `open`, and never by a disabled handle (which would read
    /// cell paths relative to the working directory) or a second time (a reader is never respawned).
    #[test]
    fn start_reader_never_starts_a_second_reader_or_one_without_a_store() {
        let mut disabled = Cache::disabled();
        disabled.start_reader();
        assert!(disabled.reader.is_none() && disabled.reader_thread.lock().unwrap().is_none());
        let dir = std::env::temp_dir().join(format!("pokerai-cache-reader-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cache = Cache::open(dir.clone(), CACHE_QUOTA_BYTES);
        let reader = |c: &Cache| c.reader_thread.lock().unwrap().as_ref().map(|h| (h.thread().id(), h.thread().name().map(str::to_owned)));
        let first = reader(&cache);
        cache.start_reader();
        assert_eq!(reader(&cache), first, "the running reader is kept");
        assert_eq!(first.and_then(|(_, name)| name), Some("cache-reader".to_owned()));
        drop(cache);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The ordinary teardown, on the actual `Drop` path: an idle reader wakes on its closed queue,
    /// ends, and is joined -- it has ended by the time the drop returns, well inside the bound.
    #[test]
    fn dropping_an_idle_cache_joins_its_reader_promptly() {
        let dir = test_dir::TempDir::new("idle-drop");
        let (cache, e) = opened_with_fixture(&dir);
        assert!(matches!(cache.lookup(&root_query(&e)), crate::lookup::Lookup::Exact { .. }), "the reader serves, then goes idle");
        let started = std::time::Instant::now();
        drop(cache);
        assert!(started.elapsed() < READER_STOP_BOUND, "an idle reader is joined, never waited out ({:?})", started.elapsed());
        assert!(seams::trace(dir.path()).contains(&seams::Point::Exit), "the reader had ended when the drop returned");
    }
}
