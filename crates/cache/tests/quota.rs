//! Two-representative retention, oldest-`last_hit` quota eviction, and the `Cache` handle's
//! single writer thread (spec section 10.4; plan 4 task 6). The pure decision functions
//! (`victims`, `retain_two`) are exercised directly; everything else is driven through the real
//! `Cache` handle against a real temp directory, because the whole point of this task is the
//! writer's on-disk behavior: atomic publication, at most two entries per cell, monotone
//! `last_hit`, and eviction of the oldest entry until measured bytes are under quota.
//!
//! Every filesystem test uses a per-invocation `TempDir` (the shared `support/temp_dir.rs`) and
//! every writer observation is a `StoreReceipt`, never a timed sleep: the writer processes its
//! queue strictly in order and reports each `Store` only after that command's quota pass has
//! finished, so a receipt is an exact barrier for everything queued before it.

mod support;
#[path = "support/temp_dir.rs"]
mod temp_dir;

use cache::entry::{validate_entry, CacheEntry};
use cache::key::{spr_bucket, Rational};
use cache::quota;
use cache::storage::{self, Cell};
use cache::{Cache, CacheError, CACHE_QUOTA_BYTES};
use std::path::{Path, PathBuf};
use std::time::Duration;
use temp_dir::TempDir;

/// How long a test is willing to wait for the writer thread. Generous on purpose: it is a
/// failure budget for a stuck writer, never a synchronization delay -- a healthy writer answers
/// in microseconds and no test sleeps.
const WRITER_BUDGET: Duration = Duration::from_secs(30);

// --- filesystem test helpers (the per-invocation `TempDir` is the shared `support/temp_dir.rs`) --

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
    // Farther and less accurate: dominated, so not stored -- and its receipt says so (review R2).
    assert!(!cache.store_tracked(&entry_at(493, 0.006)).wait(WRITER_BUDGET), "a dominated insertion is not stored");

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
    let mut index = quota::Index::default();
    match quota::store_entry(dir.path(), e, &mut index) {
        Err(CacheError::Invalid(msg)) => assert_eq!(msg, "decoded cell payload exceeds DECODED_MAX"),
        other => panic!("expected the explicit DECODED_MAX rejection, got {other:?}"),
    }
    assert!(!path.exists(), "an oversized cell is never published");
    assert!(temp_files(dir.path()).is_empty(), "an oversized cell never reaches a temp file either");
    assert!(index.rows().is_empty(), "a rejected store must not leave an index row behind");
}

// --- fix round 1 (review `task-6-review.md`): behavioral regressions -----------------------------

/// Stores a copy of `entry` whose caller-proposed `last_hit` is `hit` and returns the receipt. In
/// a store whose writer counter is below `hit`, `hit` is exactly what the writer persists.
fn store_at(cache: &Cache, entry: &CacheEntry, hit: u64) -> bool {
    let mut e = entry.clone();
    e.last_hit = hit;
    cache.store_tracked(&e).wait(WRITER_BUDGET)
}

/// The persisted `last_hit` of every entry in cell `key`, ascending, or `None` when the cell is
/// not on disk.
fn last_hits(root: &Path, key: [u8; 32]) -> Option<Vec<u64>> {
    let mut hits = cell_on_disk(root, key)?.iter().map(|e| e.last_hit).collect::<Vec<_>>();
    hits.sort_unstable();
    Some(hits)
}

/// The encoded (and therefore on-disk) size of a cell holding exactly these entries, each carrying
/// the paired `last_hit`.
fn cell_size(entries: &[(&CacheEntry, u64)]) -> u64 {
    let entries = entries
        .iter()
        .map(|(e, hit)| {
            let mut e = (*e).clone();
            e.last_hit = *hit;
            e
        })
        .collect();
    storage::encode(&Cell { entries }).unwrap().len() as u64
}

/// Review R1's two-cell / three-entry scenario: cell A holds entries hit at 1 and 100, cell B one
/// entry hit at 50, and the quota is one byte under their total. Eviction is by *entry*, oldest
/// `last_hit` first across the whole store, so A's entry at 1 goes -- rewriting A to its survivor
/// -- and B's entry at 50, which is newer, stays. Ranking whole cells by their newest entry would
/// instead evict B (A's newest is 100) and keep A's oldest entry alive.
#[test]
fn quota_evicts_the_globally_oldest_entry_across_cells() {
    let a_old = entry_at(495, 0.004); // closest to the bucket centre
    let a_new = entry_at(500, 0.001); // strictly more accurate: both are kept as representatives
    let b = entry_in_cell("cell_b_test_v1", 500, 0.004);
    let (a_key, b_key) = (a_old.key.digest(), b.key.digest());
    let total = cell_size(&[(&a_old, 1), (&a_new, 100)]) + cell_size(&[(&b, 50)]);
    let quota = total - 1;
    assert!(cell_size(&[(&a_new, 100)]) + cell_size(&[(&b, 50)]) <= quota, "removing only A's oldest entry must be enough, or this test is mis-sized");

    let dir = TempDir::new("global");
    let cache = Cache::open(dir.path().to_path_buf(), quota);
    assert!(store_at(&cache, &a_old, 1));
    assert!(store_at(&cache, &b, 50));
    assert_eq!(last_hits(dir.path(), a_key), Some(vec![1]), "nothing is evicted while under quota");
    assert!(store_at(&cache, &a_new, 100), "the store that crosses the quota is not itself the victim");
    assert_eq!(last_hits(dir.path(), a_key), Some(vec![100]), "the globally oldest entry (A, hit 1) is the one evicted");
    assert_eq!(last_hits(dir.path(), b_key), Some(vec![50]), "B's entry is newer than A's oldest entry and must survive");
    cache.shutdown();
}

/// The same ordering after a touch: A holds entries hit at 1 and 2, B one at 3; a touch re-dates
/// A's entry at 2 to 4, and a third cell C (hit 5) crosses the quota. The oldest entry is still
/// A's entry at 1, so it goes and B survives -- ranking cells by their newest entry would see A as
/// fresh (4) and evict B.
#[test]
fn quota_evicts_the_globally_oldest_entry_after_a_touch() {
    let a_old = entry_at(495, 0.004);
    let a_new = entry_at(500, 0.001);
    let b = entry_in_cell("cell_b_test_v1", 500, 0.004);
    let c = entry_in_cell("cell_c_test_v1", 500, 0.004);
    let (a_key, b_key, c_key) = (a_old.key.digest(), b.key.digest(), c.key.digest());
    let total = cell_size(&[(&a_old, 1), (&a_new, 4)]) + cell_size(&[(&b, 3)]) + cell_size(&[(&c, 5)]);
    let quota = total - 1;
    assert!(
        cell_size(&[(&a_new, 4)]) + cell_size(&[(&b, 3)]) + cell_size(&[(&c, 5)]) <= quota,
        "removing only A's oldest entry must be enough, or this test is mis-sized"
    );
    assert!(cell_size(&[(&a_old, 1), (&a_new, 2)]) + cell_size(&[(&b, 3)]) <= quota, "nothing may be evicted before C arrives, or this test is mis-sized");

    let dir = TempDir::new("global-touch");
    let cache = Cache::open(dir.path().to_path_buf(), quota);
    assert!(store_at(&cache, &a_old, 1));
    assert!(store_at(&cache, &a_new, 2));
    assert!(store_at(&cache, &b, 3));
    cache.touch(a_key, quota::entry_digest(&a_new), 0); // the writer persists max(0, 3 + 1) = 4
    assert!(store_at(&cache, &c, 5), "the store that crosses the quota is not itself the victim");
    assert_eq!(last_hits(dir.path(), a_key), Some(vec![4]), "A's untouched entry (hit 1) is the globally oldest and is evicted");
    assert_eq!(last_hits(dir.path(), b_key), Some(vec![3]), "B (hit 3) is newer than A's oldest entry and must survive");
    assert_eq!(last_hits(dir.path(), c_key), Some(vec![5]));
    cache.shutdown();
}

/// The same ordering rebuilt from disk: the scan on open must index every *entry* with its own
/// `last_hit`, not one row per cell carrying its newest entry's.
#[test]
fn quota_evicts_the_globally_oldest_entry_after_a_restart() {
    let a_old = entry_at(495, 0.004);
    let a_new = entry_at(500, 0.001);
    let b = entry_in_cell("cell_b_test_v1", 500, 0.004);
    let (a_key, b_key) = (a_old.key.digest(), b.key.digest());
    let dir = TempDir::new("global-restart");

    let first = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    assert!(store_at(&first, &a_old, 1));
    assert!(store_at(&first, &b, 50));
    assert!(store_at(&first, &a_new, 100));
    let total = measured_bytes(dir.path(), a_key) + measured_bytes(dir.path(), b_key);
    first.shutdown();
    assert!(cell_size(&[(&a_new, 100)]) + measured_bytes(dir.path(), b_key) <= total - 1, "removing only A's oldest entry must be enough, or this test is mis-sized");

    let reopened = Cache::open(dir.path().to_path_buf(), total - 1);
    quota_pass(&reopened);
    assert_eq!(last_hits(dir.path(), a_key), Some(vec![100]), "the globally oldest entry (A, hit 1) is the one evicted after a restart");
    assert_eq!(last_hits(dir.path(), b_key), Some(vec![50]), "B's entry is newer than A's oldest entry and must survive a restart");
    reopened.shutdown();
}

/// Review R2: a receipt reports the outcome *after* the store's own quota pass. A store that pass
/// evicts again -- a zero quota, or a quota smaller than the one cell -- is not on disk, so its
/// receipt is `false`; exactly one cell's bytes is at quota, not over it, so that store is kept.
#[test]
fn a_store_evicted_by_its_own_quota_pass_is_reported_as_not_stored() {
    let e = entry_at(500, 0.004);
    let one = cell_size(&[(&e, 1)]);
    for (label, quota, kept) in [("zero", 0, false), ("under-one-cell", one - 1, false), ("exactly-one-cell", one, true)] {
        let dir = TempDir::new(&format!("receipt-{label}"));
        let cache = Cache::open(dir.path().to_path_buf(), quota);
        assert_eq!(cache.store_tracked(&e).wait(WRITER_BUDGET), kept, "{label}: the receipt must say whether the cell survived its quota pass");
        assert_eq!(storage::entry_path(dir.path(), e.key.digest()).exists(), kept, "{label}: the receipt must agree with the disk");
        cache.shutdown();
    }
}

/// Review R2's dominated insertion, defined: the incoming entry is farther from the centre and
/// less accurate than both representatives the cell already keeps, so it is not stored at all --
/// the receipt is `false` and the cell is untouched. A re-store of a payload the cell already
/// holds is a replacement (re-dated), so that one *is* retained and reported `true`.
#[test]
fn a_dominated_insertion_is_reported_as_not_stored() {
    let dir = TempDir::new("receipt-dominated");
    let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    let closest = entry_at(495, 0.004);
    store_and_wait(&cache, &closest);
    store_and_wait(&cache, &entry_at(500, 0.001));
    let path = storage::entry_path(dir.path(), closest.key.digest());
    let before = std::fs::read(&path).unwrap();

    assert!(!cache.store_tracked(&entry_at(493, 0.006)).wait(WRITER_BUDGET), "a dominated insertion is not on disk, so its receipt is false");
    assert_eq!(std::fs::read(&path).unwrap(), before, "a dominated insertion leaves the cell untouched");

    assert!(cache.store_tracked(&closest).wait(WRITER_BUDGET), "a re-store of a payload already in the cell replaces it and is retained");
    cache.shutdown();
}

/// Review R3: Windows refuses to delete a file another handle holds open without delete sharing.
/// That failed deletion must leave the cell accounted -- the bytes are still on disk -- so a later
/// quota pass retries and removes it once the handle is gone.
#[test]
fn an_undeletable_cell_stays_accounted_and_is_evicted_once_released() {
    let dir = TempDir::new("undeletable");
    let e = entry_at(500, 0.004);
    let filling = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    store_and_wait(&filling, &e);
    filling.shutdown();
    let path = storage::entry_path(dir.path(), e.key.digest());

    let holder = hold_without_delete_sharing(&path);
    let cache = Cache::open(dir.path().to_path_buf(), 0);
    quota_pass(&cache);
    assert!(path.exists(), "the held file cannot be deleted, or this test is not exercising a failed deletion");
    drop(holder);
    quota_pass(&cache);
    assert!(!path.exists(), "the failed deletion must have kept the cell accounted, so a later pass retries and removes it");
    cache.shutdown();
}

/// Two single-entry cells `(older, newer)` where `newer` sorts first on every tie-break eviction
/// could fall back to (its cell key and its canonical payload digest are both smaller), so a
/// `last_hit` tie between them would make the newer one the victim.
fn tie_pair() -> (CacheEntry, CacheEntry) {
    let candidates = ["cell_a_test_v1", "cell_b_test_v1", "cell_c_test_v1", "cell_d_test_v1", "cell_e_test_v1", "cell_f_test_v1"]
        .map(|signature| entry_in_cell(signature, 500, 0.004));
    for older in &candidates {
        for newer in &candidates {
            if newer.key.digest() < older.key.digest() && quota::entry_digest(newer) < quota::entry_digest(older) {
                return (older.clone(), newer.clone());
            }
        }
    }
    panic!("no candidate pair sorts the same way on both tie-breaks");
}

/// Review R5, the reviewer's probe: with a one-cell quota, an older store proposing `u64::MAX` and
/// a newer one proposing 1. Saturating the writer's counter gave both the same `last_hit`, the tie
/// fell back to key order, and the *newer* store was evicted. The newest store must never be the
/// victim ahead of an older entry: a timestamp with no headroom above it is refused outright.
#[test]
fn the_newest_store_is_never_evicted_through_a_last_hit_tie() {
    let (older, newer) = tie_pair();
    let (older_size, newer_size) = (cell_size(&[(&older, 1)]), cell_size(&[(&newer, 1)]));
    let quota = older_size.max(newer_size);
    assert!(older_size + newer_size > quota, "the quota must hold only one of the two cells, or this test is mis-sized");

    let dir = TempDir::new("tie");
    let cache = Cache::open(dir.path().to_path_buf(), quota);
    let _ = store_at(&cache, &older, u64::MAX);
    assert!(store_at(&cache, &newer, 1), "the newest store must be retained");
    assert!(cell_on_disk(dir.path(), newer.key.digest()).is_some(), "the newest store must still be on disk");
    assert!(cell_on_disk(dir.path(), older.key.digest()).is_none(), "the older entry, not the newest store, is what cannot stay");
    cache.shutdown();
}

/// Review R5: a caller timestamp above `LAST_HIT_MAX` (here `u64::MAX`) is refused without moving
/// the writer's counter -- a store is not stored (receipt `false`), a touch re-dates nothing.
#[test]
fn an_unusable_last_hit_is_refused_without_advancing_the_counter() {
    let dir = TempDir::new("unusable-hit");
    let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    let a = entry_in_cell("cell_a_test_v1", 500, 0.004);
    let b = entry_in_cell("cell_b_test_v1", 500, 0.004);
    let c = entry_in_cell("cell_c_test_v1", 500, 0.004);

    assert!(!store_at(&cache, &a, u64::MAX), "a store proposing a timestamp with no headroom is refused");
    assert!(cell_on_disk(dir.path(), a.key.digest()).is_none(), "a refused store writes nothing");
    assert!(store_at(&cache, &b, 1));
    assert_eq!(last_hits(dir.path(), b.key.digest()), Some(vec![1]), "the refused store must not have advanced the counter");

    cache.touch(b.key.digest(), quota::entry_digest(&b), u64::MAX);
    assert!(store_at(&cache, &c, 1));
    assert_eq!(last_hits(dir.path(), b.key.digest()), Some(vec![1]), "a touch proposing an unusable timestamp re-dates nothing");
    assert_eq!(last_hits(dir.path(), c.key.digest()), Some(vec![2]), "the refused touch must not have advanced the counter either");
    cache.shutdown();
}

/// Review R5, restart side, revised by fix round 2 N1: a cell already on disk whose entry carries
/// an unusable `last_hit` must not seed the writer's counter (it would leave no headroom for the
/// strictly increasing bump) -- but it is no longer deleted at scan, only reported and excluded
/// from the index (the re-review's N1: deleting it destroyed the newest store whenever the corrupt
/// row was the writer's own bug rather than external tampering, so scan-time deletion for this
/// specific reason was removed; a misfiled cell is still deleted, see `scan_index_...` above).
#[test]
fn a_cell_on_disk_with_an_unusable_last_hit_neither_seeds_the_counter_nor_is_deleted() {
    let dir = TempDir::new("unusable-on-disk");
    let mut bad = entry_in_cell("cell_a_test_v1", 500, 0.004);
    bad.last_hit = u64::MAX;
    let bad_path = storage::entry_path(dir.path(), bad.key.digest());
    storage::write_atomic(&bad_path, &storage::encode(&Cell { entries: vec![bad] }).unwrap()).unwrap();

    let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    let b = entry_in_cell("cell_b_test_v1", 500, 0.004);
    assert!(store_at(&cache, &b, 1), "the writer must still be storing");
    assert_eq!(last_hits(dir.path(), b.key.digest()), Some(vec![1]), "a disk timestamp above LAST_HIT_MAX must not seed the counter");
    assert!(bad_path.exists(), "a cell carrying an unusable last_hit is reported and skipped, never deleted for that reason alone (fix round 2 N1)");
    cache.shutdown();
}

// --- index rebuild and crash recovery ------------------------------------------------------------

/// Review R1: the index holds one row per *entry* -- keyed by the entry's canonical payload
/// digest, carrying that entry's own `last_hit` -- plus the entry-digest-to-cell map, while each
/// cell's measured bytes are still counted exactly once (split across its entries' rows).
#[test]
fn scan_index_indexes_every_entry_with_its_own_last_hit_and_maps_it_to_its_cell() {
    let dir = TempDir::new("scan");
    assert!(quota::scan_index(&dir.path().join("missing")).rows().is_empty(), "a root that does not exist yet scans as empty");

    let mut single = entry_in_cell("cell_a_test_v1", 500, 0.004);
    single.last_hit = 7;
    let mut old = entry_at(495, 0.004);
    old.last_hit = 3;
    let mut fresh = entry_at(500, 0.001);
    fresh.last_hit = 11;
    let (single_cell, pair_cell) = (single.key.digest(), old.key.digest());
    for entries in [vec![single.clone()], vec![old.clone(), fresh.clone()]] {
        let key = entries[0].key.digest();
        let bytes = storage::encode(&Cell { entries }).unwrap();
        storage::write_atomic(&storage::entry_path(dir.path(), key), &bytes).unwrap();
    }
    let corrupt = storage::entry_path(dir.path(), [0xcc; 32]);
    std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
    std::fs::write(&corrupt, b"not a cache header").unwrap();
    let misfiled = storage::entry_path(dir.path(), [0xdd; 32]);
    std::fs::create_dir_all(misfiled.parent().unwrap()).unwrap();
    std::fs::write(&misfiled, storage::encode(&Cell { entries: vec![single.clone()] }).unwrap()).unwrap();

    let index = quota::scan_index(dir.path());
    assert_eq!(index.rows().len(), 3, "one row per entry of the two correctly filed, readable cells: {:?}", index.rows());
    for (entry, cell) in [(&single, single_cell), (&old, pair_cell), (&fresh, pair_cell)] {
        let digest = entry_key(entry);
        let row = index.rows().iter().find(|r| r.key == digest).expect("every entry has its own row, keyed by its payload digest");
        assert_eq!(row.last_hit, entry.last_hit, "a row carries its own entry's last_hit, never its cell's newest");
        assert_eq!(index.cell_of(&digest), Some(cell), "the entry-digest-to-cell map resolves every entry to its cell");
        assert!(index.contains(&digest));
    }
    for cell in [single_cell, pair_cell] {
        assert_eq!(index.cell_bytes(cell), measured_bytes(dir.path(), cell), "a cell's measured bytes are counted once, split across its entries' rows");
    }
    assert_eq!(index.used(), (measured_bytes(dir.path(), single_cell) + measured_bytes(dir.path(), pair_cell)) as u128);
    assert_eq!(index.max_last_hit(), 11);
    assert!(!corrupt.exists(), "a corrupt cell is deleted as it is scanned");
    assert!(!misfiled.exists(), "a misfiled cell is deleted as it is scanned");
}

/// The payload digest an entry's index row is keyed by.
fn entry_key(e: &CacheEntry) -> [u8; 32] {
    quota::entry_digest(e).try_into().expect("a sha256 digest is 32 bytes")
}

/// Review R5's boundary, tightened by fix round 2 N1: one below `LAST_HIT_MAX` is a usable
/// timestamp, and the store after it is strictly newer by exactly one, landing exactly *on* the
/// ceiling rather than past it (checked arithmetic, never a saturating tie, and never persisted
/// above `LAST_HIT_MAX` either) -- so under a one-cell quota the older entry is the victim and the
/// newest store survives, even though the newest store sorts first on every tie-break.
#[test]
fn the_newest_store_survives_at_the_last_hit_boundary() {
    let (older, newer) = tie_pair();
    let boundary = quota::LAST_HIT_MAX;
    let (older_size, newer_size) = (cell_size(&[(&older, boundary - 1)]), cell_size(&[(&newer, boundary)]));
    let quota = older_size.max(newer_size);
    assert!(older_size + newer_size > quota, "the quota must hold only one of the two cells, or this test is mis-sized");

    let dir = TempDir::new("tie-boundary");
    let cache = Cache::open(dir.path().to_path_buf(), quota);
    assert!(store_at(&cache, &older, boundary - 1), "one below LAST_HIT_MAX is a usable timestamp");
    assert!(store_at(&cache, &newer, 1), "the newest store must be retained");
    assert_eq!(last_hits(dir.path(), newer.key.digest()), Some(vec![boundary]), "the store after boundary-1 is exactly one newer, landing exactly on LAST_HIT_MAX");
    assert!(cell_on_disk(dir.path(), older.key.digest()).is_none(), "the older entry is the victim");
    cache.shutdown();
}

/// Fix round 2 N1: a `last_hit` at or above `LAST_HIT_MAX` is validated the same way whether it
/// arrives as a caller's raw proposal or as the writer's own monotonic bump -- one below, at, and
/// one above the ceiling, for both a store and a touch. Below and at are usable; one above is
/// refused as an error that changes nothing, however it was reached.
#[test]
fn store_and_touch_reject_last_hit_one_above_the_ceiling_however_it_is_reached() {
    let dir = TempDir::new("ceiling-boundary");
    let cache = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    let ceiling = quota::LAST_HIT_MAX;

    // Directly proposed: below, at, and one above the ceiling.
    let below = entry_in_cell("cell_a_test_v1", 500, 0.004);
    assert!(store_at(&cache, &below, ceiling - 1), "one below the ceiling is a usable proposal");
    let at = entry_in_cell("cell_b_test_v1", 500, 0.004);
    assert!(store_at(&cache, &at, ceiling), "the ceiling itself is a usable proposal, and the bump from ceiling-1 lands exactly on it");
    assert_eq!(last_hits(dir.path(), at.key.digest()), Some(vec![ceiling]));
    let over = entry_in_cell("cell_c_test_v1", 500, 0.004);
    assert!(!store_at(&cache, &over, ceiling + 1), "a proposal one above the ceiling is refused outright");
    assert!(cell_on_disk(dir.path(), over.key.digest()).is_none(), "a refused store writes nothing");

    // The counter is now exactly at the ceiling (from `at`'s store): the next bump would itself
    // land one past it, so an otherwise-ordinary proposal must be refused too, not silently
    // persisted above LAST_HIT_MAX -- the writer can never produce that value itself (fix round 2
    // N1).
    let bumped = entry_in_cell("cell_d_test_v1", 500, 0.004);
    assert!(!store_at(&cache, &bumped, 1), "a store whose bump would land one past the ceiling is refused");
    assert!(cell_on_disk(dir.path(), bumped.key.digest()).is_none());

    cache.touch(at.key.digest(), quota::entry_digest(&at), ceiling);
    quota_pass(&cache);
    assert_eq!(last_hits(dir.path(), at.key.digest()), Some(vec![ceiling]), "a touch proposing exactly the ceiling is usable");

    cache.touch(at.key.digest(), quota::entry_digest(&at), ceiling + 1);
    quota_pass(&cache);
    assert_eq!(last_hits(dir.path(), at.key.digest()), Some(vec![ceiling]), "a touch proposing one above the ceiling re-dates nothing");

    cache.touch(below.key.digest(), quota::entry_digest(&below), 1); // an ordinary proposal, but the bump would land one past the ceiling
    quota_pass(&cache);
    assert_eq!(last_hits(dir.path(), below.key.digest()), Some(vec![ceiling - 1]), "a touch whose bump would land one past the ceiling re-dates nothing either");

    assert!(cell_on_disk(dir.path(), below.key.digest()).is_some(), "no earlier store is disturbed by any of the refusals");
    cache.shutdown();
}

/// Fix round 2 N1: `store_entry` and `apply_touch` validate `last_hit` against `LAST_HIT_MAX`
/// directly, not only through the writer's `next_last_hit` gate -- so a caller reaching either
/// function any other way still gets an error and changes nothing on disk.
#[test]
fn store_entry_rejects_an_over_ceiling_last_hit_directly() {
    let dir = TempDir::new("store-entry-ceiling");
    let mut e = entry_at(500, 0.004);
    e.last_hit = quota::LAST_HIT_MAX + 1;
    let path = storage::entry_path(dir.path(), e.key.digest());
    let mut index = quota::Index::default();
    match quota::store_entry(dir.path(), e, &mut index) {
        Err(CacheError::Invalid(_)) => {}
        other => panic!("expected an over-ceiling last_hit to be rejected directly, got {other:?}"),
    }
    assert!(!path.exists(), "a rejected store publishes nothing");
    assert!(index.rows().is_empty());
}

/// The same direct validation on the touch path.
#[test]
fn apply_touch_rejects_an_over_ceiling_last_hit_directly() {
    let dir = TempDir::new("apply-touch-ceiling");
    let e = entry_at(500, 0.004);
    let _path = publish(dir.path(), &e);
    let mut index = quota::scan_index(dir.path());
    match quota::apply_touch(dir.path(), e.key.digest(), &quota::entry_digest(&e), quota::LAST_HIT_MAX + 1, &mut index) {
        Err(CacheError::Invalid(_)) => {}
        other => panic!("expected an over-ceiling last_hit to be rejected directly, got {other:?}"),
    }
    assert_eq!(last_hits(dir.path(), e.key.digest()), Some(vec![e.last_hit]), "a rejected touch changes nothing");
}

/// Fix round 2 N1: a persisted `last_hit` above `LAST_HIT_MAX` can now only reach disk through
/// direct tampering -- the writer itself refuses to ever produce one (the two tests above) -- but
/// if one is found there anyway, `scan_index` must report and skip that one row, never delete the
/// cell that holds it or disturb any other, legitimate store made in the same session. This is the
/// re-review's own reachable state: a store at exactly `LAST_HIT_MAX`, followed (pre-fix) by
/// another store whose bump landed one past it.
#[test]
fn a_restart_reports_and_skips_a_persisted_over_ceiling_row_without_deleting_any_cell() {
    let dir = TempDir::new("corrupt-last-hit-restart");
    let legitimate = entry_in_cell("cell_a_test_v1", 500, 0.004);
    let first = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    store_and_wait(&first, &legitimate);
    first.shutdown();

    // What the pre-fix writer bug could persist: a correctly filed, otherwise valid cell whose
    // entry carries a last_hit one above the ceiling.
    let mut corrupt = entry_in_cell("cell_b_test_v1", 500, 0.004);
    corrupt.last_hit = quota::LAST_HIT_MAX + 1;
    let corrupt_path = storage::entry_path(dir.path(), corrupt.key.digest());
    storage::write_atomic(&corrupt_path, &storage::encode(&Cell { entries: vec![corrupt.clone()] }).unwrap()).unwrap();

    let index = quota::scan_index(dir.path());
    assert!(corrupt_path.exists(), "a persisted over-ceiling row must not be deleted at scan");
    assert!(!index.contains(&entry_key(&corrupt)), "the corrupt row is skipped, never indexed");
    assert!(index.contains(&entry_key(&legitimate)), "the newest legitimate store survives the scan untouched");
    assert_eq!(index.max_last_hit(), legitimate.last_hit, "the corrupt row must not seed the writer's counter");
    assert_eq!(last_hits(dir.path(), legitimate.key.digest()), Some(vec![legitimate.last_hit]), "the legitimate cell's content is untouched");

    let reopened = Cache::open(dir.path().to_path_buf(), CACHE_QUOTA_BYTES);
    assert!(cell_on_disk(dir.path(), legitimate.key.digest()).is_some(), "the restart itself must not remove the legitimate store");
    assert!(corrupt_path.exists(), "the restart itself must not remove the corrupt file either");
    let next = entry_in_cell("cell_c_test_v1", 500, 0.004);
    assert!(store_at(&reopened, &next, 1), "the writer must still be able to store after opening onto a corrupt row");
    reopened.shutdown();
}

/// Review R3's sharing mode: read and write sharing, but no `FILE_SHARE_DELETE` (0x4), so Windows
/// refuses to delete the file while this handle is open.
const FILE_SHARE_READ_WRITE: u32 = 0x1 | 0x2;

fn hold_without_delete_sharing(path: &Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ_WRITE).open(path).unwrap()
}

/// Publishes a single-entry cell for `e` directly (no writer thread) and returns its path.
fn publish(dir: &Path, e: &CacheEntry) -> PathBuf {
    let path = storage::entry_path(dir, e.key.digest());
    storage::write_atomic(&path, &storage::encode(&Cell { entries: vec![e.clone()] }).unwrap()).unwrap();
    path
}

/// Review R3 at the helper level: `enforce_quota` keeps a cell whose deletion failed in the index
/// -- its row and its measured bytes -- and removes both only once a later pass has actually
/// deleted the file.
#[test]
fn enforce_quota_keeps_a_failed_deletion_accounted_until_the_file_is_gone() {
    let dir = TempDir::new("failed-delete");
    let e = entry_at(500, 0.004);
    let path = publish(dir.path(), &e);
    let mut index = quota::scan_index(dir.path());
    let bytes = measured_bytes(dir.path(), e.key.digest()) as u128;
    assert_eq!(index.used(), bytes);

    let holder = hold_without_delete_sharing(&path);
    quota::enforce_quota(dir.path(), 0, &mut index);
    assert!(path.exists(), "the held file cannot be deleted, or this test is not exercising a failed deletion");
    assert!(index.contains(&entry_key(&e)), "a failed deletion keeps the entry's row");
    assert_eq!(index.used(), bytes, "a failed deletion keeps the file's measured bytes accounted");

    drop(holder);
    quota::enforce_quota(dir.path(), 0, &mut index);
    assert!(!path.exists(), "a later pass retries the deletion");
    assert!(index.rows().is_empty(), "the row goes only once the file is confirmed gone");
    assert_eq!(index.used(), 0);
}

/// A cell that cannot be deleted is skipped for the rest of that pass -- its bytes still count --
/// and the pass carries on with the next oldest entry, so measured bytes still come back under
/// quota instead of the loop stopping (or spinning) on the stuck file.
#[test]
fn enforce_quota_skips_an_undeletable_cell_and_evicts_the_next_oldest() {
    let dir = TempDir::new("skip-stuck");
    let mut older = entry_in_cell("cell_a_test_v1", 500, 0.004);
    older.last_hit = 1;
    let mut newer = entry_in_cell("cell_b_test_v1", 500, 0.004);
    newer.last_hit = 2;
    let older_path = publish(dir.path(), &older);
    let newer_path = publish(dir.path(), &newer);
    let mut index = quota::scan_index(dir.path());
    let older_bytes = measured_bytes(dir.path(), older.key.digest());
    assert!(measured_bytes(dir.path(), newer.key.digest()) > 0, "both cells together must exceed a quota of one cell, or this test is mis-sized");

    let holder = hold_without_delete_sharing(&older_path);
    quota::enforce_quota(dir.path(), older_bytes, &mut index);
    assert!(older_path.exists(), "the held (older) cell cannot be deleted");
    assert!(!newer_path.exists(), "the pass carries on with the next oldest entry");
    assert_eq!(index.used(), older_bytes as u128, "what remains -- the stuck cell -- is accounted and at quota");
    assert!(index.contains(&entry_key(&older)));
    drop(holder);
}

/// Review R3, the `WriteCommand::Delete` path: `delete_cell` reports a refused deletion as not
/// done and keeps the cell accounted; once the file is gone -- deleted now, or already absent --
/// its rows go.
#[test]
fn delete_cell_keeps_an_undeletable_cell_accounted() {
    let dir = TempDir::new("delete-cell");
    let e = entry_at(500, 0.004);
    let cell = e.key.digest();
    let path = publish(dir.path(), &e);
    let mut index = quota::scan_index(dir.path());
    let bytes = measured_bytes(dir.path(), cell) as u128;

    let holder = hold_without_delete_sharing(&path);
    assert!(!quota::delete_cell(dir.path(), cell, &mut index), "a deletion Windows refused is not reported as done");
    assert!(path.exists());
    assert_eq!(index.used(), bytes, "the refused deletion keeps the cell's bytes accounted");
    drop(holder);

    assert!(quota::delete_cell(dir.path(), cell, &mut index), "the retry deletes it");
    assert!(!path.exists());
    assert!(index.rows().is_empty());
    assert!(quota::delete_cell(dir.path(), cell, &mut index), "an already-absent cell is confirmed gone");
}

/// Review R3, the unreadable-cell path of `apply_touch`: `read_cell`'s best-effort delete of a
/// corrupt cell can fail too, and a file still on disk stays accounted at its measured size until a
/// later pass removes it.
#[test]
fn apply_touch_keeps_an_unreadable_undeletable_cell_accounted() {
    let dir = TempDir::new("touch-unreadable");
    let e = entry_at(500, 0.004);
    let path = publish(dir.path(), &e);
    let mut index = quota::scan_index(dir.path());
    let garbage = b"no longer a cache header";
    std::fs::write(&path, garbage).unwrap();

    let holder = hold_without_delete_sharing(&path);
    assert!(quota::apply_touch(dir.path(), e.key.digest(), &quota::entry_digest(&e), 5, &mut index).is_err());
    assert!(path.exists(), "the corrupt file could not be deleted");
    assert_eq!(index.used(), garbage.len() as u128, "an unreadable cell still on disk stays accounted, at its measured size");
    drop(holder);

    quota::enforce_quota(dir.path(), 0, &mut index);
    assert!(!path.exists(), "a later pass removes it");
    assert_eq!(index.used(), 0);
}

/// Review R3 at scan time: a corrupt cell that `scan_index` cannot delete is still on disk, so it
/// is accounted (by the key its path is addressed by) until a later pass removes it.
#[test]
fn scan_index_accounts_an_undeletable_corrupt_cell_until_it_is_gone() {
    let dir = TempDir::new("scan-undeletable");
    let key = [0xcc; 32];
    let path = storage::entry_path(dir.path(), key);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let garbage = b"not a cache header";
    std::fs::write(&path, garbage).unwrap();

    let holder = hold_without_delete_sharing(&path);
    let mut index = quota::scan_index(dir.path());
    assert!(path.exists(), "the corrupt file could not be deleted");
    assert_eq!(index.cell_bytes(key), garbage.len() as u64, "a corrupt cell that is still on disk is accounted under the key its path names");
    assert_eq!(index.used(), garbage.len() as u128);
    drop(holder);

    quota::enforce_quota(dir.path(), 0, &mut index);
    assert!(!path.exists(), "a later pass removes it");
    assert_eq!(index.used(), 0);
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
