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
// queue carries all 42,120 (tier, canonical flop, scenario) slots, so tests look individual slots
// up in the queue's own saved `queue.json` (`saved`), never in a hand-written copy of it, and read
// a slot's effective progress through `Queue::item`.
//
// Fix round 1: progress belongs to the normalized game identity a slot's preparation resolves to
// (review R1), so a test that completes or reconciles work first binds the slots to identities
// (`bind`, `bind_all`: `prepared` stands in for task 16's chart replay); and retry deadlines are
// persisted as UTC and waited out on the monotonic clock (review R2), which tests drive through
// `FakeClock`.
// ---------------------------------------------------------------------------------------------

use cache::entry::CacheEntry;
use cache::key::{KeyFields, Rational};
use cache::presolver::queue::{self, GameBinding, Queue, QueueClock, QueueFile, QueueItem, TaskStatus};
use cache::storage;
use cache::{Cache, CacheError, CACHE_QUOTA_BYTES};
use proto::Card;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use temp_dir::TempDir;

/// A plausible UTC wall-clock reading (2026-09-21), in Unix milliseconds.
const UNIX: u64 = 1_790_000_000_000;

/// A settable queue clock (review R2): its monotonic and wall-clock readings move independently,
/// so a test can restart the monotonic timeline (a new process) or jump the wall clock.
#[derive(Clone)]
struct FakeClock {
    mono: Arc<AtomicU64>,
    unix: Arc<AtomicU64>,
}

impl FakeClock {
    fn new(mono: u64, unix: u64) -> Self {
        FakeClock { mono: Arc::new(AtomicU64::new(mono)), unix: Arc::new(AtomicU64::new(unix)) }
    }

    fn set_unix(&self, unix: u64) {
        self.unix.store(unix, Ordering::SeqCst);
    }
}

impl QueueClock for FakeClock {
    fn monotonic_ms(&self) -> u64 {
        self.mono.load(Ordering::SeqCst)
    }

    fn unix_ms(&self) -> u64 {
        self.unix.load(Ordering::SeqCst)
    }
}

/// The queue under `dir`, on `clock`.
fn open_at(dir: &Path, clock: &FakeClock) -> Queue {
    Queue::open_with_clock(dir.to_path_buf(), Box::new(clock.clone())).unwrap()
}

/// Opens a fresh queue on a clock reading (`mono`, `unix`), fails its first item once at `mono`,
/// saves, and returns the item's identity: a process that recorded a failure and then stopped.
fn fail_once_and_save(dir: &Path, mono: u64, unix: u64) -> String {
    let mut q = open_at(dir, &FakeClock::new(mono, unix));
    let id = q.next_pending(mono).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, mono, "error".into());
    q.save().unwrap();
    id
}

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

/// Every slot of the frozen enumeration, keyed by its identity, from the queue's own saved file.
fn slots(q: &Queue, dir: &Path) -> BTreeMap<String, QueueItem> {
    q.save().unwrap();
    saved(dir).items
}

/// The support fixture's key, built once: `support::entry()` is a whole cache entry.
fn fixture_key() -> &'static KeyFields {
    static KEY: std::sync::OnceLock<KeyFields> = std::sync::OnceLock::new();
    KEY.get_or_init(|| support::entry().key)
}

/// Stands in for task 16's preparation of `item` under the chart bundle `source`: the fixture key
/// moved onto the item's canonical flop, with range hashes derived from the bundle and the
/// scenario (so every scenario is its own game), and an exact SPR that is not the nominal
/// `depth_bb : 1`.
fn prepared(item: &QueueItem, source: &str) -> (KeyFields, Rational) {
    use sha2::{Digest, Sha256};
    let mut key = fixture_key().clone();
    key.canonical_board = item.board.clone();
    key.spr_bucket = 0;
    key.range_hash_oop = Sha256::digest(format!("{source}/oop/{}", item.scenario.id())).into();
    key.range_hash_ip = Sha256::digest(format!("{source}/ip/{}", item.scenario.id())).into();
    let spr = Rational::new(u64::from(item.scenario.depth_bb) * 2 - 5, 11).unwrap();
    (key, spr)
}

/// Binds `item` to the game `prepared` resolves it to under the bundle "chart-a".
fn bind(q: &mut Queue, item: &QueueItem) -> QueueItem {
    let (key, spr) = prepared(item, "chart-a");
    q.bind(&item.identity_hex(), &key, spr)
}

/// Binds every slot, each to its own game -- task 16's startup preparation of the whole queue.
fn bind_all(q: &mut Queue, dir: &Path) {
    for item in slots(q, dir).values() {
        bind(q, item);
    }
}

/// Binds, launches and completes `item`, then advances the cursor -- the scheduler's own sequence.
fn complete(q: &mut Queue, item: &QueueItem) {
    bind(q, item);
    q.record_launch(&item.identity_hex());
    q.record_done(&item.identity_hex());
    q.advance_cursor();
}

/// Two source/config generation fingerprints, as task 16 would compute them.
const GEN_A: [u8; 32] = [0xa1; 32];
const GEN_B: [u8; 32] = [0xb2; 32];

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

/// Brief step 1, verbatim except for two things: the shared `TempDir` (unique per call, removed
/// even when an assertion unwinds) replaces the brief's fixed `pokerai-queue-<pid>` path and its
/// manual create/remove (standing ruling: filesystem tests use unique temp dirs and clean up); and
/// the item is bound to its prepared normalized identity before it is launched (fix round 1,
/// review R1: completion is recorded against that identity, so an unbound launch cannot complete).
#[test]
fn queue_reopens_at_the_saved_cursor_without_repeating_done_work() {
    let tmp=TempDir::new("queue-reopen");
    let dir=tmp.path().to_path_buf();
    let mut q=cache::presolver::queue::Queue::open(dir.clone()).unwrap();
    let first=q.next_pending(0).unwrap();
    bind(&mut q,&first);
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
    assert_eq!((file.version, file.cursor, file.paused), (2, [0, 0, 0], false));
    assert_eq!(file.generation, queue::INITIAL_GENERATION, "no source/config generation declared yet");
    assert!(file.games.is_empty(), "no slot is bound to a normalized identity before preparation");
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
        assert_eq!((item.attempts, item.retry_after_unix_ms, item.last_error.as_deref(), item.game), (0, 0, None, None));
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
    for id in &done {
        let item = reopened.item(id).unwrap();
        assert_eq!((&item.status, item.attempts), (&TaskStatus::Done, 1), "{id}");
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
    bind_all(&mut q, tmp.path());
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
    bind_all(&mut q, tmp.path());
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
    q.bind(&id, &key, spr);
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
    let demoted = q.item(&id).unwrap();
    assert_eq!((&demoted.status, demoted.attempts, demoted.retry_after_unix_ms), (&TaskStatus::Pending, 0, 0));
    q.save().unwrap();
    let game = &saved(&root).games[&hex(&key.scenario_identity(spr))];
    assert_eq!((&game.status, game.attempts, game.retry_after_unix_ms), (&TaskStatus::Pending, 0, 0), "the demotion is the game's");
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
    q.bind(&item.identity_hex(), &key, spr);
    q.reconcile(&|it| it.identity == item.identity && queue::entry_verified(&root, it, &key, spr, TARGET_BP));
    assert_eq!(q.status_counts(), (ALL, 0, 0));
    q.reconcile(&|it| it.identity == item.identity && queue::entry_verified(&root, it, &key, spr, 52));
    assert_eq!(q.status_counts(), (ALL - 1, 1, 0), "the predicate does decide the bound game");

    let elsewhere = item_for(&q, &root, &tier(1)[0], &canonical_flops_ordered()[0]);
    assert_ne!(elsewhere.board, key.canonical_board);
    assert!(!queue::entry_verified(&root, &elsewhere, &key, spr, 52), "an entry never completes an item on another board");

    let path = storage::entry_path(&root, key.digest());
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(!queue::entry_verified(&root, &item, &key, spr, 52), "a corrupt cell is not an entry");
    cache.shutdown();
}

/// Re-review round 2, N1: `entry_verified` must check the entry it reads back against the *bound*
/// slot's own normalized identity, not merely whatever identity the caller's `key`/`spr` happen to
/// compute to. A slot bound to one identity is not completed by a valid, at-target entry that
/// belongs to a different identity, even when the caller passes that other identity's own key: the
/// mismatch between `item.game` and the computed identity is not completion.
#[test]
fn entry_verified_rejects_an_entry_for_another_identity_than_the_slots_binding() {
    let (entry, item, mut q, dir) = fixture();
    let root = dir.path().to_path_buf();
    let id = item.identity_hex();
    let (key, spr) = (entry.key.clone(), entry.source.spr);

    let cache = Cache::open(root.clone(), CACHE_QUOTA_BYTES);
    assert!(cache.store_tracked(&entry).wait(WRITER_BUDGET), "the writer confirms the store");
    assert!(queue::entry_verified(&root, &item, &key, spr, TARGET_BP), "unbound, the caller's own key reads back valid");

    let mut elsewhere = key.clone();
    let hash = core_ranges::hash_scaled(&core_ranges::parse_range("KK").unwrap());
    elsewhere.range_hash_oop = hash;
    elsewhere.range_hash_ip = hash;
    assert_ne!(key.scenario_identity(spr), elsewhere.scenario_identity(spr), "a distinct identity on the same board");

    let bound = q.bind(&id, &elsewhere, spr);
    assert_eq!(bound.game.unwrap().identity, elsewhere.scenario_identity(spr), "the slot is bound to the other identity");
    assert!(
        !queue::entry_verified(&root, &bound, &key, spr, TARGET_BP),
        "a valid entry for another identity must not complete a slot bound elsewhere"
    );
    cache.shutdown();
}

/// Brief step 5 "replace the source bundle hash", and review R1: a new bundle is a new source
/// generation, under which preparation binds the item to a new normalized identity (the cache key
/// without its bucket plus the exact SPR). Completion under the old bundle does not carry over,
/// while the old bundle's entry stays on disk, untouched, serving only its own identity.
#[test]
fn a_new_source_bundle_is_a_new_identity_and_old_entries_stay_isolated() {
    let (entry, item, mut q, dir) = fixture();
    let root = dir.path().to_path_buf();
    let id = item.identity_hex();
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
    let under = |key: &KeyFields| {
        let (key, root) = (key.clone(), root.clone());
        move |it: &QueueItem| queue::entry_verified(&root, it, &key, spr, TARGET_BP)
    };
    q.set_generation(GEN_A);
    q.bind(&id, &old, spr);
    q.reconcile(&under(&old));
    assert_eq!(q.status_counts(), (ALL - 1, 1, 0), "done under the old bundle");

    q.set_generation(GEN_B);
    assert_eq!(q.status_counts(), (ALL, 0, 0), "under a new generation the item has to be prepared again");
    let bound = q.bind(&id, &new, spr);
    assert_eq!(bound.game.map(|g| (g.generation, g.identity)), Some((GEN_B, new.scenario_identity(spr))));
    q.reconcile(&under(&new));
    assert_eq!(q.status_counts(), (ALL, 0, 0), "not done under the new bundle");
    assert!(queue::entry_verified(&root, &item, &old, spr, TARGET_BP), "the old entry still serves its own identity");
    assert_eq!(storage::read_cell(&storage::entry_path(&root, old.digest())).unwrap().entries.len(), 1);
    assert!(!storage::entry_path(&root, new.digest()).exists(), "nothing was written under the new identity");
    cache.shutdown();
}

/// Review R1: progress belongs to the normalized game identity, not to a (scenario, flop) slot.
/// Two slots whose preparation resolves to the same game share one status and one retry budget:
/// the second is not offered while the first runs the game, it shows the game's failure and its
/// completion, and one reconciliation of the game demotes both.
#[test]
fn two_slots_that_normalize_to_one_game_share_its_status() {
    let tmp = TempDir::new("queue-shared-game");
    let dir = tmp.path();
    let flops = canonical_flops_ordered();
    let t1 = tier(1);
    let mut q = open_at(dir, &FakeClock::new(0, UNIX));
    let a = item_for(&q, dir, &t1[0], &flops[0]);
    let b = item_for(&q, dir, &t1[1], &flops[0]);
    let (a_id, b_id) = (a.identity_hex(), b.identity_hex());
    let (key, spr) = prepared(&a, "chart-a");
    let game = key.scenario_identity(spr);
    for slot in [&a_id, &b_id] {
        let bound = q.bind(slot, &key, spr);
        assert_eq!(bound.game, Some(GameBinding { generation: queue::INITIAL_GENERATION, identity: game }));
    }

    assert_eq!(q.next_pending(0).unwrap().identity_hex(), a_id);
    q.record_launch(&a_id);
    assert!(is(&q.next_pending(0).unwrap(), &t1[2], &flops[0]), "the shared game is in flight, so b is not offered");
    q.record_failure(&a_id, 0, "worker error".into());
    let shared = q.item(&b_id).unwrap();
    assert_eq!((shared.status, shared.attempts, shared.last_error), (TaskStatus::Failed { n: 1 }, 1, Some("worker error".into())), "b shows the game's failure");
    assert_eq!(q.status_counts(), (ALL - 2, 0, 2));

    assert_eq!(q.next_pending(30_000).unwrap().identity_hex(), a_id, "one retry for the game");
    q.record_launch(&a_id);
    q.record_done(&a_id);
    assert_eq!(q.item(&b_id).unwrap().status, TaskStatus::Done, "b is complete with its game");
    assert_eq!(q.status_counts(), (ALL - 2, 2, 0));

    q.save().unwrap();
    let file = saved(dir);
    assert_eq!(file.games.len(), 1, "one game record for the two slots");
    let record = &file.games[&hex(&game)];
    assert_eq!((&record.status, record.attempts, &record.board, record.spr), (&TaskStatus::Done, 2, &flops[0], spr));
    for slot in [&a_id, &b_id] {
        let persisted = &file.items[slot];
        assert_eq!(persisted.game.map(|g| g.identity), Some(game), "the slot is an index into its game");
        assert_eq!((&persisted.status, persisted.attempts), (&TaskStatus::Pending, 0), "a bound slot keeps no progress of its own");
    }

    let mut reopened = open_at(dir, &FakeClock::new(0, UNIX));
    assert_eq!(reopened.status_counts(), (ALL - 2, 2, 0), "the shared status survives a reopen");
    let calls = std::cell::Cell::new(0_u32);
    reopened.reconcile(&|_| {
        calls.set(calls.get() + 1);
        false
    });
    assert_eq!(calls.get(), 1, "reconciliation decides the game once, not once per slot");
    assert_eq!(reopened.status_counts(), (ALL, 0, 0), "demoting the game demotes both slots");
    assert_eq!((reopened.item(&a_id).unwrap().attempts, reopened.item(&b_id).unwrap().attempts), (0, 0));
}

/// Review R1's failure case: four failed attempts make the game terminal, and it stays terminal
/// for as long as preparation resolves the slot to that identity -- a generation change alone
/// does not reset it. Bundle B supplying other ranges is another identity, so the slot starts
/// afresh with a full budget and bundle A's game goes with its last binding; a rake
/// (configuration) change is another identity again.
#[test]
fn a_terminal_failure_belongs_to_its_identity_and_new_ranges_or_rake_start_afresh() {
    let tmp = TempDir::new("queue-terminal-identity");
    let dir = tmp.path();
    let mut q = open_at(dir, &FakeClock::new(0, UNIX));
    let item = q.next_pending(0).unwrap();
    let id = item.identity_hex();
    let (a_key, spr) = prepared(&item, "chart-a");
    q.set_generation(GEN_A);
    q.bind(&id, &a_key, spr);
    for n in 1..=4_u64 {
        q.record_launch(&id);
        q.record_failure(&id, n * 30_000, format!("error {n}"));
    }
    assert_eq!(q.item(&id).unwrap().status, TaskStatus::Failed { n: 4 });
    assert_ne!(q.next_pending(u64::MAX / 2).unwrap().identity_hex(), id, "terminal under bundle A");

    // The same bundle under another generation (a config change that leaves this game's identity
    // alone) resolves the slot to the same identity: the terminal failure is that game's.
    q.set_generation([0xc3; 32]);
    assert_eq!(q.next_pending(0).unwrap().identity_hex(), id, "a new generation prepares the slot again");
    assert_eq!(q.bind(&id, &a_key, spr).status, TaskStatus::Failed { n: 4 }, "same identity, same terminal status");
    assert_ne!(q.next_pending(u64::MAX / 2).unwrap().identity_hex(), id);

    // Bundle B supplies usable ranges: a new identity with a full budget.
    q.set_generation(GEN_B);
    let (b_key, _) = prepared(&item, "chart-b");
    let fresh = q.bind(&id, &b_key, spr);
    assert_eq!((fresh.status, fresh.attempts, fresh.last_error), (TaskStatus::Pending, 0, None));
    assert_eq!(fresh.game.unwrap().identity, b_key.scenario_identity(spr));
    assert_eq!(q.next_pending(0).unwrap().identity_hex(), id, "launchable again");
    q.save().unwrap();
    let file = saved(dir);
    assert_eq!(file.generation, GEN_B);
    assert_eq!(file.games.keys().cloned().collect::<Vec<_>>(), vec![hex(&b_key.scenario_identity(spr))], "bundle A's game went with its last binding");

    // A rake change (configuration) is a new identity too.
    let mut raked = b_key.clone();
    raked.rake = cache::key::RakeKey::new(0.10, raked.rake.cap_over_p, raked.rake.collection_rule_version).unwrap();
    q.set_generation([0xd4; 32]);
    let other = q.bind(&id, &raked, spr);
    assert_ne!(other.game.unwrap().identity, b_key.scenario_identity(spr));
    assert_eq!((other.status, other.attempts), (TaskStatus::Pending, 0));
}

/// Review R1: a preparation failure has no normalized identity -- the slot could not be prepared --
/// so it is recorded on the slot and owned by the current generation: a source/config change
/// invalidates it, even a terminal one, and a later successful preparation clears it.
#[test]
fn a_preparation_failure_belongs_to_its_generation() {
    let tmp = TempDir::new("queue-prep-failure");
    let dir = tmp.path();
    let mut q = open_at(dir, &FakeClock::new(0, UNIX));
    q.set_generation(GEN_A);
    let first = q.next_pending(0).unwrap();
    let id = first.identity_hex();
    for n in 1..=4 {
        q.record_launch(&id);
        q.record_failure(&id, 0, format!("missing chart node {n}"));
    }
    let stuck = q.item(&id).unwrap();
    assert_eq!((stuck.status, stuck.game), (TaskStatus::Failed { n: 4 }, None), "a terminal preparation failure, on the slot");
    assert_ne!(q.next_pending(u64::MAX / 2).unwrap().identity_hex(), id);
    q.save().unwrap();

    let mut q = open_at(dir, &FakeClock::new(0, UNIX));
    q.set_generation(GEN_A);
    assert_eq!(q.item(&id).unwrap().status, TaskStatus::Failed { n: 4 }, "the same generation keeps it");
    q.set_generation(GEN_B);
    let reset = q.item(&id).unwrap();
    assert_eq!((reset.status, reset.attempts, reset.retry_after_unix_ms, reset.last_error), (TaskStatus::Pending, 0, 0, None), "a new generation invalidates it");
    assert_eq!(q.next_pending(0).unwrap().identity_hex(), id);

    // One more preparation failure, then preparation succeeds: the slot's own failure is over and
    // it shows its game's progress.
    q.record_launch(&id);
    q.record_failure(&id, 0, "transient".into());
    assert_eq!(q.item(&id).unwrap().status, TaskStatus::Failed { n: 1 });
    let bound = bind(&mut q, &first);
    assert_eq!((bound.status, bound.attempts, bound.last_error), (TaskStatus::Pending, 0, None));
    q.save().unwrap();
    let persisted = &saved(dir).items[&id];
    assert_eq!((&persisted.status, persisted.attempts, persisted.last_error.as_deref()), (&TaskStatus::Pending, 0, None));
}

/// Re-review round 2, N3 (corrected doc/comment): a game's record, including a terminal failure, is
/// forgotten the moment its last binding is released -- a rebind to another identity (J2 path 1,
/// covered elsewhere) or, as here, a preparation failure of its own stale slot under a new
/// generation (J2 path 2). Re-preparation back onto the very same identity afterwards finds a fresh
/// record, not the terminal one, even though no other slot ever bound elsewhere: the reset is
/// deliberate and visible, never masked as a survival of the old status.
#[test]
fn a_preparation_failure_under_a_new_generation_releases_the_stale_binding_and_resets_its_game() {
    let tmp = TempDir::new("queue-rebind-reset");
    let dir = tmp.path();
    let mut q = open_at(dir, &FakeClock::new(0, UNIX));
    let item = q.next_pending(0).unwrap();
    let id = item.identity_hex();
    let (key_a, spr) = prepared(&item, "chart-a");
    q.set_generation(GEN_A);
    q.bind(&id, &key_a, spr);
    for n in 1..=4_u64 {
        q.record_launch(&id);
        q.record_failure(&id, n * 30_000, format!("error {n}"));
    }
    assert_eq!(q.item(&id).unwrap().status, TaskStatus::Failed { n: 4 }, "the game is terminal under generation A");

    // The generation change alone leaves the binding in place, only stale; it is the preparation
    // failure of the now-stale slot that drops it and releases the game (J2 path 2).
    q.set_generation(GEN_B);
    q.record_launch(&id);
    q.record_failure(&id, 0, "chart replay failed".into());
    q.save().unwrap();
    assert!(
        !saved(dir).games.contains_key(&hex(&key_a.scenario_identity(spr))),
        "the stale binding's release drops the game record entirely"
    );

    let fresh = q.bind(&id, &key_a, spr);
    assert_eq!(
        (fresh.status, fresh.attempts, fresh.last_error),
        (TaskStatus::Pending, 0, None),
        "re-preparation onto the same identity finds a fresh record, not the terminal one"
    );
}

/// Review R1: `queue.json` persists each prepared slot's normalized identity and each game's exact
/// SPR -- distinct games under distinct keys, never the nominal `depth_bb : 1` -- and a reopen
/// keeps every association: an unchanged queue republishes byte-identically.
#[test]
fn the_file_persists_normalized_identities_and_exact_sprs_across_a_reopen() {
    let tmp = TempDir::new("queue-identities");
    let dir = tmp.path();
    let flops = canonical_flops_ordered();
    let mut q = open(dir);
    q.set_generation(GEN_A);
    let items: Vec<QueueItem> = [(1, 0), (1, 1), (2, 0)].iter().map(|&(t, s)| item_for(&q, dir, &tier(t)[s], &flops[3])).collect();
    let mut identities = BTreeSet::new();
    for item in &items {
        let (key, spr) = prepared(item, "chart-a");
        assert_ne!(spr, item.spr, "the exact SPR is not the nominal one");
        let bound = bind(&mut q, item);
        assert_eq!(bound.game, Some(GameBinding { generation: GEN_A, identity: key.scenario_identity(spr) }));
        identities.insert(hex(&key.scenario_identity(spr)));
    }
    assert_eq!(identities.len(), 3, "three slots, three games");
    q.save().unwrap();
    let bytes = std::fs::read(queue::queue_path(dir)).unwrap();
    let file = saved(dir);
    assert_eq!(file.games.keys().cloned().collect::<BTreeSet<_>>(), identities);
    for item in &items {
        let (key, spr) = prepared(item, "chart-a");
        let game = &file.games[&hex(&key.scenario_identity(spr))];
        assert_eq!((&game.board, game.spr, &game.status), (&item.board, spr, &TaskStatus::Pending));
        assert_eq!(file.items[&item.identity_hex()].game.map(|g| g.identity), Some(key.scenario_identity(spr)));
    }
    open(dir).save().unwrap();
    assert_eq!(std::fs::read(queue::queue_path(dir)).unwrap(), bytes, "a reopen keeps every association");
    let reopened = open(dir);
    assert_eq!(reopened.generation(), GEN_A);
    for item in &items {
        assert_eq!(reopened.item(&item.identity_hex()).unwrap().game, q.item(&item.identity_hex()).unwrap().game);
    }
}

/// Review R1: completion is decided per normalized identity. Reconciliation consults only slots
/// bound under the current generation -- an unbound slot has no identity to check -- and a freshly
/// bound slot is decided on its own with `reconcile_item`, before any solve is launched for it.
#[test]
fn reconciliation_decides_bound_identities_only() {
    let (entry, item, mut q, dir) = fixture();
    let root = dir.path().to_path_buf();
    let (key, spr) = (entry.key.clone(), entry.source.spr);
    let id = item.identity_hex();
    let cache = Cache::open(root.clone(), CACHE_QUOTA_BYTES);
    assert!(cache.store_tracked(&entry).wait(WRITER_BUDGET));

    let calls = std::cell::Cell::new(0_u32);
    q.reconcile(&|_| {
        calls.set(calls.get() + 1);
        true
    });
    assert_eq!((calls.get(), q.status_counts()), (0, (ALL, 0, 0)), "no slot is bound: nothing to decide");

    let bound = q.bind(&id, &key, spr);
    q.reconcile_item(&id, queue::entry_verified(&root, &bound, &key, spr, TARGET_BP));
    assert_eq!(q.item(&id).unwrap().status, TaskStatus::Done, "found on disk: done without a solve");
    assert_eq!(q.status_counts(), (ALL - 1, 1, 0), "and nothing else changed");
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
    q.bind(&id, &key, spr);
    q.record_launch(&id);
    q.save().unwrap();
    std::fs::create_dir_all(cell.parent().unwrap()).unwrap();
    std::fs::write(cell.with_extension("4242.0.tmp"), &bytes).unwrap();
    drop(q);
    let mut restarted = open(&root);
    assert_eq!(restarted.status_counts(), (ALL, 0, 0));
    restarted.reconcile(&verified);
    assert_eq!(restarted.status_counts(), (ALL, 0, 0), "a temp file is not a stored entry");
    let refunded = restarted.item(&id).unwrap();
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
    //    completion comes back from the cache, not from the torn file -- once preparation has
    //    bound the item to its identity again (the rebuilt queue knows no identity).
    restarted.save().unwrap();
    let whole = std::fs::read(queue::queue_path(&root)).unwrap();
    std::fs::write(queue::queue_path(&root), &whole[..whole.len() / 2]).unwrap();
    let mut rebuilt = open(&root);
    assert_eq!((rebuilt.cursor(), rebuilt.status_counts()), ([0, 0, 0], (ALL, 0, 0)));
    rebuilt.reconcile(&verified);
    assert_eq!(rebuilt.status_counts(), (ALL, 0, 0), "no identity is bound yet");
    rebuilt.bind(&id, &key, spr);
    rebuilt.reconcile(&verified);
    assert_eq!(rebuilt.status_counts(), (ALL - 1, 1, 0));
}

/// Brief step 5: failures 1, 2 and 3 retry after 30 s; the fourth failure is terminal, and stays
/// terminal across a restart. Each retry deadline is persisted as the absolute UTC time of the
/// failure plus the backoff (review R2).
#[test]
fn failed_attempts_one_to_three_retry_after_thirty_seconds_and_the_fourth_is_terminal() {
    let tmp = TempDir::new("queue-retry");
    let start = 1_000_000_u64;
    let clock = FakeClock::new(start, UNIX);
    let mut q = open_at(tmp.path(), &clock);
    let id = q.next_pending(start).unwrap().identity_hex();
    let mut now = start;
    for n in 1..=3_u8 {
        clock.set_unix(UNIX + (now - start));
        assert_eq!(q.next_pending(now).unwrap().identity_hex(), id, "attempt {n} is the earliest launchable item");
        q.record_launch(&id);
        q.record_failure(&id, now, format!("worker error {n}"));
        assert_eq!(q.status_counts(), (ALL - 1, 0, 1));
        assert_ne!(q.next_pending(now + 29_999).unwrap().identity_hex(), id, "failure {n} waits 30 s");
        let due = q.next_pending(now + 30_000).unwrap();
        assert_eq!(due.identity_hex(), id, "failure {n} retries at 30 s");
        let utc = UNIX + (now - start) + 30_000;
        assert_eq!((due.status, due.attempts, due.retry_after_unix_ms), (TaskStatus::Failed { n }, n, utc), "failure {n}: persisted as UTC");
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
    let reopened = open_at(tmp.path(), &FakeClock::new(0, UNIX + 86_400_000));
    assert_eq!(reopened.status_counts(), (ALL - 1, 0, 1));
    assert_ne!(reopened.next_pending(u64::MAX).unwrap().identity_hex(), id);
}

/// Brief step 5: a live-work cancellation and a restart both refund the interrupted attempt, so
/// an item still gets exactly four real attempts.
#[test]
fn cancellation_and_restart_do_not_burn_retries() {
    let tmp = TempDir::new("queue-cancel");
    let clock = FakeClock::new(0, UNIX);
    let mut q = open_at(tmp.path(), &clock);
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
    // The new process starts after the retry was due: its monotonic clock restarts at 777.
    let mut now = 777_u64;
    let mut q = open_at(tmp.path(), &FakeClock::new(now, UNIX + 30_000 + 5));
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

/// Review R2: the persisted retry deadline is an absolute UTC time -- the wall clock at the
/// failure plus the backoff -- whatever the caller's monotonic reading was; the in-process wait
/// runs on the monotonic clock.
#[test]
fn a_retry_deadline_is_persisted_as_absolute_utc_and_waited_out_on_the_monotonic_clock() {
    let tmp = TempDir::new("queue-utc");
    let t = 5_000_000_000_u64;
    let mut q = open_at(tmp.path(), &FakeClock::new(t, UNIX));
    let id = q.next_pending(t).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, t, "error".into());
    assert_ne!(q.next_pending(t + 29_999).unwrap().identity_hex(), id, "the backoff runs 30 s on the monotonic clock");
    assert_eq!(q.next_pending(t + 30_000).unwrap().identity_hex(), id);
    // A monotonic reading that runs backwards relative to the anchor `t` is a caller clock
    // violation, not a silent retry (re-review round 2, N2): see the dedicated panic test below.
    q.save().unwrap();
    assert_eq!(saved(tmp.path()).items[&id].retry_after_unix_ms, UNIX + 30_000, "persisted as UTC, never as the monotonic reading");
}

/// Re-review round 2, N2: `retry_due` is monotone -- a deadline more than one backoff ahead of
/// `now_ms` is never silently treated as due -- and loud. Every in-process deadline is set at most
/// one backoff ahead of the monotonic reading it was anchored at (`record_failure`'s own `now_ms`,
/// or `open_with_clock`'s restored anchor), so `now_ms` landing more than one backoff behind it can
/// only mean the caller's clock ran backwards relative to that anchor: a programming error the
/// clock seam surfaces with an always-on assert, never masks as a due retry.
#[test]
#[should_panic(expected = "now_ms ran backwards")]
fn a_now_ms_that_runs_backwards_past_one_backoff_is_a_loud_caller_clock_violation() {
    let tmp = TempDir::new("queue-clock-backwards");
    let t = 5_000_000_000_u64;
    let mut q = open_at(tmp.path(), &FakeClock::new(t, UNIX));
    let id = q.next_pending(t).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, t, "error".into());
    // `t` anchored the deadline at t + 30_000; asking at t - 3_600_000 is far more than one
    // backoff behind it.
    let _ = q.next_pending(t - 3_600_000);
}

/// Review R2: a process restarted immediately -- its monotonic clock restarted near zero, the
/// wall clock 5 s after the failure -- waits out the 25 s of backoff that remain, instead of
/// retrying at once.
#[test]
fn an_immediate_restart_keeps_the_remaining_backoff() {
    let tmp = TempDir::new("queue-restart-wait");
    let id = fail_once_and_save(tmp.path(), 5_000_000_000, UNIX);
    let q = open_at(tmp.path(), &FakeClock::new(12, UNIX + 5_000));
    assert_ne!(q.next_pending(12).unwrap().identity_hex(), id, "no immediate retry after a restart");
    assert_ne!(q.next_pending(12 + 24_999).unwrap().identity_hex(), id, "25 s of the backoff remain");
    assert_eq!(q.next_pending(12 + 25_000).unwrap().identity_hex(), id);
}

/// Review R2: a persisted deadline the wall clock has already reached is due as soon as the
/// queue reopens.
#[test]
fn a_persisted_deadline_already_reached_is_due_at_restart() {
    let tmp = TempDir::new("queue-restart-due");
    let id = fail_once_and_save(tmp.path(), 5_000_000_000, UNIX);
    for unix in [UNIX + 30_000, UNIX + 30_000 + 3_600_000] {
        let q = open_at(tmp.path(), &FakeClock::new(12, unix));
        assert_eq!(q.next_pending(12).unwrap().identity_hex(), id, "wall clock {unix}: the deadline has passed");
    }
}

/// Review R2: a persisted deadline more than one backoff ahead of the wall clock (the clock was
/// set back an hour while the app was closed) waits one full backoff and no longer: the queue
/// never stalls on it.
#[test]
fn a_persisted_deadline_far_ahead_of_the_wall_clock_waits_one_backoff_at_most() {
    let tmp = TempDir::new("queue-restart-clamp");
    let id = fail_once_and_save(tmp.path(), 5_000_000_000, UNIX);
    let q = open_at(tmp.path(), &FakeClock::new(12, UNIX - 3_600_000));
    assert_ne!(q.next_pending(12 + 29_999).unwrap().identity_hex(), id, "clamped to one backoff, not dropped");
    assert_eq!(q.next_pending(12 + 30_000).unwrap().identity_hex(), id, "and never more than one backoff");
}

/// Review R2: once a wait is established in-process -- by a failure, or by reopening -- a
/// wall-clock jump in either direction does not change it.
#[test]
fn wall_clock_jumps_leave_an_established_wait_alone() {
    let tmp = TempDir::new("queue-wall-jump");
    let clock = FakeClock::new(1_000, UNIX);
    let mut q = open_at(tmp.path(), &clock);
    let id = q.next_pending(1_000).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, 1_000, "error".into());
    for jump in [UNIX + 3_600_000, UNIX - 3_600_000] {
        clock.set_unix(jump);
        assert_ne!(q.next_pending(30_999).unwrap().identity_hex(), id, "wall clock {jump}: still waiting");
        assert_eq!(q.next_pending(31_000).unwrap().identity_hex(), id, "wall clock {jump}: due on the monotonic clock");
    }
    q.save().unwrap();
    drop(q);

    // Reopened 10 s after the failure: 20 s remain, anchored at monotonic 50.
    let reopened = FakeClock::new(50, UNIX + 10_000);
    let q = open_at(tmp.path(), &reopened);
    for jump in [UNIX + 10_000 + 3_600_000, UNIX + 10_000 - 3_600_000] {
        reopened.set_unix(jump);
        assert_ne!(q.next_pending(20_049).unwrap().identity_hex(), id, "wall clock {jump}: the restored wait stands");
        assert_eq!(q.next_pending(20_050).unwrap().identity_hex(), id, "wall clock {jump}: due when the restored wait ends");
    }
}

/// The persisted UTC deadline is computed wide and checked against the millisecond ceiling:
/// exactly at the ceiling it is stored and reloads; one past it is refused loudly, never
/// saturated.
#[test]
fn a_retry_deadline_at_the_millisecond_ceiling_is_kept() {
    let tmp = TempDir::new("queue-ceiling");
    let clock = FakeClock::new(0, queue::RETRY_DEADLINE_MAX_MS - 30_000);
    let mut q = open_at(tmp.path(), &clock);
    let id = q.next_pending(0).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, 0, "error".into());
    q.save().unwrap();
    assert_eq!(saved(tmp.path()).items[&id].retry_after_unix_ms, i64::MAX as u64);
    assert_eq!(open_at(tmp.path(), &clock).status_counts(), (ALL - 1, 0, 1), "the ceiling itself reloads");
}

#[test]
#[should_panic(expected = "retry deadline")]
fn a_retry_deadline_past_the_millisecond_ceiling_is_refused_loudly() {
    let tmp = TempDir::new("queue-past-ceiling");
    let mut q = open_at(tmp.path(), &FakeClock::new(0, queue::RETRY_DEADLINE_MAX_MS - 29_999));
    let id = q.next_pending(0).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, 0, "error".into());
}

/// The in-process (monotonic) deadline has the same ceiling: a monotonic reading that would
/// overflow it is refused loudly too.
#[test]
#[should_panic(expected = "retry deadline")]
fn a_monotonic_reading_past_the_millisecond_ceiling_is_refused_loudly() {
    let tmp = TempDir::new("queue-past-ceiling-mono");
    let mut q = open_at(tmp.path(), &FakeClock::new(0, UNIX));
    let id = q.next_pending(0).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_failure(&id, queue::RETRY_DEADLINE_MAX_MS - 29_999, "error".into());
}

/// The largest file the queue can ever publish: every slot bound (under a non-initial generation)
/// to a game of its own -- so every slot carries a binding and every game a record -- each game's
/// exact SPR at its widest possible representation (re-review round 2, N4: a reduced `u64/u64`
/// ratio of two consecutive integers, already in lowest terms, rather than the few-digit SPRs
/// `prepared` hands out elsewhere in this file), and every game failed four times with an error
/// exactly `LAST_ERROR_MAX_BYTES` long and needing a JSON escape on every byte, so `bounded_error`
/// has no need to cut it (a cut message loses more to the cut mark than it keeps escaped). A bound
/// slot keeps no progress of its own and an unbound slot has no game, so no other reachable state
/// is larger. It still fits the 64 MiB bound, so a save can never be locked out, and it reopens
/// intact.
#[test]
fn the_largest_possible_queue_file_still_fits_the_bound_and_reopens() {
    let tmp = TempDir::new("queue-largest");
    let mut q = open(tmp.path());
    q.set_generation([0xff; 32]);
    // Consecutive integers are already coprime, so this is a reduced ratio at the widest possible
    // `u64` digit count on both sides -- nothing `Rational::new` could store is larger.
    let worst_spr = Rational::new(u64::MAX, u64::MAX - 1).unwrap();
    for item in slots(&q, tmp.path()).into_values() {
        let (key, _) = prepared(&item, "chart-a");
        q.bind(&item.identity_hex(), &key, worst_spr);
    }
    let ids: Vec<String> = slots(&q, tmp.path()).into_keys().collect();
    let worst_error = "\"".repeat(128) + &"\\".repeat(128);
    assert_eq!(worst_error.len(), queue::LAST_ERROR_MAX_BYTES, "the worst-case message is exactly the bound, uncut");
    for id in &ids {
        for _ in 0..4 {
            q.record_launch(id);
            q.record_failure(id, 0, worst_error.clone());
        }
    }
    // Fix round 2 (ruling 15-R3b): the widest failure-journal record -- this game, this SPR, this
    // message -- stays inside JOURNAL_RECORD_MAX_BYTES even with a 20-digit sequence number.
    let queue::JournalAppend::Appended { bytes } = q.journal_outcome(&ids[0]).unwrap() else { panic!("an empty journal has room") };
    let widest = bytes + 19;
    eprintln!("the widest failure-journal record measures {widest} bytes (JOURNAL_RECORD_MAX_BYTES {})", queue::JOURNAL_RECORD_MAX_BYTES);
    assert!(widest <= queue::JOURNAL_RECORD_MAX_BYTES as u64);
    q.set_paused(true);
    q.save().expect("the worst-case queue file must stay within the bound");
    // ... and so does the snapshot with the widest `journal_seq`.
    let mut widest_file = saved(tmp.path());
    widest_file.journal_seq = u64::MAX;
    queue::save_queue(&queue::queue_path(tmp.path()), &widest_file).expect("the worst-case queue file must stay within the bound");
    let len = std::fs::metadata(queue::queue_path(tmp.path())).unwrap().len();
    eprintln!(
        "the largest possible queue.json measures {len} bytes ({:.1}% of the {}-byte QUEUE_FILE_MAX bound)",
        100.0 * len as f64 / queue::QUEUE_FILE_MAX as f64,
        queue::QUEUE_FILE_MAX
    );
    assert!(len <= queue::QUEUE_FILE_MAX, "{len} bytes exceeds QUEUE_FILE_MAX ({})", queue::QUEUE_FILE_MAX);
    let file = saved(tmp.path());
    assert_eq!(file.games.len(), ALL as usize, "one game per slot");
    let game = file.items[&ids[0]].game.unwrap().identity;
    let record = &file.games[&hex(&game)];
    assert_eq!(record.spr, worst_spr, "the widest possible SPR representation is preserved");
    assert_eq!(record.last_error.as_deref(), Some(worst_error.as_str()), "the maximal message must not be cut");
    assert!(!record.last_error.as_ref().unwrap().chars().any(char::is_control));
    let reopened = open(tmp.path());
    assert_eq!((reopened.paused(), reopened.status_counts()), (true, (0, 0, ALL)));
}

/// Fix round 3 (re-review 1, N2): `Queue::open_with_clock` must refund a dead process's in-flight
/// attempt on the loaded snapshot before replaying the journal over it, not after. A checkpoint is
/// usually taken while a job runs, so the snapshot it holds usually has one launched game; slot A
/// here is launched and the checkpoint saved while it runs (process 1, gone without a further
/// save). Process 2 opens under a new generation and journals slot B's own preparation failure
/// before its first checkpoint, then is gone too. Replaying that record declares the new generation
/// on process 1's snapshot -- whose slot A is still bound under the old one, still `launched` if
/// nothing refunded it first -- and `validate` then refuses the whole file ("a game in flight
/// without a current binding"), discarding the journaled failure along with it.
#[test]
fn a_journaled_failure_survives_a_checkpoint_with_a_job_in_flight_and_a_later_generation() {
    let tmp = TempDir::new("queue-inflight-generation");
    let dir = tmp.path();

    // Process 1: slot A is launched (its game in flight) and the checkpoint saved while it runs.
    let (a, b) = {
        let mut q = open_at(dir, &FakeClock::new(0, UNIX));
        q.set_generation(GEN_A);
        let mut items = slots(&q, dir).into_values();
        let a = items.next().unwrap();
        let b = items.next().unwrap();
        bind(&mut q, &a);
        q.record_launch(&a.identity_hex()); // never resolved: the checkpoint below saves it in flight
        q.save().unwrap();
        (a, b)
    };
    // Process 2 (process 1 is simply gone -- no shutdown, no cancel): declares a new generation
    // (as `Scheduler::open` does before anything else) and journals slot B's own preparation
    // failure -- charged to the slot, since it was never bound -- before its first checkpoint.
    {
        let mut q = open_at(dir, &FakeClock::new(1_000, UNIX + 1_000));
        q.set_generation(GEN_B);
        q.record_launch(&b.identity_hex());
        q.record_failure(&b.identity_hex(), 1_000, "no chart node".into());
        assert!(
            matches!(q.journal_outcome(&b.identity_hex()), Ok(queue::JournalAppend::Appended { .. })),
            "the journal has room for one small record"
        );
        // process 2 is gone too: no save, no shutdown.
    }
    // Process 3: N2's fix refunds slot A's in-flight attempt on the loaded snapshot first, so it
    // is no longer `launched` by the time the journal's generation change is validated against it.
    let q3 = open_at(dir, &FakeClock::new(2_000, UNIX + 2_000));
    let (item_a, item_b) = (q3.item(&a.identity_hex()).unwrap(), q3.item(&b.identity_hex()).unwrap());
    assert_eq!(
        (item_b.status, item_b.attempts),
        (TaskStatus::Failed { n: 1 }, 1),
        "N2: the journaled failure must survive the crash -- a stale in-flight game elsewhere must not invalidate it"
    );
    assert_eq!(
        (item_a.status, item_a.attempts),
        (TaskStatus::Pending, 0),
        "slot A's in-flight attempt is refunded either way"
    );
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

    /// The records a tampering case may reach: one done item (bound to its game, whose record is
    /// `game`), one failed item (an unbound preparation failure: attempt 1, retry due) and one
    /// untouched pending item, plus values guaranteed to differ from the pending item's and from
    /// the game's.
    struct Keys {
        done: String,
        failed: String,
        pending: String,
        game: String,
        other_board: Value,
        other_game_board: Value,
        other_depth: u64,
    }
    let value: Value = serde_json::from_slice(&good).unwrap();
    let (done, failed) = (first.identity_hex(), failed.identity_hex());
    let pending = value["items"].as_object().unwrap().keys().find(|k| **k != done && **k != failed).unwrap().clone();
    let pending_board: Vec<Card> = serde_json::from_value(value["items"][&pending]["board"].clone()).unwrap();
    let other_board = serde_json::to_value(canonical_flops_ordered().iter().find(|b| **b != pending_board).unwrap()).unwrap();
    let other_game_board = serde_json::to_value(canonical_flops_ordered().iter().find(|b| **b != first.board).unwrap()).unwrap();
    let other_depth = value["items"][&pending]["scenario"]["depth_bb"].as_u64().unwrap() + 1;
    let game = value["items"][&done]["game"]["identity"].as_str().unwrap().to_owned();
    assert_eq!(value["games"].as_object().unwrap().keys().collect::<Vec<_>>(), vec![&game], "the done item's game is the one game");
    let keys = Keys { done, failed, pending, game, other_board, other_game_board, other_depth };

    let untouched = serde_json::to_vec(&value).unwrap();
    std::fs::write(&path, &untouched).unwrap();
    assert_eq!(progress(&open(tmp.path())), ([0, 0, 2], true, (ALL - 2, 1, 1)), "the harness's own re-encoding is accepted");
    let mut regenerated = value.clone();
    regenerated["generation"] = json!("11".repeat(32));
    std::fs::write(&path, serde_json::to_vec(&regenerated).unwrap()).unwrap();
    assert_eq!(progress(&open(tmp.path())), ([0, 0, 2], true, (ALL - 1, 0, 1)), "another generation is valid: its bindings are stale");

    let text = String::from_utf8(untouched.clone()).unwrap();
    let entry = format!("\"{}\":{}", keys.done, serde_json::to_string(&value["items"][&keys.done]).unwrap());
    let duplicated = text.replacen("\"items\":{", &format!("\"items\":{{{entry},"), 1);
    let record = format!("\"{}\":{}", keys.game, serde_json::to_string(&value["games"][&keys.game]).unwrap());
    let duplicated_game = text.replacen("\"games\":{", &format!("\"games\":{{{record},"), 1);
    let raw: Vec<(&str, Vec<u8>)> = vec![
        ("truncated", good[..good.len() / 2].to_vec()),
        ("empty", Vec::new()),
        ("not JSON", b"queue".to_vec()),
        ("a duplicated key", duplicated.into_bytes()),
        ("a duplicated game key", duplicated_game.into_bytes()),
    ];
    let edits: Vec<(&str, fn(&mut Value, &Keys))> = vec![
        ("version 1", |v, _| v["version"] = json!(1)),
        ("version 3", |v, _| v["version"] = json!(3)),
        ("an uppercase generation", |v, _| v["generation"] = json!("AB".repeat(32))),
        ("a short generation", |v, _| v["generation"] = json!("ab")),
        ("a bound item with a status of its own", |v, k| v["items"][&k.done]["status"] = json!("done")),
        ("a bound item with an attempt of its own", |v, k| v["items"][&k.done]["attempts"] = json!(1)),
        ("a binding to a missing game", |v, k| drop(v["games"].as_object_mut().unwrap().remove(&k.game))),
        ("a binding with a short identity", |v, k| v["items"][&k.done]["game"]["identity"] = json!("00")),
        ("an unknown binding field", |v, k| v["items"][&k.done]["game"]["note"] = json!("x")),
        ("an unreferenced game", |v, k| {
            let record = v["games"][&k.game].clone();
            v["games"].as_object_mut().unwrap().insert("ab".repeat(32), record);
        }),
        ("a game on another board", |v, k| v["games"][&k.game]["board"] = k.other_game_board.clone()),
        ("a game with a zero SPR", |v, k| v["games"][&k.game]["spr"] = json!({"num": 0, "den": 1})),
        ("a game done with an error", |v, k| v["games"][&k.game]["last_error"] = json!("error")),
        ("a game done with five attempts", |v, k| v["games"][&k.game]["attempts"] = json!(5)),
        ("an unknown game field", |v, k| v["games"][&k.game]["note"] = json!("x")),
        ("a game in flight behind a stale binding", |v, k| {
            v["generation"] = json!("11".repeat(32));
            v["games"][&k.game]["status"] = json!("pending");
            v["games"][&k.game]["attempts"] = json!(1);
        }),
        ("a game and an item in flight", |v, k| {
            v["games"][&k.game]["status"] = json!("pending");
            v["games"][&k.game]["attempts"] = json!(1);
            v["items"][&k.failed]["attempts"] = json!(2);
        }),
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
    let mut q = open(tmp.path());
    let bound = q.next_pending(0).unwrap();
    bind(&mut q, &bound);
    q.save().unwrap();
    let path = queue::queue_path(tmp.path());
    let before = std::fs::read(&path).unwrap();
    let file = saved(tmp.path());
    let key = file.items.keys().find(|k| **k != bound.identity_hex()).unwrap().clone();
    let (slot, game) = (bound.identity_hex(), file.games.keys().next().unwrap().clone());
    let (game2, game3) = (game.clone(), game.clone());
    let cases: [(&str, Box<dyn Fn(&mut QueueFile)>); 9] = [
        ("version 1", Box::new(|f| f.version = 1)),
        ("cursor", Box::new(|f| f.cursor = [0, 0, 4])),
        ("a missing item", Box::new(|f| drop(f.items.pop_first()))),
        ("failed with n 0", Box::new(move |f| f.items.get_mut(&key).unwrap().status = TaskStatus::Failed { n: 0 })),
        ("a moved board", Box::new(|f| f.items.values_mut().next().unwrap().board.reverse())),
        ("a key that is not its identity", Box::new(move |f| {
            let item = f.items.remove(&slot).unwrap();
            f.items.insert("00".repeat(32), item);
        })),
        ("a binding to a missing game", Box::new(move |f| drop(f.games.remove(&game)))),
        ("an unreferenced game", Box::new(move |f| {
            let record = f.games[&game2].clone();
            f.games.insert("ab".repeat(32), record);
        })),
        ("a game on another board", Box::new(move |f| f.games.get_mut(&game3).unwrap().board.reverse())),
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

/// Review R1: completion is recorded against a normalized game identity, so a launch whose slot
/// has none bound under the current generation (a preparation that failed) can only fail or be
/// cancelled.
#[test]
#[should_panic(expected = "normalized game identity")]
fn completing_a_launch_with_no_bound_identity_is_refused() {
    let tmp = TempDir::new("queue-done-unbound");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();
    q.record_launch(&id);
    q.record_done(&id);
}

#[test]
#[should_panic(expected = "canonical flop")]
fn binding_a_key_on_another_board_is_refused() {
    let tmp = TempDir::new("queue-bind-board");
    let mut q = open(tmp.path());
    let first = q.next_pending(0).unwrap();
    let (mut key, spr) = prepared(&first, "chart-a");
    key.canonical_board = canonical_flops_ordered()[1].clone();
    q.bind(&first.identity_hex(), &key, spr);
}

#[test]
#[should_panic(expected = "in flight")]
fn rebinding_the_item_in_flight_is_refused() {
    let tmp = TempDir::new("queue-bind-in-flight");
    let mut q = open(tmp.path());
    let first = q.next_pending(0).unwrap();
    bind(&mut q, &first);
    q.record_launch(&first.identity_hex());
    let (key, spr) = prepared(&first, "chart-b");
    q.bind(&first.identity_hex(), &key, spr);
}

#[test]
#[should_panic(expected = "in flight")]
fn changing_the_generation_while_a_job_is_in_flight_is_refused() {
    let tmp = TempDir::new("queue-generation-in-flight");
    let mut q = open(tmp.path());
    let first = q.next_pending(0).unwrap();
    bind(&mut q, &first);
    q.record_launch(&first.identity_hex());
    q.set_generation(GEN_B);
}

#[test]
#[should_panic(expected = "no normalized identity")]
fn reconciling_an_unbound_item_is_refused() {
    let tmp = TempDir::new("queue-reconcile-unbound");
    let mut q = open(tmp.path());
    let id = q.next_pending(0).unwrap().identity_hex();
    q.reconcile_item(&id, true);
}

// ---------------------------------------------------------------------------------------------
// Task 15 (spec section 10.5): the scheduler -- the 30 s idle gate, live pre-emption, the launch
// path, checkpointed saves, chunked reconciliation and the `presolver` thread -- built on the
// task 14 queue under the orchestrator's rulings S1-S8 (task 14 re-review, section 4).
//
// The scheduler core (`Scheduler`) is driven one iteration at a time against `Fake`, an executor
// whose clock is a `FakeClock` and whose jobs are scripted `JobPoll`s, so every core test is
// deterministic and none sleeps. The two thread tests (`Presolver::start`) wait for events the
// fake executor sends, bounded by a failure budget that only ever cuts a hang short.
// ---------------------------------------------------------------------------------------------

use cache::presolver::scheduler::{
    JobPoll, LiveSignal, PreparedJob, PresolveExecutor, Presolver, PresolverCommand, Scheduler, BUSY_PUBLISH_MS, CHECKPOINT_MS,
    MAILBOX_CAPACITY, RECONCILE_CHUNK, RECONCILE_PERIOD_MS, RESCAN_MS, SAVE_RETRY_MS, TICK_MS,
};
use cache::presolver::PresolverStatus;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// Brief step 1, verbatim.
#[test]
fn idle_requires_no_hand_and_thirty_seconds() {
    use cache::presolver::scheduler::eligible;
    assert!(!eligible(false,false,29_999,0));assert!(eligible(false,false,30_000,0));
    assert!(!eligible(true,false,60_000,0));assert!(!eligible(false,true,60_000,0));
    assert!(!eligible(false,false,30_000,30_000));
}

/// Brief step 1, verbatim.
#[test]
fn presolver_status_round_trips_as_json() {
    // cross-plan Or7: plan 5 renders this struct directly, so it must serialize.
    let status=cache::presolver::scheduler::PresolverStatus{paused:true,
        running:Some("t1-100bb-Btn-open-Bb-call".into()),pending:7,done:2,failed:1,
        tier_done:[2,0,0],tier_total:[7020,14040,21060],measured_p50_s:Some(27.0),
        estimated_remaining_s:Some(189.0),scenario_hits:vec![("t1-100bb-Btn-open-Bb-call".into(),3,10)]};
    let text=serde_json::to_string(&status).unwrap();
    let back:cache::presolver::scheduler::PresolverStatus=serde_json::from_str(&text).unwrap();
    assert_eq!(status,back);
    assert!(text.contains("\"tier_total\":[7020,14040,21060]"));
    // the parent path Plan 5 and Engine::presolver_status name resolves to the same type
    let parent:cache::presolver::PresolverStatus=status.clone();
    assert_eq!(parent,status);
    assert_eq!(cache::presolver::remaining_seconds(7,Some(27.0)),Some(189.0));
}

/// Brief step 5, verbatim.
#[test]
fn live_request_cancels_background_before_new_launch() {
    use cache::presolver::scheduler::{next_action,ScheduleAction};
    assert!(matches!(next_action(Some(7),true,true),ScheduleAction::Cancel(7)));
    assert!(matches!(next_action(Some(7),false,true),ScheduleAction::Wait));
    assert!(matches!(next_action(None,false,true),ScheduleAction::Launch));
}

/// `PresolverStatus` derives `ts_rs::TS` under the `typescript` feature (cross-plan Or7). It is not
/// `#[ts(export)]`ed -- that would make every `--features typescript` test run write
/// `crates/cache/bindings/PresolverStatus.ts` into the tree -- so plan 5 takes the declaration
/// from `TS::decl` the way `proto::bindings` does; this pins its shape.
#[cfg(feature = "typescript")]
#[test]
fn presolver_status_declares_its_typescript_shape() {
    use ts_rs::TS;
    let decl = PresolverStatus::decl(&ts_rs::Config::new().with_large_int("number"));
    for field in [
        "paused: boolean",
        "running: string | null",
        "pending: number",
        "tier_total: [number, number, number]",
        "measured_p50_s: number | null",
        "scenario_hits: Array<[string, number, number]>",
    ] {
        assert!(decl.contains(field), "{field} missing from {decl}");
    }
}

impl FakeClock {
    /// Time passes in one process: both readings move forward by `ms`.
    fn advance(&self, ms: u64) {
        self.mono.fetch_add(ms, Ordering::SeqCst);
        self.unix.fetch_add(ms, Ordering::SeqCst);
    }

    fn mono(&self) -> u64 {
        self.mono.load(Ordering::SeqCst)
    }
}

/// What the fake executor tells a thread test, which waits for these instead of sleeping.
#[derive(Debug, PartialEq)]
enum Event {
    /// `prepare` was entered (before any gate blocks it).
    Preparing,
    Submitted(u64),
    Cancelled(u64),
    /// The scheduler computed a status to publish (`scenario_hits` is asked once per publication).
    Published,
}

/// The fake executor's state, shared between the test's handle and the scheduler's.
#[derive(Default)]
struct FakeState {
    generation: [u8; 32],
    /// What successive `poll`s report; `Running` once the script is used up.
    polls: VecDeque<JobPoll>,
    /// Whether `store_and_verify` finds the completed entry durable at target.
    durable: bool,
    /// The games whose entries are on disk at target.
    exists: BTreeSet<[u8; 32]>,
    /// Every game's entry is on disk at target.
    exists_everywhere: bool,
    /// Slots whose preparation fails.
    failing_preparation: BTreeSet<String>,
    /// Slots whose preparation hands back a key on another board.
    wrong_board: BTreeSet<String>,
    submit_error: Option<String>,
    next_job: u64,
    /// Every submission: job id, slot id and the job's cancel flag.
    submitted: Vec<(u64, String, Arc<AtomicBool>)>,
    cancels: Vec<u64>,
    /// The binding of every `entry_exists_at_target` question, in order.
    asked: Vec<GameBinding>,
    events: Option<mpsc::Sender<Event>>,
    /// A one-shot gate the next `prepare` blocks on until the test sends on it.
    gate: Option<mpsc::Receiver<()>>,
}

#[derive(Clone)]
struct Fake {
    state: Arc<Mutex<FakeState>>,
    clock: FakeClock,
}

impl Fake {
    fn new(clock: FakeClock, generation: [u8; 32]) -> Fake {
        let fake = Fake { state: Arc::default(), clock };
        fake.state().generation = generation;
        fake
    }

    /// The same executor state -- the cache on disk, the scripted worker -- in a new process whose
    /// clock is `clock`.
    fn restarted(&self, clock: FakeClock) -> Fake {
        Fake { state: Arc::clone(&self.state), clock }
    }

    fn state(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap()
    }

    fn submitted(&self) -> Vec<(u64, String, Arc<AtomicBool>)> {
        self.state().submitted.clone()
    }

    fn slots_submitted(&self) -> Vec<String> {
        self.state().submitted.iter().map(|(_, slot, _)| slot.clone()).collect()
    }

    fn cancels(&self) -> Vec<u64> {
        self.state().cancels.clone()
    }

    fn script(&self, polls: impl IntoIterator<Item = JobPoll>) {
        self.state().polls.extend(polls);
    }

    fn send(&self, event: Event) {
        if let Some(events) = &self.state().events {
            let _ = events.send(event); // the test may already have stopped listening
        }
    }
}

/// The game `Fake::prepare` resolves `item` to under `generation`: task 14's `prepared` stand-in
/// with the generation as the source bundle, so a new generation is a new identity.
fn game_of(item: &QueueItem, generation: [u8; 32]) -> [u8; 32] {
    let (key, spr) = prepared(item, &hex(&generation));
    key.scenario_identity(spr)
}

/// The shared fixture entry, built once: what a completed fake job hands back.
fn fixture_entry() -> &'static CacheEntry {
    static ENTRY: OnceLock<CacheEntry> = OnceLock::new();
    ENTRY.get_or_init(support::entry)
}

fn finished() -> JobPoll {
    JobPoll::Complete(fixture_entry().clone())
}

/// A background solve input on `board`, shaped like the fixture entry's.
fn solve_input(board: &[Card]) -> proto::SolveInput {
    let e = fixture_entry();
    proto::SolveInput {
        root: proto::StreetRootSnapshot {
            street: proto::Street::Flop,
            board: board.to_vec(),
            oop: proto::Seat(2),
            ip: proto::Seat(0),
            pot_root: e.source.pot,
            stack_oop_root: e.source.stack_oop,
            stack_ip_root: e.source.stack_ip,
            dead_this_street: 0,
            projected_from: 0,
            history: vec![],
            bb_chips: e.source.bb_chips,
        },
        ranges: e.source.ranges.clone(),
        tree: e.tree.clone(),
        target_bp: TARGET_BP,
    }
}

const RAKE: proto::Rake = proto::Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: true };

impl PresolveExecutor for Fake {
    fn clock(&self) -> Box<dyn QueueClock> {
        Box::new(self.clock.clone())
    }

    fn generation(&self) -> [u8; 32] {
        self.state().generation
    }

    fn prepare(&mut self, item: &QueueItem) -> Result<PreparedJob, String> {
        self.send(Event::Preparing);
        let gate = self.state().gate.take();
        if let Some(gate) = gate {
            gate.recv().expect("the test releases the gated preparation");
        }
        let state = self.state();
        let id = item.identity_hex();
        if state.failing_preparation.contains(&id) {
            return Err(format!("no chart node for {}", item.scenario.id()));
        }
        let (mut key, spr) = prepared(item, &hex(&state.generation));
        if state.wrong_board.contains(&id) {
            key.canonical_board = canonical_flops_ordered().iter().find(|b| **b != item.board).unwrap().clone();
        }
        Ok(PreparedJob { item: item.clone(), input: solve_input(&item.board), rake: RAKE, bb_chips: 2, key, spr })
    }

    fn submit(&mut self, job: PreparedJob, cancel: Arc<AtomicBool>) -> Result<u64, String> {
        let id = {
            let mut state = self.state();
            if let Some(error) = state.submit_error.clone() {
                return Err(error);
            }
            state.next_job += 1;
            let id = state.next_job;
            state.submitted.push((id, job.item.identity_hex(), cancel));
            id
        };
        self.send(Event::Submitted(id));
        Ok(id)
    }

    fn poll(&mut self, _job: u64) -> JobPoll {
        self.state().polls.pop_front().unwrap_or(JobPoll::Running)
    }

    fn cancel(&mut self, job: u64) {
        self.state().cancels.push(job);
        self.send(Event::Cancelled(job));
    }

    fn store_and_verify(&mut self, item: &QueueItem, _entry: &CacheEntry) -> bool {
        let mut state = self.state();
        let game = item.game.expect("a completed job's slot is bound").identity;
        if state.durable {
            state.exists.insert(game);
        }
        state.durable
    }

    fn entry_exists_at_target(&mut self, item: &QueueItem) -> bool {
        let mut state = self.state();
        let binding = item.game.expect("the scheduler asks only about bound slots");
        state.asked.push(binding);
        state.exists_everywhere || state.exists.contains(&binding.identity)
    }

    fn measured_p50_s(&self) -> Option<f64> {
        Some(27.0)
    }

    fn scenario_hits(&self) -> Vec<(String, u64, u64)> {
        self.send(Event::Published);
        vec![("t1-100bb-Btn-open-Bb-call".into(), 3, 10)]
    }
}

/// A scheduler over the cache root `dir`, driven by `fake`, and the live signal the engine's
/// admission raises (`Presolver::notify_live_request` / `notify_hand(true)`).
fn scheduler(dir: &Path, fake: &Fake) -> (Scheduler, LiveSignal) {
    let live = LiveSignal::default();
    (Scheduler::open(dir.to_path_buf(), Box::new(fake.clone()), live.clone()).unwrap(), live)
}

/// Moves the fake clock by `ms`, then runs one scheduler iteration.
fn tick(s: &mut Scheduler, fake: &Fake, ms: u64) {
    fake.clock.advance(ms);
    s.step();
}

/// The first `n` slots in sweep order, as the scheduler's queue shows them.
fn first_slots(s: &Scheduler, n: usize) -> Vec<QueueItem> {
    s.queue().sweep_order()[..n].iter().map(|id| s.queue().item(id).unwrap()).collect()
}

fn board_text(board: &[Card]) -> String {
    board.iter().map(ToString::to_string).collect()
}

/// Brief step 5: no submission before 30 idle seconds, then exactly one job while it runs -- bound
/// to its normalized identity before it was launched, with its attempt charged.
#[test]
fn nothing_is_submitted_before_thirty_idle_seconds_and_one_job_runs_at_a_time() {
    let tmp = TempDir::new("sched-idle");
    let fake = Fake::new(FakeClock::new(1_000, UNIX), GEN_A);
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    s.step();
    tick(&mut s, &fake, 29_999);
    assert!(fake.submitted().is_empty(), "no submission before 30 idle seconds");
    tick(&mut s, &fake, 1);
    assert_eq!(fake.submitted().len(), 1, "the first idle-eligible iteration submits one job");
    for _ in 0..5 {
        tick(&mut s, &fake, 60_000);
    }
    assert_eq!(fake.submitted().len(), 1, "exactly one job while it runs");

    let item = s.queue().item(&fake.slots_submitted()[0]).unwrap();
    assert!(is(&item, &tier(1)[0], &canonical_flops_ordered()[0]), "the sweep starts at tier 1, board 0, scenario 0");
    assert_eq!((item.status.clone(), item.attempts), (TaskStatus::Pending, 1), "the running attempt is charged");
    assert_eq!(item.game.map(|g| (g.generation, g.identity)), Some((GEN_A, game_of(&item, GEN_A))), "bound before it was launched");
    let status = s.status();
    assert_eq!(status.running, Some(format!("{} {}", tier(1)[0].id(), board_text(&item.board))));
    assert_eq!((status.paused, status.pending, status.done, status.failed), (false, ALL, 0, 0));
    assert_eq!((status.tier_done, status.tier_total), ([0, 0, 0], [7_020, 14_040, 21_060]));
    assert_eq!((status.measured_p50_s, status.estimated_remaining_s), (Some(27.0), Some(f64::from(ALL) * 27.0)));
    assert_eq!(status.scenario_hits, vec![("t1-100bb-Btn-open-Bb-call".to_string(), 3, 10)]);
}

/// Brief step 5 and the standing ruling: live admission pre-empts the running job at once -- the
/// admitting thread sets the job's own cancel flag, without the scheduler or the executor -- and
/// the scheduler records the cancel (refunding the attempt) without waiting for the job to wind
/// down. A hand pre-empts the same way and holds scheduling off until it ends plus 30 s.
#[test]
fn a_live_request_preempts_the_running_job_at_once_and_its_attempt_is_refunded() {
    let tmp = TempDir::new("sched-live");
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, live) = scheduler(tmp.path(), &fake);
    tick(&mut s, &fake, 30_000);
    let (job, slot, flag) = fake.submitted()[0].clone();
    assert!(!flag.load(Ordering::SeqCst));

    live.raise(); // engine admission, on the admitting thread
    assert!(flag.load(Ordering::SeqCst), "admission sets the running job's cancel flag itself, before the scheduler runs");
    assert!(fake.cancels().is_empty(), "admission neither calls nor waits for the executor");
    s.apply(PresolverCommand::LiveRequest);
    s.step();
    assert_eq!(fake.cancels(), vec![job], "one cancellation, and no waiting for the job to wind down");
    let item = s.queue().item(&slot).unwrap();
    assert_eq!((item.status, item.attempts), (TaskStatus::Pending, 0), "the cancelled attempt is refunded");
    assert_eq!(s.status().running, None);
    tick(&mut s, &fake, 29_999);
    assert_eq!(fake.submitted().len(), 1, "live work restarts the 30 s idle gate");
    tick(&mut s, &fake, 1);
    let second = fake.submitted()[1].clone();
    assert_eq!(second.1, slot, "the pre-empted slot runs again first");
    assert!(!second.2.load(Ordering::SeqCst), "every job gets a fresh cancel flag");

    live.raise(); // Presolver::notify_hand(true)
    s.apply(PresolverCommand::HandInProgress(true));
    s.step();
    assert_eq!(fake.cancels(), vec![job, second.0]);
    assert!(second.2.load(Ordering::SeqCst));
    tick(&mut s, &fake, 120_000);
    assert_eq!(fake.submitted().len(), 2, "nothing is scheduled while a hand is in progress");
    s.apply(PresolverCommand::HandInProgress(false));
    tick(&mut s, &fake, 29_999);
    assert_eq!(fake.submitted().len(), 2, "the idle gate restarts when the hand ends");
    tick(&mut s, &fake, 1);
    assert_eq!(fake.submitted().len(), 3);
    assert_eq!(fake.cancels().len(), 2, "one cancellation per pre-emption");
    let item = s.queue().item(&slot).unwrap();
    assert_eq!((item.status, item.attempts), (TaskStatus::Pending, 1), "two pre-emptions cost no attempt");
}

/// Brief step 5 and fix round 1, R4 (ruling 15-R4): a worker terminal is not completion. When the
/// completed entry does not read back durably at target the game is not Done: it is Pending again
/// with its attempt refunded -- durability is the cache's problem, not the solver's, so the retry
/// budget is untouched -- and the slot cools down in memory for one backoff before it is offered
/// again, so a solve that never verifies cannot hot-loop. It becomes Done only once
/// `store_and_verify` confirms the entry on disk.
#[test]
fn a_completed_job_is_done_only_once_its_entry_reads_back_durably() {
    let tmp = TempDir::new("sched-durable");
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    fake.script([JobPoll::Running, finished()]);
    tick(&mut s, &fake, 30_000);
    let a = fake.slots_submitted()[0].clone();
    tick(&mut s, &fake, 1); // completes, but the entry is not durable at target
    let after = s.queue().item(&a).unwrap();
    assert_eq!(
        (after.status, after.attempts, after.last_error, after.retry_after_unix_ms),
        (TaskStatus::Pending, 0, None, 0),
        "an unverified completion is Pending again, its attempt not consumed"
    );
    assert_eq!(s.status().done, 0);

    fake.state().durable = true;
    fake.script([finished()]);
    tick(&mut s, &fake, queue::RETRY_BACKOFF_MS - 1); // one millisecond short of the cooldown
    let b = fake.slots_submitted()[1].clone();
    assert_ne!(b, a, "A cools down for one backoff on the scheduler's clock: the next slot runs instead");
    assert_eq!(s.queue().item(&b).unwrap().status, TaskStatus::Done);
    fake.script([finished()]);
    tick(&mut s, &fake, 1); // the cooldown is over
    assert_eq!(fake.slots_submitted()[2], a, "then A is offered again, ahead of the sweep");
    let done = s.queue().item(&a).unwrap();
    assert_eq!((done.status, done.attempts), (TaskStatus::Done, 1), "Done once the entry reads back at target, on its only charged attempt");
    assert_eq!(s.status().done, 2);
}

/// Fix round 3 (N3): the not-durable log line must say what the slot actually returns to -- only
/// `Pending` for a first attempt, and the retained `Failed{n}` wording when a retry returns to its
/// own `Failed{n}` -- never a fixed "stays pending" regardless of which. This is a pure formatting
/// function, unit-tested directly: no test in this file captures stderr.
#[test]
fn the_not_durable_message_reports_the_status_the_slot_actually_returns_to() {
    use cache::presolver::queue::TaskStatus;
    use cache::presolver::scheduler::not_durable_message;
    let pending = not_durable_message("t1-100bb-Btn-open-Bb-call 2c3d4h", &TaskStatus::Pending);
    assert!(pending.contains("Pending"), "{pending}");
    assert!(!pending.contains("Failed"), "a first attempt returns to Pending, not Failed: {pending}");
    let retried = not_durable_message("t1-100bb-Btn-open-Bb-call 2c3d4h", &TaskStatus::Failed { n: 1 });
    assert!(
        retried.contains("Failed { n: 1 }"),
        "N3: a retry's own Failed{{n}} must be named, not a fixed \"stays pending\": {retried}"
    );
}

/// Brief step 5: a pause lets the running job finish and launches nothing more; the cursor, the
/// counters, the failure record and the pause itself survive a restart; a resume waits for 30 idle
/// seconds and continues without repeating completed work, its due retry first.
#[test]
fn pause_restart_and_resume_keep_the_cursor_and_the_failure_counts() {
    let tmp = TempDir::new("sched-restart");
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().durable = true;
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    fake.script([finished()]);
    tick(&mut s, &fake, 30_000); // A launched and completed in one iteration
    fake.script([JobPoll::Failed("worker exited".into())]);
    tick(&mut s, &fake, 1); // B launched and failed
    tick(&mut s, &fake, 1); // C launched, running
    let ids = fake.slots_submitted();
    assert_eq!(ids.len(), 3);
    s.apply(PresolverCommand::Pause);
    fake.script([finished()]);
    tick(&mut s, &fake, 1);
    assert_eq!(s.queue().item(&ids[2]).unwrap().status, TaskStatus::Done, "a pause lets the running job finish");
    tick(&mut s, &fake, 600_000);
    assert_eq!(fake.submitted().len(), 3, "a paused queue launches nothing");
    let (cursor, counts) = (s.queue().cursor(), s.queue().status_counts());
    assert!(s.status().paused);
    assert_eq!(counts, (ALL - 3, 2, 1));
    s.shutdown();
    drop(s);

    // A new process: its monotonic clock restarts, and the wall clock has moved on 20 minutes.
    let restarted = fake.restarted(FakeClock::new(42, UNIX + 1_200_000));
    let (mut s, _live) = scheduler(tmp.path(), &restarted);
    assert_eq!((s.queue().cursor(), s.queue().status_counts()), (cursor, counts), "the cursor and the counters survive the restart");
    assert!(s.status().paused, "and so does the pause");
    let failed = s.queue().item(&ids[1]).unwrap();
    assert_eq!((failed.status, failed.attempts, failed.last_error.as_deref()), (TaskStatus::Failed { n: 1 }, 1, Some("worker exited")));
    tick(&mut s, &restarted, 600_000);
    assert_eq!(restarted.submitted().len(), 3, "still paused after the restart");
    s.apply(PresolverCommand::Resume);
    tick(&mut s, &restarted, 29_999);
    assert_eq!(restarted.submitted().len(), 3, "a resume waits for 30 idle seconds");
    tick(&mut s, &restarted, 1);
    assert_eq!(restarted.slots_submitted()[3], ids[1], "the due retry runs first ...");
    assert_eq!(s.queue().cursor(), cursor, "... as a revisit that leaves the cursor where it was");
    assert!(!restarted.slots_submitted()[3..].iter().any(|slot| *slot == ids[0] || *slot == ids[2]), "completed work is not repeated");
    assert_eq!(s.queue().status_counts(), (ALL - 3, 2, 1));
}

/// Ruling S1 and the task 14 re-review's N2 test, through the real wiring: the queue runs on the
/// executor's clock, and every `now_ms` the scheduler passes it comes from that same clock. A job
/// that failed is restarted 5 s later (by the wall clock) on another monotonic timeline: 25 s of its
/// backoff remain on the executor's clock, and the retry is the first job after the restart.
#[test]
fn a_restart_waits_out_the_remaining_backoff_on_the_executors_clock() {
    let tmp = TempDir::new("sched-backoff");
    let fake = Fake::new(FakeClock::new(5_000_000_000, UNIX), GEN_A);
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    fake.script([JobPoll::Failed("worker exited".into())]);
    tick(&mut s, &fake, 30_000); // launched and failed at UNIX + 30 s
    let a = fake.slots_submitted()[0].clone();
    assert_eq!(s.queue().item(&a).unwrap().retry_after_unix_ms, UNIX + 60_000, "the deadline comes from the executor's wall clock");
    s.shutdown();
    drop(s);

    let t = 7_000_000_000;
    let later = fake.restarted(FakeClock::new(t, UNIX + 35_000));
    let (mut s, _live) = scheduler(tmp.path(), &later);
    assert_ne!(s.queue().next_pending(t + 24_999).unwrap().identity_hex(), a, "25 s of the backoff remain on the executor's clock");
    assert_eq!(s.queue().next_pending(t + 25_000).unwrap().identity_hex(), a);
    tick(&mut s, &later, 29_999);
    assert_eq!(later.submitted().len(), 1, "the idle gate runs on the same clock");
    tick(&mut s, &later, 1);
    assert_eq!(later.slots_submitted()[1], a, "the retry is the first job after the restart");
}

/// A queue under `dir` whose first slot completed under generation A; returns that slot.
fn first_slot_done_under_generation_a(dir: &Path) -> QueueItem {
    let mut q = open_at(dir, &FakeClock::new(0, UNIX));
    q.set_generation(GEN_A);
    let first = q.next_pending(0).unwrap();
    let (key, spr) = prepared(&first, &hex(&GEN_A));
    q.bind(&first.identity_hex(), &key, spr);
    q.record_launch(&first.identity_hex());
    q.record_done(&first.identity_hex());
    q.save().unwrap();
    first
}

/// Ruling S2: the executor's generation is declared before any reconciliation. Opened under a new
/// generation, an old binding is stale at once -- pending, and never judged against the new
/// generation's keys -- while under its own generation the startup sweep does check it.
#[test]
fn the_generation_is_declared_before_any_reconciliation() {
    let (stale_dir, current_dir) = (TempDir::new("sched-generation-b"), TempDir::new("sched-generation-a"));

    let first = first_slot_done_under_generation_a(stale_dir.path());
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_B);
    fake.state().exists.insert(game_of(&first, GEN_A));
    let (mut s, _live) = scheduler(stale_dir.path(), &fake);
    assert_eq!(s.queue().generation(), GEN_B, "the executor's generation is declared on open");
    assert_eq!(s.status().done, 0, "a stale binding does not count as done under the new generation");
    assert!(s.reconciling());
    tick(&mut s, &fake, 30_000);
    assert!(!s.reconciling(), "the startup sweep has visited every slot");
    let asked = fake.state().asked.clone();
    assert!(!asked.is_empty());
    assert!(asked.iter().all(|b| b.generation == GEN_B), "no verdict is taken on a binding of another generation: {asked:?}");

    let first = first_slot_done_under_generation_a(current_dir.path());
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().exists.insert(game_of(&first, GEN_A));
    let (mut s, _live) = scheduler(current_dir.path(), &fake);
    assert_eq!(s.status().done, 0, "a persisted completion is published only once the startup sweep verifies it (fix round 1, R1)");
    tick(&mut s, &fake, 30_000);
    assert!(!s.reconciling());
    let checked = GameBinding { generation: GEN_A, identity: game_of(&first, GEN_A) };
    assert!(fake.state().asked.contains(&checked), "under its own generation the binding is reconciled");
    assert_eq!(s.queue().item(&first.identity_hex()).unwrap().status, TaskStatus::Done);
    assert_eq!(s.status().done, 1, "and, verified, it is published");
}

/// Ruling S3: the launch path is prepare -> bind -> reconcile_item -> re-check `next_pending` ->
/// submit -> record_launch. An entry already on disk completes its slot without a solve; a failed
/// preparation, or one that hands back a key on another board, is a failure of the slot itself (no
/// identity, no panic); a submission refused after binding is charged to the game.
#[test]
fn the_launch_path_prepares_binds_checks_the_disk_and_submits_only_what_is_still_pending() {
    let tmp = TempDir::new("sched-launch-path");
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    let slots = first_slots(&s, 4);
    let (a, b, c, d) = (&slots[0], &slots[1], &slots[2], &slots[3]);
    fake.state().exists.insert(game_of(a, GEN_A));
    fake.state().failing_preparation.insert(b.identity_hex());
    fake.state().wrong_board.insert(c.identity_hex());

    tick(&mut s, &fake, 30_000);
    let found = s.queue().item(&a.identity_hex()).unwrap();
    assert_eq!((found.status, found.attempts, found.game.map(|g| g.identity)), (TaskStatus::Done, 0, Some(game_of(a, GEN_A))));
    assert!(fake.submitted().is_empty(), "an entry already at target is never solved again");
    assert_eq!(fake.state().asked.last().map(|g| g.identity), Some(game_of(a, GEN_A)), "the disk is checked for the freshly bound game");
    assert_eq!(s.wait_hint(), Duration::ZERO, "a slot resolved without a solve lets the next one follow at once");

    s.step();
    let failed = s.queue().item(&b.identity_hex()).unwrap();
    assert_eq!(
        (failed.status, failed.game, failed.last_error),
        (TaskStatus::Failed { n: 1 }, None, Some(format!("no chart node for {}", b.scenario.id()))),
        "a preparation failure belongs to the slot"
    );
    s.step();
    let wrong = s.queue().item(&c.identity_hex()).unwrap();
    assert_eq!((wrong.status, wrong.game), (TaskStatus::Failed { n: 1 }, None), "a key on another board is refused, never bound");
    assert!(wrong.last_error.unwrap().contains("canonical flop"));

    fake.state().submit_error = Some("worker unavailable".into());
    s.step();
    let refused = s.queue().item(&d.identity_hex()).unwrap();
    assert_eq!(
        (refused.status, refused.attempts, refused.game.map(|g| g.identity)),
        (TaskStatus::Failed { n: 1 }, 1, Some(game_of(d, GEN_A))),
        "a refused submission is a failed attempt of the bound game"
    );
    assert!(fake.submitted().is_empty());
    assert_eq!(s.queue().cursor(), [0, 1, 0], "the cursor stepped over all four");
    s.shutdown();
    assert_eq!(saved(tmp.path()).games[&hex(&game_of(d, GEN_A))].status, TaskStatus::Failed { n: 1 }, "persisted as the game's failure");
}

/// Ruling S5: a source/config change reaches the scheduler as `PresolverCommand::SourceChanged`.
/// An unchanged fingerprint changes nothing; a new one cancels the running job, records the cancel
/// (its attempt refunded), then declares the new generation and saves it, and the slot is prepared
/// again under the new generation.
#[test]
fn a_source_change_cancels_the_running_job_records_the_cancel_and_starts_a_new_generation() {
    let tmp = TempDir::new("sched-source");
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    tick(&mut s, &fake, 30_000);
    let (job, slot, flag) = fake.submitted()[0].clone();
    let item = s.queue().item(&slot).unwrap();
    let saves = s.save_stats().saves;

    s.apply(PresolverCommand::SourceChanged);
    assert!(fake.cancels().is_empty(), "an unchanged fingerprint is no change");
    assert_eq!(s.save_stats().saves, saves, "and writes nothing");

    fake.state().generation = GEN_B;
    s.apply(PresolverCommand::SourceChanged);
    assert_eq!(fake.cancels(), vec![job]);
    assert!(flag.load(Ordering::SeqCst), "the job's cancel flag is set");
    assert_eq!(s.queue().generation(), GEN_B);
    assert_eq!(s.save_stats().saves, saves + 1, "a generation change is saved at once");
    let file = saved(tmp.path());
    assert_eq!(file.generation, GEN_B);
    let old = &file.games[&hex(&game_of(&item, GEN_A))];
    assert_eq!((&old.status, old.attempts), (&TaskStatus::Pending, 0), "the cancel was recorded before the generation changed");

    s.step();
    assert_eq!(fake.slots_submitted()[1], slot, "the slot is prepared again under the new generation");
    let rebound = s.queue().item(&slot).unwrap().game.unwrap();
    assert_eq!((rebound.generation, rebound.identity), (GEN_B, game_of(&item, GEN_B)));
}

/// Ruling S7: after a generation change the counts start over -- every stale slot is pending until
/// it is prepared again -- and lazy rebinding costs one iteration per slot but no idle tick: slots
/// whose new entries are already on disk are rebound and found back to back.
#[test]
fn after_a_generation_change_the_counts_start_over_and_rebinding_waits_for_no_tick() {
    let tmp = TempDir::new("sched-rebind");
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().durable = true;
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    fake.script([finished()]);
    tick(&mut s, &fake, 30_000);
    fake.script([finished()]);
    s.step();
    assert_eq!(s.status().done, 2);
    let (a, b) = (s.queue().item(&fake.slots_submitted()[0]).unwrap(), s.queue().item(&fake.slots_submitted()[1]).unwrap());

    fake.state().generation = GEN_B;
    s.apply(PresolverCommand::SourceChanged);
    let status = s.status();
    assert_eq!((status.pending, status.done, status.tier_done), (ALL, 0, [0, 0, 0]), "stale slots count as pending until rebound");

    fake.state().exists.extend([game_of(&a, GEN_B), game_of(&b, GEN_B)]);
    s.step();
    assert_eq!(s.wait_hint(), Duration::ZERO);
    s.step();
    assert_eq!(s.wait_hint(), Duration::ZERO);
    assert_eq!(s.status().done, 2, "both found on disk under the new generation");
    assert_eq!(fake.submitted().len(), 2, "found, not solved again");
    s.step();
    assert_eq!(fake.submitted().len(), 3);
    assert_eq!(s.wait_hint(), Duration::from_millis(TICK_MS), "a running job is polled once per tick");
}

/// Ruling S8: the whole ~32 MB file is rewritten on each save, so saves happen only when the queue
/// changed, and -- except for a pause, a resume, a generation change and shutdown, which are saved at
/// once -- at most once per `CHECKPOINT_MS`. A launch is never saved on its own: opening refunds an
/// in-flight attempt, so a saved launch and an unsaved one reopen identically. Measured here with
/// every slot bound, against the plan's save after every launch and every outcome.
#[test]
fn saves_are_checkpointed_on_state_changes_and_their_volume_is_measured() {
    let tmp = TempDir::new("sched-volume");
    let dir = tmp.path();
    {
        let mut q = open_at(dir, &FakeClock::new(0, UNIX));
        q.set_generation(GEN_A);
        for item in slots(&q, dir).into_values() {
            let (key, spr) = prepared(&item, &hex(&GEN_A));
            q.bind(&item.identity_hex(), &key, spr);
        }
        q.save().unwrap();
    }
    let size = std::fs::metadata(queue::queue_path(dir)).unwrap().len();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().durable = true;
    let (mut s, _live) = scheduler(dir, &fake);
    const JOBS: u64 = 24;
    const SOLVE_MS: u64 = 27_000; // the i16 worst-board p50 of spec section 10.5
    tick(&mut s, &fake, 29_999);
    for _ in 0..JOBS {
        tick(&mut s, &fake, 1);
        fake.script([finished()]);
        tick(&mut s, &fake, SOLVE_MS);
    }
    assert_eq!(s.status().done, JOBS as u32);
    let elapsed = fake.clock.mono();
    let run = s.save_stats();
    assert!(run.saves >= 1 && run.saves <= elapsed / CHECKPOINT_MS, "{} saves in {elapsed} ms: at most one per checkpoint", run.saves);
    assert_eq!(run.failures, 0);
    assert!(run.bytes >= run.saves * (size - 1_000), "every save rewrites the whole file (a done game is 3 bytes shorter than a pending one)");

    // Rulings 15-R3 and 15-R3b: a failure is an outcome the cache cannot reconstruct, on disk at
    // once -- as one small journal record, not a whole-file save -- and folded into the next
    // snapshot, which empties the journal.
    fake.script([JobPoll::Failed("worker exited".into())]);
    tick(&mut s, &fake, 1);
    let failed = s.save_stats();
    assert_eq!((failed.saves, failed.journal_appends), (run.saves, 1), "a failure is journaled at once, with no snapshot");
    let per_failure = failed.journal_bytes;
    assert!(per_failure < 1_024, "a failure costs a {per_failure}-byte journal record");
    assert_eq!(std::fs::metadata(queue::journal_path(dir)).unwrap().len(), per_failure);

    s.apply(PresolverCommand::Pause);
    let paused = s.save_stats();
    assert_eq!(paused.saves, failed.saves + 1, "a pause is saved at once");
    assert_eq!(std::fs::metadata(queue::journal_path(dir)).unwrap().len(), 0, "and the snapshot folds the journal");
    for _ in 0..10 {
        tick(&mut s, &fake, CHECKPOINT_MS);
    }
    s.shutdown();
    assert_eq!(s.save_stats(), paused, "an unchanged queue is never rewritten, not even at shutdown");

    let plan = 2 * JOBS * size; // the plan's save after every launch and every outcome
    let tier_one_checkpoints = 7_020 * SOLVE_MS / CHECKPOINT_MS;
    eprintln!(
        "queue.json with every slot bound: {size} bytes; {JOBS} jobs over {elapsed} ms: {} saves, {} bytes \
         (the plan's policy: {} saves, {plan} bytes); tier 1 (7,020 jobs x {SOLVE_MS} ms) projects to at most \
         {tier_one_checkpoints} checkpoint saves, {} bytes (plan: {} saves, {} bytes); each failure outcome adds \
         one journal record at once: {per_failure} bytes",
        run.saves,
        run.bytes,
        2 * JOBS,
        tier_one_checkpoints * size,
        2 * 7_020,
        2 * 7_020 * size
    );
    assert!(run.bytes * 4 < plan, "checkpointing writes a fraction of the plan's volume");
}

/// Ruling S8: a save that fails is counted and reported, the in-memory queue keeps the change, and
/// the save is retried at the next checkpoint -- never dropped with `let _ = queue.save()`.
#[test]
fn a_failed_save_is_reported_and_retried_never_ignored() {
    let tmp = TempDir::new("sched-save-error");
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    let path = queue::queue_path(tmp.path());
    std::fs::create_dir(&path).unwrap(); // queue.json is a directory: every publication fails
    s.apply(PresolverCommand::Pause);
    assert_eq!((s.save_stats().saves, s.save_stats().failures), (0, 1), "the failure is counted, not ignored");
    assert!(s.status().paused, "the in-memory queue keeps the change");
    tick(&mut s, &fake, CHECKPOINT_MS - 1);
    assert_eq!(s.save_stats().failures, 1, "the retry waits for the next checkpoint");
    std::fs::remove_dir(&path).unwrap();
    tick(&mut s, &fake, 1);
    assert_eq!((s.save_stats().saves, s.save_stats().failures), (1, 1), "and then succeeds");
    assert!(saved(tmp.path()).paused);
}

/// Fix round 1, R3 (ruling 15-R3), the reviewer's reproduction, through the failure journal (fix
/// round 2, ruling 15-R3b): an outcome the cache cannot reconstruct -- a failure, with its attempt
/// count and absolute retry deadline -- is on disk before the scheduler goes on, as one journal
/// record, with no snapshot between the failures. The same slot fails four times, 30 s apart;
/// after each failure, a process that starts 10 s later by the wall clock (the old one lost, no
/// shutdown) replays that failure from the journal with the 20 s of backoff that remain, and the
/// fourth as terminal. Reopened for real without a shutdown, the retry budget is still spent.
#[test]
fn every_failure_survives_a_crash_before_the_checkpoint_with_its_remaining_backoff() {
    let tmp = TempDir::new("sched-crash-failures");
    let dir = tmp.path();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(dir, &fake);
    let slot = s.queue().sweep_order()[0].clone();
    for n in 1..=4_u8 {
        let journaled = journal_len(dir);
        fake.script([JobPoll::Failed("worker exited".into())]);
        tick(&mut s, &fake, queue::RETRY_BACKOFF_MS); // the idle gate, then each retry as it comes due
        assert_eq!(fake.slots_submitted().last(), Some(&slot), "attempt {n} runs the same slot");
        let mono = 5_000; // the next process's own monotonic timeline
        let crashed = open_at(dir, &FakeClock::new(mono, fake.clock.unix_ms() + 10_000));
        let item = crashed.item(&slot).unwrap();
        assert_eq!((item.status, item.attempts), (TaskStatus::Failed { n }, n), "failure {n} survives the crash");
        if n < 4 {
            let due = |at: u64| crashed.next_pending(at).map(|i| i.identity_hex()) == Some(slot.clone());
            assert!(!due(mono + 19_999) && due(mono + 20_000), "failure {n}: 20 s of its backoff remain after the crash");
        } else {
            assert_eq!(item.retry_after_unix_ms, u64::MAX, "the fourth failure is terminal");
        }
        let stats = s.save_stats();
        assert_eq!((stats.saves, stats.journal_appends), (0, u64::from(n)), "one journal record per failure, and no snapshot at all");
        let record = journal_len(dir) - journaled;
        assert!(record > 0 && record < 1_024, "failure {n} appended a {record}-byte record");
        assert!(!queue::queue_path(dir).exists(), "no snapshot has been written");
    }
    assert_eq!(s.save_stats().journal_bytes, journal_len(dir), "the journal holds exactly what was appended");
    drop(s); // process loss: no shutdown, no forced save
    let restarted = fake.restarted(FakeClock::new(0, UNIX + 120_001));
    let (s, _live) = scheduler(dir, &restarted);
    let after = s.queue().item(&slot).unwrap();
    assert_eq!((after.status, after.attempts), (TaskStatus::Failed { n: 4 }, 4), "all four failures survive: the retry budget stays spent");
}

/// Fix round 1, R3, through the journal (fix round 2, ruling 15-R3b): when the journal write of
/// such an outcome fails, nothing new is launched until it is on disk. The launch path tries the
/// write again every `SAVE_RETRY_MS`, and launches resume once it succeeds.
#[test]
fn a_failure_whose_journal_write_fails_holds_new_launches_until_it_is_written() {
    let tmp = TempDir::new("sched-unsaved-failure");
    let dir = tmp.path();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(dir, &fake);
    let path = queue::journal_path(dir);
    std::fs::create_dir(&path).unwrap(); // queue.journal is a directory: every append fails
    fake.script([JobPoll::Failed("worker exited".into())]);
    tick(&mut s, &fake, 30_000);
    let stats = s.save_stats();
    assert_eq!((stats.journal_appends, stats.journal_failures, stats.saves), (0, 1, 0), "the failure's journal write was tried at once");
    tick(&mut s, &fake, SAVE_RETRY_MS - 1);
    assert_eq!((fake.submitted().len(), s.save_stats().journal_failures), (1, 1), "no launch, and no second try, before SAVE_RETRY_MS");
    std::fs::remove_dir(&path).unwrap();
    tick(&mut s, &fake, 1);
    assert_eq!((s.save_stats().journal_appends, s.save_stats().journal_failures), (1, 1), "the write is tried again, and succeeds");
    assert_eq!(fake.submitted().len(), 2, "then launches resume");
    let slot = &fake.slots_submitted()[0];
    let crashed = open_at(dir, &FakeClock::new(0, UNIX + 60_000));
    assert_eq!(crashed.item(slot).unwrap().status, TaskStatus::Failed { n: 1 }, "the failure is on disk");
}

/// The failure journal's size on disk under the cache root `dir` (0 when it does not exist).
fn journal_len(dir: &Path) -> u64 {
    std::fs::metadata(queue::journal_path(dir)).map_or(0, |m| m.len())
}

/// Fix round 2 (ruling 15-R3b): a sweep of systematic preparation failures -- here 200 slots whose
/// chart node is missing, back to back -- writes one small journal record per failure, kilobytes
/// in all, and never a snapshot per failure; and every one of them survives a crash.
#[test]
fn a_sweep_of_systematic_preparation_failures_writes_kilobytes_never_a_snapshot_per_failure() {
    const N: usize = 200;
    let tmp = TempDir::new("sched-systematic");
    let dir = tmp.path();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(dir, &fake);
    let slots = first_slots(&s, N);
    fake.state().failing_preparation.extend(slots.iter().map(QueueItem::identity_hex));
    tick(&mut s, &fake, 30_000);
    for _ in 1..N {
        assert_eq!(s.wait_hint(), Duration::ZERO, "each failure is followed at once by the next preparation");
        s.step();
    }
    let stats = s.save_stats();
    let written = stats.bytes + stats.journal_bytes;
    eprintln!(
        "{N} systematic preparation failures wrote {written} bytes to the queue directory: {} journal records, {} bytes ({} bytes each on average), {} snapshots",
        stats.journal_appends,
        stats.journal_bytes,
        stats.journal_bytes / N as u64,
        stats.saves
    );
    assert_eq!((stats.saves, stats.journal_appends), (0, N as u64), "one journal record per failure, never a snapshot");
    assert!(written <= 200 * 1_024, "{written} bytes: kilobytes, not a snapshot per failure");
    assert_eq!(journal_len(dir), stats.journal_bytes, "the bytes on disk are the bytes counted");
    assert!(fake.submitted().is_empty());

    let crashed = open_at(dir, &FakeClock::new(0, UNIX + 40_000));
    for slot in &slots {
        let item = crashed.item(&slot.identity_hex()).unwrap();
        assert_eq!((item.status, item.attempts, item.game), (TaskStatus::Failed { n: 1 }, 1, None), "every failure survives a crash");
    }
}

/// Fix round 2 (ruling 15-R3b): the journal is folded into the next snapshot and emptied only
/// after it; a crash between a journal append and the next snapshot replays the record over the
/// older snapshot -- attempt count and remaining backoff restored -- and a crash after a snapshot
/// but before the journal was emptied never replays a record the snapshot already holds over a
/// later change.
#[test]
fn journal_replay_after_a_crash_between_an_append_and_the_next_snapshot() {
    // A crash after the second failure's append, with the first folded into a checkpoint.
    let tmp = TempDir::new("sched-journal-replay");
    let dir = tmp.path();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(dir, &fake);
    let slot = s.queue().sweep_order()[0].clone();
    fake.script([JobPoll::Failed("worker exited".into())]);
    tick(&mut s, &fake, 30_000); // failure 1: journaled
    assert!(journal_len(dir) > 0);
    tick(&mut s, &fake, CHECKPOINT_MS); // the retry runs, and the checkpoint folds the journal
    assert_eq!(s.save_stats().saves, 1);
    assert_eq!(journal_len(dir), 0, "the journal is emptied once the snapshot holding it is durable");
    let snapshot = saved(dir);
    assert_eq!(snapshot.journal_seq, 1, "the snapshot names the last record it holds");
    let game = snapshot.items[&slot].game.unwrap().identity_hex();
    assert_eq!((&snapshot.games[&game].status, snapshot.games[&game].attempts), (&TaskStatus::Failed { n: 1 }, 2), "saved with the retry in flight");
    fake.script([JobPoll::Failed("worker exited again".into())]);
    tick(&mut s, &fake, 1); // failure 2: journaled only
    assert_eq!(s.save_stats().saves, 1, "no snapshot for the second failure");
    drop(s); // crash
    let mono = 9_000;
    let reopened = open_at(dir, &FakeClock::new(mono, fake.clock.unix_ms() + 10_000));
    let item = reopened.item(&slot).unwrap();
    assert_eq!(
        (item.status, item.attempts, item.last_error.as_deref()),
        (TaskStatus::Failed { n: 2 }, 2, Some("worker exited again")),
        "the journal record replays over the older snapshot"
    );
    let due = |at: u64| reopened.next_pending(at).map(|i| i.identity_hex()) == Some(slot.clone());
    assert!(!due(mono + 19_999) && due(mono + 20_000), "with the 20 s of backoff that remain");

    // A crash after a checkpoint's snapshot but before the journal was emptied.
    let tmp = TempDir::new("sched-journal-stale");
    let dir = tmp.path();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().durable = true;
    let (mut s, _live) = scheduler(dir, &fake);
    let slot = s.queue().sweep_order()[0].clone();
    fake.script([JobPoll::Failed("worker exited".into())]);
    tick(&mut s, &fake, 30_000); // failure 1: journaled
    let stale = std::fs::read(queue::journal_path(dir)).unwrap();
    fake.script([finished()]);
    tick(&mut s, &fake, 30_000); // the retry completes: Done (a verified completion, not journaled)
    assert_eq!(s.queue().item(&slot).unwrap().status, TaskStatus::Done);
    tick(&mut s, &fake, CHECKPOINT_MS); // the checkpoint holds Done and empties the journal
    assert_eq!((s.save_stats().saves, journal_len(dir)), (1, 0));
    drop(s);
    std::fs::write(queue::journal_path(dir), &stale).unwrap(); // as if the emptying never happened
    let reopened = open_at(dir, &FakeClock::new(0, UNIX + 400_000));
    assert_eq!(reopened.item(&slot).unwrap().status, TaskStatus::Done, "a record the snapshot already holds is never replayed over a later change");
}

/// Fix round 2 (ruling 15-R3b): the journal never grows past `JOURNAL_MAX_BYTES`. A failure whose
/// record would not fit is folded, with everything else, into a snapshot at once -- which empties
/// the journal -- and survives a crash like any other.
#[test]
fn a_full_journal_is_folded_into_a_snapshot_instead_of_growing() {
    use sha2::{Digest, Sha256};
    let tmp = TempDir::new("sched-journal-full");
    let dir = tmp.path();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(dir, &fake);
    let first = s.queue().sweep_order()[0].clone();
    fake.state().failing_preparation.insert(first.clone());
    tick(&mut s, &fake, 30_000); // one real record, to copy
    let line = std::fs::read_to_string(queue::journal_path(dir)).unwrap();
    s.shutdown(); // its snapshot holds the record and empties the journal
    drop(s);

    // Fill the journal to its bound with records the snapshot already holds (sequence numbers up to
    // its `journal_seq`): valid, replayed over nothing, but taking up the room.
    let json: serde_json::Value = serde_json::from_str(line.trim_end().split_once(' ').unwrap().1).unwrap();
    let mut filler = Vec::new();
    let mut seq = 0_u64;
    loop {
        let mut record = json.clone();
        record["seq"] = serde_json::json!(seq + 1);
        let body = serde_json::to_string(&record).unwrap();
        let next = format!("{} {body}\n", hex(&Sha256::digest(body.as_bytes()).into()));
        if (filler.len() + next.len()) as u64 > queue::JOURNAL_MAX_BYTES {
            break;
        }
        filler.extend_from_slice(next.as_bytes());
        seq += 1;
    }
    let mut snapshot = saved(dir);
    snapshot.journal_seq = seq;
    queue::save_queue(&queue::queue_path(dir), &snapshot).unwrap();
    std::fs::write(queue::journal_path(dir), &filler).unwrap();
    assert!(queue::JOURNAL_MAX_BYTES - (filler.len() as u64) < 400, "the room left is smaller than a game's record");

    let restarted = fake.restarted(FakeClock::new(0, UNIX + 100_000));
    restarted.state().failing_preparation.clear();
    restarted.state().submit_error = Some("worker unavailable".into());
    let (mut s, _live) = scheduler(dir, &restarted);
    tick(&mut s, &restarted, 30_000); // the first slot is prepared and bound; its submission is refused
    let stats = s.save_stats();
    assert_eq!((stats.journal_appends, stats.saves), (0, 1), "the record does not fit: a snapshot folds it instead");
    assert_eq!(journal_len(dir), 0, "and empties the journal");
    drop(s); // crash
    let crashed = open_at(dir, &FakeClock::new(0, UNIX + 140_000));
    let item = crashed.item(&first).unwrap();
    assert_eq!((item.status, item.attempts, item.game.is_some()), (TaskStatus::Failed { n: 1 }, 1, true), "the failure survives the crash");
}

/// Fix round 2 (ruling 15-R3b): a torn or corrupt journal is handled deterministically. Replay
/// stops at the last complete, valid record -- a torn last line, a checksum mismatch or garbage
/// ends it, and everything from there on is ignored (reported on stderr, never a panic) -- and the
/// damaged tail is cut before the next append, so a later record is never stranded behind it.
#[test]
fn a_torn_or_corrupt_journal_stops_replay_at_the_last_complete_record() {
    let tmp = TempDir::new("sched-journal-torn");
    let dir = tmp.path();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(dir, &fake);
    let slots: Vec<String> = first_slots(&s, 4).iter().map(QueueItem::identity_hex).collect();
    fake.state().failing_preparation.extend(slots.iter().cloned());
    tick(&mut s, &fake, 30_000);
    s.step();
    s.step(); // three preparation failures: three records
    drop(s);
    let journal = std::fs::read(queue::journal_path(dir)).unwrap();
    let lines: Vec<&[u8]> = journal.split_inclusive(|b| *b == b'\n').collect();
    assert_eq!(lines.len(), 3, "one line per record");
    let failed = |dir: &Path| -> Vec<bool> {
        let q = open_at(dir, &FakeClock::new(0, UNIX + 40_000));
        slots.iter().map(|id| matches!(q.item(id).unwrap().status, TaskStatus::Failed { .. })).collect()
    };
    let mut flipped = lines[1].to_vec();
    flipped[100] ^= 0x01; // inside the JSON: the checksum no longer matches
    let cases: Vec<(&str, Vec<u8>, Vec<bool>)> = vec![
        ("intact", journal.clone(), vec![true, true, true, false]),
        ("a torn last line", [lines[0], lines[1], &lines[2][..lines[2].len() / 2]].concat(), vec![true, true, false, false]),
        ("a last line without its line end", [lines[0], lines[1], &lines[2][..lines[2].len() - 1]].concat(), vec![true, true, false, false]),
        ("a corrupt middle record", [lines[0], &flipped[..], lines[2]].concat(), vec![true, false, false, false]),
        ("a record out of sequence", [lines[0], lines[2], lines[1]].concat(), vec![true, false, true, false]),
        ("garbage", b"not a journal\n\x00\xff\xfe".to_vec(), vec![false, false, false, false]),
        ("empty", Vec::new(), vec![false, false, false, false]),
    ];
    for (name, bytes, expected) in cases {
        std::fs::write(queue::journal_path(dir), &bytes).unwrap();
        assert_eq!(failed(dir), expected, "{name}: replay stops at the last complete, valid record");
    }

    // A new process on a torn journal: its next record lands after the last valid one and replays.
    std::fs::write(queue::journal_path(dir), [lines[0], &lines[1][..10]].concat()).unwrap();
    let restarted = fake.restarted(FakeClock::new(0, UNIX + 100_000));
    let (mut s, _live) = scheduler(dir, &restarted);
    tick(&mut s, &restarted, 30_000); // the first slot's retry is due and fails again
    s.step(); // the second slot (its record was torn off) fails again
    s.step(); // and the third
    drop(s);
    assert_eq!(failed(dir), vec![true, true, true, false], "the torn bytes were cut before the next append");
}

/// Fix round 1, R3: a preparation failure (charged to the slot's own record) and a refused
/// submission (charged to the bound game) are saved before the scheduler goes on too.
#[test]
fn preparation_and_submission_failures_survive_a_crash_before_the_checkpoint() {
    let tmp = TempDir::new("sched-crash-launch");
    let dir = tmp.path();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let (mut s, _live) = scheduler(dir, &fake);
    let slots = first_slots(&s, 2);
    fake.state().failing_preparation.insert(slots[0].identity_hex());
    fake.state().submit_error = Some("worker unavailable".into());
    tick(&mut s, &fake, 30_000); // the first slot's preparation fails
    let crashed = open_at(dir, &FakeClock::new(0, UNIX + 30_000));
    let a = crashed.item(&slots[0].identity_hex()).unwrap();
    assert_eq!((a.status, a.attempts, a.game), (TaskStatus::Failed { n: 1 }, 1, None), "a preparation failure is on disk at once");
    s.step(); // the second slot is prepared and bound, and its submission is refused
    let crashed = open_at(dir, &FakeClock::new(0, UNIX + 30_000));
    let b = crashed.item(&slots[1].identity_hex()).unwrap();
    assert_eq!(
        (b.status, b.attempts, b.game.map(|g| g.identity)),
        (TaskStatus::Failed { n: 1 }, 1, Some(game_of(&slots[1], GEN_A))),
        "a refused submission is on disk at once, as the bound game's failure"
    );
    assert_eq!((s.save_stats().saves, s.save_stats().journal_appends), (0, 2), "two journal records, no snapshot");
    assert!(fake.submitted().is_empty());
}

/// Task 14 left the reconciliation cadence and chunking to this task (its report, F4 OQ4): the
/// startup sweep and each periodic one decide at most `RECONCILE_CHUNK` games per idle iteration, so
/// commands are handled between chunks; a pause stops it like any background work; the games whose
/// entries were evicted are demoted, the others stay done; and a sweep starts again every
/// `RECONCILE_PERIOD_MS`. Fix round 1, R1: each iteration reconciles its chunk before it chooses
/// work, so the first job is the earliest evicted slot, and only verified completions are
/// published.
#[test]
fn reconciliation_runs_in_chunks_between_commands_and_again_periodically() {
    let tmp = TempDir::new("sched-sweep");
    let dir = tmp.path();
    let n = 2 * RECONCILE_CHUNK + RECONCILE_CHUNK / 2;
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    let mut games = BTreeSet::new();
    let order = {
        let mut q = open_at(dir, &FakeClock::new(0, UNIX));
        q.set_generation(GEN_A);
        let order = q.sweep_order().to_vec();
        for (i, id) in order[..n].iter().enumerate() {
            let item = q.item(id).unwrap();
            let (key, spr) = prepared(&item, &hex(&GEN_A));
            q.bind(id, &key, spr);
            q.record_launch(id);
            q.record_done(id);
            games.insert(game_of(&item, GEN_A));
            if i % 2 == 0 {
                fake.state().exists.insert(game_of(&item, GEN_A)); // the odd ones were evicted
            }
        }
        q.save().unwrap();
        order
    };
    let asked = |fake: &Fake| fake.state().asked.iter().filter(|b| games.contains(&b.identity)).count();
    let (mut s, _live) = scheduler(dir, &fake);
    assert!(s.reconciling(), "a sweep starts with the scheduler");
    assert_eq!(s.status().done, 0, "no persisted completion is published before the sweep verifies it");
    tick(&mut s, &fake, 29_999);
    assert_eq!(asked(&fake), 0, "nothing runs before the idle gate");
    tick(&mut s, &fake, 1); // reconciles one chunk, then launches the earliest slot it demoted
    assert_eq!(fake.slots_submitted(), vec![order[1].clone()], "the first evicted slot runs first");
    assert_eq!(asked(&fake), RECONCILE_CHUNK + 1, "one chunk, and the launch path's own check of the bound game");
    assert_eq!(s.status().done, (RECONCILE_CHUNK / 2) as u32, "the chunk's verified half is published");
    s.apply(PresolverCommand::Pause);
    s.step();
    assert_eq!(asked(&fake), RECONCILE_CHUNK + 1, "a paused scheduler reconciles nothing either");
    s.apply(PresolverCommand::Resume);
    tick(&mut s, &fake, 30_000);
    assert_eq!(asked(&fake), 2 * RECONCILE_CHUNK + 1);
    tick(&mut s, &fake, 1);
    assert_eq!(asked(&fake), n + 1);
    assert!(!s.reconciling(), "the sweep is complete");
    assert_eq!(s.status().done, (n / 2) as u32, "the evicted half is demoted, the rest stays done");
    tick(&mut s, &fake, 1);
    assert_eq!(asked(&fake), n + 1, "each game once per sweep");

    tick(&mut s, &fake, RECONCILE_PERIOD_MS);
    assert!(s.reconciling(), "a new sweep starts every RECONCILE_PERIOD_MS");
    tick(&mut s, &fake, 1);
    assert_eq!(asked(&fake), n + 1 + RECONCILE_CHUNK, "the running game is skipped, one chunk is decided");
}

/// One iteration, with a completing job scripted for whatever it launches.
fn step_completing(s: &mut Scheduler, fake: &Fake) {
    if fake.state().polls.is_empty() {
        fake.script([finished()]);
    }
    s.step();
}

/// Fix round 1, R1 (ruling 15-R1), the reviewer's reproduction across a tier boundary: after a
/// restart, no launch overtakes a persisted completion earlier in sweep order that the startup
/// sweep has not verified yet, and only verified completions are published. A previous process
/// completed tier 1 and tier 2's first two boards; since then the cache evicted three of those
/// entries, on both sides of the tier boundary. The jobs run in sweep order -- tier 1's first
/// slot, tier 1's last, the tier-2 one -- and only then does the sweep go on at the saved cursor in
/// tier 2, although verifying everything before it takes hundreds of chunks, between which
/// commands are handled.
#[test]
fn a_restart_verifies_earlier_completions_before_launching_later_work_across_a_tier_boundary() {
    let tmp = TempDir::new("sched-startup-order");
    let dir = tmp.path();
    const DONE: usize = 7_020 + 16; // tier 1, and tier 2's eight scenarios on its first two boards
    let evicted = [0, 7_019, 7_025];
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().durable = true;
    let order = {
        let mut q = open_at(dir, &FakeClock::new(0, UNIX));
        q.set_generation(GEN_A);
        let order = q.sweep_order().to_vec();
        for (i, id) in order[..DONE].iter().enumerate() {
            let item = q.item(id).unwrap();
            let (key, spr) = prepared(&item, &hex(&GEN_A));
            q.bind(id, &key, spr);
            q.record_launch(id);
            q.record_done(id);
            if !evicted.contains(&i) {
                fake.state().exists.insert(key.scenario_identity(spr));
            }
        }
        q.advance_cursor();
        q.save().unwrap();
        order
    };
    let (mut s, _live) = scheduler(dir, &fake);
    assert_eq!((s.queue().cursor(), s.queue().tier_counts().0), ([1, 2, 0], [7_020, 16, 0]), "the saved sweep had moved on into tier 2");
    let status = s.status();
    assert_eq!((status.done, status.tier_done), (0, [0, 0, 0]), "no persisted completion is published before the startup sweep verifies it");

    tick(&mut s, &fake, 29_999);
    assert!(fake.submitted().is_empty() && fake.state().asked.is_empty(), "nothing runs before the idle gate");
    fake.script([finished()]);
    tick(&mut s, &fake, 1);
    let at = |id: &String| order.iter().position(|o| o == id).unwrap();
    assert_eq!(fake.slots_submitted().iter().map(at).collect::<Vec<_>>(), vec![0], "the first chunk found tier 1's first slot missing: it runs first");
    let status = s.status();
    let chunk = RECONCILE_CHUNK as u32;
    assert_eq!((status.done, status.tier_done), (chunk, [chunk, 0, 0]), "published: the chunk's verified completions and the re-solved slot");

    // The sweep goes on chunk by chunk; a pause between two chunks stops it, a resume lets it go on.
    for _ in 0..10 {
        step_completing(&mut s, &fake);
    }
    assert_eq!(fake.submitted().len(), 1, "the cursor slot waits behind completions the sweep has not verified");
    let asked = fake.state().asked.len();
    s.apply(PresolverCommand::Pause);
    step_completing(&mut s, &fake);
    assert_eq!(fake.state().asked.len(), asked, "a command between two chunks is handled at once");
    s.apply(PresolverCommand::Resume);
    tick(&mut s, &fake, 30_000);
    let mut steps = 12;
    while fake.submitted().len() < 4 {
        step_completing(&mut s, &fake);
        steps += 1;
        assert!(steps < 1_000, "the startup sweep ends");
    }
    assert_eq!(
        fake.slots_submitted().iter().map(at).collect::<Vec<_>>(),
        vec![0, 7_019, 7_025, DONE],
        "the evicted slots in sweep order, across the tier boundary, and only then the cursor slot"
    );
    assert!(steps >= (DONE - evicted.len()) / RECONCILE_CHUNK, "{steps} iterations: every earlier completion was verified first, in chunks");
    assert!(!s.reconciling());
    let status = s.status();
    assert_eq!((status.done, status.tier_done), (DONE as u32 + 1, [7_020, 17, 0]), "every completion verified or re-solved, and the cursor slot done");
}

/// Fix round 1, R1: the same ordering holds when a periodic sweep uncovers earlier work. Once it
/// starts, no candidate later in sweep order than a completion it has not verified yet is
/// launched, so an entry evicted since the last sweep is solved again before the cursor moves on.
#[test]
fn a_periodic_sweep_that_uncovers_earlier_work_runs_it_before_later_work() {
    let tmp = TempDir::new("sched-periodic-order");
    let dir = tmp.path();
    let n = 2 * RECONCILE_CHUNK + RECONCILE_CHUNK / 2;
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().durable = true;
    let order = {
        let mut q = open_at(dir, &FakeClock::new(0, UNIX));
        q.set_generation(GEN_A);
        let order = q.sweep_order().to_vec();
        for id in &order[..n] {
            let item = q.item(id).unwrap();
            let (key, spr) = prepared(&item, &hex(&GEN_A));
            q.bind(id, &key, spr);
            q.record_launch(id);
            q.record_done(id);
            fake.state().exists.insert(key.scenario_identity(spr));
        }
        q.advance_cursor();
        q.save().unwrap();
        order
    };
    let (mut s, _live) = scheduler(dir, &fake);
    tick(&mut s, &fake, 30_000);
    while s.reconciling() || fake.submitted().is_empty() {
        s.step(); // the startup sweep, and the first job at the cursor
    }
    assert_eq!(fake.slots_submitted(), vec![order[n].clone()]);

    // Later the cache evicts a completed entry that the next sweep reaches only in its third chunk.
    let late = n - 5;
    fake.state().exists.remove(&game_of(&s.queue().item(&order[late]).unwrap(), GEN_A));
    tick(&mut s, &fake, RECONCILE_PERIOD_MS);
    assert!(s.reconciling(), "a periodic sweep started while the job runs");
    fake.script([finished()]);
    let mut steps = 0;
    while fake.submitted().len() < 2 {
        step_completing(&mut s, &fake);
        steps += 1;
        assert!(steps < 100);
    }
    assert_eq!(s.queue().item(&order[n]).unwrap().status, TaskStatus::Done, "the running job finished");
    let second = order.iter().position(|o| *o == fake.slots_submitted()[1]).unwrap();
    assert_eq!(second, late, "the uncovered earlier work (sweep position {late}) runs before the cursor (position {}) moves on", n + 1);
}

/// Fix round 3 (re-review 1, N1): a real generation change must restart a verifying sweep, not
/// just clear stale bindings, so a generation that becomes current again is re-checked before its
/// persisted completions count -- the same guarantee ruling 15-R1 already gives the generation a
/// queue opens under. Twenty slots are persisted `Done` under GEN_A; slots 16-19 straddle the
/// `RECONCILE_CHUNK` (16) boundary, and slot 19's entry has since been evicted. The startup sweep
/// verifies slots 0-15, then the source flips to GEN_B -- ending the sweep in one idle iteration,
/// since nothing is bound under GEN_B yet -- and back to GEN_A before the sweep ever reaches slot
/// 19. Without a fresh sweep, slot 19 is stuck showing `Done` (only the next `RECONCILE_PERIOD_MS`
/// sweep, hours later, would catch it) while its stale completion is published at once and slot
/// 20 -- genuinely new work -- runs right past it; with one, published counts stay conservative
/// until verified, and slot 19 is found and relaunched before slot 20 runs.
#[test]
fn a_generation_flip_back_during_the_startup_sweep_reverifies_before_publishing() {
    let tmp = TempDir::new("sched-generation-flip");
    let dir = tmp.path();
    const DONE: usize = 20;
    const MISSING: usize = DONE - 1; // slot 19: in the leftover chunk past RECONCILE_CHUNK (16)
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().durable = true;
    let order = {
        let mut q = open_at(dir, &FakeClock::new(0, UNIX));
        q.set_generation(GEN_A);
        let order = q.sweep_order().to_vec();
        for (i, id) in order[..DONE].iter().enumerate() {
            let item = q.item(id).unwrap();
            let (key, spr) = prepared(&item, &hex(&GEN_A));
            q.bind(id, &key, spr);
            q.record_launch(id);
            q.record_done(id);
            if i != MISSING {
                fake.state().exists.insert(key.scenario_identity(spr));
            }
        }
        q.save().unwrap();
        order
    };
    let at = |id: &String| order.iter().position(|o| o == id).unwrap();
    let (mut s, _live) = scheduler(dir, &fake);

    tick(&mut s, &fake, 30_000); // the startup sweep's first chunk verifies slots 0-15
    assert!(s.reconciling(), "the RECONCILE_CHUNK boundary leaves slots 16-19 unverified");
    assert!(fake.submitted().is_empty(), "nothing later than the unverified slots runs (ruling 15-R1)");

    fake.state().generation = GEN_B;
    s.apply(PresolverCommand::SourceChanged);
    s.step(); // one idle iteration under B: nothing is bound yet, so the sweep ends at once
    assert!(!s.reconciling(), "the gap fix round 1 left: the sweep ends without ever reaching slot 19");

    fake.state().generation = GEN_A;
    s.apply(PresolverCommand::SourceChanged);
    assert!(s.reconciling(), "N1: a real generation change restarts a verifying sweep");
    assert_eq!(
        s.status().done,
        0,
        "N1: slots 1-19's persisted completions are not published before this fresh sweep verifies them"
    );

    let mut steps = 0;
    loop {
        let submitted: Vec<usize> = fake.slots_submitted().iter().map(at).collect();
        if submitted.contains(&MISSING) && submitted.contains(&DONE) {
            break;
        }
        step_completing(&mut s, &fake);
        steps += 1;
        assert!(
            steps < 1_000,
            "N1: slot 19 is stuck (only the next periodic sweep, hours later, would find it) and slot 20 never waits for it"
        );
    }
    let submitted: Vec<usize> = fake.slots_submitted().iter().map(at).collect();
    let missing_at = submitted.iter().position(|&p| p == MISSING).unwrap();
    let later_at = submitted.iter().position(|&p| p == DONE).unwrap();
    assert!(missing_at < later_at, "N1: slot 19's missing completion is relaunched before slot 20 -- genuinely new work -- runs");
}

/// A queue with nothing to launch is not rescanned every tick -- with every slot bound, a scan that
/// finds nothing reads all 42,120 slots (about 36 ms in a release build): the scheduler asks again
/// once the queue changes, or after `RESCAN_MS`.
#[test]
fn an_exhausted_queue_is_rescanned_after_a_change_or_every_rescan_ms() {
    let tmp = TempDir::new("sched-rescan");
    let dir = tmp.path();
    {
        let mut q = open_at(dir, &FakeClock::new(0, UNIX));
        q.set_generation(GEN_A);
        for item in slots(&q, dir).into_values() {
            let (key, spr) = prepared(&item, &hex(&GEN_A));
            q.bind(&item.identity_hex(), &key, spr);
        }
        q.reconcile(&|_| true);
        q.save().unwrap();
    }
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().exists_everywhere = true;
    let (mut s, _live) = scheduler(dir, &fake);
    assert_eq!(s.status().done, 0, "persisted completions await the startup sweep's verification (fix round 1, R1)");
    tick(&mut s, &fake, 30_000);
    assert_eq!(s.pending_scans(), 1, "the first idle iteration looks for work");
    for _ in 0..3 {
        tick(&mut s, &fake, 1);
    }
    assert_eq!(s.pending_scans(), 1, "nothing changed: no rescan");
    tick(&mut s, &fake, RESCAN_MS);
    assert_eq!(s.pending_scans(), 2, "RESCAN_MS later it looks again (a retry may have come due)");
    fake.state().generation = GEN_B;
    s.apply(PresolverCommand::SourceChanged);
    s.step();
    assert_eq!(s.pending_scans(), 3, "a change is looked at in the next iteration");
    assert_eq!(fake.state().asked.last().map(|b| b.generation), Some(GEN_B), "and the first stale slot is prepared again");
}

/// While iterations follow back to back (`wait_hint` zero), the thread publishes a status at most
/// once per `BUSY_PUBLISH_MS` of queue-clock time -- a publication after a change recounts every
/// slot (about 24 ms in a release build) -- and after every iteration otherwise.
#[test]
fn back_to_back_iterations_publish_at_most_once_per_busy_publish_ms() {
    let tmp = TempDir::new("sched-publish");
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().exists_everywhere = true; // every slot is found on disk: back-to-back iterations
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    assert!(s.should_publish(), "the first status is published");
    tick(&mut s, &fake, 30_000);
    assert_eq!(s.wait_hint(), Duration::ZERO);
    assert!(s.should_publish(), "a busy stretch publishes once ...");
    for _ in 0..5 {
        s.step();
        assert_eq!(s.wait_hint(), Duration::ZERO);
        assert!(!s.should_publish(), "... and then waits BUSY_PUBLISH_MS");
    }
    tick(&mut s, &fake, BUSY_PUBLISH_MS);
    assert!(s.should_publish());
    assert_eq!(s.status().done, 7, "seven slots found on disk so far");
    fake.state().exists_everywhere = false;
    s.step(); // the next slot is solved: its job runs, and the thread waits a tick again
    assert_eq!(fake.submitted().len(), 1);
    assert_eq!(s.wait_hint(), Duration::from_millis(TICK_MS));
    assert!(s.should_publish());
    s.step();
    assert!(s.should_publish(), "every iteration publishes while the thread waits a tick between them");
}

/// Waits for the fake executor's next event matching `wanted`. The budget only cuts a hang short;
/// it never paces the test.
fn wait_for(events: &mpsc::Receiver<Event>, wanted: impl Fn(&Event) -> bool) -> Event {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(event) if wanted(&event) => return event,
            Ok(_) => {}
            Err(e) => panic!("the presolver thread never produced the expected event: {e}"),
        }
    }
}

/// Waits until the published status satisfies `cond`, re-reading it after each publication.
fn wait_status(p: &Presolver, events: &mpsc::Receiver<Event>, cond: impl Fn(&PresolverStatus) -> bool) -> PresolverStatus {
    loop {
        let status = p.status();
        if cond(&status) {
            return status;
        }
        wait_for(events, |e| *e == Event::Published);
    }
}

/// The `presolver` thread end to end: it runs on the executor's clock, admission sets the running
/// job's cancel flag before `notify_live_request` returns, the thread then cancels it, pause and
/// resume reach the published status, and shutdown (twice) joins the thread after saving the
/// refunded attempt.
#[test]
fn the_presolver_thread_preempts_at_admission_and_shuts_down_cleanly() {
    let tmp = TempDir::new("thread-preempt");
    let (tx, events) = mpsc::channel();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().events = Some(tx);
    let presolver = Presolver::start(tmp.path().to_path_buf(), Box::new(fake.clone()));
    wait_status(&presolver, &events, |s| s.tier_total == [7_020, 14_040, 21_060]); // the queue is open
    fake.clock.advance(30_000);
    let Event::Submitted(job) = wait_for(&events, |e| matches!(e, Event::Submitted(_))) else { unreachable!() };
    let flag = fake.submitted()[0].2.clone();

    presolver.notify_live_request();
    assert!(flag.load(Ordering::SeqCst), "admission sets the job's cancel flag before notify_live_request returns");
    wait_for(&events, |e| *e == Event::Cancelled(job));
    let status = wait_status(&presolver, &events, |s| s.running.is_none());
    assert_eq!(status.pending, ALL);
    presolver.pause();
    wait_status(&presolver, &events, |s| s.paused);
    presolver.resume();
    wait_status(&presolver, &events, |s| !s.paused);
    presolver.shutdown();
    presolver.join_for_shutdown();
    presolver.shutdown();
    presolver.join_for_shutdown(); // a repeated shutdown is a no-op

    assert_eq!(fake.submitted().len(), 1, "the idle gate held after the live request");
    let file = saved(tmp.path());
    let game = &file.games[&file.items[&fake.slots_submitted()[0]].game.unwrap().identity_hex()];
    assert_eq!((&game.status, game.attempts), (&TaskStatus::Pending, 0), "the refunded attempt was saved at shutdown");
}

/// F09 on the presolver's side: no handle call waits for the presolver thread, even while that
/// thread is blocked inside the executor; and a live request that arrives while a job is being
/// prepared stops it before submission.
#[test]
fn handle_calls_return_while_the_thread_is_blocked_and_a_live_request_stops_the_launch() {
    let tmp = TempDir::new("thread-blocked");
    let (tx, events) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().events = Some(tx);
    fake.state().gate = Some(gate);
    let presolver = Arc::new(Presolver::start(tmp.path().to_path_buf(), Box::new(fake.clone())));
    wait_status(&presolver, &events, |s| s.tier_total == [7_020, 14_040, 21_060]);
    fake.clock.advance(30_000);
    wait_for(&events, |e| *e == Event::Preparing); // the thread is now blocked inside `prepare`

    let (done, returned) = mpsc::channel();
    let handle = Arc::clone(&presolver);
    std::thread::spawn(move || {
        let _ = handle.status();
        handle.pause();
        handle.resume();
        handle.notify_hand(true);
        handle.notify_hand(false);
        handle.notify_live_request();
        handle.notify_source_changed();
        done.send(()).unwrap();
    });
    returned.recv_timeout(Duration::from_secs(120)).expect("every handle call returns while the presolver thread is blocked");
    release.send(()).unwrap();
    presolver.shutdown();
    presolver.join_for_shutdown();
    assert!(fake.submitted().is_empty(), "the live request that arrived during preparation stopped the launch");
}

/// Fix round 1, R2: a shutdown requested while the thread is blocked preparing a job -- with no
/// live request -- stops it as soon as the callback returns, without submitting that job.
#[test]
fn a_shutdown_requested_during_preparation_submits_nothing() {
    let tmp = TempDir::new("thread-stop-prepare");
    let (tx, events) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().events = Some(tx);
    fake.state().gate = Some(gate);
    let presolver = Presolver::start(tmp.path().to_path_buf(), Box::new(fake.clone()));
    wait_status(&presolver, &events, |s| s.tier_total == [7_020, 14_040, 21_060]);
    fake.clock.advance(30_000);
    wait_for(&events, |e| *e == Event::Preparing);
    presolver.shutdown();
    release.send(()).unwrap();
    presolver.join_for_shutdown();
    assert!(fake.submitted().is_empty(), "no job is submitted once shutdown is requested");
}

/// Fix round 1, R2 (ruling 15-R2): the handle's notifications go to a bounded, coalescing mailbox.
/// With the presolver thread blocked inside the executor, a flood from four threads never keeps
/// more than `MAILBOX_CAPACITY` notifications pending -- the latest hand and pause states, the
/// source-change and activity signals, the shutdown latch -- however many calls are made. Once the
/// callback returns, the thread applies the final states (the last pause, the new generation)
/// before the latched shutdown, and stops at once although producers keep notifying, without
/// submitting the job it was preparing.
#[test]
fn a_notification_flood_while_the_thread_is_blocked_stays_bounded_and_delivers_the_final_state() {
    let tmp = TempDir::new("thread-flood");
    let (tx, events) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().events = Some(tx);
    fake.state().gate = Some(gate);
    let presolver = Arc::new(Presolver::start(tmp.path().to_path_buf(), Box::new(fake.clone())));
    wait_status(&presolver, &events, |s| s.tier_total == [7_020, 14_040, 21_060]);
    fake.clock.advance(30_000);
    wait_for(&events, |e| *e == Event::Preparing); // the thread is now blocked inside `prepare`

    const ROUNDS: usize = 20_000;
    let flood: Vec<_> = (0..4)
        .map(|_| {
            let p = Arc::clone(&presolver);
            std::thread::spawn(move || {
                let mut peak = 0;
                for _ in 0..ROUNDS {
                    p.pause();
                    p.notify_hand(true);
                    p.notify_live_request();
                    p.resume();
                    p.notify_source_changed();
                    p.notify_hand(false);
                    peak = peak.max(p.pending_notifications());
                }
                peak
            })
        })
        .collect();
    let peak = flood.into_iter().map(|t| t.join().unwrap()).max().unwrap();
    assert!(peak <= MAILBOX_CAPACITY, "{} calls left at most {peak} notifications pending, never more than {MAILBOX_CAPACITY}", 4 * ROUNDS * 6);

    fake.state().generation = GEN_B;
    presolver.notify_source_changed();
    presolver.pause();
    presolver.shutdown();
    assert!(presolver.pending_notifications() <= MAILBOX_CAPACITY);
    let stop = Arc::new(AtomicBool::new(false));
    let noise = {
        let (p, stop) = (Arc::clone(&presolver), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                p.notify_live_request();
                p.notify_hand(false);
                p.notify_source_changed();
            }
        })
    };
    release.send(()).unwrap(); // the callback returns
    let (joined, stopped) = mpsc::channel();
    let p = Arc::clone(&presolver);
    std::thread::spawn(move || {
        p.join_for_shutdown();
        joined.send(()).unwrap();
    });
    stopped.recv_timeout(Duration::from_secs(120)).expect("the thread stops once the callback returns, while notifications keep arriving");
    stop.store(true, Ordering::SeqCst);
    noise.join().unwrap();

    let file = saved(tmp.path());
    assert!(file.paused, "the last pause state reached the queue before the shutdown");
    assert_eq!(file.generation, GEN_B, "and so did the last source change");
    assert!(fake.submitted().is_empty(), "the job being prepared was never submitted");
}

/// The scheduler over all of tier 1 (7,020 jobs): each slot is launched exactly once, in sweep
/// order, tier 2 starts only after tier 1, and nothing is saved until shutdown within one
/// checkpoint interval.
#[test]
#[cfg_attr(not(feature = "exhaustive"), ignore = "enable the exhaustive feature for the full tier-1 run")]
fn the_scheduler_completes_tier_one_in_sweep_order_exactly_once() {
    let tmp = TempDir::new("sched-tier-one");
    let fake = Fake::new(FakeClock::new(0, UNIX), GEN_A);
    fake.state().durable = true;
    let (mut s, _live) = scheduler(tmp.path(), &fake);
    tick(&mut s, &fake, 29_999);
    for _ in 0..7_020 {
        fake.script([finished()]);
        tick(&mut s, &fake, 1);
    }
    let order = s.queue().sweep_order()[..7_020].to_vec();
    assert_eq!(fake.slots_submitted(), order, "every tier-1 slot once, in sweep order");
    assert_eq!(s.queue().tier_counts().0, [7_020, 0, 0]);
    assert_eq!(s.queue().cursor(), [1, 0, 0]);
    tick(&mut s, &fake, 1);
    assert!(is(&s.queue().item(fake.slots_submitted().last().unwrap()).unwrap(), &tier(2)[0], &canonical_flops_ordered()[0]));
    assert_eq!(s.save_stats().saves, 0, "no checkpoint within one interval");
    s.shutdown();
    assert_eq!(s.save_stats().saves, 1);
}
