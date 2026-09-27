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
//! - `Presolver` is the handle the engine owns: it starts the thread and forwards commands. None of
//!   its methods waits for the thread, the executor or any I/O.
//!
//! ## When jobs run
//!
//! A job is launched only when `eligible`: no hand is in progress, the queue is not paused, and no
//! live request, hand event or resume has happened for `IDLE_MS` (30 s) on the queue clock. At most
//! one job runs at a time. A pause lets a running job finish and launches nothing more; a resume
//! keeps the cursor and waits for the idle gate again. When the queue offers nothing to launch (all
//! done, terminal, or waiting out a backoff), it is scanned again only once it changes or after
//! `RESCAN_MS`: with every slot bound, a scan that finds nothing reads all 42,120 slots.
//!
//! ## The thread
//!
//! The `presolver` thread waits up to `TICK_MS` for commands, applies every command waiting, runs
//! one iteration, and publishes the status. After an iteration that resolved a slot without starting
//! a solve, or while a reconciliation sweep runs with nothing else to do, it does not wait
//! (`wait_hint` is zero) and publishes at most once per `BUSY_PUBLISH_MS`, since a publication after
//! a change recounts every slot. Commands are applied between iterations, never lost: the channel is
//! unbounded.
//!
//! ## Live pre-emption
//!
//! Every job gets its own cancel flag (`Arc<AtomicBool>`), handed to the executor with the job at
//! `submit`; the executor's worker owner observes it. `LiveSignal::raise` -- called by
//! `Presolver::notify_live_request` and `notify_hand(true)` on the engine's admitting thread --
//! sets the running job's flag at once, so the worker hears about live work without waiting for
//! this thread, its channel or the executor. The scheduler then cancels the job in its next
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
//! is `Done` only if `store_and_verify` reads its entry back durably at target; otherwise it is a
//! failed attempt, retried after the backoff.
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
//! new generation is declared and saved. Stale slots count as pending until the launch path
//! prepares them again, one slot per iteration; an iteration that resolves a slot without starting
//! a solve is followed at once by the next (`wait_hint` is zero), so rebinding costs preparations,
//! not idle ticks.
//!
//! ## Saves (ruling S8)
//!
//! `queue.json` is rewritten whole (about 32 MB once every slot is bound), so it is saved only when
//! the queue changed: at once for a pause, a resume, a generation change and shutdown, and otherwise
//! at most once per `CHECKPOINT_MS`. A launch is never saved on its own: opening the queue refunds
//! an in-flight attempt, so a saved launch and an unsaved one reopen identically, and what a crash
//! can lose -- completions, bindings, the cursor -- is re-derived from the cache by the launch path
//! and reconciliation (at most a few failure counts are lost). A failed save is counted
//! (`SaveStats::failures`), reported on stderr once per distinct error, kept dirty, and retried at
//! the next checkpoint.
//!
//! ## Reconciliation
//!
//! A sweep over every slot bound under the current generation (`reconcile_item` per game, in
//! `Queue::sweep_order`) starts with the scheduler and again every `RECONCILE_PERIOD_MS`. It runs
//! only while idle-eligible, at most `RECONCILE_CHUNK` games per iteration, so commands are handled
//! between chunks; it skips the running game, and a game the launch path already decided in this
//! sweep.

use super::queue::{self, Queue, QueueClock, QueueItem};
use crate::entry::CacheEntry;
use crate::key::{KeyFields, Rational};
use crate::CacheError;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
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
/// The failure recorded when a completed job's entry does not read back durably at target.
pub const NOT_DURABLE: &str = "entry not durable at target";
/// When the queue offers nothing to launch, how long the scheduler waits before scanning it again
/// unless it changes first, in queue-clock milliseconds: with every slot bound, a scan that finds
/// nothing reads all 42,120 slots (measured about 36 ms in a release build), and only a retry
/// coming due can make work appear without a change.
pub const RESCAN_MS: u64 = 5_000;
/// While iterations follow back to back, the least time between two status publications, in
/// queue-clock milliseconds: a publication after a change recounts every slot (measured about
/// 24 ms in a release build with every slot bound).
pub const BUSY_PUBLISH_MS: u64 = 1_000;

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
}

impl LiveSignal {
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
    /// The games already decided in this sweep.
    seen: BTreeSet<[u8; 32]>,
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
}

impl Scheduler {
    /// Opens the queue in `cache_root` on the executor's clock and declares the executor's
    /// generation before anything reconciles (rulings S1, S2). The idle gate starts now, and so
    /// does the startup reconciliation sweep.
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
            sweep: Some(Sweep::default()),
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
                    self.touched();
                    self.flush(true);
                }
            }
            PresolverCommand::Shutdown => return false,
        }
        true
    }

    /// One iteration: at most one scheduling decision (cancel for live work, or launch when idle),
    /// progress of the running job, a reconciliation chunk while idle, and a checkpoint save.
    pub fn step(&mut self) {
        let now = self.refresh_now();
        let live = self.live.take() || self.hand_in_progress;
        if live {
            self.last_activity = now;
        }
        let can_start = eligible(self.hand_in_progress, self.queue.paused(), now, self.last_activity);
        let mut progressed = false;
        match next_action(self.active.as_ref().map(|a| a.job), live, can_start) {
            ScheduleAction::Cancel(_) => self.cancel_active(),
            ScheduleAction::Launch => progressed = self.launch(),
            ScheduleAction::Wait => {}
        }
        progressed |= self.progress();
        let idle = can_start && !self.live.pending();
        if idle {
            self.sweep_chunk();
        }
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

    /// The status to publish. Recounts the queue only after it changed.
    pub fn status(&mut self) -> PresolverStatus {
        if self.counts_stale {
            self.counts = (self.queue.status_counts(), self.queue.tier_counts());
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
    }

    /// The launch path (module doc; ruling S3). Returns whether a slot was taken from the queue.
    /// After a scan that found nothing, the queue is scanned again only once it changed or
    /// `RESCAN_MS` has passed.
    fn launch(&mut self) -> bool {
        if self.exhausted_at.is_some_and(|at| self.now.saturating_sub(at) < RESCAN_MS) {
            return false;
        }
        self.scans += 1;
        let Some(item) = self.queue.next_pending(self.now) else {
            self.exhausted_at = Some(self.now);
            return false;
        };
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
        if self.queue.next_pending(now).is_some_and(|next| next.identity == item.identity) {
            let cancel = Arc::new(AtomicBool::new(false));
            self.live.install(Arc::clone(&cancel));
            if self.live.pending() {
                // Live work arrived while the job was being prepared: leave the slot unlaunched.
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
                    }
                }
            }
        }
        self.queue.advance_cursor();
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
                } else {
                    self.queue.record_failure(&active.slot, now, NOT_DURABLE.into());
                }
            }
            JobPoll::Cancelled => self.queue.record_cancel(&active.slot),
            JobPoll::Failed(error) => {
                let now = self.refresh_now();
                self.queue.record_failure(&active.slot, now, error);
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
                self.touched();
            }
            decided += 1;
        }
        if sweep.next < order.len() {
            self.sweep = Some(sweep);
        }
    }

    /// Saves the queue if it changed: now if `force`, else once `CHECKPOINT_MS` has passed since
    /// the last attempt. A failure is counted, reported once per distinct error, and left dirty.
    fn flush(&mut self, force: bool) {
        if !self.dirty || (!force && self.now.saturating_sub(self.last_save) < CHECKPOINT_MS) {
            return;
        }
        self.last_save = self.now;
        let path = queue::queue_path(&self.root);
        match self.queue.save() {
            Ok(()) => {
                self.dirty = false;
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

/// The handle the engine owns (task 16: `Engine.presolver: Option<Arc<Presolver>>`). No method
/// waits for the presolver thread, the executor or any I/O; commands travel on an unbounded channel,
/// so none is ever dropped.
pub struct Presolver {
    commands: mpsc::Sender<PresolverCommand>,
    status: Arc<RwLock<PresolverStatus>>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Set synchronously by `notify_live_request` and `notify_hand(true)` at engine admission, so a
    /// live job is protected even before the presolver thread drains its channel (section 7).
    live: LiveSignal,
}

impl Presolver {
    /// Starts the `presolver` thread over the cache root `dir`. The queue is opened on the thread,
    /// so this returns at once; until it is open, `status` is the default.
    pub fn start(dir: PathBuf, executor: Box<dyn PresolveExecutor>) -> Presolver {
        let (commands, inbox) = mpsc::channel();
        let status = Arc::new(RwLock::new(PresolverStatus::default()));
        let live = LiveSignal::default();
        let (thread_status, thread_live) = (Arc::clone(&status), live.clone());
        let handle = std::thread::Builder::new()
            .name("presolver".into())
            .spawn(move || run(dir, executor, &inbox, &thread_status, thread_live))
            .map_err(|error| eprintln!("presolver: the thread did not start ({error}); background presolving is off"))
            .ok();
        Presolver { commands, status, handle: Mutex::new(handle), live }
    }

    pub fn pause(&self) {
        self.send(PresolverCommand::Pause);
    }

    pub fn resume(&self) {
        self.send(PresolverCommand::Resume);
    }

    pub fn status(&self) -> PresolverStatus {
        self.status.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn notify_hand(&self, in_progress: bool) {
        if in_progress {
            self.live.raise();
        }
        self.send(PresolverCommand::HandInProgress(in_progress));
    }

    /// Synchronous flag plus an asynchronous event: admission never waits for the channel.
    pub fn notify_live_request(&self) {
        self.live.raise();
        self.send(PresolverCommand::LiveRequest);
    }

    /// The executor's source/config snapshot changed (ruling S5); call after the executor's
    /// `generation` already describes the new snapshot.
    pub fn notify_source_changed(&self) {
        self.send(PresolverCommand::SourceChanged);
    }

    /// Signals the running job's cancellation, then asks the thread to record it, save the queue
    /// and stop. The UI command path never joins the thread; `join_for_shutdown` is called only
    /// from `Engine::shutdown`. Repeating it is harmless.
    pub fn shutdown(&self) {
        self.live.cancel_job();
        self.send(PresolverCommand::Shutdown);
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

    fn send(&self, command: PresolverCommand) {
        // A send fails only once the thread has stopped (after `Shutdown`, or a panic that
        // `join_for_shutdown` reports); there is nothing left to tell it.
        let _ = self.commands.send(command);
    }
}

/// The `presolver` thread: opens the scheduler, then alternates between draining commands and one
/// iteration, publishing the status after each. The status lock is held only to swap the value in,
/// never across I/O or an executor call.
fn run(
    dir: PathBuf,
    executor: Box<dyn PresolveExecutor>,
    inbox: &mpsc::Receiver<PresolverCommand>,
    status: &RwLock<PresolverStatus>,
    live: LiveSignal,
) {
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
        let mut commands = Vec::new();
        let mut stop = false;
        match inbox.recv_timeout(scheduler.wait_hint()) {
            Ok(command) => commands.push(command),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => stop = true,
        }
        loop {
            match inbox.try_recv() {
                Ok(command) => commands.push(command),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    stop = true;
                    break;
                }
            }
        }
        for command in commands {
            if !scheduler.apply(command) {
                stop = true;
                break;
            }
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
