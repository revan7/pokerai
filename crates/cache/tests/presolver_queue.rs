mod support;
#[path = "support/temp_dir.rs"]
mod temp_dir;

use cache::presolver::scenarios::*;
use proto::Position::*;

/// The 24-scenario contract, frozen (fix round 1, review R1): task 14's queue keys `queue.json`
/// items off `Scenario::id`, so this is the exact ordered list task 14 depends on, spelled out
/// field-by-field rather than reproduced from the production loops -- a swapped pair, a changed
/// depth, a duplicated scenario or a changed id would all leave a counts-only test green, but
/// not this one.
fn expected_scenarios() -> Vec<Scenario> {
    vec![
        // Tier 1: 4 SRP lines, 100bb.
        Scenario { tier: 1, depth_bb: 100, opener: Btn, caller: Bb, three_bettor: None },
        Scenario { tier: 1, depth_bb: 100, opener: Co, caller: Bb, three_bettor: None },
        Scenario { tier: 1, depth_bb: 100, opener: Hj, caller: Bb, three_bettor: None },
        Scenario { tier: 1, depth_bb: 100, opener: Utg, caller: Bb, three_bettor: None },
        // Tier 2: 4 more SRP lines, 100bb.
        Scenario { tier: 2, depth_bb: 100, opener: Sb, caller: Bb, three_bettor: None },
        Scenario { tier: 2, depth_bb: 100, opener: Btn, caller: Sb, three_bettor: None },
        Scenario { tier: 2, depth_bb: 100, opener: Co, caller: Btn, three_bettor: None },
        Scenario { tier: 2, depth_bb: 100, opener: Hj, caller: Btn, three_bettor: None },
        // Tier 2: 4 3-bet lines, 100bb (`caller` is the original opener calling the 3-bet).
        Scenario { tier: 2, depth_bb: 100, opener: Btn, caller: Btn, three_bettor: Some(Bb) },
        Scenario { tier: 2, depth_bb: 100, opener: Co, caller: Co, three_bettor: Some(Btn) },
        Scenario { tier: 2, depth_bb: 100, opener: Btn, caller: Btn, three_bettor: Some(Sb) },
        Scenario { tier: 2, depth_bb: 100, opener: Hj, caller: Hj, three_bettor: Some(Btn) },
        // Tier 3: the same twelve lines again, at 200bb.
        Scenario { tier: 3, depth_bb: 200, opener: Btn, caller: Bb, three_bettor: None },
        Scenario { tier: 3, depth_bb: 200, opener: Co, caller: Bb, three_bettor: None },
        Scenario { tier: 3, depth_bb: 200, opener: Hj, caller: Bb, three_bettor: None },
        Scenario { tier: 3, depth_bb: 200, opener: Utg, caller: Bb, three_bettor: None },
        Scenario { tier: 3, depth_bb: 200, opener: Sb, caller: Bb, three_bettor: None },
        Scenario { tier: 3, depth_bb: 200, opener: Btn, caller: Sb, three_bettor: None },
        Scenario { tier: 3, depth_bb: 200, opener: Co, caller: Btn, three_bettor: None },
        Scenario { tier: 3, depth_bb: 200, opener: Hj, caller: Btn, three_bettor: None },
        Scenario { tier: 3, depth_bb: 200, opener: Btn, caller: Btn, three_bettor: Some(Bb) },
        Scenario { tier: 3, depth_bb: 200, opener: Co, caller: Co, three_bettor: Some(Btn) },
        Scenario { tier: 3, depth_bb: 200, opener: Btn, caller: Btn, three_bettor: Some(Sb) },
        Scenario { tier: 3, depth_bb: 200, opener: Hj, caller: Hj, three_bettor: Some(Btn) },
    ]
}

/// `Scenario::id()` for every entry of `expected_scenarios()`, in the same order, computed by
/// hand against the format strings in `crates/cache/src/presolver/scenarios.rs` rather than by
/// calling `.id()` on the fixture itself, so a change to the format strings cannot silently
/// agree with itself.
fn expected_ids() -> Vec<&'static str> {
    vec![
        "t1-100bb-Btn-open-Bb-call",
        "t1-100bb-Co-open-Bb-call",
        "t1-100bb-Hj-open-Bb-call",
        "t1-100bb-Utg-open-Bb-call",
        "t2-100bb-Sb-open-Bb-call",
        "t2-100bb-Btn-open-Sb-call",
        "t2-100bb-Co-open-Btn-call",
        "t2-100bb-Hj-open-Btn-call",
        "t2-100bb-Btn-open-Bb-3bet-Btn-call",
        "t2-100bb-Co-open-Btn-3bet-Co-call",
        "t2-100bb-Btn-open-Sb-3bet-Btn-call",
        "t2-100bb-Hj-open-Btn-3bet-Hj-call",
        "t3-200bb-Btn-open-Bb-call",
        "t3-200bb-Co-open-Bb-call",
        "t3-200bb-Hj-open-Bb-call",
        "t3-200bb-Utg-open-Bb-call",
        "t3-200bb-Sb-open-Bb-call",
        "t3-200bb-Btn-open-Sb-call",
        "t3-200bb-Co-open-Btn-call",
        "t3-200bb-Hj-open-Btn-call",
        "t3-200bb-Btn-open-Bb-3bet-Btn-call",
        "t3-200bb-Co-open-Btn-3bet-Co-call",
        "t3-200bb-Btn-open-Sb-3bet-Btn-call",
        "t3-200bb-Hj-open-Btn-3bet-Hj-call",
    ]
}

#[test]
fn tier_scenarios_and_flop_order_are_complete() {
    let s = scenarios();
    let expected = expected_scenarios();
    assert_eq!(s.len(), 24);
    assert_eq!(s, expected, "scenarios() must equal the frozen 24-entry fixture, in order");

    assert_eq!(s.iter().filter(|x| x.tier == 1).count(), 4);
    assert_eq!(s.iter().filter(|x| x.tier == 2).count(), 8);
    assert_eq!(s.iter().filter(|x| x.tier == 3).count(), 12);

    let ids: Vec<String> = s.iter().map(Scenario::id).collect();
    let expected_ids: Vec<String> = expected_ids().into_iter().map(String::from).collect();
    assert_eq!(ids, expected_ids, "Scenario::id() must equal the frozen id strings, in order");

    let mut sorted_ids = ids.clone();
    sorted_ids.sort();
    sorted_ids.dedup();
    assert_eq!(sorted_ids.len(), 24, "every scenario id must be unique: {ids:?}");

    // The cheap half of the contract: the count is asserted from the frozen constant, and the
    // expensive enumeration runs only under `--features exhaustive` (review m5).
    assert_eq!(CANONICAL_FLOP_COUNT, 1755);
    assert_eq!(1755 * 4, 7020);
    assert_eq!(1755 * 24, 42120);
}

// Deviation (orchestrator pre-flight ruling, plan-mandated): the brief's Step 1 draft calls
// `core_iso::orbit_size(b)` on `b: &Vec<proto::Card>`, which does not compile because
// `core_iso::orbit_size` takes `&CanonicalBoard`. This uses `core_iso::orbit_size_of(b)` instead
// (`pub fn orbit_size_of(board: &[Card]) -> u8` in `crates/core-iso/src/lib.rs`), which accepts
// the `&[Card]` slice `canonical_flops_ordered()` actually produces.
//
// Fix round 1 (review R2): kept as an unconditional `#[test]` so it type-checks in the default
// build (`cargo test --workspace --locked` compiles it); only its *execution* is gated, via
// `ignore`, behind the `exhaustive` feature -- `cargo test -p cache --features exhaustive` runs
// it as before.
#[test]
#[cfg_attr(not(feature = "exhaustive"), ignore = "enable the exhaustive feature for the full enumeration")]
fn canonical_flop_enumeration_matches_the_frozen_count() {
    let f = canonical_flops_ordered();
    assert_eq!(f.len(), CANONICAL_FLOP_COUNT);
    assert_eq!(raw_flops().count(), 22_100);

    // Fix round 1 (review R1): every canonical board is exactly 3 distinct, ascending cards; is
    // its own canonical representative (canonicalizing it again reproduces it bit-for-bit); and
    // appears exactly once across the whole ordered list.
    let mut seen = std::collections::BTreeSet::new();
    for board in f {
        assert_eq!(board.len(), 3, "canonical flop board must have exactly 3 cards: {board:?}");
        assert!(
            board[0] < board[1] && board[1] < board[2],
            "canonical flop cards must be strictly ascending by id: {board:?}"
        );
        let (canonical, _) = core_iso::canonicalize(board, &[]);
        assert_eq!(
            canonical.cards(),
            board.as_slice(),
            "board is not its own canonical representative: {board:?}"
        );
        assert!(seen.insert(board.clone()), "duplicate canonical board in the ordered list: {board:?}");
    }
    assert_eq!(seen.len(), CANONICAL_FLOP_COUNT);

    // Fix round 1 (review R1): the ordering key the brief defines -- descending orbit size
    // primary, ascending canonical card-id key secondary -- not just "non-increasing orbit".
    let orbits = f.iter().map(|b| core_iso::orbit_size_of(b)).collect::<Vec<_>>();
    assert!(orbits.windows(2).all(|w| w[0] >= w[1]), "orbit 24 before 12 before 4");
    for i in 0..f.len().saturating_sub(1) {
        if orbits[i] == orbits[i + 1] {
            assert!(
                f[i] < f[i + 1],
                "boards with equal orbit size must be in ascending canonical-card order: {:?} then {:?}",
                f[i], f[i + 1]
            );
        }
    }
    assert_eq!(orbits.iter().map(|&o| o as usize).sum::<usize>(), 22_100);
}

// ---------------------------------------------------------------------------------------------
// Task 14 (spec section 10.5): the durable `queue.json` -- deterministic identities, the saved
// cursor, retries, cancellation, restart, and completion verified by reading the entry back.
//
// Every filesystem test uses the shared per-invocation `TempDir` (removed on unwind too). The
// queue carries all 42,120 (tier, canonical flop, scenario) items, so tests look individual items
// up in the queue's own saved `queue.json` (`saved`), never in a hand-written copy of it.
// ---------------------------------------------------------------------------------------------

use cache::entry::CacheEntry;
use cache::key::Rational;
use cache::presolver::queue::{self, Queue, QueueFile, QueueItem, TaskStatus};
use cache::storage;
use cache::{Cache, CacheError, CACHE_QUOTA_BYTES};
use proto::Card;
use std::collections::BTreeSet;
use std::path::Path;
use temp_dir::TempDir;

/// The pre-solver's target (spec section 10.5: `target_bp` 50, i.e. raw `<= 0.005`).
const TARGET_BP: u16 = 50;
/// A failure budget for the cache writer thread, never a synchronization delay.
const WRITER_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);
/// Every item the frozen enumeration produces: 1,755 canonical flops x 24 scenarios.
const ALL: u32 = 42_120;

fn open(dir: &Path) -> Queue {
    Queue::open(dir.to_path_buf()).unwrap()
}

/// The queue file as `Queue::save` last published it.
fn saved(dir: &Path) -> QueueFile {
    serde_json::from_slice(&std::fs::read(queue::queue_path(dir)).unwrap()).unwrap()
}

/// The scenarios of tier `t`, in their frozen order.
fn tier(t: u8) -> Vec<Scenario> {
    scenarios().into_iter().filter(|s| s.tier == t).collect()
}

fn is(item: &QueueItem, scenario: &Scenario, board: &[Card]) -> bool {
    item.scenario == *scenario && item.board == board
}

/// The queue item for `scenario` on `board`, from the queue's own freshly saved file.
fn item_for(q: &Queue, dir: &Path, scenario: &Scenario, board: &[Card]) -> QueueItem {
    q.save().unwrap();
    saved(dir)
        .items
        .into_values()
        .find(|i| is(i, scenario, board))
        .unwrap_or_else(|| panic!("no queue item for {} on {board:?}", scenario.id()))
}

/// Launches and completes `item`, then advances the cursor -- the scheduler's own sequence.
fn complete(q: &mut Queue, item: &QueueItem) {
    q.record_launch(&item.identity_hex());
    q.record_done(&item.identity_hex());
    q.advance_cursor();
}

/// The shared fixture entry: its board (`Kh7d2c` canonicalized) is one of the 1,755 canonical
/// flops, so it stands in for task 16's completed presolve of that board's tier-1 BTN/BB item.
fn fixture() -> (CacheEntry, QueueItem, Queue, TempDir) {
    let dir = TempDir::new("queue-fixture");
    let entry = support::entry();
    let q = open(dir.path());
    let item = item_for(&q, dir.path(), &tier(1)[0], &entry.key.canonical_board);
    (entry, item, q, dir)
}

/// Brief step 1, verbatim.
#[test]
fn presolver_done_requires_a_valid_entry() {
    use cache::presolver::queue::TaskStatus;
    let mut status=TaskStatus::Done;
    cache::presolver::queue::reconcile_status(&mut status,false);
    assert!(matches!(status,TaskStatus::Pending));
    cache::presolver::queue::reconcile_status(&mut status,true);
    assert!(matches!(status,TaskStatus::Done));
}

/// Brief step 1, verbatim except where the directory comes from: the shared `TempDir` (unique
/// per call, removed even when an assertion unwinds) replaces the brief's fixed
/// `pokerai-queue-<pid>` path and its manual create/remove (standing ruling: filesystem tests use
/// unique temp dirs and clean up).
#[test]
fn queue_reopens_at_the_saved_cursor_without_repeating_done_work() {
    let tmp=TempDir::new("queue-reopen");
    let dir=tmp.path().to_path_buf();
    let mut q=cache::presolver::queue::Queue::open(dir.clone()).unwrap();
    let first=q.next_pending(0).unwrap();
    q.record_launch(&first.identity_hex());q.record_done(&first.identity_hex());
    q.advance_cursor();q.save().unwrap();
    let mut reopened=cache::presolver::queue::Queue::open(dir.clone()).unwrap();
    let next=reopened.next_pending(0).unwrap();
    assert_ne!(next.identity_hex(),first.identity_hex());
    // a missing cache cell demotes Done back to Pending on reconciliation
    reopened.reconcile(&|_item|false);
    assert!(reopened.next_pending(0).is_some());
    let (pending,done,failed)=reopened.status_counts();
    assert_eq!((done,failed),(0,0));assert!(pending>0);
}

/// Brief step 5, verbatim.
#[test]
fn three_retries_follow_initial_attempt() {
    use cache::presolver::queue::retry_delay;
    assert_eq!(retry_delay(1).unwrap().as_secs(),30);
    assert!(retry_delay(3).is_some());assert!(retry_delay(4).is_none());
}

/// The initial attempt plus three retries: a 30 s delay follows failures 1, 2 and 3 and none
/// follows the fourth, for every attempt count a `u8` can hold.
#[test]
fn retry_delay_is_thirty_seconds_through_the_third_failure_and_none_after() {
    for attempts in 0..=u8::MAX {
        let expected = (attempts <= 3).then_some(std::time::Duration::from_secs(30));
        assert_eq!(queue::retry_delay(attempts), expected, "attempts {attempts}");
    }
    assert_eq!(queue::MAX_ATTEMPTS, 4);
    assert_eq!(queue::RETRY_BACKOFF_MS, 30_000);
}

/// A rebuild is the frozen enumeration: one Pending item per (scenario, canonical flop), keyed
/// by `identity_hex()`, identities unique and independent of when the rebuild ran, and two
/// rebuilds publish byte-identical files.
#[test]
fn the_rebuilt_queue_is_the_frozen_enumeration_keyed_by_deterministic_identities() {
    let (a, b) = (TempDir::new("queue-rebuild-a"), TempDir::new("queue-rebuild-b"));
    let qa = open(a.path());
    assert_eq!(qa.cursor(), [0, 0, 0]);
    assert!(!qa.paused());
    assert_eq!(qa.status_counts(), (ALL, 0, 0));
    assert_eq!(qa.tier_counts(), ([0, 0, 0], [7_020, 14_040, 21_060]));
    qa.save().unwrap();
    open(b.path()).save().unwrap();
    let bytes = std::fs::read(queue::queue_path(a.path())).unwrap();
    assert_eq!(bytes, std::fs::read(queue::queue_path(b.path())).unwrap(), "two rebuilds must publish byte-identical files");
    assert_eq!(queue::queue_path(a.path()), a.path().join("queue.json"));

    let file: QueueFile = serde_json::from_slice(&bytes).unwrap();
    assert_eq!((file.version, file.cursor, file.paused), (1, [0, 0, 0], false));
    assert_eq!(file.items.len(), ALL as usize);
    let boards: BTreeSet<Vec<Card>> = canonical_flops_ordered().iter().cloned().collect();
    let ids: BTreeSet<String> = scenarios().iter().map(Scenario::id).collect();
    let (mut identities, mut slots) = (BTreeSet::new(), BTreeSet::new());
    for (key, item) in &file.items {
        assert_eq!(*key, item.identity_hex(), "items are keyed by their identity");
        assert_eq!(key.len(), 64);
        assert!(key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{key}");
        assert!(identities.insert(item.identity), "duplicate identity {key}");
        assert!(boards.contains(&item.board), "{:?} is not a canonical flop", item.board);
        assert!(ids.contains(&item.scenario.id()));
        assert!(slots.insert((item.scenario.id(), item.board.clone())), "one item per (scenario, canonical flop)");
        assert_eq!(item.spr, Rational::new(item.scenario.depth_bb as u64, 1).unwrap(), "the nominal scenario SPR");
        assert_eq!(item.status, TaskStatus::Pending);
        assert_eq!((item.attempts, item.retry_after_unix_ms, item.last_error.as_deref()), (0, 0, None));
    }
    assert_eq!(slots.len(), 24 * CANONICAL_FLOP_COUNT);
}

/// Brief step 5: the queue walks (tier, canonical flop, scenario) -- every scenario of a tier on
/// a board before the next board -- and a cursor saved after tier 1, board 0, scenario 2 reopens
/// there: the next job is scenario 2 and the completed earlier entries are not repeated.
#[test]
fn the_queue_walks_boards_with_all_tier_scenarios_and_resumes_at_the_saved_cursor() {
    let tmp = TempDir::new("queue-cursor");
    let flops = canonical_flops_ordered();
    let t1 = tier(1);
    let mut q = open(tmp.path());
    let mut done = Vec::new();
    for scenario in &t1[..2] {
        let item = q.next_pending(0).unwrap();
        assert!(is(&item, scenario, &flops[0]), "expected {} on board 0, got {} on {:?}", scenario.id(), item.scenario.id(), item.board);
        complete(&mut q, &item);
        done.push(item.identity_hex());
    }
    assert_eq!(q.cursor(), [0, 0, 2]);
    q.set_paused(true);
    q.save().unwrap();

    let mut reopened = open(tmp.path());
    assert_eq!(reopened.cursor(), [0, 0, 2], "the saved cursor");
    assert!(reopened.paused(), "the saved pause flag");
    assert_eq!(reopened.status_counts(), (ALL - 2, 2, 0));
    let next = reopened.next_pending(0).unwrap();
    assert!(is(&next, &t1[2], &flops[0]), "the next job is tier 1, board 0, scenario 2");
    assert!(!done.contains(&next.identity_hex()), "completed work is not repeated");
    for scenario in &t1[2..] {
        let item = reopened.next_pending(0).unwrap();
        assert!(is(&item, scenario, &flops[0]));
        complete(&mut reopened, &item);
        done.push(item.identity_hex());
    }
    assert_eq!(reopened.cursor(), [0, 1, 0], "board 0's four tier-1 scenarios are done together");
    assert!(is(&reopened.next_pending(0).unwrap(), &t1[0], &flops[1]));
    reopened.save().unwrap();
    let file = saved(tmp.path());
    for id in &done {
        assert_eq!((&file.items[id].status, file.items[id].attempts), (&TaskStatus::Done, 1), "{id}");
    }
}

/// Tier 1 finishes before tier 2 starts; the cursor skips work reconciliation already verified;
/// and a completed item whose entry disappears is revisited ahead of the sweep, while the sweep
/// cursor itself stays put.
#[test]
fn a_tier_finishes_before_the_next_and_invalidated_work_is_revisited_first() {
    let tmp = TempDir::new("queue-tiers");
    let flops = canonical_flops_ordered();
    let mut q = open(tmp.path());
    q.reconcile(&|item| item.scenario.tier == 1);
    assert_eq!(q.tier_counts(), ([7_020, 0, 0], [7_020, 14_040, 21_060]));
    assert!(is(&q.next_pending(0).unwrap(), &tier(2)[0], &flops[0]), "tier 2 starts on board 0 once tier 1 is done");
    q.advance_cursor();
    assert_eq!(q.cursor(), [1, 0, 0], "the cursor steps over verified work instead of repeating it");

    let (scenario, board) = (tier(1)[3].clone(), flops[7].clone());
    q.reconcile(&|item| item.scenario.tier == 1 && !is(item, &scenario, &board));
    assert_eq!(q.tier_counts().0, [7_019, 0, 0], "the evicted entry demotes exactly its item");
    let revisit = q.next_pending(0).unwrap();
    assert!(is(&revisit, &scenario, &board), "an invalidated tier-1 item runs before tier 2 continues");
    complete(&mut q, &revisit);
    assert_eq!(q.cursor(), [1, 0, 0], "a revisit does not move the sweep cursor");
    assert!(is(&q.next_pending(0).unwrap(), &tier(2)[0], &flops[0]));
    let item = q.next_pending(0).unwrap();
    complete(&mut q, &item);
    assert_eq!(q.cursor(), [1, 0, 1]);
}

/// Past the last item the cursor sits at the explicit `CURSOR_END` sentinel: it never wraps back
/// to the start of tier 3, and the sentinel survives a save and reopen.
#[test]
fn the_cursor_stops_at_an_explicit_end_and_never_wraps_back() {
    let tmp = TempDir::new("queue-end");
    let mut q = open(tmp.path());
    q.reconcile(&|_| true);
    assert_eq!(q.status_counts(), (0, ALL, 0));
    assert!(q.next_pending(0).is_none());
    assert_eq!(queue::CURSOR_END, [2, CANONICAL_FLOP_COUNT, 0]);
    q.advance_cursor();
    assert_eq!(q.cursor(), queue::CURSOR_END);
    q.advance_cursor();
    assert_eq!(q.cursor(), queue::CURSOR_END, "no wrap back to tier 3's first board");
    q.save().unwrap();
    let reopened = open(tmp.path());
    assert_eq!((reopened.cursor(), reopened.status_counts()), (queue::CURSOR_END, (0, ALL, 0)));
}

/// Brief step 5 "delete its cache cell then reconcile: Pending": completion is decided by reading
/// the stored entry back (`entry_verified`), never by the writer's receipt; a deleted cell demotes
/// Done back to Pending with a fresh budget.
#[test]
fn done_is_decided_by_reading_the_entry_back_and_a_deleted_cell_demotes_it() {
    let (entry, item, mut q, dir) = fixture();
    let root = dir.path().to_path_buf();
    let (key, spr) = (entry.key.clone(), entry.source.spr);
    let id = item.identity_hex();
    let verified = |it: &QueueItem| it.identity == item.identity && queue::entry_verified(&root, it, &key, spr, TARGET_BP);
    assert!(!queue::entry_verified(&root, &item, &key, spr, TARGET_BP), "nothing is stored yet");

    let cache = Cache::open(root.clone(), CACHE_QUOTA_BYTES);
    q.record_launch(&id);
    assert!(cache.store_tracked(&entry).wait(WRITER_BUDGET), "the writer confirms the store");
    assert!(queue::entry_verified(&root, &item, &key, spr, TARGET_BP), "the entry reads back valid at target");
    q.record_done(&id);
    q.reconcile(&verified);
    assert_eq!(q.status_counts(), (ALL - 1, 1, 0));

    std::fs::remove_file(storage::entry_path(&root, key.digest())).unwrap();
    assert!(!queue::entry_verified(&root, &item, &key, spr, TARGET_BP));
    q.reconcile(&verified);
    assert_eq!(q.status_counts(), (ALL, 0, 0), "a deleted cell moves Done back to Pending");
    q.save().unwrap();
    let demoted = &saved(&root).items[&id];
    assert_eq!((&demoted.status, demoted.attempts, demoted.retry_after_unix_ms), (&TaskStatus::Pending, 0, 0));
    cache.shutdown();
}

/// A confirmed store is not a completion: an entry that reads back above target in raw accuracy,
/// an entry for another board, and a corrupt cell all fail verification.
#[test]
fn a_confirmed_store_above_target_another_board_or_a_corrupt_cell_is_not_done() {
    let (mut entry, item, mut q, dir) = fixture();
    let root = dir.path().to_path_buf();
    entry.exploitability_over_P = 0.0051; // raw 51 bp: above the 50 bp target, never rounded
    let (key, spr) = (entry.key.clone(), entry.source.spr);
    let cache = Cache::open(root.clone(), CACHE_QUOTA_BYTES);
    assert!(cache.store_tracked(&entry).wait(WRITER_BUDGET), "the writer confirms the store");
    assert!(!queue::entry_verified(&root, &item, &key, spr, TARGET_BP), "stored, but not at target");
    assert!(queue::entry_verified(&root, &item, &key, spr, 52), "the same entry passes a looser raw target");
    q.reconcile(&|it| it.identity == item.identity && queue::entry_verified(&root, it, &key, spr, TARGET_BP));
    assert_eq!(q.status_counts(), (ALL, 0, 0));

    let elsewhere = item_for(&q, &root, &tier(1)[0], &canonical_flops_ordered()[0]);
    assert_ne!(elsewhere.board, key.canonical_board);
    assert!(!queue::entry_verified(&root, &elsewhere, &key, spr, 52), "an entry never completes an item on another board");

    let path = storage::entry_path(&root, key.digest());
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(!queue::entry_verified(&root, &item, &key, spr, 52), "a corrupt cell is not an entry");
    cache.shutdown();
}

/// Brief step 5 "replace the source bundle hash": new ranges are a new normalized identity (the
/// cache key without its bucket plus the exact SPR), so completion under the old bundle does not
/// carry over, while the old bundle's entry stays on disk, untouched, serving only its own
/// identity.
#[test]
fn a_new_source_bundle_is_a_new_identity_and_old_entries_stay_isolated() {
    let (entry, item, mut q, dir) = fixture();
    let root = dir.path().to_path_buf();
    let spr = entry.source.spr;
    let old = entry.key.clone();
    let mut new = old.clone();
    let hash = core_ranges::hash_scaled(&core_ranges::parse_range("KK").unwrap());
    new.range_hash_oop = hash;
    new.range_hash_ip = hash;
    assert_ne!(old.scenario_identity(spr), new.scenario_identity(spr), "a new bundle hash is a new identity");
    assert_ne!(old.digest(), new.digest());

    let cache = Cache::open(root.clone(), CACHE_QUOTA_BYTES);
    assert!(cache.store_tracked(&entry).wait(WRITER_BUDGET));
    let under = |key: &cache::key::KeyFields| {
        let key = key.clone();
        let (root, target) = (root.clone(), item.identity);
        move |it: &QueueItem| it.identity == target && queue::entry_verified(&root, it, &key, spr, TARGET_BP)
    };
    q.reconcile(&under(&old));
    assert_eq!(q.status_counts(), (ALL - 1, 1, 0), "done under the old bundle");
    q.reconcile(&under(&new));
    assert_eq!(q.status_counts(), (ALL, 0, 0), "not done under the new bundle");
    assert!(queue::entry_verified(&root, &item, &old, spr, TARGET_BP), "the old entry still serves its own identity");
    assert_eq!(storage::read_cell(&storage::entry_path(&root, old.digest())).unwrap().entries.len(), 1);
    assert!(!storage::entry_path(&root, new.digest()).exists(), "nothing was written under the new identity");
    cache.shutdown();
}

/// Brief step 5: a crash before the entry rename, after it, or in the middle of the queue save
/// never marks the item Done on its own. Done is only ever re-established by reading the entry
/// back; the interrupted launch is refunded on restart.
#[test]
fn a_crash_around_the_entry_rename_or_the_queue_save_never_marks_done() {
    let (entry, item, mut q, dir) = fixture();
    let root = dir.path().to_path_buf();
    let (key, spr) = (entry.key.clone(), entry.source.spr);
    let id = item.identity_hex();
    let verified = |it: &QueueItem| it.identity == item.identity && queue::entry_verified(&root, it, &key, spr, TARGET_BP);
    let cell = storage::entry_path(&root, key.digest());
    let bytes = storage::encode(&storage::Cell { entries: vec![entry.clone()] }).unwrap();

    // 1. Killed before the rename: only write_atomic's fsynced temp sibling exists.
    q.record_launch(&id);
    q.save().unwrap();
    std::fs::create_dir_all(cell.parent().unwrap()).unwrap();
    std::fs::write(cell.with_extension("4242.0.tmp"), &bytes).unwrap();
    drop(q);
    let mut restarted = open(&root);
    assert_eq!(restarted.status_counts(), (ALL, 0, 0));
    restarted.reconcile(&verified);
    assert_eq!(restarted.status_counts(), (ALL, 0, 0), "a temp file is not a stored entry");
    restarted.save().unwrap();
    let refunded = &saved(&root).items[&id];
    assert_eq!((&refunded.status, refunded.attempts), (&TaskStatus::Pending, 0), "the interrupted launch is refunded");

    // 2. Killed after the rename, before the queue save: queue.json still says "launched".
    restarted.record_launch(&id);
    restarted.save().unwrap();
    storage::write_atomic(&cell, &bytes).unwrap();
    drop(restarted);
    let mut restarted = open(&root);
    assert_eq!(restarted.status_counts(), (ALL, 0, 0), "the queue file alone never claims Done");
    restarted.reconcile(&verified);
    assert_eq!(restarted.status_counts(), (ALL - 1, 1, 0), "Done only because the entry reads back valid at target");

    // 3. Killed in the middle of the queue save: a torn queue.json is rejected and rebuilt, and
    //    completion comes back from the cache, not from the torn file.
    restarted.save().unwrap();
    let whole = std::fs::read(queue::queue_path(&root)).unwrap();
    std::fs::write(queue::queue_path(&root), &whole[..whole.len() / 2]).unwrap();
    let mut rebuilt = open(&root);
    assert_eq!((rebuilt.cursor(), rebuilt.status_counts()), ([0, 0, 0], (ALL, 0, 0)));
    rebuilt.reconcile(&verified);
    assert_eq!(rebuilt.status_counts(), (ALL - 1, 1, 0));
}

/// Brief step 5: failures 1, 2 and 3 retry after 30 s; the fourth failure is terminal, and stays
/// terminal across a restart.
#[test]
fn failed_attempts_one_to_three_retry_after_thirty_seconds_and_the_fourth_is_terminal() {
    let tmp = TempDir::new("queue-retry");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();
    let mut now = 1_000_000_u64;
    for n in 1..=3_u8 {
        assert_eq!(q.next_pending(now).unwrap().identity_hex(), id, "attempt {n} is the earliest launchable item");
        q.record_launch(&id);
        q.record_failure(&id, now, format!("worker error {n}"));
        assert_eq!(q.status_counts(), (ALL - 1, 0, 1));
        assert_ne!(q.next_pending(now + 29_999).unwrap().identity_hex(), id, "failure {n} waits 30 s");
        let due = q.next_pending(now + 30_000).unwrap();
        assert_eq!(due.identity_hex(), id, "failure {n} retries at 30 s");
        assert_eq!((due.status, due.attempts, due.retry_after_unix_ms), (TaskStatus::Failed { n }, n, now + 30_000));
        assert_eq!(due.last_error, Some(format!("worker error {n}")));
        now += 30_000;
    }
    q.record_launch(&id);
    q.record_failure(&id, now, "worker error 4".into());
    for later in [now, now + 30_000, now + 86_400_000] {
        assert_ne!(q.next_pending(later).unwrap().identity_hex(), id, "the fourth failure is terminal");
    }
    q.save().unwrap();
    let terminal = &saved(tmp.path()).items[&id];
    assert_eq!((&terminal.status, terminal.attempts, terminal.retry_after_unix_ms), (&TaskStatus::Failed { n: 4 }, 4, u64::MAX));
    let reopened = open(tmp.path());
    assert_eq!(reopened.status_counts(), (ALL - 1, 0, 1));
    assert_ne!(reopened.next_pending(u64::MAX).unwrap().identity_hex(), id);
}

/// Brief step 5: a live-work cancellation and a restart both refund the interrupted attempt, so
/// an item still gets exactly four real attempts.
#[test]
fn cancellation_and_restart_do_not_burn_retries() {
    let tmp = TempDir::new("queue-cancel");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();

    q.record_launch(&id);
    q.record_cancel(&id);
    let again = q.next_pending(0).unwrap();
    assert_eq!((again.identity_hex(), again.status, again.attempts), (id.clone(), TaskStatus::Pending, 0), "a cancelled first attempt costs nothing");

    let mut now = 0_u64;
    q.record_launch(&id);
    q.record_failure(&id, now, "error 1".into());
    now += 30_000;
    q.record_launch(&id);
    q.record_cancel(&id);
    let again = q.next_pending(now).unwrap();
    assert_eq!((again.identity_hex(), again.status, again.attempts), (id.clone(), TaskStatus::Failed { n: 1 }, 1), "a cancelled retry is still just one failure, still due");

    q.record_launch(&id);
    q.save().unwrap(); // persisted before launch; then the process dies mid-solve
    drop(q);
    let mut q = open(tmp.path());
    let again = q.next_pending(now).unwrap();
    assert_eq!((again.identity_hex(), again.status, again.attempts), (id.clone(), TaskStatus::Failed { n: 1 }, 1), "a restart refunds the interrupted retry");

    for n in 2..=4_u8 {
        assert_eq!(q.next_pending(now).unwrap().identity_hex(), id, "attempt {n} is still available");
        q.record_launch(&id);
        q.record_failure(&id, now, format!("error {n}"));
        now += 30_000;
    }
    assert_ne!(q.next_pending(now).unwrap().identity_hex(), id, "four real failures, then terminal");
    q.save().unwrap();
    let item = &saved(tmp.path()).items[&id];
    assert_eq!((&item.status, item.attempts), (&TaskStatus::Failed { n: 4 }, 4));
}

/// The backoff runs on the caller's monotonic clock; a persisted deadline cannot stall the queue
/// by more than one backoff when the clock jumps backwards or restarts near zero.
#[test]
fn a_clock_jump_or_a_restart_cannot_stall_a_retry_beyond_thirty_seconds() {
    let tmp = TempDir::new("queue-clock");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();
    let t = 5_000_000_000_u64;
    q.record_launch(&id);
    q.record_failure(&id, t, "error".into());
    assert_ne!(q.next_pending(t + 29_999).unwrap().identity_hex(), id);
    assert_eq!(q.next_pending(t + 30_000).unwrap().identity_hex(), id);
    assert_eq!(q.next_pending(t - 3_600_000).unwrap().identity_hex(), id, "a clock that jumped back an hour does not wait an hour");
    q.save().unwrap();
    let reopened = open(tmp.path());
    assert_eq!(reopened.next_pending(12).unwrap().identity_hex(), id, "a restarted monotonic clock does not wait for the old deadline");
    assert_ne!(reopened.next_pending(t + 1).unwrap().identity_hex(), id, "a deadline within one backoff is honoured");
}

/// The retry deadline is computed wide and checked against the millisecond ceiling: exactly at
/// the ceiling it is stored and reloads; one past it is refused loudly, never saturated.
#[test]
fn a_retry_deadline_at_the_millisecond_ceiling_is_kept() {
    let tmp = TempDir::new("queue-ceiling");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, queue::RETRY_DEADLINE_MAX_MS - 30_000, "error".into());
    q.save().unwrap();
    assert_eq!(saved(tmp.path()).items[&id].retry_after_unix_ms, i64::MAX as u64);
    assert_eq!(open(tmp.path()).status_counts(), (ALL - 1, 0, 1), "the ceiling itself reloads");
}

#[test]
#[should_panic(expected = "retry deadline")]
fn a_retry_deadline_past_the_millisecond_ceiling_is_refused_loudly() {
    let tmp = TempDir::new("queue-past-ceiling");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, queue::RETRY_DEADLINE_MAX_MS - 29_999, "error".into());
}

/// Every item failing four times with an error far past the stored bound, every character of it
/// needing a JSON escape, is the largest file the queue can ever publish: it still fits the
/// 64 MiB bound, so a save can never be locked out, and it reopens intact.
#[test]
fn the_largest_possible_queue_file_still_fits_the_bound_and_reopens() {
    let tmp = TempDir::new("queue-largest");
    let mut q = open(tmp.path());
    q.save().unwrap();
    let ids: Vec<String> = saved(tmp.path()).items.into_keys().collect();
    let worst = "\"\\".repeat(300);
    for id in &ids {
        for _ in 0..4 {
            q.record_launch(id);
            q.record_failure(id, 0, worst.clone());
        }
    }
    q.set_paused(true);
    q.save().expect("the worst-case queue file must stay within the bound");
    let len = std::fs::metadata(queue::queue_path(tmp.path())).unwrap().len();
    assert!(len <= queue::QUEUE_FILE_MAX, "{len} bytes");
    let file = saved(tmp.path());
    let stored = file.items[&ids[0]].last_error.clone().unwrap();
    assert!(stored.len() <= queue::LAST_ERROR_MAX_BYTES && !stored.chars().any(char::is_control), "{stored:?}");
    let reopened = open(tmp.path());
    assert_eq!((reopened.paused(), reopened.status_counts()), (true, (0, 0, ALL)));
}

/// A failure message is stored on one line (control characters become spaces) and within
/// `LAST_ERROR_MAX_BYTES`, cut on a character boundary and marked as cut.
#[test]
fn a_failure_message_is_stored_on_one_line_within_its_bound() {
    let tmp = TempDir::new("queue-error");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, 0, "solver\nexited\u{7}".into());
    assert_eq!(q.next_pending(30_000).unwrap().last_error.as_deref(), Some("solver exited "));
    q.record_launch(&id);
    q.record_failure(&id, 30_000, "\u{e9}".repeat(400));
    let kept = (queue::LAST_ERROR_MAX_BYTES - 3) / 2;
    assert_eq!(q.next_pending(60_000).unwrap().last_error, Some(format!("{}...", "\u{e9}".repeat(kept))));
}

/// The standing ruling: a corrupt, truncated or tampered queue file is rejected whole and rebuilt
/// from the frozen enumeration -- never partially trusted.
#[test]
fn a_corrupt_truncated_or_tampered_queue_file_is_rebuilt_never_partially_trusted() {
    use serde_json::{json, Value};
    let tmp = TempDir::new("queue-tamper");
    let path = queue::queue_path(tmp.path());
    let mut q = open(tmp.path());
    let first = q.next_pending(0).unwrap();
    complete(&mut q, &first);
    let failed = q.next_pending(0).unwrap();
    q.record_launch(&failed.identity_hex());
    q.record_failure(&failed.identity_hex(), 0, "error".into());
    q.advance_cursor();
    q.set_paused(true);
    q.save().unwrap();
    let good = std::fs::read(&path).unwrap();
    let progress = |q: &Queue| (q.cursor(), q.paused(), q.status_counts());
    assert_eq!(progress(&open(tmp.path())), ([0, 0, 2], true, (ALL - 2, 1, 1)), "the untampered file keeps its progress");

    /// The items a tampering case may reach: one done, one failed (attempt 1, retry due at 30 s)
    /// and one untouched pending item, plus values guaranteed to differ from the pending item's.
    struct Keys {
        done: String,
        failed: String,
        pending: String,
        other_board: Value,
        other_depth: u64,
    }
    let value: Value = serde_json::from_slice(&good).unwrap();
    let (done, failed) = (first.identity_hex(), failed.identity_hex());
    let pending = value["items"].as_object().unwrap().keys().find(|k| **k != done && **k != failed).unwrap().clone();
    let pending_board: Vec<Card> = serde_json::from_value(value["items"][&pending]["board"].clone()).unwrap();
    let other_board = serde_json::to_value(canonical_flops_ordered().iter().find(|b| **b != pending_board).unwrap()).unwrap();
    let other_depth = value["items"][&pending]["scenario"]["depth_bb"].as_u64().unwrap() + 1;
    let keys = Keys { done, failed, pending, other_board, other_depth };

    let untouched = serde_json::to_vec(&value).unwrap();
    std::fs::write(&path, &untouched).unwrap();
    assert_eq!(progress(&open(tmp.path())), ([0, 0, 2], true, (ALL - 2, 1, 1)), "the harness's own re-encoding is accepted");

    let text = String::from_utf8(untouched.clone()).unwrap();
    let entry = format!("\"{}\":{}", keys.done, serde_json::to_string(&value["items"][&keys.done]).unwrap());
    let duplicated = text.replacen("\"items\":{", &format!("\"items\":{{{entry},"), 1);
    let raw: Vec<(&str, Vec<u8>)> = vec![
        ("truncated", good[..good.len() / 2].to_vec()),
        ("empty", Vec::new()),
        ("not JSON", b"queue".to_vec()),
        ("a duplicated key", duplicated.into_bytes()),
    ];
    let edits: Vec<(&str, fn(&mut Value, &Keys))> = vec![
        ("version 2", |v, _| v["version"] = json!(2)),
        ("cursor past tier 1's four scenarios", |v, _| v["cursor"] = json!([0, 0, 4])),
        ("cursor in a fourth tier", |v, _| v["cursor"] = json!([3, 0, 0])),
        ("cursor past the last flop", |v, _| v["cursor"] = json!([0, 1755, 0])),
        ("a missing item", |v, k| drop(v["items"].as_object_mut().unwrap().remove(&k.pending))),
        ("a key that is not its identity", |v, k| {
            let items = v["items"].as_object_mut().unwrap();
            let moved = items.remove(&k.pending).unwrap();
            items.insert("00".repeat(32), moved);
        }),
        ("an identity that is not its key", |v, k| v["items"][&k.pending]["identity"] = json!(vec![0_u8; 32])),
        ("a moved board", |v, k| v["items"][&k.pending]["board"] = k.other_board.clone()),
        ("a changed scenario", |v, k| v["items"][&k.pending]["scenario"]["depth_bb"] = json!(k.other_depth)),
        ("a changed SPR", |v, k| v["items"][&k.pending]["spr"] = json!({"num": 99, "den": 1})),
        ("an unreduced SPR", |v, k| v["items"][&k.pending]["spr"] = json!({"num": 200, "den": 2})),
        ("failed with n 0", |v, k| v["items"][&k.failed]["status"] = json!({"failed": {"n": 0}})),
        ("failed with n 5", |v, k| v["items"][&k.failed]["status"] = json!({"failed": {"n": 5}})),
        ("an unknown status", |v, k| v["items"][&k.failed]["status"] = json!("running")),
        ("failed with no attempt", |v, k| v["items"][&k.failed]["attempts"] = json!(0)),
        ("failed with no error", |v, k| v["items"][&k.failed]["last_error"] = Value::Null),
        ("failed with a control character in its error", |v, k| v["items"][&k.failed]["last_error"] = json!("a\nb")),
        ("failed with an oversized error", |v, k| v["items"][&k.failed]["last_error"] = json!("x".repeat(queue::LAST_ERROR_MAX_BYTES + 1))),
        ("failed with a retry before any backoff", |v, k| v["items"][&k.failed]["retry_after_unix_ms"] = json!(0)),
        ("failed with a retry past the ceiling", |v, k| v["items"][&k.failed]["retry_after_unix_ms"] = json!(i64::MAX as u64 + 1)),
        ("terminal with a retry deadline", |v, k| {
            v["items"][&k.failed]["status"] = json!({"failed": {"n": 4}});
            v["items"][&k.failed]["attempts"] = json!(4);
        }),
        ("pending with two attempts", |v, k| v["items"][&k.pending]["attempts"] = json!(2)),
        ("pending with a retry deadline", |v, k| v["items"][&k.pending]["retry_after_unix_ms"] = json!(30_000)),
        ("pending with an error", |v, k| v["items"][&k.pending]["last_error"] = json!("error")),
        ("done with an error", |v, k| v["items"][&k.done]["last_error"] = json!("error")),
        ("done with five attempts", |v, k| v["items"][&k.done]["attempts"] = json!(5)),
        ("two items in flight", |v, k| {
            v["items"][&k.pending]["attempts"] = json!(1);
            v["items"][&k.failed]["attempts"] = json!(2);
        }),
        ("an unknown file field", |v, _| v["extra"] = json!(true)),
        ("an unknown item field", |v, k| v["items"][&k.pending]["note"] = json!("x")),
    ];
    // One tampered file at a time: each is a full 17 MB queue.
    let edited = edits.into_iter().map(|(why, tamper)| {
        let mut v = value.clone();
        tamper(&mut v, &keys);
        (why, serde_json::to_vec(&v).unwrap())
    });
    for (why, bytes) in raw.into_iter().chain(edited) {
        assert_ne!(bytes, untouched, "{why}: the tampering must change the file");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(progress(&open(tmp.path())), ([0, 0, 0], false, (ALL, 0, 0)), "{why}: must be rejected and rebuilt");
    }

    // Past the 64 MiB bound the file is refused before it is read, even though it is valid JSON.
    let mut padded = good.clone();
    padded.resize(queue::QUEUE_FILE_MAX as usize + 1, b' ');
    std::fs::write(&path, &padded).unwrap();
    assert_eq!(progress(&open(tmp.path())), ([0, 0, 0], false, (ALL, 0, 0)), "oversized");
}

/// The other serde direction: `save_queue` validates too, refusing a file `Queue::open` would
/// reject, and leaves the published file untouched.
#[test]
fn save_refuses_a_queue_file_that_would_not_reopen() {
    let tmp = TempDir::new("queue-save-validate");
    let q = open(tmp.path());
    q.save().unwrap();
    let path = queue::queue_path(tmp.path());
    let before = std::fs::read(&path).unwrap();
    let file = saved(tmp.path());
    let key = file.items.keys().next().unwrap().clone();
    let cases: [(&str, Box<dyn Fn(&mut QueueFile)>); 6] = [
        ("version 2", Box::new(|f| f.version = 2)),
        ("cursor", Box::new(|f| f.cursor = [0, 0, 4])),
        ("a missing item", Box::new(|f| drop(f.items.pop_first()))),
        ("failed with n 0", Box::new(|f| f.items.values_mut().next().unwrap().status = TaskStatus::Failed { n: 0 })),
        ("a moved board", Box::new(|f| f.items.values_mut().next().unwrap().board.reverse())),
        ("a key that is not its identity", Box::new(move |f| {
            let item = f.items.remove(&key).unwrap();
            f.items.insert("00".repeat(32), item);
        })),
    ];
    for (why, mutate) in cases {
        let mut bad = file.clone();
        mutate(&mut bad);
        assert!(matches!(queue::save_queue(&path, &bad), Err(CacheError::Invalid(_))), "{why}: must be refused");
        assert_eq!(std::fs::read(&path).unwrap(), before, "{why}: the published file is untouched");
    }
    queue::save_queue(&path, &file).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before, "an unchanged file republishes byte-identically");
}

// Scheduler-contract violations are bugs, not states: always-on assertions, never silent no-ops.

#[test]
#[should_panic(expected = "in flight")]
fn a_second_launch_while_a_job_is_in_flight_is_refused() {
    let tmp = TempDir::new("queue-double-launch");
    let mut q = open(tmp.path());
    let first = q.next_pending(0).unwrap().identity_hex();
    q.record_launch(&first);
    let second = q.next_pending(0).unwrap().identity_hex();
    assert_ne!(first, second, "the running job is not offered again");
    q.record_launch(&second);
}

#[test]
#[should_panic(expected = "not the job in flight")]
fn recording_done_without_a_launch_is_refused() {
    let tmp = TempDir::new("queue-done-unlaunched");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();
    q.record_done(&id);
}

#[test]
#[should_panic(expected = "not launchable")]
fn launching_a_terminal_item_is_refused() {
    let tmp = TempDir::new("queue-launch-terminal");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();
    for n in 1..=4 {
        q.record_launch(&id);
        q.record_failure(&id, 0, format!("error {n}"));
    }
    q.record_launch(&id);
}

#[test]
#[should_panic(expected = "no queue item")]
fn an_unknown_identity_is_refused() {
    let tmp = TempDir::new("queue-unknown");
    let mut q = open(tmp.path());
    q.record_launch(&"00".repeat(32));
}
