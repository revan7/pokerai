//! Spec section 10.5: the background pre-solver's scheduler -- the `presolver` thread that runs the
//! task 14 queue's jobs while the table is idle and hands the worker back the moment live work
//! arrives.
//!
//! ## Parts
//!
//! - `PresolveExecutor` is the downward callback the engine supplies (task 16): it prepares a job
//!   by chart replay, submits it to the single worker owner, polls and cancels it, stores and
//!   verifies a completed entry, and answers whether a game's entry is already on disk at target.
//!   The cache crate never imports the engine, replay or preflop crates.
//! - `Scheduler` is the thread's body as a synchronous state machine: `apply` takes one
//!   `PresolverCommand`, `step` runs one iteration. Tests drive it directly with a fake executor
//!   and a fake clock.
//! - `Presolver` is the handle the engine owns: it starts the thread and posts notifications to its
//!   mailbox. None of its methods waits for the thread, the executor or any I/O.
//!
//! ## When jobs run
//!
//! A job is launched only when `eligible`: no hand is in progress, the queue is not paused, and no
//! live request, hand event or resume has happened for `IDLE_MS` (30 s) on the queue clock. At most
//! one job runs at a time. A pause lets a running job finish and launches nothing more; a resume
//! keeps the cursor and waits for the idle gate again. When the queue offers nothing to launch (all
//! done, terminal, waiting out a backoff or cooling down), it is scanned again only once it changes
//! or after `RESCAN_MS`: with every slot bound, a scan that finds nothing reads all 42,120 slots.
//!
//! ## The thread and its mailbox (fix round 1, R2)
//!
//! The handle never queues one message per call. Its notifications land in a mailbox of at most
//! `MAILBOX_CAPACITY` entries: the latest hand state and the latest pause/resume state (a newer one
//! replaces an older one the thread has not taken yet), a source-change signal, an activity signal
//! (a live request) and the shutdown latch. So a flood of calls costs no memory beyond those
//! entries, and the state the thread applies is always the latest; nothing is refused because the
//! mailbox is full. Live pre-emption does not go through the mailbox at all (below).
//!
//! The `presolver` thread waits up to `TICK_MS` for a notification, takes everything in the mailbox
//! at once -- a batch of at most `MAILBOX_CAPACITY` commands, applied as hand, pause/resume, source
//! change, live request, then shutdown, so the final states reach the queue before a shutdown saves
//! it -- runs one iteration, and publishes the status. After an iteration that resolved a slot
//! without starting a solve, or while a reconciliation sweep runs with nothing else to do, it does
//! not wait (`wait_hint` is zero) and publishes at most once per `BUSY_PUBLISH_MS`, since a
//! publication after a change recounts every slot. Once shutdown is requested no job is submitted
//! any more, so the thread stops as soon as an executor call it is blocked in returns.
//!
//! ## Live pre-emption
//!
//! Every job gets its own cancel flag (`Arc<AtomicBool>`), handed to the executor with the job at
//! `submit`; the executor's worker owner observes it. `LiveSignal::raise` -- called by
//! `Presolver::notify_live_request` and `notify_hand(true)` on the engine's admitting thread --
//! sets the running job's flag at once, so the worker hears about live work without waiting for
//! this thread, its mailbox or the executor. The scheduler then cancels the job in its next
//! iteration: the queue records the cancel first (refunding the attempt, `Queue::record_cancel`),
//! then the executor is told (`cancel`), and the scheduler moves on without waiting for the job to
//! wind down -- the work is lost, as section 10.5 accepts. A live request that arrives while a job
//! is being prepared stops it before submission: the job's flag is installed before the scheduler
//! re-reads the live signal, so one of the two always sees the other.
//!
//! ## The launch path (rulings S3, S6)
//!
//! `next_pending` names a slot; the executor prepares it; the prepared key and exact SPR `bind` the
//! slot to its normalized game identity; `reconcile_item` decides that game from the disk, so an
//! entry already stored at target completes it without a solve; only if `next_pending` still names
//! the slot is the job submitted, and then `record_launch` charges the attempt. A preparation
//! failure (including a prepared key that is not a flop root on the slot's canonical flop) is a
//! failure of the slot itself; a refused submission is a failure of the bound game. A completed job
//! is `Done` only if `store_and_verify` reads its entry back durably at target. Otherwise (fix
//! round 1, R4) the solve did not fail -- the cache could not keep its result -- so the attempt is
//! refunded and the game is `Pending` again (a retry returns to its `Failed{n}`, still due), the
//! outcome is logged, and the slot cools down in memory for `NOT_DURABLE_COOLDOWN_MS` before
//! `next_pending` may offer it again, so a solve that never verifies cannot hot-loop.
//!
//! ## Clocks (ruling S1)
//!
//! The queue is opened on the executor's clock (`PresolveExecutor::clock`), and the scheduler takes
//! every `now_ms` from that same clock instance (`Queue::now_ms`), never decreasing, so the queue's
//! restored retry waits and the idle gate run on one timeline.
//!
//! ## Generations (rulings S2, S5, S7)
//!
//! `open` declares the executor's source/config fingerprint (`PresolveExecutor::generation`)
//! before any reconciliation. A runtime change arrives as `PresolverCommand::SourceChanged`: when
//! the fingerprint really changed, the running job is cancelled and the cancel recorded, then the
//! new generation is declared, a fresh startup-style sweep begins (fix round 3, N1: a generation
//! that becomes current again -- A, then B, then A -- must be re-verified before its persisted
//! completions count, exactly as `open`'s own generation is), and the change is saved. Stale slots
//! count as pending until the launch path prepares them again, one slot per iteration; an iteration
//! that resolves a slot without starting a solve is followed at once by the next (`wait_hint` is
//! zero), so rebinding costs preparations, not idle ticks.
//!
//! ## Saves (ruling S8, refined by rulings 15-R3 and 15-R3b)
//!
//! `queue.json` is rewritten whole (about 32 MB once every slot is bound), so it is saved only when
//! the queue changed. An outcome the cache cannot reconstruct -- a failure with its attempt count
//! and absolute retry deadline, whether the worker, the preparation or the submission failed -- is
//! on disk before the scheduler goes on, as one small record appended to the queue's failure
//! journal (`Queue::journal_outcome`; the queue module doc, "The failure journal"), never as a
//! snapshot. If that append fails, nothing new is launched until it succeeds, tried again every
//! `SAVE_RETRY_MS`, or until a snapshot holding the outcome is saved; a journal that is full is
//! folded into a snapshot at once instead. A pause, a resume, a generation change and shutdown are
//! saved at once. Everything else -- a launch, a completion proven by a verified entry, a
//! cancellation, reconciliation's verdicts, the cursor -- waits for the next checkpoint, at most
//! `CHECKPOINT_MS` later: opening the queue refunds an in-flight attempt, so a saved launch and an
//! unsaved one reopen identically, and what a crash can lose there is re-derived from the cache by
//! the launch path and reconciliation. Every snapshot folds the journal: it holds every record so
//! far, and the queue empties the journal once it is durable. A failed save is counted
//! (`SaveStats::failures`), reported on stderr once per distinct error, kept dirty, and retried.
//!
//! ## Reconciliation (fix round 1, R1)
//!
//! A sweep over every slot bound under the current generation (`reconcile_item` per game, in
//! `Queue::sweep_order`) starts with the scheduler and again every `RECONCILE_PERIOD_MS`. It runs
//! only while idle-eligible, at most `RECONCILE_CHUNK` games per iteration, so commands are handled
//! between chunks; it skips the running game, and a game the launch path already decided in this
//! sweep. Each iteration reconciles its chunk before it chooses work, and while a sweep is under
//! way no candidate is launched that lies later in sweep order than a `Done` game the sweep has
//! not verified yet: the launch waits until the sweep has passed the last such game before the
//! candidate, so an entry that went missing (evicted, corrupt) is found and solved again before any
//! later work -- tier 1 before tier 2. Until the startup sweep ends, the published `done` and
//! `tier_done` count only completions verified in this process (by the sweep, the launch path or a
//! job's own read-back); a persisted `Done` not yet verified counts as pending.

use super::queue::{self, JournalAppend, Queue, QueueClock, QueueItem, RETRY_BACKOFF_MS};
use crate::entry::CacheEntry;
use crate::key::{KeyFields, Rational};
use crate::CacheError;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

/// Section 10.5: no request (and no hand) for this long, in milliseconds, before a job may start.
pub const IDLE_MS: u64 = 30_000;
/// The presolver thread's wait between iterations while nothing needs it sooner, in milliseconds:
/// how often a running job is polled.
pub const TICK_MS: u64 = 250;
/// The longest a changed queue waits to be saved, in queue-clock milliseconds, unless a pause, a
/// resume, a generation change or shutdown saves it at once (module doc, "Saves").
pub const CHECKPOINT_MS: u64 = 300_000;
/// The most games one reconciliation iteration decides (module doc, "Reconciliation").
pub const RECONCILE_CHUNK: usize = 16;
/// How often a reconciliation sweep starts again, in queue-clock milliseconds: six hours.
pub const RECONCILE_PERIOD_MS: u64 = 6 * 60 * 60 * 1_000;
/// What the scheduler logs when a completed job's entry does not read back durably at target (fix
/// round 1, R4: the slot stays pending, nothing is recorded as a failure).
pub const NOT_DURABLE: &str = "entry not durable at target";
/// How long a slot whose completed entry did not read back durably is held back before
/// `next_pending` may offer it again, in queue-clock milliseconds: the standard retry backoff, kept
/// in memory only (fix round 1, R4).
pub const NOT_DURABLE_COOLDOWN_MS: u64 = RETRY_BACKOFF_MS;
/// After the journal write of an outcome the cache cannot reconstruct failed, how often it is tried
/// again, in queue-clock milliseconds; nothing new is launched meanwhile (module doc, "Saves").
pub const SAVE_RETRY_MS: u64 = 30_000;
/// The most notifications the handle's mailbox holds: the latest hand state, the latest
/// pause/resume state, the source-change and activity signals and the shutdown latch (module doc,
/// "The thread and its mailbox").
pub const MAILBOX_CAPACITY: usize = 5;
/// When the queue offers nothing to launch, how long the scheduler waits before scanning it again
/// unless it changes first, in queue-clock milliseconds: with every slot bound, a scan that finds
/// nothing reads all 42,120 slots (measured about 36 ms in a release build), and only a retry
/// coming due can make work appear without a change.
pub const RESCAN_MS: u64 = 5_000;
/// While iterations follow back to back, the least time between two status publications, in
/// queue-clock milliseconds: a publication after a change recounts every slot (measured about
/// 24 ms in a release build with every slot bound).
pub const BUSY_PUBLISH_MS: u64 = 1_000;

/// What the scheduler logs when a completed job's entry does not read back durably at target (fix
/// round 1, R4; fix round 3, N3): the attempt is refunded and the slot -- not the game -- returns
/// to exactly the record `record_cancel` leaves it in: a fresh `Pending` for a first attempt, its
/// own `Failed{n}` unchanged for a retry. `status` names which, so the log never claims "pending"
/// for a retry that is really `Failed{n}`.
pub fn not_durable_message(label: &str, status: &queue::TaskStatus) -> String {
    format!("presolver: {label} completed, but its {NOT_DURABLE}; its attempt is refunded (now {status:?}) and the slot is held back for {NOT_DURABLE_COOLDOWN_MS} ms")
}

/// A job the executor prepared for one queue slot: the slot as the queue showed it, the background
/// solve input, the rake and blind that input was built with, and -- for `Queue::bind` (ruling S3)
/// -- the prepared cache key and the exact scenario SPR `eff : P`, whose
/// `KeyFields::scenario_identity` is the job's normalized game identity.
#[derive(Clone, Debug)]
pub struct PreparedJob {
    pub item: QueueItem,
    pub input: proto::SolveInput,
    pub rake: proto::Rake,
    pub bb_chips: u32,
    pub key: KeyFields,
    pub spr: Rational,
}

/// What `PresolveExecutor::poll` reports about a submitted job.
pub enum JobPoll {
    Running,
    /// The worker's validated solution, normalized into a cache entry, not yet stored.
    Complete(CacheEntry),
    /// The job was cancelled, by this scheduler or by the executor's own worker owner.
    Cancelled,
    Failed(String),
}

impl std::fmt::Debug for JobPoll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobPoll::Running => f.write_str("Running"),
            JobPoll::Complete(entry) => write!(f, "Complete(entry on {:?})", entry.key.canonical_board),
            JobPoll::Cancelled => f.write_str("Cancelled"),
            JobPoll::Failed(error) => f.debug_tuple("Failed").field(error).finish(),
        }
    }
}

/// The downward callback the engine implements (task 16), distinct from its `WorkerLink`. Every
/// method is called only on the `presolver` thread, one at a time. Background jobs use
/// `flop_fast_v1`, deadline 600000 ms, target 50 bp and extraction 600 ms (spec section 10.5); they
/// never emit user-facing decision events or snapshots and never overwrite live evidence -- all of
/// which is the executor's side.
pub trait PresolveExecutor: Send + 'static {
    /// The clock the queue runs on (ruling S1): its monotonic timeline times the idle gate, retry
    /// waits and checkpoints, and its wall clock stamps persisted retry deadlines. Called once.
    fn clock(&self) -> Box<dyn QueueClock>;

    /// The fingerprint of the source bundle, `HandConfig` and template/tree versions preparation
    /// currently runs under (ruling S2): the queue's generation. It must describe the snapshot
    /// `prepare` uses; after changing that snapshot the engine sends `SourceChanged`.
    fn generation(&self) -> [u8; 32];

    /// Replays `item.scenario`'s chart line on `item.board` and builds its background job. An error
    /// (a missing chart node, an unusable configuration) is recorded as the slot's own failure.
    fn prepare(&mut self, item: &QueueItem) -> Result<PreparedJob, String>;

    /// Hands the job to the single worker owner. `cancel` is the job's own cancel flag: the owner
    /// stops the job at its next iteration boundary once it is set, which may happen at any time
    /// from any thread (live admission sets it directly). Returns the job's id.
    fn submit(&mut self, job: PreparedJob, cancel: Arc<AtomicBool>) -> Result<u64, String>;

    /// Never blocks.
    fn poll(&mut self, id: u64) -> JobPoll;

    /// Asks the owner to cancel the job. Never blocks, and never waits for the job to wind down;
    /// the scheduler forgets the job once this returns, so any later result of it is discarded.
    fn cancel(&mut self, id: u64);

    /// Blocks only inside the presolver thread: validates and durably stores the entry, then
    /// re-reads it and checks raw accuracy at target (section 10.5 "done only with a valid entry";
    /// `queue::entry_verified`). `item` is the slot's view, bound to the job's game.
    fn store_and_verify(&mut self, item: &QueueItem, entry: &CacheEntry) -> bool;

    /// Whether a validated at-target entry for `item`'s bound game exists on disk (normally
    /// `queue::entry_verified` with the key the executor prepared for `item.game`). Asked right
    /// after each `bind` and by the reconciliation sweeps, only for slots bound under the current
    /// generation.
    fn entry_exists_at_target(&mut self, item: &QueueItem) -> bool;

    /// Asked once per status publication (every iteration): keep it cheap.
    fn measured_p50_s(&self) -> Option<f64>;

    /// Per-scenario `(scenario id, hits, requests)` from the decision log; asked once per status
    /// publication: keep it cheap.
    fn scenario_hits(&self) -> Vec<(String, u64, u64)>;
}

/// What plan 5 renders for `presolver_status` (cross-plan Or7/M9). Under the `typescript` feature
/// it derives `ts_rs::TS` without `#[ts(export)]`, which would make every such test run write
/// `crates/cache/bindings/PresolverStatus.ts` into the tree: plan 5 takes the declaration from
/// `TS::decl` with `Config::with_large_int("number")`, as `proto::bindings` does, so the `u64` hit
/// counts are declared as the numbers serde_json sends rather than ts-rs's default `bigint`.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct PresolverStatus {
    pub paused: bool,
    /// The running job: its scenario id and canonical board.
    pub running: Option<String>,
    pub pending: u32,
    pub done: u32,
    pub failed: u32,
    pub tier_done: [u32; 3],
    pub tier_total: [u32; 3],
    pub measured_p50_s: Option<f64>,
    pub estimated_remaining_s: Option<f64>,
    pub scenario_hits: Vec<(String, u64, u64)>,
}

/// §10.5: no hand in progress, not paused, and no request for 30 s.
pub fn eligible(hand_in_progress: bool, paused: bool, now: u64, last_activity: u64) -> bool {
    !hand_in_progress && !paused && now.saturating_sub(last_activity) >= IDLE_MS
}

/// One scheduling decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleAction {
    Wait,
    Cancel(u64),
    Launch,
}

/// Live work cancels the running job; an idle scheduler with nothing running launches one.
pub fn next_action(active: Option<u64>, live: bool, can_start: bool) -> ScheduleAction {
    match (active, live, can_start) {
        (Some(id), true, _) => ScheduleAction::Cancel(id),
        (None, false, true) => ScheduleAction::Launch,
        _ => ScheduleAction::Wait,
    }
}

/// What the `presolver` thread is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresolverCommand {
    /// A hand began (`true`) or ended (`false`); either restarts the idle gate.
    HandInProgress(bool),
    /// A live request was admitted; restarts the idle gate.
    LiveRequest,
    Pause,
    /// Also restarts the idle gate.
    Resume,
    /// The executor's source/config snapshot may have changed (ruling S5): re-read its generation.
    SourceChanged,
    Shutdown,
}

/// The synchronous half of live admission, shared by the `Presolver` handle and the scheduler: a
/// live flag plus the running job's own cancel flag (module doc, "Live pre-emption").
#[derive(Clone, Debug, Default)]
pub struct LiveSignal {
    live: Arc<AtomicBool>,
    job: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    /// Set once shutdown is requested (`Presolver::shutdown`, or the handle dropped): no job is
    /// submitted after it.
    stopping: Arc<AtomicBool>,
}

impl LiveSignal {
    /// Shutdown was requested: sets the running job's cancel flag and stops any further submission.
    fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.cancel_job();
    }

    fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// Live admission or a hand start: marks live work for the scheduler and sets the running job's
    /// cancel flag, if a job is running, before returning. Never blocks beyond a lock held only for
    /// a pointer swap.
    pub fn raise(&self) {
        self.live.store(true, Ordering::SeqCst);
        self.cancel_job();
    }

    /// Sets the running job's cancel flag, if a job is running, and nothing else.
    pub fn cancel_job(&self) {
        if let Some(flag) = self.slot().as_ref() {
            flag.store(true, Ordering::SeqCst);
        }
    }

    /// Takes the live mark: whether live work arrived since the last take.
    fn take(&self) -> bool {
        self.live.swap(false, Ordering::SeqCst)
    }

    fn pending(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }

    fn install(&self, flag: Arc<AtomicBool>) {
        *self.slot() = Some(flag);
    }

    fn clear(&self) {
        *self.slot() = None;
    }

    fn slot(&self) -> MutexGuard<'_, Option<Arc<AtomicBool>>> {
        self.job.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// How often and how much the scheduler has written `queue.json` (ruling S8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveStats {
    /// Successful saves.
    pub saves: u64,
    /// Bytes those saves published: the file's size after each.
    pub bytes: u64,
    /// Saves that failed (each retried at the next checkpoint).
    pub failures: u64,
    /// Failure outcomes appended to the failure journal (ruling 15-R3b).
    pub journal_appends: u64,
    /// Bytes those appends wrote.
    pub journal_bytes: u64,
    /// Journal appends that failed (each retried every `SAVE_RETRY_MS`, launches held meanwhile).
    pub journal_failures: u64,
}

/// The job running in the worker.
struct Active {
    job: u64,
    slot: String,
    game: [u8; 32],
    cancel: Arc<AtomicBool>,
    label: String,
}

/// A reconciliation sweep in progress.
#[derive(Default)]
struct Sweep {
    /// The next index into `Queue::sweep_order`.
    next: usize,
    /// The games already decided in this sweep, or completed and read back by a job during it:
    /// verified by this process.
    seen: BTreeSet<[u8; 32]>,
    /// The sweep that starts with the scheduler: until it ends, only verified completions are
    /// published (module doc, "Reconciliation").
    startup: bool,
    /// Launches wait until `next` reaches this sweep position: one past the last `Done` game the
    /// sweep has not verified before the candidate the last scan chose. Zero: no wait. Reset by
    /// any change to the queue, which may change the candidate.
    gate: usize,
}

/// The presolver thread's body as a synchronous state machine (module doc).
pub struct Scheduler {
    root: PathBuf,
    queue: Queue,
    executor: Box<dyn PresolveExecutor>,
    live: LiveSignal,
    hand_in_progress: bool,
    last_activity: u64,
    /// The latest queue-clock reading taken; never decreases.
    now: u64,
    active: Option<Active>,
    sweep: Option<Sweep>,
    last_sweep_start: u64,
    /// The queue changed since it was last saved.
    dirty: bool,
    last_save: u64,
    save_error: Option<String>,
    stats: SaveStats,
    /// `(status_counts, tier_counts)`, recomputed when `counts_stale`.
    counts: ((u32, u32, u32), ([u32; 3], [u32; 3])),
    counts_stale: bool,
    /// The next iteration should follow without waiting a tick.
    busy: bool,
    scans: u64,
    /// When the last scan found nothing to launch; cleared by any change to the queue.
    exhausted_at: Option<u64>,
    /// When a status was last published.
    published: Option<u64>,
    /// Slots held back after a completion whose entry did not read back durably, each until the
    /// queue-clock reading given (fix round 1, R4). Runtime-only.
    cooldown: BTreeMap<String, u64>,
    /// The slot whose failure outcome is not on disk yet: its journal write failed. Nothing new is
    /// launched until it is written, or a snapshot holding it is saved (rulings 15-R3, 15-R3b).
    unjournaled: Option<String>,
    /// When that write was last tried.
    unjournaled_at: u64,
    /// The last journal write error reported, so each distinct one is reported once.
    journal_error: Option<String>,
}

impl Scheduler {
    /// Opens the queue in `cache_root` on the executor's clock and declares the executor's
    /// generation before anything reconciles (rulings S1, S2). The idle gate starts now, and so
    /// does the startup reconciliation sweep, which verifies the persisted completions before any
    /// later work is launched and before they are published (fix round 1, R1).
    ///
    /// # Errors
    /// Whatever `Queue::open_with_clock` returns (today it rebuilds every unusable file instead).
    pub fn open(cache_root: PathBuf, executor: Box<dyn PresolveExecutor>, live: LiveSignal) -> Result<Scheduler, CacheError> {
        let mut queue = Queue::open_with_clock(cache_root.clone(), executor.clock())?;
        let generation = executor.generation();
        let changed = generation != queue.generation();
        queue.set_generation(generation);
        let now = queue.now_ms();
        Ok(Scheduler {
            root: cache_root,
            queue,
            executor,
            live,
            hand_in_progress: false,
            last_activity: now,
            now,
            active: None,
            sweep: Some(Sweep { startup: true, ..Sweep::default() }),
            last_sweep_start: now,
            dirty: changed,
            last_save: now,
            save_error: None,
            stats: SaveStats::default(),
            counts: ((0, 0, 0), ([0; 3], [0; 3])),
            counts_stale: true,
            busy: false,
            scans: 0,
            exhausted_at: None,
            published: None,
            cooldown: BTreeMap::new(),
            unjournaled: None,
            unjournaled_at: now,
            journal_error: None,
        })
    }

    /// Applies one command; `false` for `Shutdown` (the caller then calls `shutdown`).
    pub fn apply(&mut self, command: PresolverCommand) -> bool {
        let now = self.refresh_now();
        match command {
            PresolverCommand::HandInProgress(in_progress) => {
                self.hand_in_progress = in_progress;
                self.last_activity = now;
            }
            PresolverCommand::LiveRequest => self.last_activity = now,
            PresolverCommand::Pause => {
                if !self.queue.paused() {
                    self.queue.set_paused(true);
                    self.touched();
                    self.flush(true);
                }
            }
            PresolverCommand::Resume => {
                if self.queue.paused() {
                    self.queue.set_paused(false);
                    self.touched();
                    self.flush(true);
                }
                self.last_activity = now;
            }
            PresolverCommand::SourceChanged => {
                let generation = self.executor.generation();
                if generation != self.queue.generation() {
                    self.cancel_active();
                    self.queue.set_generation(generation);
                    // Fix round 3 (re-review 1, N1): a generation that becomes current again must
                    // be re-verified before its persisted completions count, so a real change
                    // always restarts a startup-style sweep -- the same gate that already keeps a
                    // periodic sweep from being overtaken (module doc, "Reconciliation") applies to
                    // it too, and published counts stay conservative until it clears them.
                    self.sweep = Some(Sweep { startup: true, ..Sweep::default() });
                    self.last_sweep_start = now;
                    self.touched();
                    self.flush(true);
                }
            }
            PresolverCommand::Shutdown => return false,
        }
        true
    }

    /// One iteration: a reconciliation chunk while idle, then at most one scheduling decision
    /// (cancel for live work, or launch when idle), progress of the running job, and a checkpoint
    /// save. The chunk comes first so that the launch sees its verdicts (fix round 1, R1).
    pub fn step(&mut self) {
        let now = self.refresh_now();
        let live = self.live.take() || self.hand_in_progress;
        if live {
            self.last_activity = now;
        }
        let can_start = !self.live.stopping() && eligible(self.hand_in_progress, self.queue.paused(), now, self.last_activity);
        let idle = can_start && !self.live.pending();
        if idle {
            self.sweep_chunk();
        }
        let mut progressed = false;
        match next_action(self.active.as_ref().map(|a| a.job), live, can_start) {
            ScheduleAction::Cancel(_) => self.cancel_active(),
            ScheduleAction::Launch => progressed = self.launch(),
            ScheduleAction::Wait => {}
        }
        progressed |= self.progress();
        if self.sweep.is_none() && now.saturating_sub(self.last_sweep_start) >= RECONCILE_PERIOD_MS {
            self.sweep = Some(Sweep::default());
            self.last_sweep_start = now;
        }
        self.flush(false);
        self.busy = self.active.is_none() && idle && (progressed || self.sweep.is_some());
    }

    /// Cancels the running job (recording the cancel first) and saves the queue if it changed.
    pub fn shutdown(&mut self) {
        self.refresh_now();
        self.cancel_active();
        self.flush(true);
    }

    /// The status to publish. Recounts the queue only after it changed. Until the startup sweep
    /// ends, `done` and `tier_done` count only the completions verified in this process, and a
    /// persisted `Done` not yet verified counts as pending (fix round 1, R1).
    pub fn status(&mut self) -> PresolverStatus {
        if self.counts_stale {
            self.counts = match self.sweep.as_ref().filter(|sweep| sweep.startup) {
                Some(sweep) => self.queue.counts_verified(&|game| sweep.seen.contains(game)),
                None => (self.queue.status_counts(), self.queue.tier_counts()),
            };
            self.counts_stale = false;
        }
        let ((pending, done, failed), (tier_done, tier_total)) = self.counts;
        let measured_p50_s = self.executor.measured_p50_s();
        PresolverStatus {
            paused: self.queue.paused(),
            running: self.active.as_ref().map(|a| a.label.clone()),
            pending,
            done,
            failed,
            tier_done,
            tier_total,
            measured_p50_s,
            estimated_remaining_s: crate::presolver::remaining_seconds(pending, measured_p50_s),
            scenario_hits: self.executor.scenario_hits(),
        }
    }

    /// How long the thread may wait for a command before the next iteration: zero right after an
    /// iteration that resolved a slot without starting a solve, or while a sweep is under way with
    /// nothing running; otherwise `TICK_MS`.
    pub fn wait_hint(&self) -> Duration {
        if self.busy {
            Duration::ZERO
        } else {
            Duration::from_millis(TICK_MS)
        }
    }

    /// Whether a reconciliation sweep is under way.
    pub fn reconciling(&self) -> bool {
        self.sweep.is_some()
    }

    /// How many times the scheduler has asked the queue for the next pending slot to launch.
    pub fn pending_scans(&self) -> u64 {
        self.scans
    }

    /// Whether the thread publishes a status after this iteration: always when it will wait a tick
    /// before the next one, and at most once per `BUSY_PUBLISH_MS` while iterations follow back to
    /// back. Answering `true` counts as the publication.
    pub fn should_publish(&mut self) -> bool {
        let due = !self.busy || self.published.map_or(true, |at| self.now.saturating_sub(at) >= BUSY_PUBLISH_MS);
        if due {
            self.published = Some(self.now);
        }
        due
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn save_stats(&self) -> SaveStats {
        self.stats
    }

    /// The queue clock's reading, never behind one already taken.
    ///
    /// # Panics
    /// If the clock's monotonic timeline ran backwards (always-on; the `QueueClock` contract).
    fn refresh_now(&mut self) -> u64 {
        let now = self.queue.now_ms();
        assert!(now >= self.now, "presolver: the queue clock ran backwards from {} ms to {now} ms", self.now);
        self.now = now;
        now
    }

    fn touched(&mut self) {
        self.dirty = true;
        self.counts_stale = true;
        self.exhausted_at = None;
        if let Some(sweep) = self.sweep.as_mut() {
            sweep.gate = 0;
        }
    }

    /// The launch path (module doc; ruling S3). Returns whether a slot was taken from the queue.
    /// Nothing is launched while a failure outcome is not on disk (its journal write is retried
    /// here every `SAVE_RETRY_MS`), nor while a sweep has yet to verify a `Done` game before the
    /// candidate.
    /// After a scan that found nothing, the queue is scanned again only once it changed or
    /// `RESCAN_MS` has passed.
    fn launch(&mut self) -> bool {
        if let Some(slot) = self.unjournaled.clone() {
            if self.now.saturating_sub(self.unjournaled_at) < SAVE_RETRY_MS {
                return false;
            }
            self.journal_outcome(&slot);
            if self.unjournaled.is_some() {
                return false;
            }
        }
        if self.sweep.as_ref().is_some_and(|sweep| sweep.next < sweep.gate) {
            return false;
        }
        if self.exhausted_at.is_some_and(|at| self.now.saturating_sub(at) < RESCAN_MS) {
            return false;
        }
        self.scans += 1;
        let now = self.now;
        self.cooldown.retain(|_, until| *until > now);
        let cooling = &self.cooldown;
        let Some((at, item)) = self.queue.next_pending_at(now, &|slot| cooling.contains_key(slot)) else {
            self.exhausted_at = Some(now);
            return false;
        };
        if let Some(sweep) = self.sweep.as_mut() {
            // Ruling 15-R1: no candidate later in sweep order than a completion the sweep has not
            // verified yet; wait until the sweep has passed the last one before the candidate.
            let seen = &sweep.seen;
            let unverified = self.queue.done_bindings(sweep.next..at).filter(|(_, game)| !seen.contains(game)).last();
            if let Some((last, _)) = unverified {
                sweep.gate = last + 1;
                return false;
            }
        }
        let id = item.identity_hex();
        self.touched();
        let prepared = self.executor.prepare(&item).and_then(|job| checked(&item, job));
        let now = self.refresh_now();
        let job = match prepared {
            Ok(job) => job,
            Err(error) => {
                self.queue.record_launch(&id);
                self.queue.record_failure(&id, now, error);
                self.queue.advance_cursor();
                self.journal_outcome(&id);
                return true;
            }
        };
        let view = self.queue.bind(&id, &job.key, job.spr);
        let game = view.game.expect("bind binds the slot").identity;
        let exists = self.executor.entry_exists_at_target(&view);
        self.queue.reconcile_item(&id, exists);
        if let Some(sweep) = self.sweep.as_mut() {
            sweep.seen.insert(game);
        }
        let now = self.refresh_now();
        let cooling = &self.cooldown;
        let mut refused = false;
        if self.queue.next_pending_at(now, &|slot| cooling.contains_key(slot)).is_some_and(|(_, next)| next.identity == item.identity) {
            let cancel = Arc::new(AtomicBool::new(false));
            self.live.install(Arc::clone(&cancel));
            if self.live.pending() || self.live.stopping() {
                // Live work (or shutdown) arrived while the job was being prepared: leave the slot
                // unlaunched.
                self.live.clear();
            } else {
                let label = format!("{} {}", item.scenario.id(), cards(&item.board));
                match self.executor.submit(job, Arc::clone(&cancel)) {
                    Ok(job) => {
                        self.queue.record_launch(&id);
                        self.active = Some(Active { job, slot: id.clone(), game, cancel, label });
                    }
                    Err(error) => {
                        self.live.clear();
                        let now = self.refresh_now();
                        self.queue.record_launch(&id);
                        self.queue.record_failure(&id, now, error);
                        refused = true;
                    }
                }
            }
        }
        self.queue.advance_cursor();
        if refused {
            self.journal_outcome(&id);
        }
        true
    }

    /// Polls the running job and records its outcome. Returns whether the job ended.
    fn progress(&mut self) -> bool {
        let Some(job) = self.active.as_ref().map(|a| a.job) else { return false };
        let outcome = match self.executor.poll(job) {
            JobPoll::Running => return false,
            outcome => outcome,
        };
        let active = self.active.take().expect("checked above");
        self.live.clear();
        self.touched();
        match outcome {
            JobPoll::Running => unreachable!("handled above"),
            JobPoll::Complete(entry) => {
                let view = self.queue.item(&active.slot).expect("the running slot is a queue slot");
                let verified = self.executor.store_and_verify(&view, &entry);
                let now = self.refresh_now();
                if verified {
                    self.queue.record_done(&active.slot);
                    if let Some(sweep) = self.sweep.as_mut() {
                        sweep.seen.insert(active.game);
                    }
                } else {
                    // Ruling 15-R4: the cache could not keep a solve that did not fail. The attempt
                    // is refunded -- the game is pending again, or (fix round 3, N3) a retry's own
                    // `Failed{n}` is restored unchanged -- and the slot cools down in memory so a
                    // solve that never verifies cannot hot-loop.
                    self.queue.record_cancel(&active.slot);
                    self.cooldown.insert(active.slot.clone(), now.saturating_add(NOT_DURABLE_COOLDOWN_MS));
                    let status = self.queue.item(&active.slot).expect("the cancelled slot is a queue slot").status;
                    eprintln!("{}", not_durable_message(&active.label, &status));
                }
            }
            JobPoll::Cancelled => self.queue.record_cancel(&active.slot),
            JobPoll::Failed(error) => {
                let now = self.refresh_now();
                self.queue.record_failure(&active.slot, now, error);
                self.journal_outcome(&active.slot);
            }
        }
        true
    }

    /// Cancels the running job, if any: the queue records the cancel before anything else, then the
    /// job's flag is set and the executor told. Never waits for the job.
    fn cancel_active(&mut self) {
        let Some(active) = self.active.take() else { return };
        self.queue.record_cancel(&active.slot);
        active.cancel.store(true, Ordering::SeqCst);
        self.live.clear();
        self.executor.cancel(active.job);
        self.touched();
    }

    /// Decides up to `RECONCILE_CHUNK` games of the sweep under way.
    fn sweep_chunk(&mut self) {
        let Some(mut sweep) = self.sweep.take() else { return };
        let order = self.queue.sweep_order();
        let generation = self.queue.generation();
        let running = self.active.as_ref().map(|a| a.game);
        let mut decided = 0;
        while decided < RECONCILE_CHUNK && sweep.next < order.len() {
            let id = &order[sweep.next];
            sweep.next += 1;
            let view = self.queue.item(id).expect("a slot of the frozen enumeration");
            let Some(binding) = view.game.filter(|b| b.generation == generation) else { continue };
            if running == Some(binding.identity) || !sweep.seen.insert(binding.identity) {
                continue;
            }
            let valid = self.executor.entry_exists_at_target(&view);
            self.queue.reconcile_item(id, valid);
            if self.queue.item(id).map(|after| after.status) != Some(view.status) {
                self.touched(); // the sweep is out of `self.sweep` here: reset its gate below
                sweep.gate = 0;
            }
            decided += 1;
        }
        if sweep.startup && decided > 0 {
            self.counts_stale = true; // more completions are verified, so more are published
        }
        if sweep.next < order.len() {
            self.sweep = Some(sweep);
        } else if sweep.startup {
            self.counts_stale = true;
        }
    }

    /// Puts slot `id`'s just-recorded failure outcome on disk before the scheduler goes on
    /// (rulings 15-R3, 15-R3b): one journal record, or -- when the journal is full -- a snapshot
    /// that folds it. If neither reaches the disk, nothing new is launched until one does
    /// (`launch` tries the journal again every `SAVE_RETRY_MS`).
    fn journal_outcome(&mut self, id: &str) {
        self.unjournaled_at = self.now;
        self.unjournaled = Some(id.to_owned());
        match self.queue.journal_outcome(id) {
            Ok(JournalAppend::Appended { bytes }) => {
                self.unjournaled = None;
                self.stats.journal_appends += 1;
                self.stats.journal_bytes += bytes;
                if self.journal_error.take().is_some() {
                    eprintln!("presolver queue: the failure journal {} is written again", queue::journal_path(&self.root).display());
                }
            }
            Ok(JournalAppend::Full) => self.flush(true), // a saved snapshot holds the outcome
            Err(error) => {
                self.stats.journal_failures += 1;
                let message = error.to_string();
                if self.journal_error.as_deref() != Some(message.as_str()) {
                    eprintln!(
                        "presolver queue: appending to the failure journal {} failed ({message}); nothing new is launched until the failure is on disk (retried every {SAVE_RETRY_MS} ms)",
                        queue::journal_path(&self.root).display()
                    );
                }
                self.journal_error = Some(message);
            }
        }
    }

    /// Saves the queue if it changed: now if `force`, else once `CHECKPOINT_MS` has passed since
    /// the last attempt. The snapshot folds the failure journal, so a saved one also puts an
    /// unjournaled outcome on disk. A failure is counted, reported once per distinct error, and left
    /// dirty.
    fn flush(&mut self, force: bool) {
        if !self.dirty || (!force && self.now.saturating_sub(self.last_save) < CHECKPOINT_MS) {
            return;
        }
        self.last_save = self.now;
        let path = queue::queue_path(&self.root);
        match self.queue.save() {
            Ok(()) => {
                self.dirty = false;
                self.unjournaled = None;
                self.stats.saves += 1;
                self.stats.bytes += std::fs::metadata(&path).map_or(0, |m| m.len());
                if self.save_error.take().is_some() {
                    eprintln!("presolver queue: {} saved again", path.display());
                }
            }
            Err(error) => {
                self.stats.failures += 1;
                let message = error.to_string();
                if self.save_error.as_deref() != Some(message.as_str()) {
                    eprintln!(
                        "presolver queue: saving {} failed ({message}); the queue keeps its progress in memory and retries at the next checkpoint",
                        path.display()
                    );
                }
                self.save_error = Some(message);
            }
        }
    }
}

/// Refuses a prepared job the queue could not bind: a key that is not a flop root on the slot's own
/// canonical flop, or a zero SPR. Such a job is the slot's preparation failure, never a panic.
fn checked(item: &QueueItem, job: PreparedJob) -> Result<PreparedJob, String> {
    if job.key.root_street != proto::Street::Flop {
        return Err(format!("preparation returned a key rooted on {:?}, not the flop", job.key.root_street));
    }
    if job.key.canonical_board != item.board {
        return Err(format!(
            "preparation returned a key on {} instead of the slot's canonical flop {}",
            cards(&job.key.canonical_board),
            cards(&item.board)
        ));
    }
    if job.spr.num() == 0 {
        return Err("preparation returned a zero scenario SPR".into());
    }
    Ok(job)
}

fn cards(board: &[proto::Card]) -> String {
    board.iter().map(ToString::to_string).collect()
}

/// What the handle has told the thread and the thread has not taken yet (module doc, "The thread
/// and its mailbox"): a newer state replaces an older one, the signals are flags, and shutdown is
/// a latch. At most `MAILBOX_CAPACITY` entries, however many calls were made.
#[derive(Debug, Default)]
struct Inbox {
    /// The latest `notify_hand` state.
    hand: Option<bool>,
    /// The latest `pause` (`true`) or `resume` (`false`).
    pause: Option<bool>,
    source_changed: bool,
    /// A live request: activity that restarts the idle gate.
    activity: bool,
    /// Latched: never cleared once set.
    shutdown: bool,
}

impl Inbox {
    fn len(&self) -> usize {
        usize::from(self.hand.is_some())
            + usize::from(self.pause.is_some())
            + usize::from(self.source_changed)
            + usize::from(self.activity)
            + usize::from(self.shutdown)
    }

    /// The batch as the commands the scheduler applies, in this order: the hand state, the pause
    /// state, the source change, the live request, and shutdown last, so the final states reach the
    /// queue before a shutdown saves it.
    fn commands(&self) -> impl Iterator<Item = PresolverCommand> {
        [
            self.hand.map(PresolverCommand::HandInProgress),
            self.pause.map(|paused| if paused { PresolverCommand::Pause } else { PresolverCommand::Resume }),
            self.source_changed.then_some(PresolverCommand::SourceChanged),
            self.activity.then_some(PresolverCommand::LiveRequest),
            self.shutdown.then_some(PresolverCommand::Shutdown),
        ]
        .into_iter()
        .flatten()
    }
}

/// The bounded, coalescing mailbox between the handle and the thread (ruling 15-R2). Its lock is
/// held only to update or take the `Inbox`, never across I/O or an executor call.
#[derive(Default)]
struct Mailbox {
    inbox: Mutex<Inbox>,
    wake: Condvar,
}

impl Mailbox {
    /// Records a notification and wakes the thread. Never blocks beyond the lock.
    fn post(&self, note: impl FnOnce(&mut Inbox)) {
        note(&mut self.lock());
        self.wake.notify_one();
    }

    /// Waits up to `timeout` for a notification, then takes everything waiting as one batch. The
    /// shutdown latch stays set.
    fn take(&self, timeout: Duration) -> Inbox {
        let inbox = self.lock();
        let (mut inbox, _) = self.wake.wait_timeout_while(inbox, timeout, |inbox| inbox.len() == 0).unwrap_or_else(PoisonError::into_inner);
        let batch = std::mem::take(&mut *inbox);
        inbox.shutdown = batch.shutdown;
        batch
    }

    fn len(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> MutexGuard<'_, Inbox> {
        self.inbox.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The handle the engine owns (task 16: `Engine.presolver: Option<Arc<Presolver>>`). No method
/// waits for the presolver thread, the executor or any I/O. Notifications go to a bounded,
/// coalescing mailbox (module doc, "The thread and its mailbox"): the thread always applies the
/// latest hand and pause states, nothing is refused, and nothing piles up. Dropping the handle
/// shuts the thread down, as `shutdown` does, without waiting for it.
pub struct Presolver {
    mailbox: Arc<Mailbox>,
    status: Arc<RwLock<PresolverStatus>>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Set synchronously by `notify_live_request` and `notify_hand(true)` at engine admission, so a
    /// live job is protected even before the presolver thread takes its mailbox (section 7).
    live: LiveSignal,
}

impl Presolver {
    /// Starts the `presolver` thread over the cache root `dir`. The queue is opened on the thread,
    /// so this returns at once; until it is open, `status` is the default.
    pub fn start(dir: PathBuf, executor: Box<dyn PresolveExecutor>) -> Presolver {
        let mailbox = Arc::new(Mailbox::default());
        let status = Arc::new(RwLock::new(PresolverStatus::default()));
        let live = LiveSignal::default();
        let (thread_mailbox, thread_status, thread_live) = (Arc::clone(&mailbox), Arc::clone(&status), live.clone());
        let handle = std::thread::Builder::new()
            .name("presolver".into())
            .spawn(move || run(dir, executor, &thread_mailbox, &thread_status, thread_live))
            .map_err(|error| eprintln!("presolver: the thread did not start ({error}); background presolving is off"))
            .ok();
        Presolver { mailbox, status, handle: Mutex::new(handle), live }
    }

    pub fn pause(&self) {
        self.mailbox.post(|inbox| inbox.pause = Some(true));
    }

    pub fn resume(&self) {
        self.mailbox.post(|inbox| inbox.pause = Some(false));
    }

    pub fn status(&self) -> PresolverStatus {
        self.status.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn notify_hand(&self, in_progress: bool) {
        if in_progress {
            self.live.raise();
        }
        self.mailbox.post(|inbox| inbox.hand = Some(in_progress));
    }

    /// Synchronous flag plus an asynchronous signal: admission never waits for the thread.
    pub fn notify_live_request(&self) {
        self.live.raise();
        self.mailbox.post(|inbox| inbox.activity = true);
    }

    /// The executor's source/config snapshot changed (ruling S5); call after the executor's
    /// `generation` already describes the new snapshot.
    pub fn notify_source_changed(&self) {
        self.mailbox.post(|inbox| inbox.source_changed = true);
    }

    /// Signals the running job's cancellation and stops any further submission, then asks the
    /// thread to apply the latest states, record the cancel, save the queue and stop. The UI
    /// command path never joins the thread; `join_for_shutdown` is called only from
    /// `Engine::shutdown`. Repeating it is harmless.
    pub fn shutdown(&self) {
        self.live.stop();
        self.mailbox.post(|inbox| inbox.shutdown = true);
    }

    /// Waits for the thread to stop; a second call returns at once.
    pub fn join_for_shutdown(&self) {
        let handle = self.handle.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(handle) = handle {
            if handle.join().is_err() {
                eprintln!("presolver: the thread panicked; queue.json holds what it last saved");
            }
        }
    }

    /// How many notifications the thread has not taken yet: never more than `MAILBOX_CAPACITY`.
    pub fn pending_notifications(&self) -> usize {
        self.mailbox.len()
    }
}

impl Drop for Presolver {
    /// The engine let go of the handle: the thread stops as after `shutdown` (it is not joined).
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The `presolver` thread: opens the scheduler, then alternates between taking the mailbox (one
/// bounded batch) and one iteration, publishing the status after each. The status lock is held
/// only to swap the value in, never across I/O or an executor call.
fn run(dir: PathBuf, executor: Box<dyn PresolveExecutor>, mailbox: &Mailbox, status: &RwLock<PresolverStatus>, live: LiveSignal) {
    let mut scheduler = match Scheduler::open(dir, executor, live) {
        Ok(scheduler) => scheduler,
        Err(error) => {
            eprintln!("presolver: the queue could not be opened ({error}); background presolving is off");
            return;
        }
    };
    if scheduler.should_publish() {
        publish(status, scheduler.status());
    }
    loop {
        let batch = mailbox.take(scheduler.wait_hint());
        let mut stop = false;
        for command in batch.commands() {
            stop |= !scheduler.apply(command);
        }
        if stop {
            scheduler.shutdown();
            publish(status, scheduler.status());
            return;
        }
        scheduler.step();
        if scheduler.should_publish() {
            publish(status, scheduler.status());
        }
    }
}

fn publish(status: &RwLock<PresolverStatus>, value: PresolverStatus) {
    *status.write().unwrap_or_else(PoisonError::into_inner) = value;
}
