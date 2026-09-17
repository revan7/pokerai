//! Two-representative retention, oldest-`last_hit` quota eviction, and the `Cache` handle's
//! single writer thread (spec section 10.4; plan 4 task 6). The pure decision functions
//! (`victims`, `retain_two`) are exercised directly; everything else is driven through the real
//! `Cache` handle against a real temp directory, because the whole point of this task is the
//! writer's on-disk behavior: atomic publication, at most two entries per cell, monotone
//! `last_hit`, and eviction of the oldest entry until measured bytes are under quota.
//!
//! Every filesystem test uses a per-invocation `TempDir` and every writer observation is a
//! `StoreReceipt`, never a timed sleep: the writer processes its queue strictly in order and
//! reports each `Store` only after that command's quota pass has finished, so a receipt is an
//! exact barrier for everything queued before it.

mod support;

use cache::entry::{validate_entry, CacheEntry};
use cache::key::{spr_bucket, Rational};
use cache::quota;
use cache::storage::{self, Cell};
use cache::{Cache, CacheError, CACHE_QUOTA_BYTES};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// How long a test is willing to wait for the writer thread. Generous on purpose: it is a
/// failure budget for a stuck writer, never a synchronization delay -- a healthy writer answers
/// in microseconds and no test sleeps.
const WRITER_BUDGET: Duration = Duration::from_secs(30);

// --- filesystem test helper ---------------------------------------------------------------------

static UNIQUE: AtomicU64 = AtomicU64::new(0);

/// A per-invocation-unique temp directory, exclusively created and removed on `Drop`. Identical
/// in behavior to `crates/cache/tests/storage.rs`'s helper (task 5, review R5); it is duplicated
/// rather than shared because the two files are separate test crates and the only place they
/// could share it from -- `crates/cache/tests/support/mod.rs` -- is outside this task's file
/// list.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = UNIQUE.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pokerai-quota-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir(&dir).unwrap_or_else(|e| panic!("TempDir::new must get a fresh, exclusively-created directory at {dir:?}: {e}"));
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every `*.tmp` file under `dir` and its shard subdirectories -- a crashed or aborted
/// `write_atomic` is the only thing that can leave one behind, so an empty result is how these
/// tests assert "nothing half-written was left on disk".
fn temp_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(children) = std::fs::read_dir(&next) else { continue };
        for child in children.flatten() {
            let path = child.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
                out.push(path);
            }
        }
    }
    out
}

// --- entry fixtures ------------------------------------------------------------------------------

/// The shared fixture with its effective stack (and so its exact SPR) and its raw accuracy
/// retuned. `spr_bucket` stays 81 and no key field moves, so every variant lands in the *same*
/// cell -- which is what makes them candidate representatives of one another. Bucket 81's
/// geometric center is `1.02^81 = 4.97294...`, so 495/100 (4.95, relative distance 0.00461) is
/// closer to it than 500/100 (5.0, 0.00544), and 493/100 (4.93, 0.00864) is farther than both.
fn entry_at(stacks: u32, exploitability: f64) -> CacheEntry {
    let mut e = support::entry();
    e.source.stack_oop = stacks;
    e.source.stack_ip = stacks;
    e.source.spr = Rational::new(stacks as u64, e.source.pot as u64).unwrap();
    e.exploitability_over_P = exploitability;
    assert_eq!(e.key.spr_bucket, spr_bucket(e.source.spr), "stacks {stacks} must stay inside the fixture's SPR bucket, or this is a different cell");
    assert!(validate_entry(&e).is_ok(), "the retuned fixture must still be a valid entry");
    e
}

/// A variant in a *different* cell: `tree_signature` is a key field that no other entry
/// invariant is derived from, so changing it moves the entry to its own cell without disturbing
/// anything `validate_entry` checks.
fn entry_in_cell(signature: &str, stacks: u32, exploitability: f64) -> CacheEntry {
    let mut e = entry_at(stacks, exploitability);
    e.key.tree_signature = signature.into();
    assert!(validate_entry(&e).is_ok(), "retargeting the cell must not invalidate the entry");
    e
}

fn cell_on_disk(root: &Path, key: [u8; 32]) -> Option<Vec<CacheEntry>> {
    storage::read_cell(&storage::entry_path(root, key)).map(|c| c.entries)
}

fn measured_bytes(root: &Path, key: [u8; 32]) -> u64 {
    std::fs::metadata(storage::entry_path(root, key)).map(|m| m.len()).unwrap_or(0)
}

fn stored_last_hit(root: &Path, key: [u8; 32]) -> u64 {
    cell_on_disk(root, key).expect("the cell must be on disk").iter().map(|e| e.last_hit).max().expect("a cell always holds at least one entry")
}

fn store_and_wait(cache: &Cache, e: &CacheEntry) {
    assert!(cache.store_tracked(e).wait(WRITER_BUDGET), "the writer must report a durable, validated cell for {}", hex(e.key.digest()));
}

/// A sleep-free writer barrier that drives one quota pass without changing any cell: an entry
/// that cannot validate is refused by `store_entry` before any encode or rename, and the writer
/// still runs its quota pass for that command and answers the receipt only afterwards. Waiting
/// for that answer therefore proves every command queued earlier -- and every quota pass they
/// triggered -- has already finished.
fn quota_pass(cache: &Cache) {
    let mut rejected = support::entry();
    rejected.key.schema_version = 2;
    assert!(validate_entry(&rejected).is_err(), "the barrier entry must be refused by validation, or it would change the store");
    assert!(!cache.store_tracked(&rejected).wait(WRITER_BUDGET), "the barrier store must be refused, not stored");
}

fn hex(key: [u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

// --- brief step 1 / step 5, verbatim -------------------------------------------------------------

#[test]
fn quota_uses_last_hit_not_creation_order() {
    use cache::quota::{victims, IndexRow};
    let rows = vec![IndexRow { key: [1; 32], bytes: 60, last_hit: 9 }, IndexRow { key: [2; 32], bytes: 60, last_hit: 3 }];
    assert_eq!(victims(&rows, 100), vec![[2; 32]]);
}

#[test]
fn quota_ties_are_deterministic() {
    use cache::quota::{victims, IndexRow};
    let rows = vec![IndexRow { key: [2; 32], bytes: 10, last_hit: 1 }, IndexRow { key: [1; 32], bytes: 10, last_hit: 1 }];
    assert_eq!(victims(&rows, 10), vec![[1; 32]]);
}

// --- victims: wide arithmetic, no clamping -------------------------------------------------------

/// Quota arithmetic is `u64` in, `u128` inside (standing ruling (a)/(c)): a total that overflows
/// `u64` must still be compared exactly against the quota rather than wrapping or saturating
/// into a "nothing to evict" answer.
#[test]
fn victims_sums_in_wide_arithmetic_without_clamping() {
    let rows = vec![
        quota::IndexRow { key: [3; 32], bytes: u64::MAX, last_hit: 5 },
        quota::IndexRow { key: [4; 32], bytes: u64::MAX, last_hit: 9 },
    ];
    // 2 * u64::MAX wraps to u64::MAX - 1 in u64, which is *under* this quota and would mean
    // "nothing to evict"; in u128 the footprint is genuinely over it, so the oldest row is
    // evicted -- and only that one, because removing it is already enough.
    assert_eq!(quota::victims(&rows, u64::MAX), vec![[3; 32]]);
    assert!(quota::victims(&[], 0).is_empty(), "an empty index has nothing to evict");
    assert!(quota::victims(&[quota::IndexRow { key: [1; 32], bytes: 10, last_hit: 1 }], 10).is_empty(), "exactly at quota is not over quota");
}

// --- retain_two ----------------------------------------------------------------------------------

/// Spec section 10.4 "replacement keeps at most two entries per cell": the closest SPR
/// representative and the most accurate one, deduplicated when one entry wins both.
#[test]
fn retain_two_keeps_the_closest_and_the_most_accurate_representative() {
    let closest = entry_at(495, 0.004);
    let accurate = entry_at(500, 0.001);
    let kept = quota::retain_two(vec![accurate.clone(), closest.clone()]);
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].source.spr.value(), 4.95, "the closest-SPR representative comes first");
    assert_eq!(kept[1].exploitability_over_P, 0.001, "the most accurate representative comes second");

    // One entry that is both closest and most accurate is kept once, not twice.
    let both = entry_at(495, 0.001);
    let worse = entry_at(500, 0.004);
    let kept = quota::retain_two(vec![both, worse]);
    assert_eq!(kept.len(), 1, "an entry that wins both roles is not duplicated");
    assert_eq!(kept[0].source.spr.value(), 4.95);

    assert!(quota::retain_two(vec![]).is_empty());
    assert_eq!(quota::retain_two(vec![entry_at(500, 0.004)]).len(), 1);
}

/// "A candidate both farther and less accurate cannot replace either representative."
#[test]
fn retain_two_drops_a_dominated_candidate() {
    let closest = entry_at(495, 0.004);
    let accurate = entry_at(500, 0.001);
    let dominated = entry_at(493, 0.006);
    let kept = quota::retain_two(vec![closest, accurate, dominated]);
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].source.spr.value(), 4.95);
    assert_eq!(kept[1].exploitability_over_P, 0.001);
    assert!(kept.iter().all(|e| e.source.spr.value() != 4.93), "the dominated candidate must not displace either representative");
}

/// "ties use canonical payload digest, never arrival order" and the timestamps are excluded from
/// that digest, so a hit (which only moves `created`/`last_hit`) can never reorder replacement.
#[test]
fn retain_two_ties_use_the_payload_digest_and_ignore_timestamps() {
    let mut x = entry_at(500, 0.004);
    let mut y = entry_at(500, 0.004);
    x.iterations = 111;
    y.iterations = 222;
    assert_ne!(quota::entry_digest(&x), quota::entry_digest(&y), "the two tied candidates must be distinguishable by digest");

    let forwards = quota::retain_two(vec![x.clone(), y.clone()]);
    let backwards = quota::retain_two(vec![y.clone(), x.clone()]);
    assert_eq!(forwards.len(), 1, "two identically-placed, identically-accurate candidates keep one representative");
    assert_eq!(forwards[0].iterations, backwards[0].iterations, "arrival order must not decide the tie");

    let mut touched = x.clone();
    touched.created = 999_999;
    touched.last_hit = 999_999;
    assert_eq!(quota::entry_digest(&touched), quota::entry_digest(&x), "created/last_hit must not enter the tie digest");
    let after_a_hit = quota::retain_two(vec![touched, y]);
    assert_eq!(after_a_hit[0].iterations, forwards[0].iterations, "a hit must not change replacement ordering");
}

// --- the writer: publication, retention, monotone last_hit ---------------------------------------

#[test]
fn cache_store_publishes_a_validated_cell_at_its_sharded_path() {
    let dir = TempDir::new("store");
    let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    assert_eq!(cache.root(), dir.path());
    let e = entry_at(500, 0.004);
    store_and_wait(&cache, &e);

    let path = storage::entry_path(dir.path(), e.key.digest());
    assert!(path.exists(), "the cell must be published at its sharded path");
    let entries = cell_on_disk(dir.path(), e.key.digest()).expect("the published cell must read back");
    assert_eq!(entries.len(), 1);
    assert!(validate_entry(&entries[0]).is_ok());
    assert!(temp_files(dir.path()).is_empty(), "a successful publication leaves no temp file behind");
    cache.shutdown();
}

/// The live path's own entry point: `store` is fire-and-forget (no receipt to wait on), so the
/// barrier behind it is the queue's own ordering.
#[test]
fn cache_store_is_fire_and_forget_but_still_publishes() {
    let dir = TempDir::new("store-untracked");
    let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    let e = entry_at(500, 0.004);
    cache.store(&e);
    quota_pass(&cache);
    assert!(cell_on_disk(dir.path(), e.key.digest()).is_some(), "a best-effort store still reaches disk");
    cache.shutdown();
}

/// The handle is shared across the engine's threads (the writer thread here, the reader in task
/// 7, the pre-solver in task 16), so it has to be `Send + Sync` -- and a receipt has to be
/// movable to whichever thread awaits it.
#[test]
fn the_cache_handle_is_shareable_across_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send<T: Send>() {}
    assert_send_sync::<Cache>();
    assert_send::<cache::StoreReceipt>();
    assert_send::<cache::WriteCommand>();
}

#[test]
fn cache_keeps_at_most_two_entries_per_cell_and_drops_a_dominated_insertion() {
    let dir = TempDir::new("two-per-cell");
    let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    let closest = entry_at(495, 0.004);
    store_and_wait(&cache, &closest);
    store_and_wait(&cache, &entry_at(500, 0.001));
    store_and_wait(&cache, &entry_at(493, 0.006)); // farther and less accurate: dominated

    let entries = cell_on_disk(dir.path(), closest.key.digest()).expect("the cell must be on disk");
    assert_eq!(entries.len(), 2, "a cell never holds more than two entries");
    let sprs = entries.iter().map(|e| e.source.spr.value()).collect::<Vec<_>>();
    assert_eq!(sprs, vec![4.95, 5.0], "the two representatives are the closest SPR and the most accurate entry");
    assert!(entries.iter().all(|e| validate_entry(e).is_ok()));
    cache.shutdown();
}

/// "Assign persisted `last_hit` on the writer using `max(unix_ms, previous_max_last_hit + 1)`...
/// so clock reversal cannot make a fresh hit the oldest entry."
#[test]
fn writer_last_hit_is_monotone_even_when_the_callers_clock_goes_backwards() {
    let dir = TempDir::new("monotone");
    let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    let mut first = entry_in_cell("cell_a_test_v1", 500, 0.004);
    first.last_hit = 10_000_000;
    let mut second = entry_in_cell("cell_b_test_v1", 500, 0.004);
    second.last_hit = 5; // the clock went backwards between the two stores
    store_and_wait(&cache, &first);
    store_and_wait(&cache, &second);

    let a = stored_last_hit(dir.path(), first.key.digest());
    let b = stored_last_hit(dir.path(), second.key.digest());
    assert!(a >= 10_000_000, "a caller timestamp ahead of the writer's maximum is honored, got {a}");
    assert!(b > a, "the later store must persist a strictly greater last_hit than the earlier one ({b} vs {a})");
    cache.shutdown();
}

// --- quota eviction ------------------------------------------------------------------------------

/// Three same-sized cells against a two-cell quota: the oldest `last_hit` cell goes, and only it.
#[test]
fn cache_evicts_the_oldest_cell_when_measured_bytes_exceed_the_quota() {
    let dir = TempDir::new("evict");
    let a = entry_in_cell("cell_a_test_v1", 500, 0.004);
    let b = entry_in_cell("cell_b_test_v1", 500, 0.004);
    let c = entry_in_cell("cell_c_test_v1", 500, 0.004);
    let one = storage::encode(&Cell { entries: vec![a.clone()] }).unwrap().len() as u64;
    let quota = 2 * one + 256; // room for two of these cells, never three

    let cache = Cache::open(dir.path().to_path_buf(), quota);
    store_and_wait(&cache, &a);
    store_and_wait(&cache, &b);
    let (a_bytes, b_bytes) = (measured_bytes(dir.path(), a.key.digest()), measured_bytes(dir.path(), b.key.digest()));
    assert!(a_bytes + b_bytes <= quota, "two cells ({a_bytes} + {b_bytes}) must fit the quota {quota}, or this test is mis-sized");
    assert!(cell_on_disk(dir.path(), a.key.digest()).is_some(), "nothing is evicted while under quota");

    store_and_wait(&cache, &c);
    let c_bytes = measured_bytes(dir.path(), c.key.digest());
    assert!(a_bytes + b_bytes + c_bytes > quota, "three cells must genuinely exceed the quota, or this test is mis-sized");
    assert!(b_bytes + c_bytes <= quota, "evicting exactly one cell must bring the store back under quota, or this test is mis-sized");

    assert!(cell_on_disk(dir.path(), a.key.digest()).is_none(), "the oldest cell must be evicted");
    assert!(!storage::entry_path(dir.path(), a.key.digest()).exists(), "an evicted single-entry cell's file is deleted");
    assert!(cell_on_disk(dir.path(), b.key.digest()).is_some(), "a newer cell must survive");
    assert!(cell_on_disk(dir.path(), c.key.digest()).is_some(), "the cell just stored must survive");
    cache.shutdown();
}

/// A touch re-dates a cell, so the *other* cell becomes the eviction victim -- the quota orders
/// by `last_hit`, not by creation order, through the writer just as `victims` does in isolation.
#[test]
fn cache_quota_follows_last_hit_after_a_touch() {
    let dir = TempDir::new("touch-evict");
    let a = entry_in_cell("cell_a_test_v1", 500, 0.004);
    let b = entry_in_cell("cell_b_test_v1", 500, 0.004);
    let c = entry_in_cell("cell_c_test_v1", 500, 0.004);
    let one = storage::encode(&Cell { entries: vec![a.clone()] }).unwrap().len() as u64;
    let quota = 2 * one + 256;

    let cache = Cache::open(dir.path().to_path_buf(), quota);
    store_and_wait(&cache, &a);
    store_and_wait(&cache, &b);
    let a_before = stored_last_hit(dir.path(), a.key.digest());
    let b_before = stored_last_hit(dir.path(), b.key.digest());
    assert!(a_before < b_before, "a must start out as the older cell ({a_before} vs {b_before})");

    // The lookup side's "this entry was served" report: the cell key plus the served entry's
    // canonical payload digest (which excludes the mutable timestamps).
    cache.touch(a.key.digest(), quota::entry_digest(&a), 1);
    store_and_wait(&cache, &c); // the receipt is also the barrier for the queued touch

    let a_after = stored_last_hit(dir.path(), a.key.digest());
    assert!(a_after > b_before, "the touch must persist a fresher last_hit on a ({a_after} vs {b_before})");
    assert!(cell_on_disk(dir.path(), b.key.digest()).is_none(), "the touch makes b the oldest cell, so b is evicted");
    assert!(cell_on_disk(dir.path(), a.key.digest()).is_some(), "the touched cell must survive");
    assert!(cell_on_disk(dir.path(), c.key.digest()).is_some());
    cache.shutdown();
}

/// A two-entry cell loses its individual oldest entry first, by rewriting the survivor -- the
/// file is only deleted once both entries are gone.
#[test]
fn cache_evicts_the_oldest_entry_of_a_two_entry_cell_by_rewriting_the_survivor() {
    let dir = TempDir::new("evict-entry");
    let closest = entry_at(495, 0.004);
    let accurate = entry_at(500, 0.001);
    let key = closest.key.digest();

    // Fill the cell first under a generous quota, then measure it and re-open over the same root
    // one byte under that measurement -- a quota that exactly one entry's removal satisfies, with
    // no guessing about how well two near-identical entries compress.
    let filling = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    store_and_wait(&filling, &closest);
    store_and_wait(&filling, &accurate);
    assert_eq!(cell_on_disk(dir.path(), key).unwrap().len(), 2, "the cell must hold two entries before eviction");
    let two = measured_bytes(dir.path(), key);
    filling.shutdown();

    let cache = Cache::open(dir.path().to_path_buf(), two - 1);
    quota_pass(&cache);
    let entries = cell_on_disk(dir.path(), key).expect("the cell keeps its file while one entry survives");
    assert_eq!(entries.len(), 1, "exactly the oldest of the two entries is evicted");
    assert_eq!(entries[0].source.spr.value(), 5.0, "the survivor is the newer (later-stored) entry");
    assert!(validate_entry(&entries[0]).is_ok(), "a rewritten survivor is still a valid cell");
    assert!(measured_bytes(dir.path(), key) <= two - 1, "the rewrite must bring the store back under quota");
    assert!(temp_files(dir.path()).is_empty(), "a rewrite leaves no temp file behind");
    cache.shutdown();
}

/// Once both entries are gone the file itself is deleted, not left as an empty cell.
#[test]
fn cache_deletes_a_cells_file_once_its_last_entry_is_evicted() {
    let dir = TempDir::new("evict-file");
    let e = entry_at(500, 0.004);
    let filling = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    store_and_wait(&filling, &e);
    filling.shutdown();

    let cache = Cache::open(dir.path().to_path_buf(), 0);
    quota_pass(&cache);
    assert!(!storage::entry_path(dir.path(), e.key.digest()).exists(), "a single-entry cell over quota is deleted outright");
    cache.shutdown();
}

/// The index is rebuilt from disk on open, so eviction order and the monotone `last_hit` counter
/// both survive a restart (no second authoritative index file exists).
#[test]
fn cache_quota_order_and_last_hit_counter_survive_a_restart() {
    let dir = TempDir::new("restart");
    let a = entry_in_cell("cell_a_test_v1", 500, 0.004);
    let b = entry_in_cell("cell_b_test_v1", 500, 0.004);
    let c = entry_in_cell("cell_c_test_v1", 500, 0.004);
    let one = storage::encode(&Cell { entries: vec![a.clone()] }).unwrap().len() as u64;

    let first = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    store_and_wait(&first, &a);
    store_and_wait(&first, &b);
    let b_before = stored_last_hit(dir.path(), b.key.digest());
    first.shutdown();

    let reopened = Cache::open(dir.path().to_path_buf(), 2 * one + 256);
    store_and_wait(&reopened, &c);
    assert!(cell_on_disk(dir.path(), a.key.digest()).is_none(), "the restarted writer must still evict the oldest cell from disk state");
    assert!(cell_on_disk(dir.path(), b.key.digest()).is_some());
    let c_after = stored_last_hit(dir.path(), c.key.digest());
    assert!(c_after > b_before, "the restarted writer must resume the last_hit counter above what disk already holds ({c_after} vs {b_before})");
    reopened.shutdown();
}

// --- failure paths -------------------------------------------------------------------------------

/// "A failure to create the directory yields a disabled cache: every lookup is `Miss` and every
/// store is dropped" -- and `store_tracked`'s receipt reports that honestly instead of hanging.
#[test]
fn a_disabled_cache_drops_every_write_and_reports_no_durable_store() {
    let dir = TempDir::new("disabled");
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"a file where the cache root should be").unwrap();
    let cache = Cache::open(blocker.join("v3"), CACHE_QUOTA_BYTES);
    assert_eq!(cache.root(), Path::new(""), "an unopenable root yields the disabled handle");

    let e = entry_at(500, 0.004);
    assert!(!cache.store_tracked(&e).wait(Duration::from_millis(100)), "a disabled cache never reports a durable store");
    cache.store(&e); // best-effort and silent
    cache.touch(e.key.digest(), quota::entry_digest(&e), 1);
    cache.shutdown();

    let disabled = Cache::disabled();
    assert_eq!(disabled.root(), Path::new(""));
    assert!(!disabled.store_tracked(&e).wait(Duration::from_millis(100)));
    disabled.shutdown();
}

/// An injected publication failure (the cell's own path occupied by a directory, so `rename`
/// cannot replace it): the receipt reports failure, no temp file is left behind, and the writer
/// stays alive for the next store -- a write failure never takes the cache down with it.
#[test]
fn a_failed_publication_is_reported_and_leaves_no_temp_file() {
    let dir = TempDir::new("write-fail");
    let blocked = entry_in_cell("cell_a_test_v1", 500, 0.004);
    let path = storage::entry_path(dir.path(), blocked.key.digest());
    std::fs::create_dir_all(&path).unwrap(); // a directory exactly where the cell file belongs

    let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    assert!(!cache.store_tracked(&blocked).wait(WRITER_BUDGET), "a publication that cannot rename must report failure");
    assert!(path.is_dir(), "the blocking directory is untouched");
    assert!(temp_files(dir.path()).is_empty(), "a failed publication removes its own temp file");

    let next = entry_in_cell("cell_b_test_v1", 500, 0.004);
    store_and_wait(&cache, &next);
    assert!(cell_on_disk(dir.path(), next.key.digest()).is_some(), "the writer survives a failed publication");
    cache.shutdown();
}

/// An oversized cell is rejected by `encode`'s preflight, so nothing is ever written or renamed:
/// no cell file, no temp file, and no index row for it.
#[test]
fn store_entry_rejects_an_oversized_cell_before_anything_is_renamed() {
    let dir = TempDir::new("oversize");
    let mut e = entry_at(500, 0.004);
    e.key.tree_signature = "x".repeat((storage::DECODED_MAX + 4096) as usize);
    let path = storage::entry_path(dir.path(), e.key.digest());
    let mut index = Vec::new();
    match quota::store_entry(dir.path(), e, &mut index) {
        Err(CacheError::Invalid(msg)) => assert_eq!(msg, "decoded cell payload exceeds DECODED_MAX"),
        other => panic!("expected the explicit DECODED_MAX rejection, got {other:?}"),
    }
    assert!(!path.exists(), "an oversized cell is never published");
    assert!(temp_files(dir.path()).is_empty(), "an oversized cell never reaches a temp file either");
    assert!(index.is_empty(), "a rejected store must not leave an index row behind");
}

// --- index rebuild and crash recovery ------------------------------------------------------------

#[test]
fn scan_index_rebuilds_measured_bytes_and_the_maximum_last_hit_per_cell() {
    let dir = TempDir::new("scan");
    assert!(quota::scan_index(&dir.path().join("missing")).is_empty(), "a root that does not exist yet scans as empty");

    let mut single = entry_in_cell("cell_a_test_v1", 500, 0.004);
    single.last_hit = 7;
    let mut old = entry_at(495, 0.004);
    old.last_hit = 3;
    let mut fresh = entry_at(500, 0.001);
    fresh.last_hit = 11;

    for entries in [vec![single.clone()], vec![old.clone(), fresh.clone()]] {
        let key = entries[0].key.digest();
        let bytes = storage::encode(&Cell { entries }).unwrap();
        storage::write_atomic(&storage::entry_path(dir.path(), key), &bytes).unwrap();
    }
    // A corrupt cell and a cell filed under the wrong key are both quota garbage: neither can
    // ever be served, so neither may occupy an index row.
    let corrupt = storage::entry_path(dir.path(), [0xcc; 32]);
    std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
    std::fs::write(&corrupt, b"not a cache header").unwrap();
    let misfiled = storage::entry_path(dir.path(), [0xdd; 32]);
    let good = storage::encode(&Cell { entries: vec![single.clone()] }).unwrap();
    std::fs::create_dir_all(misfiled.parent().unwrap()).unwrap();
    std::fs::write(&misfiled, &good).unwrap();

    let mut rows = quota::scan_index(dir.path());
    rows.sort_by_key(|r| r.key);
    assert_eq!(rows.len(), 2, "only the two correctly filed, readable cells are indexed: {rows:?}");
    for row in &rows {
        assert_eq!(row.bytes, measured_bytes(dir.path(), row.key), "a row's bytes are the cell file's measured length");
    }
    let by_key = |key: [u8; 32]| rows.iter().find(|r| r.key == key).map(|r| r.last_hit);
    assert_eq!(by_key(single.key.digest()), Some(7));
    assert_eq!(by_key(old.key.digest()), Some(11), "a two-entry cell is as fresh as its freshest entry");
    assert!(!corrupt.exists(), "a corrupt cell is deleted as it is scanned");
    assert!(!misfiled.exists(), "a misfiled cell is deleted as it is scanned");
}

/// A crashed writer leaves a temp file and an intact old cell: the published bytes are never
/// half-written, and `sweep_temporaries` clears the leftovers without touching any cell.
#[test]
fn sweep_temporaries_clears_crashed_writes_and_keeps_published_cells() {
    let dir = TempDir::new("sweep");
    let e = entry_at(500, 0.004);
    let path = storage::entry_path(dir.path(), e.key.digest());
    let bytes = storage::encode(&Cell { entries: vec![e.clone()] }).unwrap();
    storage::write_atomic(&path, &bytes).unwrap();

    // What a crash between `write_all` and `rename` leaves behind: write_atomic's own temp
    // naming in the shard directory, plus a root-level one (Task 14 publishes `queue.json`
    // through the same function).
    let shard_tmp = path.with_extension(format!("{}.7.tmp", std::process::id()));
    std::fs::write(&shard_tmp, &bytes[..bytes.len() / 2]).unwrap();
    let root_tmp = dir.path().join("queue.4242.0.tmp");
    std::fs::write(&root_tmp, b"half a queue").unwrap();

    assert!(storage::read_cell(&path).is_some(), "a half-written temp sibling must leave the published cell readable");
    quota::sweep_temporaries(dir.path());
    assert!(!shard_tmp.exists(), "a crashed shard temp file is swept");
    assert!(!root_tmp.exists(), "a crashed root temp file is swept");
    assert!(storage::read_cell(&path).is_some(), "sweeping must never touch a published cell");
    assert!(temp_files(dir.path()).is_empty());
}

// --- constants -----------------------------------------------------------------------------------

#[test]
fn quota_default_and_cache_root_match_the_spec() {
    assert_eq!(CACHE_QUOTA_BYTES, 10 * 1024 * 1024 * 1024, "spec section 10.4: default cache_quota_bytes = 10 GiB");
    let root = cache::default_cache_root();
    assert!(root.ends_with(Path::new("PokerAI/cache/v3")), "spec section 10.4 storage root, got {root:?}");
}
