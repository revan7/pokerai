//! Replacement, accounting and eviction for the on-disk cell store (spec section 10.4:
//! "replacement keeps at most two entries per cell (closest SPR, best accuracy)... Default
//! `cache_quota_bytes = 10 GiB`; oldest `last_hit` eviction"). Three pure decision functions --
//! `entry_digest`, `retain_two`, `victims` -- the writer's in-memory `Index`, and the helpers the
//! single writer thread in `crate::lib` drives them with: `scan_index`, `sweep_temporaries`,
//! `store_entry`, `apply_touch`, `delete_cell` and `enforce_quota`.
//!
//! ## Accounting: one row per entry, a digest-to-cell map, measured bytes counted once
//!
//! `scan_index` rebuilds the whole `Index` from the files themselves on every open -- it is never
//! a second authoritative file that could disagree with the store (there is no index file at all).
//! The index is the brief's *entry index* plus the *entry-digest-to-cell map* beside it (fix round
//! 1, review R1):
//!
//! - one `IndexRow` per stored **entry**, keyed by that entry's canonical payload digest
//!   (`entry_digest`, a sha256), carrying that entry's **own** `last_hit`;
//! - a map from each entry's payload digest to the key of the cell that holds it.
//!
//! A cell's measured on-disk bytes are counted exactly once: they are split across its entries'
//! rows (`Index::set_cell`), so `victims`'s `sum(bytes)` over the rows is the store's true
//! measured footprint, and re-measured after every rewrite. Every total is accumulated in `u128`
//! and compared against the `u64` quota without clamping (standing rulings (a)/(c)).
//!
//! ## Eviction: globally oldest entry first
//!
//! `enforce_quota` hands the rows to `victims` -- exactly the brief's contract: order by
//! `(last_hit, key)`, oldest first -- takes its first answer, and evicts that one entry: its cell
//! (found through the digest-to-cell map) is rewritten with its surviving entry, or deleted once it
//! would hold none. It then re-measures and repeats until the measured footprint is back under
//! quota. Because every entry has its own row, a cell holding a very old and a very new entry
//! loses its old entry before any other cell loses a newer one; ranking whole cells by their
//! newest entry (the defect review R1 found) cannot happen.
//!
//! ## Failed deletions stay accounted (review R3)
//!
//! A file's bytes leave the index only once the file is *confirmed* gone. Every deletion -- an
//! eviction, `WriteCommand::Delete`, the best-effort cleanup of an unreadable cell -- is followed
//! by a re-stat (`settle`): `NotFound` drops the cell's rows, a file still present (Windows refuses
//! to delete a file another handle holds open without delete sharing) keeps them, re-measured. A
//! cell that cannot be deleted is skipped for the rest of that quota pass -- its bytes still
//! count, so the pass carries on with the next oldest entry -- and is retried on every later pass.
//! A corrupt or otherwise unusable file that `scan_index` cannot delete is accounted by a
//! placeholder row keyed by the cell key its path names, with `last_hit` 0, so it is the first
//! thing a later pass retries.
//!
//! ## `last_hit` domain (review R5, fix round 2 N1)
//!
//! The writer persists `max(proposed, previous + 1)` with a *checked* `+ 1` (`crate::lib`), so the
//! counter is strictly increasing and a fresh hit can never tie with, let alone fall behind, an
//! older one. `LAST_HIT_MAX` is validated in this same, wide form at every entry point -- store,
//! touch, scan -- rather than only at the writer thread's own gate: a caller's proposal above it is
//! refused (`crate::next_last_hit` returns `None`; `store_entry` and `apply_touch` refuse it again,
//! directly, so a caller reaching either function any other way is still safe), and so is a bump
//! that would land above it even though `previous` and `proposed` were each individually usable --
//! the re-review's own finding (N1): the old code let that bump through, the writer persisted the
//! result, and its own next scan then deleted the cell as unusable, taking a legitimate, newer
//! store down with it. With the bump validated the same way, the writer can never itself produce a
//! value above `LAST_HIT_MAX`; this leaves 2^63 increments of headroom below the ceiling, so it
//! cannot legitimately be exhausted. A row above `LAST_HIT_MAX` found on disk regardless (only
//! reachable now by tampering, never by the writer) is therefore corrupt in a narrower way than a
//! misfiled or unreadable cell: `scan_index` reports it and excludes just that row from the index,
//! never deleting the file over it -- a corrupt timestamp on one entry must not destroy the cell,
//! or any other entry sharing it, including the newest store.
//!
//! ## Replacement reference
//!
//! The spec leaves the reference point for "closest SPR" unspecified, so this task defines it
//! (plan 4 task 6): the cell's own geometric center, `1.02^spr_bucket`, with distance measured
//! relatively (`|SPR - center| / center`). This is a *storage-selection* rule only -- a lookup
//! always measures `delta` against the actual query SPR (`crate::lookup::compare`), never against
//! this center. Ties fall back to raw accuracy and then to the canonical payload digest, so
//! arrival order never decides which representative survives, and `entry_digest` excludes the
//! mutable `created`/`last_hit` timestamps so that merely *hitting* an entry cannot reorder
//! replacement.

use crate::entry::{validate_entry, CacheEntry};
use crate::storage::{encode, entry_path, read_cell, write_atomic, Cell};
use crate::CacheError;
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The largest `last_hit` a caller may propose or a stored entry may carry: `i64::MAX`, the
/// largest signed 64-bit millisecond timestamp. Anything above it is refused rather than clamped
/// (standing ruling (a)), which keeps 2^63 increments of headroom above every value the writer's
/// strictly increasing counter can start from, so its checked `+ 1` can never run out.
pub const LAST_HIT_MAX: u64 = i64::MAX as u64;

/// One stored entry's accounting row: its canonical payload digest (`entry_digest`, the row's
/// identity; `Index::cell_of` maps it to the cell holding the entry), its share of that cell's
/// measured on-disk bytes (a cell's rows sum to exactly its measured size), and the entry's own
/// `last_hit`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexRow {
    pub key: [u8; 32],
    pub bytes: u64,
    pub last_hit: u64,
}

/// The rows to evict, oldest `last_hit` first, so that the measured footprint of what remains is
/// at or under `quota`. Ties on `last_hit` are broken by key, so the choice is deterministic
/// rather than dependent on the order rows happen to sit in the index. Exactly at quota is not
/// over quota, so nothing is evicted.
pub fn victims(rows: &[IndexRow], quota: u64) -> Vec<[u8; 32]> {
    let mut order = rows.iter().collect::<Vec<_>>();
    order.sort_by_key(|r| (r.last_hit, r.key));
    let mut used: u128 = rows.iter().map(|r| r.bytes as u128).sum();
    let mut out = Vec::new();
    for r in order {
        if used <= quota as u128 {
            break;
        }
        used -= r.bytes as u128;
        out.push(r.key);
    }
    out
}

/// `entry_digest` as the fixed-size key an `IndexRow` carries.
fn entry_key(e: &CacheEntry) -> [u8; 32] {
    let mut identity = e.clone();
    identity.created = 0;
    identity.last_hit = 0;
    let bytes = serde_json::to_vec(&identity).expect("entry_digest: a validated CacheEntry always serializes (validate_entry rejects the values that cannot)");
    sha2::Sha256::digest(bytes).into()
}

/// The canonical payload digest that identifies an entry independently of when it was written or
/// last served: sha256 over the entry's canonical serialization with `created` and `last_hit`
/// zeroed. Used as the replacement tie-break (so arrival order never decides), as the identity a
/// re-store of the same payload replaces, as the key of the entry's `IndexRow`, and as the handle
/// `Cache::touch` names an entry by.
///
/// # Panics
/// Panics (in every build profile, standing ruling (b)) if the entry cannot be serialized, which
/// for this type means a nonfinite or out-of-domain matrix value that `crate::entry::validate_entry`
/// would already have rejected: every caller here validates first, or works from a cell that
/// `crate::storage::decode` already validated on the way in.
pub fn entry_digest(e: &CacheEntry) -> Vec<u8> {
    entry_key(e).to_vec()
}

/// The at most two entries a cell keeps (spec section 10.4): the representative closest to the
/// cell's geometric SPR center, plus the most accurate remaining entry when it is *strictly* more
/// accurate than that one -- so an entry that wins both roles is kept once, and a candidate that
/// is both farther from the center and no more accurate replaces neither.
///
/// Every comparison is total and deterministic: relative distance to `1.02^spr_bucket` (all
/// entries in one cell share that bucket, so one center serves them all), then raw
/// `exploitability_over_P`, then the canonical payload digest.
pub fn retain_two(entries: Vec<CacheEntry>) -> Vec<CacheEntry> {
    if entries.len() <= 1 {
        return entries;
    }
    retain_two_tagged(entries.into_iter().map(|e| (entry_key(&e), e)).collect()).into_iter().map(|(_, e)| e).collect()
}

/// `retain_two` over entries already paired with their payload digests, so each digest is
/// computed once (the brief's inline comparator recloned and re-serialized a whole entry on every
/// comparison) and the caller keeps the digests it needs for the index rows.
fn retain_two_tagged(mut tagged: Vec<([u8; 32], CacheEntry)>) -> Vec<([u8; 32], CacheEntry)> {
    if tagged.len() <= 1 {
        return tagged;
    }
    tagged.sort_by(|(da, a), (db, b)| {
        let center = 1.02_f64.powi(a.key.spr_bucket);
        let distance_a = (a.source.spr.value() - center).abs() / center;
        let distance_b = (b.source.spr.value() - center).abs() / center;
        distance_a.total_cmp(&distance_b).then(a.exploitability_over_P.total_cmp(&b.exploitability_over_P)).then(da.cmp(db))
    });
    let closest = tagged.remove(0);
    tagged.sort_by(|(da, a), (db, b)| a.exploitability_over_P.total_cmp(&b.exploitability_over_P).then(da.cmp(db)));
    if tagged[0].1.exploitability_over_P < closest.1.exploitability_over_P {
        let accurate = tagged.remove(0);
        vec![closest, accurate]
    } else {
        vec![closest]
    }
}

/// The writer's in-memory accounting (module doc): one `IndexRow` per stored entry and the
/// entry-digest-to-cell map beside it. Rebuilt from disk by `scan_index` on every open and never
/// persisted. The rows and the map always describe the same set of entries.
#[derive(Clone, Debug, Default)]
pub struct Index {
    rows: Vec<IndexRow>,
    cell_of: HashMap<[u8; 32], [u8; 32]>,
}

impl Index {
    /// Every entry's row (and a placeholder row for each undeletable unusable file, module doc).
    pub fn rows(&self) -> &[IndexRow] {
        &self.rows
    }

    /// The key of the cell holding the entry whose payload digest is `entry`.
    pub fn cell_of(&self, entry: &[u8; 32]) -> Option<[u8; 32]> {
        self.cell_of.get(entry).copied()
    }

    /// Whether the entry whose payload digest is `entry` is on disk, as far as the writer knows.
    pub fn contains(&self, entry: &[u8; 32]) -> bool {
        self.cell_of.contains_key(entry)
    }

    /// The store's accounted footprint: every cell's measured bytes, each counted once.
    pub fn used(&self) -> u128 {
        self.rows.iter().map(|r| r.bytes as u128).sum()
    }

    /// The measured bytes accounted for cell `cell` (the sum of its rows' shares).
    pub fn cell_bytes(&self, cell: [u8; 32]) -> u64 {
        self.rows.iter().filter(|r| self.cell_of.get(&r.key) == Some(&cell)).map(|r| r.bytes).sum()
    }

    /// The largest `last_hit` of any row, or 0 for an empty index: where the writer's counter
    /// resumes on open.
    pub fn max_last_hit(&self) -> u64 {
        self.rows.iter().map(|r| r.last_hit).max().unwrap_or(0)
    }

    /// Replaces every row of `cell` with one row per distinct payload digest in `entries`
    /// (`(digest, last_hit)`), splitting the cell's measured `bytes` across them so they are
    /// counted once.
    ///
    /// # Panics
    /// If `entries` is empty (always-on, standing ruling (b)): a cell on disk always holds at least
    /// one entry, and an empty row set would stop accounting its bytes.
    fn set_cell(&mut self, cell: [u8; 32], entries: &[([u8; 32], u64)], bytes: u64) {
        assert!(!entries.is_empty(), "Index::set_cell: cell {cell:02x?} must be given at least one entry row");
        self.drop_cell(cell);
        let mut distinct: Vec<([u8; 32], u64)> = Vec::new();
        for &(key, last_hit) in entries {
            match distinct.iter_mut().find(|(k, _)| *k == key) {
                Some(row) => row.1 = row.1.max(last_hit),
                None => distinct.push((key, last_hit)),
            }
        }
        distinct.sort_by_key(|(key, _)| *key);
        for ((key, last_hit), share) in distinct.iter().zip(split(bytes, distinct.len())) {
            // A payload digest names exactly one cell (the digest covers the entry's key), so this
            // only ever replaces a row of `cell` itself; never let one digest sit in two cells.
            if self.cell_of.insert(*key, cell).is_some() {
                self.rows.retain(|r| r.key != *key);
            }
            self.rows.push(IndexRow { key: *key, bytes: share, last_hit: *last_hit });
        }
    }

    /// Accounts a file that holds no usable entry but could not be deleted: one placeholder row
    /// keyed by the cell key its path names, with `last_hit` 0 so it is the first eviction
    /// candidate, which retries the deletion.
    fn account_placeholder(&mut self, cell: [u8; 32], bytes: u64) {
        self.drop_cell(cell);
        self.cell_of.insert(cell, cell);
        self.rows.push(IndexRow { key: cell, bytes, last_hit: 0 });
    }

    /// Forgets every row of `cell`. Only called once its file is confirmed gone.
    fn drop_cell(&mut self, cell: [u8; 32]) {
        let cell_of = &mut self.cell_of;
        self.rows.retain(|r| {
            let belongs = cell_of.get(&r.key) == Some(&cell);
            if belongs {
                cell_of.remove(&r.key);
            }
            !belongs
        });
    }

    /// Re-splits a freshly measured `bytes` across `cell`'s existing rows.
    fn remeasure(&mut self, cell: [u8; 32], bytes: u64) {
        let mut keys = self.rows.iter().filter(|r| self.cell_of.get(&r.key) == Some(&cell)).map(|r| r.key).collect::<Vec<_>>();
        if keys.is_empty() {
            return;
        }
        keys.sort_unstable();
        for (key, share) in keys.iter().zip(split(bytes, keys.len())) {
            if let Some(row) = self.rows.iter_mut().find(|r| r.key == *key) {
                row.bytes = share;
            }
        }
    }
}

/// `bytes` split into `n >= 1` shares that sum to exactly `bytes` (the remainder on the first).
fn split(bytes: u64, n: usize) -> Vec<u64> {
    let n = n as u64;
    let (base, remainder) = (bytes / n, bytes % n);
    (0..n).map(|i| if i == 0 { base + remainder } else { base }).collect()
}

/// `(payload digest, last_hit)` for each tagged entry: the rows `Index::set_cell` takes.
fn rows_of(tagged: &[([u8; 32], CacheEntry)]) -> Vec<([u8; 32], u64)> {
    tagged.iter().map(|(key, e)| (*key, e.last_hit)).collect()
}

/// The measured on-disk length of `path`, or `None` if it cannot be read.
fn measured(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Whether `path` is the path this cell's own key addresses -- a lookup only ever reaches a cell
/// through the path derived from its key, so a misfiled cell is unreachable quota garbage and is
/// deleted outright, at scan and during eviction alike. `decode` already guarantees the entries
/// found at `path` share one key, so checking the first is enough.
///
/// This used to also gate on every entry's `last_hit` being at most `LAST_HIT_MAX` (review R5), but
/// that conflated two different kinds of "unusable" (fix round 2 N1): a misfiled cell really is
/// unreachable garbage, but a correctly filed cell carrying one corrupt timestamp is not -- the
/// entries the timestamp doesn't touch are still exactly as valid as ever. `scan_index` and
/// `evict_entry` handle that narrower case themselves, at the row level, instead of asking this
/// function to condemn the whole file over it.
fn correctly_filed(dir: &Path, path: &Path, entries: &[CacheEntry]) -> bool {
    entries.first().is_some_and(|first| entry_path(dir, first.key.digest()) == path)
}

/// `(payload digest, last_hit)` for each tagged entry whose `last_hit` is within `LAST_HIT_MAX` --
/// the rows `Index::set_cell` may hold. An entry above the ceiling (fix round 2 N1) is dropped here
/// rather than indexed: the writer itself can no longer produce one (`crate::next_last_hit`,
/// `store_entry`, `apply_touch` all refuse it), so the only way one reaches this filter is a file
/// that predates the fix or was tampered with, and it must never re-enter the index -- doing so
/// would wrongly seed the writer's counter above the ceiling on the next restart.
fn rows_of_usable(tagged: &[([u8; 32], CacheEntry)]) -> Vec<([u8; 32], u64)> {
    tagged.iter().filter(|(_, e)| e.last_hit <= LAST_HIT_MAX).map(|(key, e)| (*key, e.last_hit)).collect()
}

/// The cell key `path` is addressed by -- its file stem as the 64 lowercase hex digits
/// `entry_path` writes, filed in the matching shard of `dir` -- or `None` for a name no key
/// produces.
fn addressed_key(dir: &Path, path: &Path) -> Option<[u8; 32]> {
    let stem = path.file_stem()?.to_str()?;
    if stem.len() != 64 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0_u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&stem[2 * i..2 * i + 2], 16).ok()?;
    }
    (entry_path(dir, key) == path).then_some(key)
}

/// Re-stats cell `cell`'s file after an operation that may or may not have removed it (review R3).
/// Only `NotFound` confirms the bytes are gone and drops the cell's rows; a file still present
/// keeps them, re-measured; any other stat failure leaves them exactly as they were. Returns
/// whether the file is confirmed gone.
fn settle(dir: &Path, cell: [u8; 32], index: &mut Index) -> bool {
    match std::fs::symlink_metadata(entry_path(dir, cell)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            index.drop_cell(cell);
            true
        }
        Ok(m) => {
            index.remeasure(cell, m.len());
            false
        }
        Err(_) => false,
    }
}

/// Deletes cell `cell`'s file (`WriteCommand::Delete`, and eviction of a cell's last entry) and
/// reports whether it is confirmed gone -- deleted now or already absent. A deletion the
/// filesystem refuses (a sharing violation, a permission error) keeps the cell's rows and its
/// re-measured bytes in `index` (review R3), so the quota keeps counting a file that is still
/// there and a later call retries it.
pub fn delete_cell(dir: &Path, cell: [u8; 32], index: &mut Index) -> bool {
    let _ = std::fs::remove_file(entry_path(dir, cell));
    settle(dir, cell, index)
}

/// Rebuilds the accounting index from the store itself: every `<key>.bin` under every two-hex
/// shard of `dir`, read through the bounded `crate::storage::read_cell` (so a corrupt cell is a
/// miss and is best-effort deleted as it is scanned, exactly as a lookup would treat it), one row
/// per entry with its own `last_hit`, and each cell's measured bytes split across its rows.
///
/// A readable cell filed under a path its own key does not produce is unreachable quota garbage
/// and is deleted outright; a file that survives its deletion stays accounted by a placeholder row
/// (module doc). A readable, correctly filed cell that carries an entry above `LAST_HIT_MAX` is
/// corrupt in a narrower way (fix round 2 N1): that one row is reported (`eprintln`) and excluded
/// from the index -- never deleted -- so a corrupt timestamp on one entry can never destroy the
/// file, or any other entry the same cell holds, including the newest store. A cell with no usable
/// entry left is itself left unindexed, on disk, rather than deleted or given a placeholder (which
/// would otherwise let eviction re-derive its rows straight from the corrupt disk content). An
/// unreadable directory -- including a cache root that does not exist yet -- scans as empty.
pub fn scan_index(dir: &Path) -> Index {
    let mut index = Index::default();
    let Ok(shards) = std::fs::read_dir(dir) else { return index };
    for shard in shards.flatten() {
        let name = shard.file_name().to_string_lossy().into_owned();
        if name.len() != 2 || !name.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            continue;
        }
        let Ok(cells) = std::fs::read_dir(shard.path()) else { continue };
        for cell in cells.flatten() {
            let path = cell.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                continue;
            }
            match read_cell(&path) {
                Some(found) if correctly_filed(dir, &path, &found.entries) => {
                    let key = found.entries[0].key.digest();
                    for e in found.entries.iter().filter(|e| e.last_hit > LAST_HIT_MAX) {
                        let hit = e.last_hit;
                        eprintln!("cache scan: cell {key:02x?} carries a last_hit {hit} above LAST_HIT_MAX ({LAST_HIT_MAX}); the row is reported and skipped, the cell is kept");
                    }
                    let rows = found.entries.iter().filter(|e| e.last_hit <= LAST_HIT_MAX).map(|e| (entry_key(e), e.last_hit)).collect::<Vec<_>>();
                    if !rows.is_empty() {
                        index.set_cell(key, &rows, measured(&path).unwrap_or(0));
                    }
                }
                found => {
                    // Unreadable (`read_cell` has already tried to delete it), or misfiled: remove
                    // it, and keep counting it if that failed.
                    if found.is_some() {
                        let _ = std::fs::remove_file(&path);
                    }
                    if let (Some(key), Ok(m)) = (addressed_key(dir, &path), std::fs::symlink_metadata(&path)) {
                        if m.is_file() {
                            index.account_placeholder(key, m.len());
                        }
                    }
                }
            }
        }
    }
    index
}

/// Removes the `*.tmp` siblings a crashed or killed writer left behind, in the cache root itself
/// (where task 14 publishes `queue.json` through the same `write_atomic`) and in every two-hex
/// shard. A temp file is never referenced by anything -- `write_atomic` picks a fresh, per-call
/// name and renames it away on success -- so removing one can never disturb a published cell.
///
/// This assumes the single-instance desktop app of spec section 3: a *second* live process
/// publishing into the same cache root could have its in-flight temp file swept from under it,
/// which would fail that publication (best-effort, recommendation unaffected) but still never
/// corrupt a cell, because a swept temp file is one that was never renamed into place.
pub fn sweep_temporaries(dir: &Path) {
    let mut directories = vec![dir.to_path_buf()];
    if let Ok(shards) = std::fs::read_dir(dir) {
        for shard in shards.flatten() {
            if shard.path().is_dir() {
                directories.push(shard.path());
            }
        }
    }
    for directory in directories {
        let Ok(files) = std::fs::read_dir(&directory) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) == Some("tmp") && path.is_file() {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// Stores `entry` into its cell: validates it, reads whatever cell is already there, applies
/// `retain_two` to the union, and publishes the result atomically before re-deriving the cell's
/// rows from the entries that actually landed and the file's measured size.
///
/// Returns the incoming entry's payload digest -- its index row key -- when the cell now holds
/// it, or `None` when `retain_two` dropped it as dominated (farther from the center and no more
/// accurate than the representatives the cell already keeps): a dominated insertion is not
/// stored. The writer answers a store's receipt with whether that digest is *still* in the index
/// once the store's own quota pass has run (review R2).
///
/// A re-store of a payload already in the cell replaces that entry (so its freshly assigned
/// `last_hit` is what persists) rather than competing with itself. Entries found at the path that
/// do not belong to this key, or that carry an unusable `last_hit`, are discarded: they cannot
/// share a cell (`crate::storage::encode` refuses entries that disagree on their key) and cannot be
/// served either. When `retain_two` drops the incoming entry and re-dates nothing, the cell on disk
/// is already exactly what should be there, so nothing is republished -- identical bytes would be
/// pure I/O, and a pointless publication would needlessly supersede a concurrent reader's
/// corrupt-file cleanup (`crate::storage`'s publication lock).
///
/// # Errors
/// `CacheError` from `validate_entry` (the entry never reaches the disk), an over-ceiling
/// `last_hit` (fix round 2 N1: validated here directly, not only through the writer thread's own
/// `crate::next_last_hit` gate, so a caller reaching this function any other way is still safe),
/// from `encode` (an oversized cell is rejected by its preflight, before anything is written or
/// renamed), or from `write_atomic` (a full disk, a read-only root, a blocked path). Every one of
/// them leaves the previously published cell exactly as it was; the cell's rows are then
/// re-settled against the disk, so a file that reading it just removed stops being counted.
pub fn store_entry(dir: &Path, entry: CacheEntry, index: &mut Index) -> Result<Option<[u8; 32]>, CacheError> {
    validate_entry(&entry)?;
    if entry.last_hit > LAST_HIT_MAX {
        return Err(CacheError::Invalid("last_hit exceeds LAST_HIT_MAX"));
    }
    let cell = entry.key.digest();
    let stored = store_into(dir, cell, entry, index);
    if stored.is_err() {
        settle(dir, cell, index);
    }
    stored
}

fn store_into(dir: &Path, cell: [u8; 32], entry: CacheEntry, index: &mut Index) -> Result<Option<[u8; 32]>, CacheError> {
    let path = entry_path(dir, cell);
    let raw = read_cell(&path).map(|c| c.entries).unwrap_or_default();
    for e in raw.iter().filter(|e| e.key == entry.key && e.last_hit > LAST_HIT_MAX) {
        let hit = e.last_hit;
        eprintln!("cache store: cell {cell:02x?} already held an entry with last_hit {hit} above LAST_HIT_MAX ({LAST_HIT_MAX}); it is corrupt and is dropped by this store");
    }
    let existing = raw.into_iter().filter(|e| e.key == entry.key && e.last_hit <= LAST_HIT_MAX).map(|e| (entry_key(&e), e)).collect::<Vec<_>>();
    if existing.is_empty() {
        // A first store (or one replacing an unreadable cell): the cell is exactly this entry.
        // Encoding before digesting means an oversized entry is refused by `encode`'s preflight
        // before a full serialization of it is ever made for its digest.
        let first = Cell { entries: vec![entry] };
        let bytes = encode(&first)?;
        write_atomic(&path, &bytes)?;
        let key = entry_key(&first.entries[0]);
        index.set_cell(cell, &[(key, first.entries[0].last_hit)], measured(&path).unwrap_or(bytes.len() as u64));
        return Ok(Some(key));
    }
    let incoming = entry_key(&entry);
    let before = rows_of(&existing);
    let mut candidates = existing.into_iter().filter(|(key, _)| *key != incoming).collect::<Vec<_>>();
    candidates.push((incoming, entry));
    let kept = retain_two_tagged(candidates);
    let after = rows_of(&kept);
    let retained = kept.iter().any(|(key, _)| *key == incoming).then_some(incoming);
    if after == before {
        let bytes = measured(&path).unwrap_or_else(|| index.cell_bytes(cell));
        index.set_cell(cell, &after, bytes);
        return Ok(retained);
    }
    let bytes = encode(&Cell { entries: kept.into_iter().map(|(_, e)| e).collect() })?;
    write_atomic(&path, &bytes)?;
    index.set_cell(cell, &after, measured(&path).unwrap_or(bytes.len() as u64));
    Ok(retained)
}

/// Records that the entry with payload digest `payload_digest` in cell `key` was just served:
/// re-reads the cell, sets `last_hit` on that entry, and republishes the cell atomically.
///
/// # Errors
/// `CacheError::Invalid` if `last_hit` exceeds `LAST_HIT_MAX` (fix round 2 N1: validated here
/// directly, not only through the writer thread's own `crate::next_last_hit` gate, so a caller
/// reaching this function any other way is still safe -- and nothing is re-read or touched first),
/// if the cell is missing or unreadable -- its rows are dropped only if the file is confirmed gone
/// (`read_cell` best-effort deletes a corrupt file, and that can fail too; review R3) -- or if no
/// entry in it carries that payload digest: a lookup result that has since been replaced or evicted
/// is not an error the caller can act on, and must not silently re-date the *wrong* entry.
/// Propagates an `encode`/`write_atomic` failure, which leaves the cell exactly as it was.
pub fn apply_touch(dir: &Path, key: [u8; 32], payload_digest: &[u8], last_hit: u64, index: &mut Index) -> Result<(), CacheError> {
    if last_hit > LAST_HIT_MAX {
        return Err(CacheError::Invalid("last_hit exceeds LAST_HIT_MAX"));
    }
    let path = entry_path(dir, key);
    let Some(found) = read_cell(&path) else {
        settle(dir, key, index);
        return Err(CacheError::Invalid("touched cache cell is missing or unreadable"));
    };
    let mut tagged = found.entries.into_iter().map(|e| (entry_key(&e), e)).collect::<Vec<_>>();
    let mut touched = false;
    for (digest, e) in tagged.iter_mut() {
        if digest.as_slice() == payload_digest {
            e.last_hit = last_hit;
            touched = true;
        }
    }
    if !touched {
        return Err(CacheError::Invalid("no entry in the touched cache cell carries that payload digest"));
    }
    // `rows_of_usable`, not `rows_of`: the touched entry itself is always usable (`last_hit` was
    // validated against `LAST_HIT_MAX` above), but a corrupt sibling entry (fix round 2 N1) must
    // ride along in the rewritten cell without re-entering the index.
    let rows = rows_of_usable(&tagged);
    let bytes = encode(&Cell { entries: tagged.into_iter().map(|(_, e)| e).collect() })?;
    write_atomic(&path, &bytes)?;
    index.set_cell(key, &rows, measured(&path).unwrap_or(bytes.len() as u64));
    Ok(())
}

/// Evicts entries, globally oldest `last_hit` first (`victims`), until the index's measured
/// footprint is at or under `quota` (spec section 10.4: "oldest `last_hit` eviction"). Each pass
/// re-measures, so a cell rewritten to its survivor is charged its new size before the next victim
/// is chosen.
///
/// A cell whose file cannot be deleted stays accounted and is set aside for the rest of this call
/// (review R3): its bytes are subtracted from the budget the remaining rows are measured against,
/// so the pass carries on with the next oldest entry that *can* go, and the stuck cell is retried
/// by every later call. An entry is therefore only ever evicted when every older entry has been
/// evicted or could not be deleted in this pass -- the newest store is never chosen ahead of an
/// older entry through a tie, because `last_hit` is strictly increasing (review R5).
///
/// The pass count is bounded by four times the starting row count: every pass removes a row or
/// sets one cell aside, and re-deriving a stale placeholder's rows adds at most two accurate rows
/// in its place, so the ceiling can only be reached by a store changing underneath the writer, in
/// which case the loop stops instead of spinning.
pub fn enforce_quota(dir: &Path, quota: u64, index: &mut Index) {
    let mut stuck: HashSet<[u8; 32]> = HashSet::new();
    let mut passes = 4 * index.rows.len() + 1;
    while passes > 0 {
        passes -= 1;
        // The same `used <= quota` test `victims` opens with, checked first so the common case (a
        // store that leaves the store under quota) costs one pass over the rows instead of a sort.
        if index.used() <= quota as u128 {
            break;
        }
        let (held, candidates): (Vec<IndexRow>, Vec<IndexRow>) = index.rows.iter().cloned().partition(|r| index.cell_of(&r.key).is_some_and(|cell| stuck.contains(&cell)));
        let held_bytes: u128 = held.iter().map(|r| r.bytes as u128).sum();
        // What the evictable rows must fit in once the stuck cells' bytes -- still on disk, still
        // counted -- are set aside; nothing at all when those alone reach the quota.
        let budget = if held_bytes >= quota as u128 { 0 } else { (quota as u128 - held_bytes) as u64 };
        let Some(&victim) = victims(&candidates, budget).first() else { break };
        let cell = index.cell_of(&victim);
        if !evict_entry(dir, victim, index) {
            stuck.extend(cell);
        }
    }
}

/// Evicts the one entry whose row key is `victim` from its cell: a cell that keeps another entry is
/// rewritten with it; a cell left empty -- or unreadable, or misfiled, or whose rewrite fails (a
/// full disk, a read-only root; leaving it would leave the store over quota) -- is deleted through
/// `delete_cell`. A row whose entry is not in its cell at all (a placeholder whose file has since
/// become readable) is stale: the cell's rows are re-derived from what is on disk and nothing is
/// removed. A corrupt sibling entry (`last_hit` above `LAST_HIT_MAX`, fix round 2 N1) rides along in
/// whatever is written back -- it is never itself evicted, and never deleted just for being corrupt
/// -- but is excluded from the re-derived rows (`rows_of_usable`), so it can never re-enter the
/// index. Returns `false` only when the cell had to be deleted and could not be.
fn evict_entry(dir: &Path, victim: [u8; 32], index: &mut Index) -> bool {
    let Some(cell) = index.cell_of(&victim) else { return true };
    let path = entry_path(dir, cell);
    let Some(found) = read_cell(&path) else { return delete_cell(dir, cell, index) };
    if !correctly_filed(dir, &path, &found.entries) {
        return delete_cell(dir, cell, index);
    }
    let (evicted, kept): (Vec<_>, Vec<_>) = found.entries.into_iter().map(|e| (entry_key(&e), e)).partition(|(key, _)| *key == victim);
    if evicted.is_empty() {
        let rows = rows_of_usable(&kept);
        if rows.is_empty() {
            index.drop_cell(cell);
        } else {
            let bytes = measured(&path).unwrap_or_else(|| index.cell_bytes(cell));
            index.set_cell(cell, &rows, bytes);
        }
        return true;
    }
    if kept.is_empty() {
        return delete_cell(dir, cell, index);
    }
    let rows = rows_of_usable(&kept);
    let rewritten = encode(&Cell { entries: kept.into_iter().map(|(_, e)| e).collect() }).and_then(|bytes| write_atomic(&path, &bytes).map(|()| bytes.len() as u64));
    match rewritten {
        Ok(len) => {
            if rows.is_empty() {
                index.drop_cell(cell);
            } else {
                index.set_cell(cell, &rows, measured(&path).unwrap_or(len));
            }
            true
        }
        Err(_) => delete_cell(dir, cell, index),
    }
}
