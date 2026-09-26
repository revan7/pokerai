//! Spec section 10.5: the pre-solver's durable job queue, persisted as `queue.json` in the cache
//! root (`queue_path`) beside the cell shards, and published only through
//! `crate::storage::write_atomic` (temp file, fsync, rename).
//!
//! ## Items and their identity
//!
//! The queue is the frozen enumeration of task 13: one item per (tier, canonical flop, scenario),
//! 1,755 x 24 = 42,120 items, walked tier by tier, each tier board by board, and on each board
//! through every scenario of that tier in frozen order (spec section 10.5: "within a tier the
//! outer loop runs over canonical flops ... and the inner loop over the tier's scenarios, so
//! scenarios advance together").
//!
//! Each item is keyed by `identity_hex()` of a deterministic identity: a domain-separated sha256
//! of the queue version, `Scenario::id()`, the canonical board's card ids and the scenario's
//! nominal SPR (`depth_bb : 1`) -- a function of the scenario and the canonical flop only, never of
//! wall-clock time, a hand id or a board index. The spec's *normalized game identity* (the cache
//! key without `spr_bucket`, plus the exact scenario SPR: `KeyFields::scenario_identity`) folds in
//! range hashes, rake, tree signature and model, which exist only once task 16 has replayed the
//! scenario's chart line; that identity is what `entry_verified` demands of the entry on disk.
//! A new source bundle is therefore a new normalized identity: the old bundle's entries stay in
//! the cache under their own keys, and no longer count as completing the item.
//!
//! ## Status, attempts and the one job in flight
//!
//! `TaskStatus` is `Pending`, `Done` or `Failed{n}` with `n` the actual number of failed attempts.
//! `attempts` counts launches charged to the item's budget of `MAX_ATTEMPTS` (the initial attempt
//! plus three retries). `record_launch` charges one and is persisted before the worker starts, so a
//! launch is never lost; the status itself only changes on an outcome:
//!
//! - `record_done`: `Done`. The caller verifies first (below).
//! - `record_failure`: `Failed{n = attempts}`, retry due after `RETRY_BACKOFF_MS`, or never once
//!   the fourth attempt has failed (`retry_after_unix_ms = u64::MAX`).
//! - `record_cancel` (live work pre-empted the job): the attempt is refunded and the item is back
//!   in its pre-launch state -- `Pending`, or `Failed{n}` still due for its retry. A cancellation
//!   never consumes a retry.
//!
//! At most one item is in flight (the engine runs one worker job). The in-flight marker itself is
//! runtime-only; on disk an in-flight item is recognizable by its charged-but-unresolved attempt
//! (`Pending` with one attempt, or `Failed{n}` with `n + 1`), and `Queue::open` refunds it: the
//! process that launched it died before any verdict, and a restart never consumes a retry.
//!
//! ## The cursor
//!
//! `cursor` is the sweep frontier `(tier_index, flop_index, scenario_index)`, `scenario_index`
//! counting within the tier. `advance_cursor` steps it over every item that is no longer fresh
//! (launched, failed or done), so it never repeats work reconciliation has already verified, and
//! a launch that was a revisit leaves it where it was. Past the last item it rests on the explicit
//! `CURSOR_END` sentinel; it never wraps. `next_pending` serves the earliest launchable item in
//! sweep order: everything before the cursor has been launched or verified at least once, so
//! anything launchable there is a revisit -- a `Done` demoted by reconciliation, or a failure whose
//! retry is due -- and runs first, keeping tier 1 ahead of tier 2.
//!
//! ## Retry deadlines and clocks
//!
//! `now_ms` is the caller's monotonic millisecond clock. A deadline is computed in `u128` and
//! checked against `RETRY_DEADLINE_MAX_MS`, refused loudly past it, never saturated. A deadline more
//! than one backoff ahead of `now_ms` cannot come from a failure recorded on this clock -- the clock
//! jumped backwards, or this is a new process whose clock restarted -- so it counts as due: a
//! persisted deadline is effectively clamped to `0..=30 s` and a clock jump can never stall the
//! queue.
//!
//! ## Verified completion
//!
//! `Done` means a validated entry at target exists on disk for the item's normalized identity,
//! never that a store was issued: a writer receipt is only a barrier. `entry_verified` reads the
//! cell back through `crate::storage::read_cell` (full decode plus `validate_entry`) and checks the
//! identity and raw accuracy (`crate::label::accuracy_ok`); `reconcile` applies such a predicate to
//! every item on startup and periodically, promoting items whose entries exist and demoting `Done`
//! items whose entries are gone, corrupt, or built from other ranges.
//!
//! ## The file
//!
//! `QueueFile` version 1 is independent of the cache's `schema_version` 3. Both serde directions
//! are validated against the frozen enumeration by the same `validate`: `save_queue` refuses to
//! publish a file `Queue::open` would reject, and `Queue::open` rejects any file that is oversized,
//! unparsable, of another version, carries an unknown field or a duplicate key, lacks or adds an
//! item, moves an identity, board, scenario or SPR, or holds inconsistent progress -- and then
//! rebuilds the whole queue from the frozen enumeration. A file is never partially trusted;
//! existing valid cache cells re-establish completion through `reconcile`.
//!
//! The file is compact JSON. Measured on the 42,120 items: a fresh queue is 17.2 MB compact
//! against 37.7 MB pretty-printed. The largest file the queue can ever publish has every item
//! failed four times with a maximal message whose every byte needs a JSON escape; with 512-byte
//! messages that is 83.5 MB pretty-printed -- past the 64 MiB `QUEUE_FILE_MAX`, which would lock
//! `save` out for good -- and 61.4 MB compact. Compact, with failure messages stored on one line
//! and cut at `LAST_ERROR_MAX_BYTES` (256), it is 39.7 MB, well inside the bound (pinned by a
//! test).

use super::scenarios::{canonical_flops_ordered, scenarios, Scenario, CANONICAL_FLOP_COUNT};
use crate::key::{spr_bucket, KeyFields, Rational};
use crate::CacheError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// `QueueFile::version`: the queue's own format, independent of the cache's `schema_version` 3.
pub const QUEUE_VERSION: u16 = 1;
/// The largest `queue.json` that is read or published, in bytes.
pub const QUEUE_FILE_MAX: u64 = 64 * 1024 * 1024;
/// The initial attempt plus three retries (spec section 10.5: "retries 3 with 30 s backoff").
pub const MAX_ATTEMPTS: u8 = 4;
/// The retry backoff, in milliseconds (spec section 10.5).
pub const RETRY_BACKOFF_MS: u64 = 30_000;
/// The largest retry deadline a failure may record: `i64::MAX` milliseconds, the same ceiling as
/// `crate::quota::LAST_HIT_MAX`. Leaves `u64::MAX` free as the terminal marker.
pub const RETRY_DEADLINE_MAX_MS: u64 = i64::MAX as u64;
/// The largest stored failure message, in UTF-8 bytes (`record_failure` cuts longer ones). With it
/// the largest possible file -- every item failed four times, every message byte needing a JSON
/// escape -- stays far inside `QUEUE_FILE_MAX` (module doc, "The file").
pub const LAST_ERROR_MAX_BYTES: usize = 256;
/// The cursor once the sweep has passed the last item: tier 3, one past its last canonical flop.
pub const CURSOR_END: [usize; 3] = [2, CANONICAL_FLOP_COUNT, 0];

/// `retry_after_unix_ms` of a terminal failure: never retried.
const NEVER: u64 = u64::MAX;
/// Appended to a failure message that `record_failure` had to cut.
const CUT_MARK: &str = "...";
/// Domain separation for item identities, so they can never collide with another sha256 input.
const IDENTITY_DOMAIN: &[u8] = b"pokerai/presolver-queue/item\0";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskStatus {
    Pending,
    Done,
    Failed { n: u8 },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueItem {
    pub identity: [u8; 32],
    pub scenario: Scenario,
    pub board: Vec<proto::Card>,
    pub spr: Rational,
    pub status: TaskStatus,
    pub retry_after_unix_ms: u64,
    pub attempts: u8,
    pub last_error: Option<String>,
}

impl QueueItem {
    pub fn identity_hex(&self) -> String {
        self.identity.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueFile {
    pub version: u16,
    pub cursor: [usize; 3],
    pub paused: bool,
    #[serde(deserialize_with = "unique_items")]
    pub items: BTreeMap<String, QueueItem>,
}

/// Decodes the `items` map, refusing a key that occurs twice (JSON objects may repeat a key, and
/// a plain map decode would silently keep the last one).
fn unique_items<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, QueueItem>, D::Error> {
    struct Items;
    impl<'de> serde::de::Visitor<'de> for Items {
        type Value = BTreeMap<String, QueueItem>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("queue items keyed by identity")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut items = BTreeMap::new();
            while let Some((key, item)) = map.next_entry::<String, QueueItem>()? {
                if items.contains_key(&key) {
                    return Err(serde::de::Error::custom(format!("duplicate queue item {key}")));
                }
                items.insert(key, item);
            }
            Ok(items)
        }
    }
    d.deserialize_map(Items)
}

/// Reconciliation's status rule: a validated entry at target makes an item `Done`; without one,
/// `Done` falls back to `Pending` and every other status is left alone.
pub fn reconcile_status(status: &mut TaskStatus, valid: bool) {
    if valid {
        *status = TaskStatus::Done;
    } else if matches!(status, TaskStatus::Done) {
        *status = TaskStatus::Pending;
    }
}

/// The backoff before the next attempt once `attempts` attempts have failed: 30 s through the
/// third failure, and `None` -- no further attempt -- once the fourth has failed.
pub fn retry_delay(attempts: u8) -> Option<std::time::Duration> {
    (attempts <= 3).then_some(std::time::Duration::from_millis(RETRY_BACKOFF_MS))
}

/// `queue.json` in the cache root (constraints: a sibling of the cell shards in the same `v3`
/// directory).
pub fn queue_path(cache_root: &Path) -> PathBuf {
    cache_root.join("queue.json")
}

/// Validates `file` exactly as `Queue::open` does, then publishes it atomically as compact JSON
/// (module doc, "The file": the pretty form would not fit the bound).
///
/// # Errors
/// `CacheError::Invalid` if `file` would not reopen (see `validate`) or serializes past
/// `QUEUE_FILE_MAX`, leaving the published file untouched; otherwise any `write_atomic` failure.
pub fn save_queue(path: &Path, file: &QueueFile) -> Result<(), CacheError> {
    validate(file)?;
    let bytes = serde_json::to_vec(file)?;
    if bytes.len() as u64 > QUEUE_FILE_MAX {
        return Err(CacheError::Invalid("queue size"));
    }
    crate::storage::write_atomic(path, &bytes)
}

/// Verified completion (spec section 10.5: "a task is `done` only when a valid entry at target
/// exists"): whether the cache under `cache_root` holds, for `item`, a validated entry whose
/// normalized identity is `key.scenario_identity(spr)` and whose raw exploitability meets
/// `target_bp`. `key` and `spr` are the prepared job's cache key and exact scenario SPR (task 16);
/// the key must be on the item's canonical flop.
///
/// The cell is read back from disk through `crate::storage::read_cell` -- a full decode that
/// re-runs `validate_entry` on every entry -- so a store that was issued, or even confirmed by the
/// writer's receipt, counts only once it reads back. A missing, corrupt or unreadable cell, an
/// entry for another identity (other ranges, rake, tree, model or SPR), or one above target is
/// `false`. So is a zero SPR, which no valid entry has. A corrupt cell meets `read_cell`'s own
/// identity-checked, best-effort deletion, exactly as on a lookup.
pub fn entry_verified(cache_root: &Path, item: &QueueItem, key: &KeyFields, spr: Rational, target_bp: u16) -> bool {
    if spr.num() == 0 || key.root_street != proto::Street::Flop || key.canonical_board != item.board {
        return false;
    }
    let identity = key.scenario_identity(spr);
    let cell = key.at_bucket(spr_bucket(spr)).digest();
    let Some(found) = crate::storage::read_cell(&crate::storage::entry_path(cache_root, cell)) else {
        return false;
    };
    found
        .entries
        .iter()
        .any(|e| e.key.scenario_identity(e.source.spr) == identity && crate::label::accuracy_ok(e.exploitability_over_P, target_bp))
}

// --- the frozen enumeration --------------------------------------------------------------------

/// The one queue every valid `queue.json` must describe, computed once per process: the items in
/// sweep order plus the index arithmetic between sweep positions, cursors and key order.
struct Frozen {
    /// Every item, `Pending` and unlaunched, in sweep order.
    items: Vec<QueueItem>,
    /// `items[i].identity_hex()`.
    keys: Vec<String>,
    /// For the `k`-th key in ascending order -- `QueueFile::items` iteration order -- its sweep index.
    by_key: Vec<usize>,
    /// Scenarios per tier.
    tier_len: [usize; 3],
    /// The sweep index of each tier's first item.
    tier_base: [usize; 3],
}

fn frozen() -> &'static Frozen {
    static FROZEN: std::sync::OnceLock<Frozen> = std::sync::OnceLock::new();
    FROZEN.get_or_init(|| {
        let boards = canonical_flops_ordered().len();
        assert_eq!(boards, CANONICAL_FLOP_COUNT, "the canonical flop enumeration must have CANONICAL_FLOP_COUNT boards");
        let mut tier_len = [0_usize; 3];
        for s in scenarios() {
            assert!((1..=3).contains(&s.tier), "scenario {} has tier {}, outside 1..=3", s.id(), s.tier);
            tier_len[usize::from(s.tier) - 1] += 1;
        }
        assert!(tier_len.iter().all(|&n| n > 0), "every tier needs a scenario: {tier_len:?}");
        let tier_base = [0, tier_len[0] * boards, (tier_len[0] + tier_len[1]) * boards];
        let items = Queue::all_items();
        assert_eq!(items.len(), tier_len.iter().sum::<usize>() * boards);
        let keys: Vec<String> = items.iter().map(QueueItem::identity_hex).collect();
        let mut by_key: Vec<usize> = (0..items.len()).collect();
        by_key.sort_by(|a, b| keys[*a].cmp(&keys[*b]));
        for pair in by_key.windows(2) {
            assert_ne!(keys[pair[0]], keys[pair[1]], "two queue items share an identity");
        }
        Frozen { items, keys, by_key, tier_len, tier_base }
    })
}

/// The sweep index `cursor` names (`items.len()` for `CURSOR_END`), or `None` if it names none.
fn sweep_index(cursor: [usize; 3]) -> Option<usize> {
    let frozen = frozen();
    if cursor == CURSOR_END {
        return Some(frozen.items.len());
    }
    let [tier, flop, scenario] = cursor;
    (tier < 3 && flop < CANONICAL_FLOP_COUNT && scenario < frozen.tier_len[tier])
        .then(|| frozen.tier_base[tier] + flop * frozen.tier_len[tier] + scenario)
}

/// The cursor naming sweep index `index`; `items.len()` is `CURSOR_END`.
///
/// # Panics
/// If `index` is past `items.len()` (always-on, standing ruling (b)).
fn cursor_at(index: usize) -> [usize; 3] {
    let frozen = frozen();
    assert!(index <= frozen.items.len(), "cursor_at: sweep index {index} is past the {} queue items", frozen.items.len());
    if index == frozen.items.len() {
        return CURSOR_END;
    }
    let tier = (0..3).rev().find(|&t| index >= frozen.tier_base[t]).expect("tier_base[0] is 0");
    let within = index - frozen.tier_base[tier];
    [tier, within / frozen.tier_len[tier], within % frozen.tier_len[tier]]
}

// --- per-item progress rules ---------------------------------------------------------------------

/// Launchable now or once its retry is due: a fresh `Pending`, or a non-terminal failure that is
/// not already relaunched.
fn launchable(item: &QueueItem) -> bool {
    match item.status {
        TaskStatus::Pending => item.attempts == 0,
        TaskStatus::Failed { n } => n < MAX_ATTEMPTS && item.attempts == n,
        TaskStatus::Done => false,
    }
}

/// Charged an attempt that has no outcome yet.
fn launched(item: &QueueItem) -> bool {
    match item.status {
        TaskStatus::Pending => item.attempts == 1,
        TaskStatus::Failed { n } => u16::from(item.attempts) == u16::from(n) + 1,
        TaskStatus::Done => false,
    }
}

/// A retry deadline is due at `now_ms`, or is more than one backoff ahead of it -- which no failure
/// recorded on this clock can produce -- and so is due as well (module doc).
fn retry_due(deadline: u64, now_ms: u64) -> bool {
    deadline <= now_ms || deadline - now_ms > RETRY_BACKOFF_MS
}

/// The retry deadline `delay` after `now_ms`, computed wide.
///
/// # Panics
/// If it exceeds `RETRY_DEADLINE_MAX_MS` (always-on): refused, never saturated.
fn retry_deadline(now_ms: u64, delay: std::time::Duration) -> u64 {
    let deadline = u128::from(now_ms) + delay.as_millis();
    assert!(
        deadline <= u128::from(RETRY_DEADLINE_MAX_MS),
        "record_failure: retry deadline {now_ms} ms + {} ms exceeds RETRY_DEADLINE_MAX_MS ({RETRY_DEADLINE_MAX_MS}); the clock reading is unusable",
        delay.as_millis()
    );
    u64::try_from(deadline).expect("bounded by RETRY_DEADLINE_MAX_MS")
}

/// `error` on one line (control characters become spaces) and within `LAST_ERROR_MAX_BYTES`, cut on
/// a character boundary and marked with `CUT_MARK` when it had to be cut. Never reads past the bound.
fn bounded_error(error: &str) -> String {
    let mut out = String::new();
    for c in error.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if out.len() + c.len_utf8() > LAST_ERROR_MAX_BYTES {
            while out.len() + CUT_MARK.len() > LAST_ERROR_MAX_BYTES {
                out.pop();
            }
            out.push_str(CUT_MARK);
            break;
        }
        out.push(c);
    }
    out
}

/// The progress rules every item must satisfy, at rest or with one attempt in flight: `Pending`
/// carries no deadline or error and at most the one in-flight attempt; `Done` carries neither and
/// at most `MAX_ATTEMPTS`; `Failed{n}` has `1 <= n <= MAX_ATTEMPTS`, a bounded one-line error,
/// `attempts` equal to `n` (or `n + 1` while a retry is in flight), and a deadline -- a real one for
/// `n < MAX_ATTEMPTS`, `u64::MAX` for the terminal `n == MAX_ATTEMPTS`.
fn check_progress(item: &QueueItem) -> Result<(), &'static str> {
    let no_extras = item.retry_after_unix_ms == 0 && item.last_error.is_none();
    match item.status {
        TaskStatus::Pending if item.attempts <= 1 && no_extras => Ok(()),
        TaskStatus::Pending => Err("pending queue item progress"),
        TaskStatus::Done if item.attempts <= MAX_ATTEMPTS && no_extras => Ok(()),
        TaskStatus::Done => Err("done queue item progress"),
        TaskStatus::Failed { n } => {
            let error = item.last_error.as_deref().is_some_and(|e| e.len() <= LAST_ERROR_MAX_BYTES && !e.chars().any(char::is_control));
            let attempts = item.attempts == n || (n < MAX_ATTEMPTS && launched(item));
            let deadline = if n == MAX_ATTEMPTS {
                item.retry_after_unix_ms == NEVER
            } else {
                (RETRY_BACKOFF_MS..=RETRY_DEADLINE_MAX_MS).contains(&item.retry_after_unix_ms)
            };
            if (1..=MAX_ATTEMPTS).contains(&n) && error && attempts && deadline {
                Ok(())
            } else {
                Err("failed queue item progress")
            }
        }
    }
}

/// Whether `file` is exactly the frozen queue with consistent progress: the version, a cursor that
/// names a sweep position or `CURSOR_END`, every frozen item present once under its own identity
/// with its frozen scenario, board and SPR, every item's progress valid, and at most one in flight.
/// `save_queue` and `Queue::open` both apply it, so the two serde directions cannot disagree.
fn validate(file: &QueueFile) -> Result<(), CacheError> {
    let invalid = |why: &'static str| -> Result<(), CacheError> { Err(CacheError::Invalid(why)) };
    if file.version != QUEUE_VERSION {
        return invalid("queue version");
    }
    if sweep_index(file.cursor).is_none() {
        return invalid("queue cursor");
    }
    let frozen = frozen();
    if file.items.len() != frozen.items.len() {
        return invalid("queue item count");
    }
    let mut in_flight = 0_usize;
    for ((key, item), &sweep) in file.items.iter().zip(&frozen.by_key) {
        let expected = &frozen.items[sweep];
        if *key != frozen.keys[sweep]
            || item.identity != expected.identity
            || item.scenario != expected.scenario
            || item.board != expected.board
            || item.spr != expected.spr
        {
            return invalid("queue item identity");
        }
        check_progress(item).map_err(CacheError::Invalid)?;
        in_flight += usize::from(launched(item));
    }
    if in_flight > 1 {
        return invalid("more than one queue item in flight");
    }
    Ok(())
}

/// `queue.json` if present and valid; `Ok(None)` if absent; `Err` with the reason it was rejected.
fn load(path: &Path) -> Result<Option<QueueFile>, String> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if meta.len() > QUEUE_FILE_MAX {
        return Err(format!("{} bytes exceeds the {QUEUE_FILE_MAX}-byte bound", meta.len()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(QUEUE_FILE_MAX + 1).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > QUEUE_FILE_MAX {
        return Err(format!("grew past the {QUEUE_FILE_MAX}-byte bound while being read"));
    }
    let file: QueueFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    validate(&file).map_err(|e| e.to_string())?;
    Ok(Some(file))
}

// --- the queue ---------------------------------------------------------------------------------

pub struct Queue {
    path: PathBuf,
    file: QueueFile,
    /// The identity of the one launched item whose outcome is not recorded yet. Runtime-only; on
    /// disk such an item is recognizable by its unresolved attempt and is refunded on open.
    in_flight: Option<String>,
}

impl Queue {
    /// Bounded read, explicit version, validated cursor/board/spr/identity/progress, no duplicate
    /// keys, no unknown fields (`validate`). A corrupt or absent file is rebuilt deterministically
    /// from `scenarios()` and `canonical_flops_ordered()`; existing valid cache cells still
    /// establish completion through `reconcile`. An item a dead process had launched gets its
    /// attempt back. Never fails today: every unusable file is replaced by the rebuild.
    pub fn open(cache_root: PathBuf) -> Result<Queue, CacheError> {
        let path = queue_path(&cache_root);
        let file = match load(&path) {
            Ok(Some(file)) => file,
            Ok(None) => Self::rebuild(),
            Err(why) => {
                eprintln!("presolver queue: {} rejected ({why}); rebuilt from the frozen enumeration", path.display());
                Self::rebuild()
            }
        };
        let mut queue = Queue { path, file, in_flight: None };
        for item in queue.file.items.values_mut().filter(|item| launched(item)) {
            item.attempts = item.attempts.checked_sub(1).expect("a launched item has an attempt");
        }
        Ok(queue)
    }

    /// Deterministic rebuild in `(tier, flop, scenario)` order. Boards come from Task 13's
    /// cached canonical order, so the same install always produces the same item set.
    fn rebuild() -> QueueFile {
        let frozen = frozen();
        let items = frozen.keys.iter().cloned().zip(frozen.items.iter().cloned()).collect();
        QueueFile { version: QUEUE_VERSION, cursor: [0, 0, 0], paused: false, items }
    }

    /// Every item, `Pending` and unlaunched, in sweep order: tier, then canonical flop, then the
    /// tier's scenarios in frozen order.
    fn all_items() -> Vec<QueueItem> {
        let scenarios = scenarios();
        let boards = canonical_flops_ordered();
        let mut out = Vec::with_capacity(scenarios.len() * boards.len());
        for tier in 1_u8..=3 {
            for board in boards {
                for scenario in scenarios.iter().filter(|s| s.tier == tier) {
                    // The exact SPR is known only at preparation time (task 16); the stored value
                    // is the scenario's nominal depth ratio and is part of the identity.
                    let spr = Rational::new(u64::from(scenario.depth_bb), 1).expect("nonzero denominator");
                    out.push(QueueItem {
                        identity: Self::identity_of(scenario, board, spr),
                        scenario: scenario.clone(),
                        board: board.clone(),
                        spr,
                        status: TaskStatus::Pending,
                        retry_after_unix_ms: 0,
                        attempts: 0,
                        last_error: None,
                    });
                }
            }
        }
        out
    }

    /// The item's deterministic identity (module doc): sha256 over a domain tag, the queue
    /// version, and the length-prefixed scenario id, canonical board card ids and nominal SPR.
    fn identity_of(scenario: &Scenario, board: &[proto::Card], spr: Rational) -> [u8; 32] {
        let id = scenario.id();
        let mut hasher = Sha256::new();
        hasher.update(IDENTITY_DOMAIN);
        hasher.update(QUEUE_VERSION.to_le_bytes());
        hasher.update((id.len() as u64).to_le_bytes());
        hasher.update(id.as_bytes());
        hasher.update((board.len() as u64).to_le_bytes());
        hasher.update(board.iter().map(|c| c.0).collect::<Vec<_>>());
        hasher.update(spr.num().to_le_bytes());
        hasher.update(spr.den().to_le_bytes());
        hasher.finalize().into()
    }

    pub fn paused(&self) -> bool {
        self.file.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.file.paused = paused;
    }

    pub fn cursor(&self) -> [usize; 3] {
        self.file.cursor
    }

    pub fn save(&self) -> Result<(), CacheError> {
        save_queue(&self.path, &self.file)
    }

    /// The earliest launchable item in sweep order whose retry, if any, is due at `now_ms`: a
    /// revisit before the cursor (an invalidated `Done`, a due retry) first, else the item at the
    /// cursor. `None` when nothing can run now.
    pub fn next_pending(&self, now_ms: u64) -> Option<QueueItem> {
        let frozen = frozen();
        assert_eq!(self.file.items.len(), frozen.by_key.len(), "the queue holds exactly the frozen items");
        self.file
            .items
            .values()
            .zip(&frozen.by_key)
            .filter(|(item, _)| launchable(item) && (!matches!(item.status, TaskStatus::Failed { .. }) || retry_due(item.retry_after_unix_ms, now_ms)))
            .min_by_key(|(_, &sweep)| sweep)
            .map(|(item, _)| item.clone())
    }

    /// Steps the cursor over every item that is no longer fresh -- launched, failed or done -- so it
    /// rests on the next item the sweep has not started, or on `CURSOR_END`. After a launch at the
    /// cursor it moves past that item; after a revisit launch it stays.
    pub fn advance_cursor(&mut self) {
        let frozen = frozen();
        let mut at = sweep_index(self.file.cursor).expect("the cursor is validated");
        while at < frozen.items.len() {
            let item = &self.file.items[frozen.keys[at].as_str()];
            if item.status == TaskStatus::Pending && item.attempts == 0 {
                break;
            }
            at += 1;
        }
        self.file.cursor = cursor_at(at);
    }

    /// Persisted before launch: status stays as it was and `attempts` increments, so a restart
    /// resumes the work instead of losing it (and refunds the attempt). In-flight state is
    /// runtime-only.
    ///
    /// # Panics
    /// If another item is in flight, `id` is unknown, or the item is not launchable (done,
    /// terminal, or already launched) -- scheduler bugs, refused loudly (always-on).
    pub fn record_launch(&mut self, id: &str) {
        if let Some(running) = &self.in_flight {
            panic!("record_launch({id}): {running} is still in flight; the queue runs one presolver job at a time");
        }
        let item = self.item_mut(id, "record_launch");
        assert!(launchable(item), "record_launch({id}): the item is not launchable (status {:?}, attempts {})", item.status, item.attempts);
        item.attempts += 1;
        self.in_flight = Some(id.to_owned());
    }

    /// The in-flight job completed and its entry was verified by reading it back
    /// (`entry_verified`); a worker terminal or a writer receipt alone is not enough.
    ///
    /// # Panics
    /// If `id` is not the job in flight (always-on).
    pub fn record_done(&mut self, id: &str) {
        self.flight(id, "record_done");
        let item = self.land(id, "record_done");
        item.status = TaskStatus::Done;
        item.last_error = None;
        item.retry_after_unix_ms = 0;
    }

    /// A live-work cancellation refunds the in-flight attempt and returns the item to its
    /// pre-launch state: `Pending`, or `Failed{n}` still due for its retry. It never burns a retry.
    ///
    /// # Panics
    /// If `id` is not the job in flight (always-on).
    pub fn record_cancel(&mut self, id: &str) {
        self.flight(id, "record_cancel");
        let item = self.land(id, "record_cancel");
        item.attempts = item.attempts.checked_sub(1).expect("an item in flight has an attempt");
    }

    /// The in-flight attempt failed: `Failed{n}` with `n` the attempts made so far, retry due
    /// `RETRY_BACKOFF_MS` after `now_ms` (the caller's monotonic clock), or never after the fourth
    /// failure. `error` is stored on one line, bounded by `LAST_ERROR_MAX_BYTES`.
    ///
    /// # Panics
    /// If `id` is not the job in flight, or the deadline would exceed `RETRY_DEADLINE_MAX_MS`
    /// (always-on; nothing is recorded).
    pub fn record_failure(&mut self, id: &str, now_ms: u64, error: String) {
        let n = self.flight(id, "record_failure").attempts;
        let retry_after = match retry_delay(n) {
            Some(delay) => retry_deadline(now_ms, delay),
            None => NEVER, // the fourth failure is terminal
        };
        let item = self.land(id, "record_failure");
        item.status = TaskStatus::Failed { n };
        item.last_error = Some(bounded_error(&error));
        item.retry_after_unix_ms = retry_after;
    }

    /// `valid` is "a validated at-target entry for this exact identity exists on disk" -- normally
    /// `entry_verified` under the current source. Promotes every item it holds for to `Done` and
    /// demotes every `Done` it does not hold for to a fresh `Pending` with a full budget (the entry
    /// was evicted, corrupted or built from other ranges). The item in flight is skipped: its
    /// outcome belongs to the running job.
    pub fn reconcile(&mut self, valid: &dyn Fn(&QueueItem) -> bool) {
        for (key, item) in &mut self.file.items {
            if self.in_flight.as_deref() == Some(key.as_str()) {
                continue;
            }
            let mut status = item.status.clone();
            reconcile_status(&mut status, valid(item));
            if status == item.status {
                continue;
            }
            if status == TaskStatus::Pending {
                item.attempts = 0;
            }
            item.status = status;
            item.last_error = None;
            item.retry_after_unix_ms = 0;
        }
    }

    /// `(pending, done, failed)` over every item; an item in flight counts under its status.
    pub fn status_counts(&self) -> (u32, u32, u32) {
        let mut counts = (0, 0, 0);
        for item in self.file.items.values() {
            match item.status {
                TaskStatus::Pending => counts.0 += 1,
                TaskStatus::Done => counts.1 += 1,
                TaskStatus::Failed { .. } => counts.2 += 1,
            }
        }
        counts
    }

    /// `(done, total)` per tier.
    pub fn tier_counts(&self) -> ([u32; 3], [u32; 3]) {
        let (mut done, mut total) = ([0; 3], [0; 3]);
        for item in self.file.items.values() {
            let t = usize::from(item.scenario.tier) - 1; // validated 1..=3
            total[t] += 1;
            if item.status == TaskStatus::Done {
                done[t] += 1;
            }
        }
        (done, total)
    }

    fn item_mut(&mut self, id: &str, op: &str) -> &mut QueueItem {
        self.file.items.get_mut(id).unwrap_or_else(|| panic!("{op}: no queue item has identity {id}"))
    }

    /// The in-flight item `id`, or a panic naming `op` if `id` is not the job in flight.
    fn flight(&self, id: &str, op: &str) -> &QueueItem {
        assert!(self.in_flight.as_deref() == Some(id), "{op}({id}): that item is not the job in flight ({:?})", self.in_flight);
        &self.file.items[id]
    }

    /// Clears the in-flight marker for `id` (already checked by `flight`) and hands back its item.
    fn land(&mut self, id: &str, op: &str) -> &mut QueueItem {
        self.in_flight = None;
        self.item_mut(id, op)
    }
}
