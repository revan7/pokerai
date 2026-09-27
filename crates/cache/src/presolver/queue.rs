//! Spec section 10.5: the pre-solver's durable job queue, persisted as `queue.json` in the cache
//! root (`queue_path`) beside the cell shards, and published only through
//! `crate::storage::write_atomic` (temp file, fsync, rename).
//!
//! ## Slots
//!
//! The queue walks the frozen enumeration of task 13: one *slot* per (tier, canonical flop,
//! scenario), 1,755 x 24 = 42,120 slots, tier by tier, each tier board by board, and on each board
//! through every scenario of that tier in frozen order (spec section 10.5: "within a tier the
//! outer loop runs over canonical flops ... and the inner loop over the tier's scenarios, so
//! scenarios advance together").
//!
//! A slot is a `QueueItem` keyed by `identity_hex()` of its deterministic *slot identity*: a
//! domain-separated sha256 of the queue version, `Scenario::id()`, the canonical board's card ids
//! and the nominal SPR `depth_bb : 1` -- a function of the scenario and the canonical flop only,
//! never of wall-clock time, a hand id or a board index. The slot identity is the scheduler's
//! handle (`next_pending` hands out slots and the `record_*` methods take their identity). It is
//! not the game.
//!
//! ## Progress belongs to the normalized game identity
//!
//! Spec section 10.5 keys task status by the *normalized game identity*: the cache key without
//! `spr_bucket`, plus the exact scenario SPR (`KeyFields::scenario_identity`). It folds in range
//! hashes, rake, tree signature, model and versions, which exist only once task 16 has replayed the
//! scenario's chart line under the active source bundle and configuration. So preparation *binds*
//! a slot to its identity (`bind`, with the prepared key and exact SPR), and from then on the slot
//! is an index into a game record: `QueueFile::games`, keyed by the identity's hex, holding the
//! game's board, exact SPR and progress. Launches, failures, completion, retry deadlines and
//! reconciliation are the game's, so two slots that normalize to the same game share one status and
//! one retry budget. A bound slot keeps no progress of its own; `item` and `next_pending` hand out
//! a slot *view* that carries its game's progress.
//!
//! Bindings belong to a source/config *generation*: an opaque fingerprint the caller declares with
//! `set_generation` (task 16: the active bundle's and configuration's revisions), persisted in the
//! file. A binding made under another generation is stale -- the slot must be prepared again, and
//! until then it counts as pending -- but the game it names keeps its record for as long as some
//! slot still binds it, so re-preparation that resolves to the same identity while that binding
//! survives finds that game's status again (a terminal failure stays terminal, a completion stays
//! done), while other ranges, rake or tree are another identity with a fresh budget. A game's
//! record, including a terminal failure, is forgotten the moment its last binding is released
//! (re-review round 2, N3, correcting an earlier overclaim here): a rebind to another identity, or a
//! failed preparation of its own stale slot (`record_launch` drops a stale binding before charging
//! anything new). Re-preparation that later lands back on the same identity then starts it over with
//! a full budget, so each user-initiated generation change can cost up to `MAX_ATTEMPTS` background
//! launches per game whose last binding it released. A game record lives exactly as long as some
//! slot is bound to it, which bounds the file at one game per slot.
//!
//! A preparation failure has no identity to charge. A launch of a slot that is not bound under the
//! current generation is charged to the slot's own record, which belongs to that generation:
//! `set_generation` clears it (even a terminal failure), and so does a later successful `bind`. Such
//! a launch can fail or be cancelled, never complete: `record_done` requires a bound identity. It
//! also drops a stale binding, since preparation under the current generation has not confirmed it.
//!
//! ## Status, attempts and the one job in flight
//!
//! `TaskStatus` is `Pending`, `Done` or `Failed{n}` with `n` the actual number of failed attempts.
//! `attempts` counts launches charged to a record's budget of `MAX_ATTEMPTS` (the initial attempt
//! plus three retries). `record_launch` charges one and is persisted before the worker starts, so a
//! launch is never lost; the status itself only changes on an outcome:
//!
//! - `record_done`: `Done`. The caller verifies first (below).
//! - `record_failure`: `Failed{n = attempts}`, retry due after `RETRY_BACKOFF_MS`, or never once
//!   the fourth attempt has failed (`retry_after_unix_ms = u64::MAX`).
//! - `record_cancel` (live work pre-empted the job): the attempt is refunded and the record is back
//!   in its pre-launch state -- `Pending`, or `Failed{n}` still due for its retry. A cancellation
//!   never consumes a retry.
//!
//! At most one launch is in flight (the engine runs one worker job). The in-flight marker itself is
//! runtime-only; on disk the launched record is recognizable by its charged-but-unresolved attempt
//! (`Pending` with one attempt, or `Failed{n}` with `n + 1`), and opening the queue refunds it: the
//! process that launched it died before any verdict, and a restart never consumes a retry. While a
//! launch is in flight its slot cannot be bound again and the generation cannot change.
//!
//! ## The cursor
//!
//! `cursor` is the sweep frontier `(tier_index, flop_index, scenario_index)`, `scenario_index`
//! counting within the tier. `advance_cursor` steps it over every slot whose progress is no longer
//! fresh (launched, failed or done), so it never repeats work reconciliation has already verified,
//! and a launch that was a revisit leaves it where it was. Past the last slot it rests on the
//! explicit `CURSOR_END` sentinel; it never wraps. `next_pending` serves the earliest launchable slot
//! in sweep order: everything before the cursor has been launched or verified at least once, so
//! anything launchable there is a revisit -- a `Done` demoted by reconciliation, a failure whose
//! retry is due, a slot to prepare again under a new generation -- and runs first, keeping tier 1
//! ahead of tier 2.
//!
//! ## Retry deadlines and clocks
//!
//! The file records a retry deadline as an absolute UTC time in milliseconds since the Unix epoch
//! (`retry_after_unix_ms`): the queue clock's wall-clock reading at the failure plus the 30 s
//! backoff. The wait itself runs on the monotonic timeline every `now_ms` comes from
//! (`QueueClock::monotonic_ms`; `Queue::open` uses `SystemClock`, `open_with_clock` any other): a
//! failure sets the in-process deadline `now_ms + 30 s`, which no wall-clock jump can move. On
//! opening, each persisted deadline is converted once into the backoff that remains at the current
//! wall-clock reading, clamped to `0..=30 s` -- a deadline already reached is due at once, and one
//! more than a backoff ahead (the wall clock was set back) waits one backoff -- and anchored at the
//! current monotonic reading. So a restart keeps what remains of a backoff, and no clock change can
//! stall a retry. `retry_due` is monotone (re-review round 2, N2): an in-process deadline is due
//! exactly when it is at or behind `now_ms`, never a moment sooner. Since every such deadline is set
//! at most one backoff ahead of the monotonic reading it was anchored at, `now_ms` landing more than
//! one backoff behind it can only mean the caller's own clock ran backwards relative to that anchor
//! -- a programming error, surfaced with an always-on assert rather than masked as a due retry. Both
//! deadlines are computed in `u128` and refused loudly past `RETRY_DEADLINE_MAX_MS`, never saturated.
//!
//! ## Verified completion
//!
//! `Done` means a validated entry at target exists on disk for the game's normalized identity,
//! never that a store was issued: a writer receipt is only a barrier. `entry_verified` reads the
//! cell back through `crate::storage::read_cell` (full decode plus `validate_entry`) and checks the
//! identity and raw accuracy (`crate::label::accuracy_ok`). `reconcile` applies such a predicate to
//! every game a slot is bound to under the current generation -- once per game, however many slots
//! share it -- promoting games whose entries exist and demoting `Done` games whose entries are gone,
//! corrupt, or built from other ranges; a slot with no current binding has no identity to check
//! yet. `reconcile_item` decides one bound slot's game the same way: the scheduler (task 15) calls
//! it after each `bind`, so an entry already on disk completes a game before any solve is launched,
//! and runs its startup and periodic sweeps through it, a few games per iteration in `sweep_order`,
//! rather than through one blocking `reconcile`.
//!
//! ## The file
//!
//! `QueueFile` version 2 (version 1 predates game identities and is rebuilt) is independent of the
//! cache's `schema_version` 3. Both serde directions are validated by the same `validate`:
//! `save_queue` refuses to publish a file `Queue::open` would reject, and `Queue::open` rejects any
//! file that is oversized, unparsable, of another version, carries an unknown field or a duplicate
//! key, lacks or adds a slot, moves a slot identity, board, scenario or SPR, holds inconsistent
//! progress, binds a slot to a missing game or to one on another board, keeps progress on a bound
//! slot, keeps a game no slot is bound to, or has more than one launch in flight -- and then
//! rebuilds the whole queue from the frozen enumeration. A file is never partially trusted;
//! existing valid cache cells re-establish completion through `bind` and `reconcile`.
//!
//! The file is compact JSON (in version 1, pretty-printing more than doubled a fresh queue: 37.7 MB
//! against 17.2 MB); the generation, bindings and game keys are lowercase hex strings. Measured: a
//! fresh queue is 17.7 MB. The largest file the queue can ever publish binds every slot to a game of
//! its own (a bound slot keeps no progress of its own and an unbound slot has no game, so nothing is
//! larger), each game's exact SPR at its widest possible representation, and each failed four times
//! with an error message exactly `LAST_ERROR_MAX_BYTES` (256) bytes long and needing a JSON escape on
//! every byte, so it is stored whole rather than losing bytes to the cut mark (re-review round 2,
//! N4: a cut message is smaller, not larger): measured, it is 56.5 MB, 84% of the 64 MiB
//! `QUEUE_FILE_MAX` (pinned by a test constructing this true maximum), so a save can never be locked
//! out.
//!
//! ## The failure journal (task 15, ruling 15-R3b)
//!
//! Most progress can be re-derived after a crash -- an in-flight launch is refunded on open, and a
//! completion is re-established from its cache entry -- so the whole-file snapshot above is written
//! only at checkpoints. A *failure* cannot be re-derived: its attempt count and retry deadline
//! exist nowhere else. So each failure outcome is appended at once, as one small record, to the
//! failure journal `queue.journal` beside `queue.json` (`journal_path`, `Queue::journal_outcome`),
//! and the journal is folded into the next snapshot: `save` publishes a snapshot whose
//! `journal_seq` names the last record it holds, and only once that snapshot is durable empties
//! the journal (through `write_atomic`). Opening the queue replays, over the snapshot, every record
//! after the snapshot's `journal_seq`, so a crash between the snapshot and the emptying -- the
//! journal still holding records the snapshot already has -- never rolls a later change back.
//!
//! A record is one line, `<sha256 hex of the JSON> <JSON>\n`, carrying a strictly increasing
//! sequence number, the generation, the slot, the game the outcome was charged to (identity,
//! board and exact SPR) or none for the slot's own record, and that record's progress after the
//! outcome: its status, attempt count, absolute UTC retry deadline and cause. Replaying one sets
//! that state -- declaring its generation and binding the slot to its game first if the snapshot
//! does not already -- so replay is idempotent. Each append is a single write followed by
//! `sync_data`, record by record: a crash can tear at most the last line. The journal holds at most
//! `JOURNAL_MAX_BYTES` (read bounded to it; the scheduler folds a full journal into a snapshot
//! instead of growing it) and a line at most `JOURNAL_RECORD_MAX_BYTES`.
//!
//! Replay stops at the last complete, valid record: a torn line, a checksum mismatch, unparsable
//! JSON, a sequence number that does not increase, a record that does not apply to the queue (an
//! unknown slot, another board, inconsistent progress), or bytes past the size bound end it, and
//! that record and everything after it are ignored -- reported once on stderr with the byte
//! offset and the reason, never a panic -- and cut from the file before the next append, so later
//! records are never stranded behind a damaged one. If the replayed queue failed `validate` as a
//! whole, the journal would be ignored entirely in the same way.

use super::scenarios::{canonical_flops_ordered, scenarios, Scenario, CANONICAL_FLOP_COUNT};
use crate::key::{spr_bucket, KeyFields, Rational};
use crate::CacheError;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// `QueueFile::version`: the queue's own format, independent of the cache's `schema_version` 3.
/// Version 2 binds slots to normalized game identities (fix round 1, review R1).
pub const QUEUE_VERSION: u16 = 2;
/// The largest `queue.json` that is read or published, in bytes.
pub const QUEUE_FILE_MAX: u64 = 64 * 1024 * 1024;
/// The initial attempt plus three retries (spec section 10.5: "retries 3 with 30 s backoff").
pub const MAX_ATTEMPTS: u8 = 4;
/// The retry backoff, in milliseconds (spec section 10.5).
pub const RETRY_BACKOFF_MS: u64 = 30_000;
/// The largest retry deadline a failure may record, on either clock: `i64::MAX` milliseconds, the
/// same ceiling as `crate::quota::LAST_HIT_MAX`. Leaves `u64::MAX` free as the terminal marker.
pub const RETRY_DEADLINE_MAX_MS: u64 = i64::MAX as u64;
/// The largest stored failure message, in UTF-8 bytes (`record_failure` cuts longer ones). With it
/// the largest possible file stays inside `QUEUE_FILE_MAX` (module doc, "The file").
pub const LAST_ERROR_MAX_BYTES: usize = 256;
/// The cursor once the sweep has passed the last slot: tier 3, one past its last canonical flop.
pub const CURSOR_END: [usize; 3] = [2, CANONICAL_FLOP_COUNT, 0];
/// The generation of a queue no caller has declared one for (`Queue::set_generation`).
pub const INITIAL_GENERATION: [u8; 32] = [0; 32];

/// The largest failure journal that is read or grown, in bytes (module doc, "The failure
/// journal"). About 8,000 records: `Queue::journal_outcome` reports a journal that would outgrow it
/// as full, and the scheduler folds it into a snapshot instead.
pub const JOURNAL_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// The longest journal line accepted, in bytes: a record with every field at its widest (a
/// 256-byte error needing a JSON escape on every byte, the widest exact SPR) stays well inside it.
pub const JOURNAL_RECORD_MAX_BYTES: usize = 2_048;

/// `retry_after_unix_ms` of a terminal failure: never retried.
const NEVER: u64 = u64::MAX;
/// Appended to a failure message that `record_failure` had to cut.
const CUT_MARK: &str = "...";
/// Domain separation for slot identities, so they can never collide with another sha256 input.
const IDENTITY_DOMAIN: &[u8] = b"pokerai/presolver-queue/item\0";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskStatus {
    Pending,
    Done,
    Failed { n: u8 },
}

/// A slot: one (tier, canonical flop, scenario) of the frozen enumeration. `identity`, `scenario`,
/// `board` and the nominal `spr` are frozen. The progress fields (`status` through `last_error`)
/// are the slot's own record while it is not bound under the current generation -- its preparation
/// attempts -- and untouched while it is; a view from `Queue::item` or `Queue::next_pending`
/// carries the effective progress instead: its game's when it is bound.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueItem {
    pub identity: [u8; 32],
    pub scenario: Scenario,
    pub board: Vec<proto::Card>,
    /// The scenario's nominal SPR `depth_bb : 1`, part of the slot identity. The exact SPR is the
    /// bound game's (`GameProgress::spr`).
    pub spr: Rational,
    pub status: TaskStatus,
    pub retry_after_unix_ms: u64,
    pub attempts: u8,
    pub last_error: Option<String>,
    /// The normalized game identity preparation last bound this slot to, and the generation it did
    /// so under; `None` before the first successful preparation.
    pub game: Option<GameBinding>,
}

impl QueueItem {
    pub fn identity_hex(&self) -> String {
        hex(&self.identity)
    }
}

/// A slot's index into `QueueFile::games`: the normalized game identity
/// (`KeyFields::scenario_identity` of the prepared key and exact SPR) and the source/config
/// generation the preparation ran under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameBinding {
    #[serde(with = "hex32")]
    pub generation: [u8; 32],
    #[serde(with = "hex32")]
    pub identity: [u8; 32],
}

impl GameBinding {
    /// The key of the bound game in `QueueFile::games`.
    pub fn identity_hex(&self) -> String {
        hex(&self.identity)
    }
}

/// The progress of one normalized game, shared by every slot bound to it: its canonical board, its
/// exact scenario SPR `eff : P`, and the same progress fields a slot's own record has.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameProgress {
    pub board: Vec<proto::Card>,
    pub spr: Rational,
    pub status: TaskStatus,
    pub retry_after_unix_ms: u64,
    pub attempts: u8,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueFile {
    pub version: u16,
    pub cursor: [usize; 3],
    pub paused: bool,
    /// The source/config generation the bindings are checked against (`Queue::set_generation`).
    #[serde(with = "hex32")]
    pub generation: [u8; 32],
    /// The slots, keyed by slot identity.
    #[serde(deserialize_with = "unique_map")]
    pub items: BTreeMap<String, QueueItem>,
    /// The games slots are bound to, keyed by normalized game identity.
    #[serde(deserialize_with = "unique_map")]
    pub games: BTreeMap<String, GameProgress>,
    /// The sequence number of the last failure-journal record this snapshot already holds (module
    /// doc, "The failure journal"): reopening replays only the records after it. A file written
    /// before the journal existed has none: 0.
    #[serde(default)]
    pub journal_seq: u64,
}

/// Decodes a map keyed by identity, refusing a key that occurs twice (JSON objects may repeat a
/// key, and a plain map decode would silently keep the last one).
fn unique_map<'de, D, V>(d: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: serde::Deserialize<'de>,
{
    struct Entries<V>(std::marker::PhantomData<V>);
    impl<'de, V: serde::Deserialize<'de>> serde::de::Visitor<'de> for Entries<V> {
        type Value = BTreeMap<String, V>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a map keyed by identity")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut entries = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, V>()? {
                if entries.contains_key(&key) {
                    return Err(serde::de::Error::custom(format!("duplicate queue key {key}")));
                }
                entries.insert(key, value);
            }
            Ok(entries)
        }
    }
    d.deserialize_map(Entries(std::marker::PhantomData))
}

/// A 32-byte identity as 64 lowercase hex digits, the only spelling accepted.
mod hex32 {
    pub fn serialize<S: serde::Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::hex(bytes))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let text = <String as serde::Deserialize>::deserialize(d)?;
        super::unhex(&text).ok_or_else(|| serde::de::Error::custom(format!("expected 64 lowercase hex digits, got {text:?}")))
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    let digits = text.as_bytes();
    if digits.len() != 64 {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut out = [0_u8; 32];
    for (byte, pair) in out.iter_mut().zip(digits.chunks_exact(2)) {
        *byte = nibble(pair[0])? << 4 | nibble(pair[1])?;
    }
    Some(out)
}

/// Reconciliation's status rule: a validated entry at target makes a game `Done`; without one,
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

/// `queue.journal` in the cache root, beside `queue.json`: the failure journal (module doc).
pub fn journal_path(cache_root: &Path) -> PathBuf {
    cache_root.join("queue.journal")
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
/// the key must be on the item's canonical flop, and, when `item` is bound under the current
/// generation (`item.game`), `key`/`spr` must normalize to that same identity (re-review round 2,
/// N1): a caller passing a mismatched key -- an incomplete generation fingerprint, or reconciling
/// before the generation is set -- can never read another identity's entry as this slot's
/// completion, even one that is itself genuinely valid and at target.
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
    if item.game.is_some_and(|binding| binding.identity != identity) {
        return false;
    }
    let cell = key.at_bucket(spr_bucket(spr)).digest();
    let Some(found) = crate::storage::read_cell(&crate::storage::entry_path(cache_root, cell)) else {
        return false;
    };
    found
        .entries
        .iter()
        .any(|e| e.key.scenario_identity(e.source.spr) == identity && crate::label::accuracy_ok(e.exploitability_over_P, target_bp))
}

// --- clocks ------------------------------------------------------------------------------------

/// The queue's two clocks (module doc, "Retry deadlines and clocks"). Every `now_ms` a caller
/// passes to the queue must be a reading of the same clock's `monotonic_ms` timeline.
pub trait QueueClock: Send {
    /// Milliseconds on a monotonic timeline: never decreases while the process runs, and has no
    /// relation to wall-clock time or to another process's timeline.
    fn monotonic_ms(&self) -> u64;
    /// UTC wall-clock time, milliseconds since the Unix epoch. May jump either way.
    fn unix_ms(&self) -> u64;
}

/// The production clock: `monotonic_ms()` for the monotonic timeline and `SystemTime` for UTC.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl QueueClock for SystemClock {
    fn monotonic_ms(&self) -> u64 {
        monotonic_ms()
    }

    /// `0` for a wall clock set before 1970, `u64::MAX` past the year 584 million: either is then
    /// refused loudly by `record_failure`'s ceiling rather than wrapped.
    fn unix_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
    }
}

/// `SystemClock`'s monotonic timeline: milliseconds since a process-wide `Instant`, taken the first
/// time any caller asks. A scheduler that opens the queue with `Queue::open` passes `now_ms` from
/// this function.
///
/// # Panics
/// Past `u64::MAX` milliseconds of uptime (always-on; never reached).
pub fn monotonic_ms() -> u64 {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    u64::try_from(ORIGIN.get_or_init(std::time::Instant::now).elapsed().as_millis()).expect("monotonic_ms: uptime exceeds u64 milliseconds")
}

// --- the frozen enumeration --------------------------------------------------------------------

/// The one slot enumeration every valid `queue.json` must describe, computed once per process: the
/// slots in sweep order plus the index arithmetic between sweep positions, cursors and key order.
struct Frozen {
    /// Every slot, unbound, `Pending` and unlaunched, in sweep order.
    items: Vec<QueueItem>,
    /// `items[i].identity_hex()`.
    keys: Vec<String>,
    /// For the `k`-th key in ascending order -- `QueueFile::items` iteration order -- its sweep index.
    by_key: Vec<usize>,
    /// Scenarios per tier.
    tier_len: [usize; 3],
    /// The sweep index of each tier's first slot.
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
            assert_ne!(keys[pair[0]], keys[pair[1]], "two queue slots share an identity");
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
    assert!(index <= frozen.items.len(), "cursor_at: sweep index {index} is past the {} queue slots", frozen.items.len());
    if index == frozen.items.len() {
        return CURSOR_END;
    }
    let tier = (0..3).rev().find(|&t| index >= frozen.tier_base[t]).expect("tier_base[0] is 0");
    let within = index - frozen.tier_base[tier];
    [tier, within / frozen.tier_len[tier], within % frozen.tier_len[tier]]
}

// --- per-record progress rules -------------------------------------------------------------------

/// The progress fields a slot's own record and a game record share (`QueueItem` keeps them flat, as
/// the brief laid it out).
#[derive(Clone, Copy)]
struct Progress<'a> {
    status: &'a TaskStatus,
    attempts: u8,
    retry_after_unix_ms: u64,
    last_error: Option<&'a str>,
}

struct ProgressMut<'a> {
    status: &'a mut TaskStatus,
    attempts: &'a mut u8,
    retry_after_unix_ms: &'a mut u64,
    last_error: &'a mut Option<String>,
}

impl ProgressMut<'_> {
    fn get(&self) -> Progress<'_> {
        Progress { status: self.status, attempts: *self.attempts, retry_after_unix_ms: *self.retry_after_unix_ms, last_error: self.last_error.as_deref() }
    }

    /// Back to untouched: `Pending`, unlaunched, no deadline, no error.
    fn reset(&mut self) {
        *self.status = TaskStatus::Pending;
        *self.attempts = 0;
        *self.retry_after_unix_ms = 0;
        *self.last_error = None;
    }
}

impl QueueItem {
    fn progress(&self) -> Progress<'_> {
        Progress { status: &self.status, attempts: self.attempts, retry_after_unix_ms: self.retry_after_unix_ms, last_error: self.last_error.as_deref() }
    }

    fn progress_mut(&mut self) -> ProgressMut<'_> {
        ProgressMut { status: &mut self.status, attempts: &mut self.attempts, retry_after_unix_ms: &mut self.retry_after_unix_ms, last_error: &mut self.last_error }
    }
}

impl GameProgress {
    fn progress(&self) -> Progress<'_> {
        Progress { status: &self.status, attempts: self.attempts, retry_after_unix_ms: self.retry_after_unix_ms, last_error: self.last_error.as_deref() }
    }

    fn progress_mut(&mut self) -> ProgressMut<'_> {
        ProgressMut { status: &mut self.status, attempts: &mut self.attempts, retry_after_unix_ms: &mut self.retry_after_unix_ms, last_error: &mut self.last_error }
    }
}

/// `Pending`, never launched, no deadline, no error: a record the sweep has not started.
fn untouched(p: Progress) -> bool {
    *p.status == TaskStatus::Pending && p.attempts == 0 && p.retry_after_unix_ms == 0 && p.last_error.is_none()
}

/// Launchable now or once its retry is due: a fresh `Pending`, or a non-terminal failure that is
/// not already relaunched.
fn launchable(p: Progress) -> bool {
    match *p.status {
        TaskStatus::Pending => p.attempts == 0,
        TaskStatus::Failed { n } => n < MAX_ATTEMPTS && p.attempts == n,
        TaskStatus::Done => false,
    }
}

/// Charged an attempt that has no outcome yet.
fn launched(p: Progress) -> bool {
    match *p.status {
        TaskStatus::Pending => p.attempts == 1,
        TaskStatus::Failed { n } => u16::from(p.attempts) == u16::from(n) + 1,
        TaskStatus::Done => false,
    }
}

/// An in-process (monotonic) retry deadline is due at `now_ms`: monotone, `deadline <= now_ms`, and
/// nothing else (re-review round 2, N2: a deadline more than one backoff ahead of `now_ms` is never
/// silently treated as due). Every such deadline is set at most one backoff ahead of the monotonic
/// reading it was anchored at (`record_failure`'s own `now_ms`, or `open_with_clock`'s restored
/// anchor), so `now_ms` landing more than one backoff behind a deadline can only mean the caller's
/// own clock ran backwards relative to that anchor -- a programming error the clock seam (module
/// doc) makes possible, never a case this queue may treat as a due retry.
///
/// # Panics
/// If `now_ms` is more than `RETRY_BACKOFF_MS` behind `deadline` (always-on): the caller's clock ran
/// backwards relative to the anchor that set this deadline.
fn retry_due(deadline: u64, now_ms: u64) -> bool {
    if deadline <= now_ms {
        return true;
    }
    assert!(
        deadline - now_ms <= RETRY_BACKOFF_MS,
        "retry_due: now_ms ran backwards relative to the persisted anchor: now_ms {now_ms} ms is {} ms behind deadline {deadline} ms, more than one RETRY_BACKOFF_MS ({RETRY_BACKOFF_MS} ms) -- a caller clock violation, never treated as a silent retry",
        deadline - now_ms
    );
    false
}

/// The retry deadline `delay` after the `clock` reading `now_ms`, computed wide.
///
/// # Panics
/// If it exceeds `RETRY_DEADLINE_MAX_MS` (always-on): refused, never saturated.
fn retry_deadline(clock: &str, now_ms: u64, delay: std::time::Duration) -> u64 {
    let deadline = u128::from(now_ms) + delay.as_millis();
    assert!(
        deadline <= u128::from(RETRY_DEADLINE_MAX_MS),
        "record_failure: {clock} retry deadline {now_ms} ms + {} ms exceeds RETRY_DEADLINE_MAX_MS ({RETRY_DEADLINE_MAX_MS}); the clock reading is unusable",
        delay.as_millis()
    );
    u64::try_from(deadline).expect("bounded by RETRY_DEADLINE_MAX_MS")
}

/// The in-process deadline a persisted retry gets on reopening: the UTC backoff that remains at the
/// wall-clock reading `unix`, clamped to `0..=RETRY_BACKOFF_MS`, after the monotonic reading `mono`.
/// `None` for a record with no retry to wait for.
///
/// # Panics
/// If `mono` is within one backoff of `u64::MAX` (always-on; the clock reading is unusable).
fn restored_wait(p: Progress, mono: u64, unix: u64) -> Option<u64> {
    let TaskStatus::Failed { n } = *p.status else { return None };
    if n >= MAX_ATTEMPTS {
        return None;
    }
    let remaining = p.retry_after_unix_ms.saturating_sub(unix).min(RETRY_BACKOFF_MS);
    Some(mono.checked_add(remaining).unwrap_or_else(|| panic!("open: monotonic reading {mono} ms + {remaining} ms overflows; the clock reading is unusable")))
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

/// The progress rules every record must satisfy, at rest or with one attempt in flight: `Pending`
/// carries no deadline or error and at most the one in-flight attempt; `Done` carries neither and
/// at most `MAX_ATTEMPTS`; `Failed{n}` has `1 <= n <= MAX_ATTEMPTS`, a bounded one-line error,
/// `attempts` equal to `n` (or `n + 1` while a retry is in flight), and a deadline -- a real one for
/// `n < MAX_ATTEMPTS`, `u64::MAX` for the terminal `n == MAX_ATTEMPTS`.
fn check_progress(p: Progress) -> Result<(), &'static str> {
    let no_extras = p.retry_after_unix_ms == 0 && p.last_error.is_none();
    match *p.status {
        TaskStatus::Pending if p.attempts <= 1 && no_extras => Ok(()),
        TaskStatus::Pending => Err("pending queue record progress"),
        TaskStatus::Done if p.attempts <= MAX_ATTEMPTS && no_extras => Ok(()),
        TaskStatus::Done => Err("done queue record progress"),
        TaskStatus::Failed { n } => {
            let error = p.last_error.is_some_and(|e| e.len() <= LAST_ERROR_MAX_BYTES && !e.chars().any(char::is_control));
            let attempts = p.attempts == n || (n < MAX_ATTEMPTS && launched(p));
            let deadline = if n == MAX_ATTEMPTS {
                p.retry_after_unix_ms == NEVER
            } else {
                (RETRY_BACKOFF_MS..=RETRY_DEADLINE_MAX_MS).contains(&p.retry_after_unix_ms)
            };
            if (1..=MAX_ATTEMPTS).contains(&n) && error && attempts && deadline {
                Ok(())
            } else {
                Err("failed queue record progress")
            }
        }
    }
}

/// Whether `file` is exactly the frozen slot enumeration with consistent progress and bindings:
/// the version; a cursor that names a sweep position or `CURSOR_END`; every frozen slot present
/// once under its own identity with its frozen scenario, board and SPR; every unbound slot's own
/// progress valid; every bound slot untouched and bound to an existing game on its own board; every
/// game bound by some slot, with a positive SPR and valid progress; a game in flight only behind a
/// binding of the current generation; and at most one launch in flight overall. `save_queue` and
/// `Queue::open` both apply it, so the two serde directions cannot disagree.
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
    // Every game some slot is bound to, and whether any of those bindings is current.
    let mut bound: BTreeMap<String, bool> = BTreeMap::new();
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
        match &item.game {
            None => {
                check_progress(item.progress()).map_err(CacheError::Invalid)?;
                in_flight += usize::from(launched(item.progress()));
            }
            Some(binding) => {
                if !untouched(item.progress()) {
                    return invalid("bound queue item keeps progress of its own");
                }
                let game = binding.identity_hex();
                match file.games.get(&game) {
                    None => return invalid("queue item bound to a missing game"),
                    Some(record) if record.board != item.board => return invalid("queue game on another board"),
                    Some(_) => {}
                }
                *bound.entry(game).or_insert(false) |= binding.generation == file.generation;
            }
        }
    }
    for (game, record) in &file.games {
        let Some(&current) = bound.get(game) else {
            return invalid("queue game no item is bound to");
        };
        if record.spr.num() == 0 {
            return invalid("queue game spr");
        }
        check_progress(record.progress()).map_err(CacheError::Invalid)?;
        if launched(record.progress()) {
            if !current {
                return invalid("queue game in flight without a current binding");
            }
            in_flight += 1;
        }
    }
    if in_flight > 1 {
        return invalid("more than one queue launch in flight");
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

// --- the failure journal (ruling 15-R3b) ----------------------------------------------------------

/// What `Queue::journal_outcome` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalAppend {
    /// The record is on disk: `bytes` were appended and synced.
    Appended { bytes: u64 },
    /// The record would take the journal past `JOURNAL_MAX_BYTES`, so nothing was written: the
    /// caller folds the journal into a snapshot (`Queue::save`), which holds the outcome too.
    Full,
}

/// One journal record (module doc, "The failure journal"): the state one outcome left `slot`'s
/// record in, charged to `game` or, when it is `None`, to the slot's own record.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalRecord {
    seq: u64,
    #[serde(with = "hex32")]
    generation: [u8; 32],
    slot: String,
    game: Option<JournalGame>,
    status: TaskStatus,
    attempts: u8,
    retry_after_unix_ms: u64,
    last_error: Option<String>,
}

/// The game a journaled outcome was charged to: enough to bind the slot to it and to create its
/// record if the snapshot predates the binding.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalGame {
    #[serde(with = "hex32")]
    identity: [u8; 32],
    board: Vec<proto::Card>,
    spr: Rational,
}

impl JournalRecord {
    fn progress(&self) -> Progress<'_> {
        Progress { status: &self.status, attempts: self.attempts, retry_after_unix_ms: self.retry_after_unix_ms, last_error: self.last_error.as_deref() }
    }

    /// `<sha256 hex of the JSON> <JSON>\n`.
    fn line(&self) -> Result<Vec<u8>, CacheError> {
        let json = serde_json::to_vec(self)?;
        let digest: [u8; 32] = Sha256::digest(&json).into();
        let mut line = hex(&digest).into_bytes();
        line.push(b' ');
        line.extend_from_slice(&json);
        line.push(b'\n');
        Ok(line)
    }

    /// The record on a line (without its line end), or why it is none.
    fn parse(line: &[u8]) -> Result<JournalRecord, String> {
        if line.len() >= JOURNAL_RECORD_MAX_BYTES {
            return Err(format!("a {}-byte line, past JOURNAL_RECORD_MAX_BYTES", line.len() + 1));
        }
        let (Some(sum), Some(b' '), Some(json)) = (line.get(..64), line.get(64), line.get(65..)) else {
            return Err("not a checksummed record".into());
        };
        let digest: [u8; 32] = Sha256::digest(json).into();
        if sum != hex(&digest).as_bytes() {
            return Err("checksum mismatch".into());
        }
        serde_json::from_slice(json).map_err(|e| format!("unparsable record: {e}"))
    }
}

/// The journal's complete, checksummed records with strictly increasing sequence numbers, each
/// with its byte range, in file order; and where and why reading stopped short of the end, if it
/// did (module doc: replay stops at the last complete, valid record). An absent journal is empty.
fn read_journal(path: &Path) -> (Vec<(JournalRecord, u64, u64)>, Option<(u64, String)>) {
    let mut bytes = Vec::new();
    match std::fs::File::open(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), None),
        Err(e) => return (Vec::new(), Some((0, format!("unreadable: {e}")))),
        Ok(f) => {
            if let Err(e) = f.take(JOURNAL_MAX_BYTES + 1).read_to_end(&mut bytes) {
                return (Vec::new(), Some((0, format!("unreadable: {e}"))));
            }
        }
    }
    let past_bound = bytes.len() as u64 > JOURNAL_MAX_BYTES;
    bytes.truncate(usize::try_from(JOURNAL_MAX_BYTES).expect("JOURNAL_MAX_BYTES fits usize"));
    let mut records: Vec<(JournalRecord, u64, u64)> = Vec::new();
    let mut at = 0_usize;
    while at < bytes.len() {
        let Some(len) = bytes[at..].iter().position(|&b| b == b'\n') else {
            let why = if past_bound { "past the JOURNAL_MAX_BYTES bound" } else { "a torn record without its line end" };
            return (records, Some((at as u64, why.into())));
        };
        let record = match JournalRecord::parse(&bytes[at..at + len]) {
            Ok(record) => record,
            Err(why) => return (records, Some((at as u64, why))),
        };
        if records.last().is_some_and(|(previous, _, _)| record.seq <= previous.seq) {
            return (records, Some((at as u64, format!("sequence number {} does not increase", record.seq))));
        }
        let end = at + len + 1;
        records.push((record, at as u64, end as u64));
        at = end;
    }
    let stop = past_bound.then(|| (at as u64, "past the JOURNAL_MAX_BYTES bound".to_owned()));
    (records, stop)
}

/// Replays, over the snapshot `file`, the journal's records after `file.journal_seq`, stopping at
/// the first one that does not apply (module doc). Returns the queue file to open -- the snapshot
/// alone if the replayed file would not `validate` -- and the length of the journal's valid
/// prefix, where the next record goes. Reports a stop on stderr; never panics.
fn replay_journal(path: &Path, mut file: QueueFile) -> (QueueFile, u64) {
    let (records, mut stop) = read_journal(path);
    if records.is_empty() && stop.is_none() {
        return (file, 0);
    }
    let snapshot = file.clone();
    let mut refs: BTreeMap<String, u32> = BTreeMap::new();
    for binding in file.items.values().filter_map(|item| item.game.as_ref()) {
        *refs.entry(binding.identity_hex()).or_insert(0) += 1;
    }
    let base = file.journal_seq;
    let (mut last, mut valid, mut replayed) = (base, 0, 0_usize);
    for (record, start, end) in &records {
        if record.seq > base {
            if let Err(why) = apply_record(&mut file, &mut refs, record) {
                stop = Some((*start, format!("record {} does not apply to the queue: {why}", record.seq)));
                break;
            }
            replayed += 1;
        }
        last = last.max(record.seq);
        valid = *end;
    }
    file.journal_seq = last;
    if let Err(e) = validate(&file) {
        eprintln!(
            "presolver queue: replaying the journal {} left a queue that does not validate ({e}); the journal is ignored and the snapshot is used as saved",
            path.display()
        );
        return (QueueFile { journal_seq: last, ..snapshot }, 0);
    }
    if let Some((at, why)) = stop {
        eprintln!("presolver queue: journal {} ignored from byte {at} ({why}); {replayed} record(s) replayed before it", path.display());
    }
    (file, valid)
}

/// Sets the state `record` holds on `file` (module doc): declares its generation if the snapshot
/// predates it, binds the slot to the record's game -- creating that game's record, and dropping a
/// game no slot binds any more -- or drops the slot's binding when the outcome was the slot's own,
/// then sets that record's progress. Checks everything first: a record that cannot apply changes
/// nothing and says why.
fn apply_record(file: &mut QueueFile, refs: &mut BTreeMap<String, u32>, record: &JournalRecord) -> Result<(), &'static str> {
    let Some(item) = file.items.get(&record.slot) else { return Err("an unknown slot") };
    check_progress(record.progress())?;
    if launched(record.progress()) {
        return Err("an outcome still in flight");
    }
    if let Some(game) = &record.game {
        if game.board != item.board {
            return Err("a game on another board than its slot");
        }
        if game.spr.num() == 0 {
            return Err("a game with a zero SPR");
        }
        if file.games.get(&hex(&game.identity)).is_some_and(|known| known.board != game.board || known.spr != game.spr) {
            return Err("a game the queue knows with another board or SPR");
        }
    }
    if record.generation != file.generation {
        // `Queue::set_generation` ran after the snapshot and before this outcome.
        file.generation = record.generation;
        for item in file.items.values_mut() {
            if !untouched(item.progress()) {
                item.progress_mut().reset();
            }
        }
    }
    let previous = file.items[&record.slot].game.map(|binding| binding.identity);
    let target = match &record.game {
        Some(game) => {
            let slot = file.items.get_mut(&record.slot).expect("checked above");
            slot.progress_mut().reset();
            slot.game = Some(GameBinding { generation: record.generation, identity: game.identity });
            let key = hex(&game.identity);
            if previous != Some(game.identity) {
                *refs.entry(key.clone()).or_insert(0) += 1;
                file.games.entry(key.clone()).or_insert_with(|| GameProgress {
                    board: game.board.clone(),
                    spr: game.spr,
                    status: TaskStatus::Pending,
                    retry_after_unix_ms: 0,
                    attempts: 0,
                    last_error: None,
                });
            }
            Rec::Game(key)
        }
        None => {
            file.items.get_mut(&record.slot).expect("checked above").game = None;
            Rec::Slot(record.slot.clone())
        }
    };
    if let Some(previous) = previous.filter(|previous| record.game.as_ref().map(|game| game.identity) != Some(*previous)) {
        let game = hex(&previous);
        if let Some(count) = refs.get_mut(&game) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                refs.remove(&game);
                file.games.remove(&game);
            }
        }
    }
    let p = match &target {
        Rec::Slot(id) => file.items.get_mut(id).expect("checked above").progress_mut(),
        Rec::Game(game) => file.games.get_mut(game).expect("bound above").progress_mut(),
    };
    *p.status = record.status.clone();
    *p.attempts = record.attempts;
    *p.retry_after_unix_ms = record.retry_after_unix_ms;
    *p.last_error = record.last_error.clone();
    Ok(())
}

/// Writes `line` at byte `at` of the journal -- first cutting whatever follows the valid prefix,
/// a torn or ignored tail -- in one write, then syncs it.
fn append_at(path: &Path, at: u64, line: &[u8]) -> Result<(), CacheError> {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(path)?;
    if f.metadata()?.len() != at {
        f.set_len(at)?;
    }
    f.seek(SeekFrom::Start(at))?;
    f.write_all(line)?;
    f.sync_data()?;
    Ok(())
}

// --- the queue ---------------------------------------------------------------------------------

/// A progress record: a slot's own (keyed by slot identity) or a game's (keyed by normalized game
/// identity).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rec {
    Slot(String),
    Game(String),
}

pub struct Queue {
    path: PathBuf,
    /// The failure journal beside `path` (module doc, "The failure journal").
    journal: PathBuf,
    /// The length of the journal's valid prefix: where the next record is written, cutting any
    /// torn or ignored tail first. Zero once a snapshot emptied it.
    journal_valid: AtomicU64,
    file: QueueFile,
    clock: Box<dyn QueueClock>,
    /// The in-process retry deadline, on the clock's monotonic timeline, of every record failed with
    /// a retry still to come. Runtime-only: the file holds the UTC deadline, converted back into a
    /// monotonic one by `open_with_clock`.
    waits: BTreeMap<Rec, u64>,
    /// How many slots (current or stale bindings) are bound to each game in `file.games`.
    refs: BTreeMap<String, u32>,
    /// The launched slot whose outcome is not recorded yet, and the record its attempt was charged
    /// to. Runtime-only; on disk the record is recognizable by its unresolved attempt and is
    /// refunded on open.
    in_flight: Option<(String, Rec)>,
}

impl Queue {
    /// `open_with_clock` on the `SystemClock`: callers pass `now_ms` from `monotonic_ms()`.
    pub fn open(cache_root: PathBuf) -> Result<Queue, CacheError> {
        Self::open_with_clock(cache_root, Box::new(SystemClock))
    }

    /// Bounded read, explicit version, validated cursor/slots/bindings/games/progress, no duplicate
    /// keys, no unknown fields (`validate`). A corrupt or absent file is rebuilt deterministically
    /// from `scenarios()` and `canonical_flops_ordered()`; existing valid cache cells still
    /// establish completion through `bind` and `reconcile`. A launch a dead process left in flight
    /// gets its attempt back. Before any of that, the failure journal's records after the
    /// snapshot's `journal_seq` are replayed over it (module doc, "The failure journal"). Each
    /// persisted UTC retry deadline is converted, once, into the backoff that remains -- clamped to
    /// `0..=RETRY_BACKOFF_MS` -- anchored at `clock`'s monotonic reading now (module doc, "Retry
    /// deadlines and clocks"). Never fails today: every unusable file is replaced by the rebuild,
    /// and a damaged journal is replayed up to its last complete, valid record.
    pub fn open_with_clock(cache_root: PathBuf, clock: Box<dyn QueueClock>) -> Result<Queue, CacheError> {
        let path = queue_path(&cache_root);
        let journal = journal_path(&cache_root);
        let file = match load(&path) {
            Ok(Some(file)) => file,
            Ok(None) => Self::rebuild(),
            Err(why) => {
                eprintln!("presolver queue: {} rejected ({why}); rebuilt from the frozen enumeration", path.display());
                Self::rebuild()
            }
        };
        let (file, valid) = replay_journal(&journal, file);
        let mut queue = Queue {
            path,
            journal,
            journal_valid: AtomicU64::new(valid),
            file,
            clock,
            waits: BTreeMap::new(),
            refs: BTreeMap::new(),
            in_flight: None,
        };
        let refund = |p: ProgressMut| {
            if launched(p.get()) {
                *p.attempts = p.attempts.checked_sub(1).expect("a launched record has an attempt");
            }
        };
        queue.file.items.values_mut().for_each(|item| refund(item.progress_mut()));
        queue.file.games.values_mut().for_each(|game| refund(game.progress_mut()));
        for binding in queue.file.items.values().filter_map(|item| item.game.as_ref()) {
            *queue.refs.entry(binding.identity_hex()).or_insert(0) += 1;
        }
        let (mono, unix) = (queue.clock.monotonic_ms(), queue.clock.unix_ms());
        for (key, item) in &queue.file.items {
            if let Some(deadline) = restored_wait(item.progress(), mono, unix) {
                queue.waits.insert(Rec::Slot(key.clone()), deadline);
            }
        }
        for (key, game) in &queue.file.games {
            if let Some(deadline) = restored_wait(game.progress(), mono, unix) {
                queue.waits.insert(Rec::Game(key.clone()), deadline);
            }
        }
        Ok(queue)
    }

    /// Deterministic rebuild in `(tier, flop, scenario)` order. Boards come from Task 13's
    /// cached canonical order, so the same install always produces the same slot set.
    fn rebuild() -> QueueFile {
        let frozen = frozen();
        let items = frozen.keys.iter().cloned().zip(frozen.items.iter().cloned()).collect();
        QueueFile {
            version: QUEUE_VERSION,
            cursor: [0, 0, 0],
            paused: false,
            generation: INITIAL_GENERATION,
            items,
            games: BTreeMap::new(),
            journal_seq: 0,
        }
    }

    /// Every slot, unbound, `Pending` and unlaunched, in sweep order: tier, then canonical flop,
    /// then the tier's scenarios in frozen order.
    fn all_items() -> Vec<QueueItem> {
        let scenarios = scenarios();
        let boards = canonical_flops_ordered();
        let mut out = Vec::with_capacity(scenarios.len() * boards.len());
        for tier in 1_u8..=3 {
            for board in boards {
                for scenario in scenarios.iter().filter(|s| s.tier == tier) {
                    // The exact SPR is known only at preparation time (task 16) and belongs to the
                    // bound game; the slot keeps the nominal depth ratio, part of its identity.
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
                        game: None,
                    });
                }
            }
        }
        out
    }

    /// The slot's deterministic identity (module doc): sha256 over a domain tag, the queue
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

    /// The source/config generation bindings are checked against.
    pub fn generation(&self) -> [u8; 32] {
        self.file.generation
    }

    /// A reading of the queue clock's monotonic timeline -- the one timeline every `now_ms` passed
    /// to this queue must come from (module doc, "Retry deadlines and clocks"). The scheduler (task
    /// 15, ruling S1) takes all its time from here, so it can never drift onto another clock.
    pub fn now_ms(&self) -> u64 {
        self.clock.monotonic_ms()
    }

    /// Every slot identity (`QueueItem::identity_hex`) in sweep order, the order `next_pending`,
    /// `advance_cursor` and `reconcile` walk. The scheduler's chunked reconciliation (task 15) walks
    /// it too, a few games per iteration, through `item` and `reconcile_item`.
    pub fn sweep_order(&self) -> &'static [String] {
        &frozen().keys
    }

    /// Publishes the snapshot (`save_queue`), which holds every journal record so far (its
    /// `journal_seq` names the last), and only then empties the failure journal through
    /// `write_atomic` (module doc, "The failure journal"). Failing to empty it is reported and
    /// harmless: the records left are the snapshot's own, never replayed again, and the next
    /// append goes after them.
    ///
    /// # Errors
    /// Whatever `save_queue` returns; the journal is then left as it was.
    pub fn save(&self) -> Result<(), CacheError> {
        save_queue(&self.path, &self.file)?;
        if std::fs::metadata(&self.journal).map_or(0, |m| m.len()) == 0 {
            self.journal_valid.store(0, Ordering::SeqCst);
            return Ok(());
        }
        match crate::storage::write_atomic(&self.journal, b"") {
            Ok(()) => self.journal_valid.store(0, Ordering::SeqCst),
            Err(e) => eprintln!(
                "presolver queue: emptying the journal {} after a snapshot failed ({e}); its records are already in the snapshot and are not replayed again",
                self.journal.display()
            ),
        }
        Ok(())
    }

    /// Appends, and syncs, one failure-journal record holding slot `id`'s record as it stands now
    /// -- its game's when it is bound under the current generation, else its own -- the moment a
    /// failure outcome is recorded (ruling 15-R3b; module doc, "The failure journal"). A record
    /// that would take the journal past `JOURNAL_MAX_BYTES` is not written: `Full` asks the caller
    /// to fold the journal into a snapshot instead.
    ///
    /// # Errors
    /// Any I/O error: nothing counts as appended, and a partly written line is cut before the next
    /// append.
    ///
    /// # Panics
    /// If `id` is unknown or a launch is in flight (always-on): an outcome is journaled once it is
    /// recorded, never while a job runs.
    pub fn journal_outcome(&mut self, id: &str) -> Result<JournalAppend, CacheError> {
        if let Some((running, _)) = &self.in_flight {
            panic!("journal_outcome({id}): {running} is in flight; journal an outcome once it is recorded");
        }
        let item = self.slot(id, "journal_outcome");
        let rec = self.rec_of(id);
        let game = match &rec {
            Rec::Game(key) => {
                let record = &self.file.games[key];
                let identity = item.game.expect("a slot charged to a game is bound to it").identity;
                Some(JournalGame { identity, board: record.board.clone(), spr: record.spr })
            }
            Rec::Slot(_) => None,
        };
        let p = self.progress(&rec);
        let record = JournalRecord {
            seq: self.file.journal_seq + 1,
            generation: self.file.generation,
            slot: id.to_owned(),
            game,
            status: p.status.clone(),
            attempts: p.attempts,
            retry_after_unix_ms: p.retry_after_unix_ms,
            last_error: p.last_error.map(str::to_owned),
        };
        let line = record.line()?;
        assert!(line.len() <= JOURNAL_RECORD_MAX_BYTES, "journal_outcome({id}): a {}-byte record exceeds JOURNAL_RECORD_MAX_BYTES", line.len());
        let bytes = line.len() as u64;
        let at = self.journal_valid.load(Ordering::SeqCst);
        if at + bytes > JOURNAL_MAX_BYTES {
            return Ok(JournalAppend::Full);
        }
        append_at(&self.journal, at, &line)?;
        self.journal_valid.store(at + bytes, Ordering::SeqCst);
        self.file.journal_seq = record.seq;
        Ok(JournalAppend::Appended { bytes })
    }

    /// The slot `id` as a view carrying its effective progress (its game's when it is bound under
    /// the current generation), or `None` for an unknown identity.
    pub fn item(&self, id: &str) -> Option<QueueItem> {
        self.file.items.contains_key(id).then(|| self.view(id))
    }

    /// Declares the source/config generation (task 16: the fingerprint of the active bundle's and
    /// configuration's revisions). A change makes every binding stale -- each slot must be prepared
    /// and bound again, and counts as pending until it is -- while the games keep their records for
    /// re-preparation to find; and it clears every slot's own preparation progress, which belonged
    /// to the old generation. The same generation again changes nothing.
    ///
    /// # Panics
    /// If a launch is in flight (always-on): the caller cancels it (`record_cancel`) first.
    pub fn set_generation(&mut self, generation: [u8; 32]) {
        if let Some((running, _)) = &self.in_flight {
            panic!("set_generation: {running} is in flight; cancel it before the source/config generation changes");
        }
        if generation == self.file.generation {
            return;
        }
        self.file.generation = generation;
        for (key, item) in &mut self.file.items {
            if !untouched(item.progress()) {
                item.progress_mut().reset();
                self.waits.remove(&Rec::Slot(key.clone()));
            }
        }
    }

    /// Records what preparation resolved slot `id` to under the current generation: the prepared
    /// cache key and exact scenario SPR, whose `KeyFields::scenario_identity` is the normalized game
    /// identity. The slot becomes an index into that game's record -- created `Pending` if no slot
    /// is bound to it yet -- and shows the game's progress from now on; the slot's own preparation
    /// progress is cleared, and a game no slot is bound to any more is dropped. Returns the slot's
    /// view. `bind` decides nothing about completion: `reconcile_item` does, and the scheduler
    /// launches the slot only if `next_pending` still offers it.
    ///
    /// # Panics
    /// Always-on: if `id` is unknown or in flight; if the key is not a flop root on the slot's own
    /// canonical flop, or the SPR is zero; or if the identity already names a game with another
    /// board or SPR (a hash collision or a caller bug).
    pub fn bind(&mut self, id: &str, key: &KeyFields, spr: Rational) -> QueueItem {
        let item = self.slot(id, "bind");
        assert!(
            self.in_flight.as_ref().map(|(running, _)| running.as_str()) != Some(id),
            "bind({id}): the item is in flight; record its outcome before preparing it again"
        );
        assert!(key.root_street == proto::Street::Flop, "bind({id}): the prepared key is rooted on {:?}, not the flop", key.root_street);
        assert!(
            key.canonical_board == item.board,
            "bind({id}): the prepared key is on {:?}, not the item's canonical flop {:?}",
            key.canonical_board,
            item.board
        );
        assert!(spr.num() > 0, "bind({id}): the exact scenario SPR must be positive, got 0/{}", spr.den());
        let identity = key.scenario_identity(spr);
        let game = hex(&identity);
        if let Some(existing) = self.file.games.get(&game) {
            assert!(
                existing.board == item.board && existing.spr == spr,
                "bind({id}): normalized identity {game} already names a game on {:?} at SPR {:?}",
                existing.board,
                existing.spr
            );
        }
        let (board, previous) = (item.board.clone(), item.game.map(|binding| binding.identity));
        let generation = self.file.generation;
        let slot = self.file.items.get_mut(id).expect("checked above");
        slot.progress_mut().reset();
        slot.game = Some(GameBinding { generation, identity });
        self.waits.remove(&Rec::Slot(id.to_owned()));
        if previous != Some(identity) {
            if let Some(previous) = previous {
                self.release(&hex(&previous));
            }
            *self.refs.entry(game.clone()).or_insert(0) += 1;
            self.file.games.entry(game).or_insert_with(|| GameProgress {
                board,
                spr,
                status: TaskStatus::Pending,
                retry_after_unix_ms: 0,
                attempts: 0,
                last_error: None,
            });
        }
        self.view(id)
    }

    /// The earliest launchable slot in sweep order whose retry, if any, is due at `now_ms` (on the
    /// clock's monotonic timeline): a revisit before the cursor (an invalidated `Done`, a due retry,
    /// a slot to prepare again) first, else the slot at the cursor. `None` when nothing can run now.
    pub fn next_pending(&self, now_ms: u64) -> Option<QueueItem> {
        frozen().keys.iter().find(|key| self.ready(key, now_ms)).map(|key| self.view(key))
    }

    /// `next_pending`, passing over every slot `skip` names by `identity_hex`, with the chosen
    /// slot's sweep position. The scheduler (task 15, fix round 1 R4) skips the slots it holds back
    /// in memory after a completion whose entry did not read back durably.
    pub fn next_pending_at(&self, now_ms: u64, skip: &dyn Fn(&str) -> bool) -> Option<(usize, QueueItem)> {
        frozen().keys.iter().enumerate().find(|(_, key)| self.ready(key, now_ms) && !skip(key)).map(|(at, key)| (at, self.view(key)))
    }

    /// The completions at sweep positions `range` (clamped to the enumeration; empty when it is
    /// reversed), in sweep order: each position whose slot is `Done` under a binding of the current
    /// generation, with that game's normalized identity. The scheduler (task 15, fix round 1 R1)
    /// launches no candidate later in sweep order than a completion its reconciliation sweep has
    /// not verified yet.
    pub fn done_bindings(&self, range: std::ops::Range<usize>) -> impl Iterator<Item = (usize, [u8; 32])> + '_ {
        let keys = &frozen().keys;
        let end = range.end.min(keys.len());
        let start = range.start.min(end);
        keys[start..end].iter().enumerate().filter_map(move |(offset, key)| {
            let binding = self.current(&self.file.items[key.as_str()])?;
            (self.file.games[&binding.identity_hex()].status == TaskStatus::Done).then_some((start + offset, binding.identity))
        })
    }

    /// Steps the cursor over every slot whose progress is no longer fresh -- launched, failed or
    /// done -- so it rests on the next slot the sweep has not started, or on `CURSOR_END`. After a
    /// launch at the cursor it moves past that slot; after a revisit launch it stays.
    pub fn advance_cursor(&mut self) {
        let frozen = frozen();
        let mut at = sweep_index(self.file.cursor).expect("the cursor is validated");
        while at < frozen.items.len() && !untouched(self.effective(&self.file.items[frozen.keys[at].as_str()])) {
            at += 1;
        }
        self.file.cursor = cursor_at(at);
    }

    /// Persisted before launch: the status stays as it was and the record's `attempts` increments,
    /// so a restart resumes the work instead of losing it (and refunds the attempt). The attempt is
    /// charged to the slot's game when it is bound under the current generation, else to the slot's
    /// own record (a preparation or submission that failed), dropping a stale binding. In-flight
    /// state is runtime-only.
    ///
    /// # Panics
    /// If another launch is in flight, `id` is unknown, or its record is not launchable (done,
    /// terminal, or already launched through another slot) -- scheduler bugs, refused loudly
    /// (always-on).
    pub fn record_launch(&mut self, id: &str) {
        if let Some((running, _)) = &self.in_flight {
            panic!("record_launch({id}): {running} is still in flight; the queue runs one presolver job at a time");
        }
        self.slot(id, "record_launch");
        let rec = self.rec_of(id);
        let p = self.progress(&rec);
        assert!(launchable(p), "record_launch({id}): the item is not launchable (status {:?}, attempts {})", p.status, p.attempts);
        if matches!(rec, Rec::Slot(_)) {
            let stale = self.file.items.get_mut(id).expect("checked above").game.take();
            if let Some(stale) = stale {
                self.release(&stale.identity_hex());
            }
        }
        *self.progress_mut(&rec).attempts += 1;
        self.in_flight = Some((id.to_owned(), rec));
    }

    /// The in-flight job completed and its entry was verified by reading it back
    /// (`entry_verified`); a worker terminal or a writer receipt alone is not enough. The game is
    /// `Done` for every slot bound to it.
    ///
    /// # Panics
    /// If `id` is not the job in flight, or its launch was charged to no normalized game identity
    /// (always-on).
    pub fn record_done(&mut self, id: &str) {
        let rec = self.flight(id, "record_done");
        assert!(
            matches!(rec, Rec::Game(_)),
            "record_done({id}): completion is recorded against a normalized game identity, and this launch had none bound under the current generation; bind the prepared key and exact SPR before launching"
        );
        self.in_flight = None;
        self.waits.remove(&rec);
        let p = self.progress_mut(&rec);
        *p.status = TaskStatus::Done;
        *p.last_error = None;
        *p.retry_after_unix_ms = 0;
    }

    /// A live-work cancellation refunds the in-flight attempt and returns its record to the
    /// pre-launch state: `Pending`, or `Failed{n}` still due for its retry. It never burns a retry.
    /// The scheduler (task 15, fix round 1 R4) refunds a completed job whose entry did not read back
    /// durably the same way: the solve did not fail, so it consumes no attempt.
    ///
    /// # Panics
    /// If `id` is not the job in flight (always-on).
    pub fn record_cancel(&mut self, id: &str) {
        let rec = self.flight(id, "record_cancel");
        self.in_flight = None;
        let p = self.progress_mut(&rec);
        *p.attempts = p.attempts.checked_sub(1).expect("a record in flight has an attempt");
    }

    /// The in-flight attempt failed: its record is `Failed{n}` with `n` the attempts made so far,
    /// or never retried after the fourth failure. Otherwise the retry waits `RETRY_BACKOFF_MS` on
    /// the monotonic clock after `now_ms`, and the file records the absolute UTC deadline: the
    /// clock's wall-clock reading plus the backoff. `error` is stored on one line, bounded by
    /// `LAST_ERROR_MAX_BYTES`.
    ///
    /// # Panics
    /// If `id` is not the job in flight, or either deadline would exceed `RETRY_DEADLINE_MAX_MS`
    /// (always-on; nothing is recorded).
    pub fn record_failure(&mut self, id: &str, now_ms: u64, error: String) {
        let rec = self.flight(id, "record_failure");
        let n = self.progress(&rec).attempts;
        let (retry_after, wait) = match retry_delay(n) {
            Some(delay) => (retry_deadline("UTC", self.clock.unix_ms(), delay), Some(retry_deadline("monotonic", now_ms, delay))),
            None => (NEVER, None), // the fourth failure is terminal
        };
        self.in_flight = None;
        match wait {
            Some(wait) => self.waits.insert(rec.clone(), wait),
            None => self.waits.remove(&rec),
        };
        let p = self.progress_mut(&rec);
        *p.status = TaskStatus::Failed { n };
        *p.last_error = Some(bounded_error(&error));
        *p.retry_after_unix_ms = retry_after;
    }

    /// `valid` is "a validated at-target entry for this slot's normalized identity exists on disk"
    /// -- normally `entry_verified` with the prepared key. It is asked once per game some slot is
    /// bound to under the current generation, with the first such slot in sweep order, and the
    /// verdict is the game's: a game it holds for becomes `Done`, and a `Done` game it does not hold
    /// for goes back to a fresh `Pending` with a full budget (the entry was evicted, corrupted or
    /// built from other ranges). A slot with no current binding has no identity to decide, and the
    /// game in flight is skipped: its outcome belongs to the running job.
    pub fn reconcile(&mut self, valid: &dyn Fn(&QueueItem) -> bool) {
        let running = self.running_game();
        let mut seen = BTreeSet::new();
        let mut verdicts = Vec::new();
        for key in &frozen().keys {
            let Some(binding) = self.current(&self.file.items[key.as_str()]) else { continue };
            let game = binding.identity_hex();
            if running.as_ref() == Some(&game) || !seen.insert(game.clone()) {
                continue;
            }
            verdicts.push((game, valid(&self.view(key))));
        }
        for (game, verdict) in verdicts {
            self.decide(&game, verdict);
        }
    }

    /// `reconcile` for slot `id`'s game alone: `valid` is whether a validated at-target entry for
    /// its normalized identity exists. The scheduler calls it after `bind`, so an entry already on
    /// disk completes the game without a solve.
    ///
    /// # Panics
    /// If `id` is unknown, not bound under the current generation, or its game is in flight
    /// (always-on).
    pub fn reconcile_item(&mut self, id: &str, valid: bool) {
        let item = self.slot(id, "reconcile_item");
        let game = self
            .current(item)
            .unwrap_or_else(|| panic!("reconcile_item({id}): the item has no normalized identity bound under the current generation; bind it first"))
            .identity_hex();
        assert!(self.running_game().as_ref() != Some(&game), "reconcile_item({id}): its game is in flight; the running job's outcome decides it");
        self.decide(&game, valid);
    }

    /// `(pending, done, failed)` over every slot, each counted under its effective status; a record
    /// in flight counts under its status.
    pub fn status_counts(&self) -> (u32, u32, u32) {
        let mut counts = (0, 0, 0);
        for item in self.file.items.values() {
            match *self.effective(item).status {
                TaskStatus::Pending => counts.0 += 1,
                TaskStatus::Done => counts.1 += 1,
                TaskStatus::Failed { .. } => counts.2 += 1,
            }
        }
        counts
    }

    /// `(done, total)` slots per tier.
    pub fn tier_counts(&self) -> ([u32; 3], [u32; 3]) {
        let (mut done, mut total) = ([0; 3], [0; 3]);
        for item in self.file.items.values() {
            let t = usize::from(item.scenario.tier) - 1; // validated 1..=3
            total[t] += 1;
            if *self.effective(item).status == TaskStatus::Done {
                done[t] += 1;
            }
        }
        (done, total)
    }

    /// `status_counts` and `tier_counts` in one pass, except that a `Done` slot counts as done only
    /// when it is bound under the current generation and `verified` accepts its game's normalized
    /// identity, and as pending otherwise. The scheduler (task 15, fix round 1 R1) publishes a
    /// completion restored from the file only once its startup sweep has read the entry back.
    pub fn counts_verified(&self, verified: &dyn Fn(&[u8; 32]) -> bool) -> ((u32, u32, u32), ([u32; 3], [u32; 3])) {
        let (mut counts, mut done, mut total) = ((0, 0, 0), [0; 3], [0; 3]);
        for item in self.file.items.values() {
            let t = usize::from(item.scenario.tier) - 1; // validated 1..=3
            total[t] += 1;
            match *self.effective(item).status {
                TaskStatus::Done if self.current(item).is_some_and(|binding| verified(&binding.identity)) => {
                    counts.1 += 1;
                    done[t] += 1;
                }
                TaskStatus::Done | TaskStatus::Pending => counts.0 += 1,
                TaskStatus::Failed { .. } => counts.2 += 1,
            }
        }
        (counts, (done, total))
    }

    fn slot(&self, id: &str, op: &str) -> &QueueItem {
        self.file.items.get(id).unwrap_or_else(|| panic!("{op}: no queue item has identity {id}"))
    }

    /// The slot's binding if it was made under the current generation.
    fn current<'a>(&self, item: &'a QueueItem) -> Option<&'a GameBinding> {
        item.game.as_ref().filter(|binding| binding.generation == self.file.generation)
    }

    /// The record slot `id`'s progress lives in: its game's when it is bound under the current
    /// generation, else its own.
    fn rec_of(&self, id: &str) -> Rec {
        match self.current(&self.file.items[id]) {
            Some(binding) => Rec::Game(binding.identity_hex()),
            None => Rec::Slot(id.to_owned()),
        }
    }

    fn progress(&self, rec: &Rec) -> Progress<'_> {
        match rec {
            Rec::Slot(id) => self.file.items[id].progress(),
            Rec::Game(game) => self.file.games[game].progress(),
        }
    }

    fn progress_mut(&mut self, rec: &Rec) -> ProgressMut<'_> {
        match rec {
            Rec::Slot(id) => self.file.items.get_mut(id).expect("a slot record exists").progress_mut(),
            Rec::Game(game) => self.file.games.get_mut(game).expect("a bound game has a record").progress_mut(),
        }
    }

    /// `item`'s effective progress: its game's when it is bound under the current generation.
    fn effective<'a>(&'a self, item: &'a QueueItem) -> Progress<'a> {
        match self.current(item) {
            Some(binding) => self.file.games[&binding.identity_hex()].progress(),
            None => item.progress(),
        }
    }

    /// Slot `key` with its effective progress in place of its own.
    fn view(&self, key: &str) -> QueueItem {
        let item = &self.file.items[key];
        let mut view = item.clone();
        if let Some(binding) = self.current(item) {
            let game = &self.file.games[&binding.identity_hex()];
            view.status = game.status.clone();
            view.retry_after_unix_ms = game.retry_after_unix_ms;
            view.attempts = game.attempts;
            view.last_error = game.last_error.clone();
        }
        view
    }

    /// Slot `key` is launchable now: its record is, and any retry it waits for is due at `now_ms`.
    fn ready(&self, key: &str, now_ms: u64) -> bool {
        let rec = self.rec_of(key);
        let p = self.progress(&rec);
        launchable(p)
            && (!matches!(p.status, TaskStatus::Failed { .. })
                || retry_due(*self.waits.get(&rec).unwrap_or_else(|| panic!("{rec:?} waits for a retry with no in-process deadline")), now_ms))
    }

    /// The game whose launch is in flight, if the launch was charged to a game.
    fn running_game(&self) -> Option<String> {
        match &self.in_flight {
            Some((_, Rec::Game(game))) => Some(game.clone()),
            _ => None,
        }
    }

    /// Applies reconciliation's verdict to `game`'s record (`reconcile_status`); a demotion starts
    /// the game over with a full budget.
    fn decide(&mut self, game: &str, valid: bool) {
        let record = self.file.games.get_mut(game).expect("a bound game has a record");
        let mut status = record.status.clone();
        reconcile_status(&mut status, valid);
        if status == record.status {
            return;
        }
        if status == TaskStatus::Pending {
            record.attempts = 0;
        }
        record.status = status;
        record.last_error = None;
        record.retry_after_unix_ms = 0;
        self.waits.remove(&Rec::Game(game.to_owned()));
    }

    /// One slot no longer binds `game`; the game's record goes with its last binding.
    fn release(&mut self, game: &str) {
        let count = self.refs.get_mut(game).expect("a bound game is counted");
        *count -= 1;
        if *count == 0 {
            assert!(self.running_game().as_deref() != Some(game), "release: game {game} is in flight");
            self.refs.remove(game);
            self.file.games.remove(game);
            self.waits.remove(&Rec::Game(game.to_owned()));
        }
    }

    /// The record the in-flight launch of `id` was charged to, or a panic naming `op` if `id` is not
    /// the job in flight.
    fn flight(&self, id: &str, op: &str) -> Rec {
        match &self.in_flight {
            Some((running, rec)) if running == id => rec.clone(),
            other => panic!("{op}({id}): that item is not the job in flight ({:?})", other.as_ref().map(|(running, _)| running)),
        }
    }
}
