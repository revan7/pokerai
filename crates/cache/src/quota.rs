//! Replacement, accounting and eviction for the on-disk cell store (spec section 10.4:
//! "replacement keeps at most two entries per cell (closest SPR, best accuracy)... Default
//! `cache_quota_bytes = 10 GiB`; oldest `last_hit` eviction"). Three pure decision functions --
//! `entry_digest`, `retain_two`, `victims` -- and the five helpers the single writer thread in
//! `crate::lib` drives them with: `scan_index`, `sweep_temporaries`, `store_entry`, `apply_touch`
//! and `enforce_quota`.
//!
//! ## Accounting: one row per cell, measured from disk
//!
//! `scan_index` rebuilds the whole index from the files themselves on every open -- it is never a
//! second authoritative file that could disagree with the store (there is no index file at all).
//! One `IndexRow` describes one *cell*: its measured on-disk byte length, and the **maximum**
//! `last_hit` across the one or two entries it holds, so a cell is exactly as fresh as its
//! freshest entry and a cell's bytes are counted once no matter how many entries share it. That
//! is what makes `victims`'s `rows.iter().map(|r| r.bytes).sum()` the true measured footprint.
//! Every total is accumulated in `u128` and compared against the `u64` quota without clamping
//! (standing rulings (a)/(c)): an index whose byte sum overflows `u64` still compares as over
//! quota rather than wrapping into "nothing to evict".
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
//!
//! ## Eviction
//!
//! `enforce_quota` evicts one *entry* at a time, oldest `last_hit` first, taking the next oldest
//! until the measured footprint is back under quota and recomputing that footprint after every
//! step: a two-entry cell loses its older entry by being rewritten with the survivor (through the
//! same atomic publication path as any other write), and a cell is only unlinked once its last
//! entry goes. A cell whose rewrite fails (full disk, read-only root) is unlinked outright rather
//! than left over quota, and the loop is bounded by twice the row count so a persistently failing
//! filesystem cannot spin it.
//!
//! ## Deviation from the brief: no entry-digest-to-cell map
//!
//! The brief asks for "an entry-digest-to-cell map beside the entry index". Nothing in this task
//! needs one: `WriteCommand::Touch` already carries the cell key *and* the served entry's payload
//! digest (the brief's own shape), so `apply_touch` resolves the entry by re-reading that one
//! cell and matching `entry_digest`, and a store resolves its cell from `KeyFields::digest()`.
//! A second, digest-keyed structure would be exactly the kind of parallel index the same brief
//! rules out ("The index is rebuilt from disk on open and is never a second authoritative file"),
//! and would have to be invalidated on every rewrite, touch and eviction. `entry_digest` is
//! exported instead, so the lookup side (task 7) can name the entry it served with the same
//! canonical digest the writer matches on.

use crate::entry::{validate_entry, CacheEntry};
use crate::storage::{encode, entry_path, read_cell, write_atomic, Cell};
use crate::CacheError;
use sha2::Digest;
use std::path::Path;

/// One cell's accounting row: its key digest (the cell's identity and its file name), its
/// measured on-disk byte length, and the maximum `last_hit` of the entries it holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexRow {
    pub key: [u8; 32],
    pub bytes: u64,
    pub last_hit: u64,
}

/// The cells to evict, oldest `last_hit` first, so that the measured footprint of what remains is
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

/// The canonical payload digest that identifies an entry independently of when it was written or
/// last served: sha256 over the entry's canonical serialization with `created` and `last_hit`
/// zeroed. Used as the replacement tie-break (so arrival order never decides), as the identity a
/// re-store of the same payload replaces, and as the handle `Cache::touch` names an entry by.
///
/// # Panics
/// Panics (in every build profile, standing ruling (b)) if the entry cannot be serialized, which
/// for this type means a nonfinite or out-of-domain matrix value that `crate::entry::validate_entry`
/// would already have rejected: every caller here validates first, or works from a cell that
/// `crate::storage::decode` already validated on the way in.
pub fn entry_digest(e: &CacheEntry) -> Vec<u8> {
    let mut identity = e.clone();
    identity.created = 0;
    identity.last_hit = 0;
    let bytes = serde_json::to_vec(&identity).expect("entry_digest: a validated CacheEntry always serializes (validate_entry rejects the values that cannot)");
    sha2::Sha256::digest(bytes).to_vec()
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
    // Each entry's digest is computed once here rather than inside the comparators (the brief's
    // inline closure recloned and re-serialized a whole entry on every comparison); the ordering
    // itself is unchanged.
    let mut tagged = entries.into_iter().map(|e| (entry_digest(&e), e)).collect::<Vec<_>>();
    tagged.sort_by(|(da, a), (db, b)| {
        let center = 1.02_f64.powi(a.key.spr_bucket);
        let distance_a = (a.source.spr.value() - center).abs() / center;
        let distance_b = (b.source.spr.value() - center).abs() / center;
        distance_a.total_cmp(&distance_b).then(a.exploitability_over_P.total_cmp(&b.exploitability_over_P)).then(da.cmp(db))
    });
    let (_, closest) = tagged.remove(0);
    tagged.sort_by(|(da, a), (db, b)| a.exploitability_over_P.total_cmp(&b.exploitability_over_P).then(da.cmp(db)));
    if tagged[0].1.exploitability_over_P < closest.exploitability_over_P {
        let (_, accurate) = tagged.remove(0);
        vec![closest, accurate]
    } else {
        vec![closest]
    }
}

/// The measured on-disk length of `path`, or `None` if it cannot be read.
fn measured(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Inserts or refreshes `key`'s accounting row.
fn set_row(index: &mut Vec<IndexRow>, key: [u8; 32], bytes: u64, last_hit: u64) {
    match index.iter_mut().find(|r| r.key == key) {
        Some(row) => {
            row.bytes = bytes;
            row.last_hit = last_hit;
        }
        None => index.push(IndexRow { key, bytes, last_hit }),
    }
}

/// Rebuilds the accounting index from the store itself: every `<key>.bin` under every two-hex
/// shard of `dir`, read through the bounded `crate::storage::read_cell` (so a corrupt cell is a
/// miss and is best-effort deleted as it is scanned, exactly as a lookup would treat it), with
/// each surviving cell's measured byte length and maximum `last_hit`.
///
/// A readable cell whose own key digest does not match the path it was found at is deleted: no
/// lookup can ever reach it (a lookup only ever builds a path *from* a key digest), so it is
/// nothing but quota garbage. An unreadable directory -- including a cache root that does not
/// exist yet -- scans as empty rather than failing.
pub fn scan_index(dir: &Path) -> Vec<IndexRow> {
    let mut rows = Vec::new();
    let Ok(shards) = std::fs::read_dir(dir) else { return rows };
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
            let Some(cell) = read_cell(&path) else { continue };
            let Some(first) = cell.entries.first() else { continue };
            let key = first.key.digest();
            if entry_path(dir, key) != path {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let bytes = measured(&path).unwrap_or(0);
            let last_hit = cell.entries.iter().map(|e| e.last_hit).max().unwrap_or(0);
            rows.push(IndexRow { key, bytes, last_hit });
        }
    }
    rows
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
/// `retain_two` to the union, and publishes the result atomically before refreshing the cell's
/// accounting row from the file that actually landed.
///
/// A re-store of a payload already in the cell replaces that entry (so its freshly assigned
/// `last_hit` is what persists) rather than competing with itself. Entries found at the path that
/// do not belong to this key are discarded: they cannot share a cell (`crate::storage::encode`
/// refuses entries that disagree on their key) and cannot be served either. When `retain_two`
/// drops the incoming entry as dominated and re-dates nothing, the cell on disk is already
/// exactly what should be there, so nothing is republished -- identical bytes would be pure I/O,
/// and a pointless publication would needlessly supersede a concurrent reader's corrupt-file
/// cleanup (`crate::storage`'s publication lock).
///
/// # Errors
/// `CacheError` from `validate_entry` (the entry never reaches the disk), from `encode` (an
/// oversized cell is rejected by its preflight, before anything is written or renamed), or from
/// `write_atomic` (a full disk, a read-only root, a blocked path). Every one of them leaves the
/// previously published cell exactly as it was.
pub fn store_entry(dir: &Path, entry: CacheEntry, index: &mut Vec<IndexRow>) -> Result<(), CacheError> {
    validate_entry(&entry)?;
    let key = entry.key.digest();
    let path = entry_path(dir, key);
    let existing = read_cell(&path)
        .map(|c| c.entries)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.key == entry.key)
        .map(|e| (entry_digest(&e), e))
        .collect::<Vec<_>>();
    let before = existing.iter().map(|(digest, e)| (digest.clone(), e.last_hit)).collect::<Vec<_>>();
    // Only needed to recognize a re-store of a payload already in the cell; skipped entirely when
    // the cell is empty, so a first store never pays for a digest it cannot match.
    let incoming = if existing.is_empty() { Vec::new() } else { entry_digest(&entry) };
    let mut candidates = existing.into_iter().filter(|(digest, _)| *digest != incoming).map(|(_, e)| e).collect::<Vec<_>>();
    candidates.push(entry);

    let kept = retain_two(candidates);
    let last_hit = kept.iter().map(|e| e.last_hit).max().unwrap_or(0);
    let after = kept.iter().map(|e| (entry_digest(e), e.last_hit)).collect::<Vec<_>>();
    if after == before {
        set_row(index, key, measured(&path).unwrap_or(0), last_hit);
        return Ok(());
    }
    let bytes = encode(&Cell { entries: kept })?;
    write_atomic(&path, &bytes)?;
    set_row(index, key, measured(&path).unwrap_or(bytes.len() as u64), last_hit);
    Ok(())
}

/// Records that the entry with payload digest `payload_digest` in cell `key` was just served:
/// re-reads the cell, sets `last_hit` on that entry, and republishes the cell atomically.
///
/// # Errors
/// `CacheError::Invalid` if the cell is missing or unreadable (its accounting row is dropped, so
/// the index stops charging the quota for a file that is no longer there) or if no entry in it
/// carries that payload digest -- a lookup result that has since been replaced or evicted is not
/// an error the caller can act on, and must not silently re-date the *wrong* entry. Propagates an
/// `encode`/`write_atomic` failure, which leaves the cell exactly as it was.
pub fn apply_touch(dir: &Path, key: [u8; 32], payload_digest: &[u8], last_hit: u64, index: &mut Vec<IndexRow>) -> Result<(), CacheError> {
    let path = entry_path(dir, key);
    let Some(cell) = read_cell(&path) else {
        index.retain(|r| r.key != key);
        return Err(CacheError::Invalid("touched cache cell is missing or unreadable"));
    };
    let mut entries = cell.entries;
    let mut touched = false;
    for e in entries.iter_mut() {
        if entry_digest(e).as_slice() == payload_digest {
            e.last_hit = last_hit;
            touched = true;
        }
    }
    if !touched {
        return Err(CacheError::Invalid("no entry in the touched cache cell carries that payload digest"));
    }
    let fresh = entries.iter().map(|e| e.last_hit).max().unwrap_or(0);
    let bytes = encode(&Cell { entries })?;
    write_atomic(&path, &bytes)?;
    set_row(index, key, measured(&path).unwrap_or(bytes.len() as u64), fresh);
    Ok(())
}

/// Evicts entries, oldest `last_hit` first, until the index's measured footprint is at or under
/// `quota` (spec section 10.4: "oldest `last_hit` eviction"). Each pass re-measures, so a cell
/// rewritten to its survivor is charged its new size before the next victim is chosen.
///
/// The pass count is bounded by twice the row count: every pass either removes one entry from a
/// cell or drops the cell (and its row) entirely, so that ceiling can only be reached by a
/// filesystem failing every operation, in which case the loop stops instead of spinning.
pub fn enforce_quota(dir: &Path, quota: u64, index: &mut Vec<IndexRow>) {
    let mut passes = index.len().saturating_mul(2).saturating_add(1);
    while passes > 0 {
        passes -= 1;
        // The same `used <= quota` test `victims` opens with, checked first so the common case (a
        // store that leaves the store under quota) costs one pass over the rows instead of a sort.
        let used: u128 = index.iter().map(|r| r.bytes as u128).sum();
        if used <= quota as u128 {
            break;
        }
        let Some(&key) = victims(index, quota).first() else { break };
        evict_oldest_entry(dir, key, index);
    }
}

/// Evicts one entry from cell `key`: a cell with two entries is rewritten with the newer one (so
/// the cache keeps the fresher representative and gives back the difference), and a cell with one
/// entry -- or one that cannot be read, or whose rewrite fails -- has its file deleted and its row
/// dropped, because leaving it in place would leave the store over quota.
fn evict_oldest_entry(dir: &Path, key: [u8; 32], index: &mut Vec<IndexRow>) {
    let path = entry_path(dir, key);
    let entries = read_cell(&path).map(|c| c.entries).unwrap_or_default();
    if entries.len() >= 2 {
        let mut ordered = entries.into_iter().map(|e| (entry_digest(&e), e)).collect::<Vec<_>>();
        // Oldest first, ties broken by the canonical payload digest so the victim never depends on
        // the order the entries happen to sit in the file.
        ordered.sort_by(|(da, a), (db, b)| a.last_hit.cmp(&b.last_hit).then(da.cmp(db)));
        ordered.remove(0);
        let kept = ordered.into_iter().map(|(_, e)| e).collect::<Vec<_>>();
        let last_hit = kept.iter().map(|e| e.last_hit).max().unwrap_or(0);
        let rewritten = encode(&Cell { entries: kept }).and_then(|bytes| write_atomic(&path, &bytes).map(|()| bytes.len() as u64));
        if let Ok(len) = rewritten {
            set_row(index, key, measured(&path).unwrap_or(len), last_hit);
            return;
        }
    }
    let _ = std::fs::remove_file(&path);
    index.retain(|r| r.key != key);
}
