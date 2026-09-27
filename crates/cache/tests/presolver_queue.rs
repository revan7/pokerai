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
    q.set_paused(true);
    q.save().expect("the worst-case queue file must stay within the bound");
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
