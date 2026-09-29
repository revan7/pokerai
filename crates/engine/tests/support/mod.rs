//! Engine integration-test support: `CacheRig` (plan 4 Task 8), the spec section 13.1 T4 harness over the real cache,
//! and (plan 4 Task 9) `engine_with_fake_worker` / `game_config`, an `Engine` over plan 2's fake worker and clock.
//!
//! A rig materializes one template with the production tree builder (`engine::tree::build_tree_full`) at a flop street
//! root (board Kh7d2c, OOP `Seat(2)`, IP `Seat(0)`, equal effective stacks, dead 0, `bb_chips: 2`, public board-only
//! AA / KK ranges), derives the key and source inputs with the production `engine::cache_bridge::key_and_source`,
//! exports every root-street node with synthetic valid matrices, and stores that one validated `CacheEntry` through
//! the real cache writer in a temporary directory it owns. Queries are built with the production
//! `engine::cache_bridge::make_cache_query` and answered by the production `cache::Cache::lookup`, so no worker solve
//! and no timing noise enters T4.
//!
//! Synthetic matrices: at exported node index `n`, every available combo plays each legal action with the uniform
//! probability and has `EV(a) = 10 * n + a` chips for the action at index `a`, except fold, whose EV is exactly 0.
//! Only combos the actor's board-blocked canonical public range supports are available. Because the EV names the node,
//! serving a wrong path or a wrong actor's node is observable in every assertion that reads it.

use cache::entry::CacheEntry;
use cache::key::Rational;
use cache::lookup::{CacheHit, CacheQuery, Lookup};
use cache::Cache;
use core_iso::SuitPerm;
use engine::cache_bridge::{key_and_source, make_cache_query};
use engine::tree::{build_tree_full, tree_signature, TemplateSelection};
use proto::{Action, Card, Rake, Range1326, Seat, SolveInput, Street, StreetRootSnapshot};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The rig's two street-root seats: `oop` is seat 2, `ip` seat 0.
pub const OOP: Seat = Seat(2);
pub const IP: Seat = Seat(0);
/// The pot rake every rig and query uses unless a case changes it: 5%, with the scenario's own cap.
pub const RATE: f32 = 0.05;
/// A lookup budget far above the 500 ms lookup bound, so only `Cache::lookup`'s own bound applies (never a tight one).
pub const BUDGET: Duration = Duration::from_secs(5);
/// The raw stored exploitability of a rig entry: 40 bp of the pot, inside the default 50 bp target.
pub const EXPLOITABILITY_OVER_P: f64 = 0.004;

/// The three spec 13.1 T4 test templates, registered once per test binary (`with_extra` replaces the whole extra
/// set, so this `Once` is the binary's single caller of it).
pub fn install_templates() {
    static TEMPLATES: std::sync::Once = std::sync::Once::new();
    TEMPLATES.call_once(engine::tree::templates::install_cache_test_templates);
}

/// Cards from a space-separated list (`"Kh 7d 2c"`).
pub fn cards(text: &str) -> Vec<Card> {
    text.split(' ').map(|c| Card::parse(c).unwrap()).collect()
}

/// One street-root decision the rig stores or asks about: every input `make_cache_query` and `key_and_source` read.
/// `history` alternates from OOP (a heads-up street prefix, which the flop-rooted templates' requests always are).
#[derive(Clone, Debug)]
pub struct Scenario {
    pub template: String,
    pub board: Vec<Card>,
    pub oop: Seat,
    pub ip: Seat,
    pub pot: u32,
    pub stack_oop: u32,
    pub stack_ip: u32,
    pub bb_chips: u32,
    /// Public street-root ranges as the replay produces them: not yet blocked by the board, never hero-conditioned.
    pub ranges: [Range1326; 2],
    pub rake: Rake,
    pub history: Vec<Action>,
    pub target_bp: u16,
}

impl Scenario {
    /// The rig's default decision: `template` at pot `p`, effective stacks `eff` on both sides and rake cap
    /// `cap_mchips`, at the street root (empty history), target 50 bp.
    pub fn new(template: &str, p: u32, eff: u32, cap_mchips: u32) -> Scenario {
        Scenario {
            template: template.into(),
            board: cards("Kh 7d 2c"),
            oop: OOP,
            ip: IP,
            pot: p,
            stack_oop: eff,
            stack_ip: eff,
            bb_chips: 2,
            ranges: [core_ranges::parse_range("AA").unwrap(), core_ranges::parse_range("KK").unwrap()],
            rake: Rake::PotRake { rate: RATE, cap_mchips, no_flop_no_drop: true },
            history: vec![],
            target_bp: 50,
        }
    }

    /// This scenario with the street history `path` (from OOP).
    pub fn with_history(mut self, path: &[Action]) -> Scenario {
        self.history = path.to_vec();
        self
    }

    /// The street-root snapshot: the history's actions alternate between the OOP and IP seats, starting with OOP.
    pub fn snapshot(&self) -> StreetRootSnapshot {
        let history = self.history.iter().enumerate().map(|(i, a)| (if i % 2 == 0 { self.oop } else { self.ip }, *a)).collect();
        StreetRootSnapshot {
            street: Street::Flop,
            board: self.board.clone(),
            oop: self.oop,
            ip: self.ip,
            pot_root: self.pot,
            stack_oop_root: self.stack_oop,
            stack_ip_root: self.stack_ip,
            dead_this_street: 0,
            projected_from: 2,
            history,
            bb_chips: self.bb_chips,
        }
    }

    /// The solve input at this street root (the production tree builder's tree for the template and history), the
    /// engine's tree signature for it, and the canonicalizing suit permutation.
    pub fn solve_input(&self) -> (SolveInput, String, SuitPerm) {
        install_templates();
        let root = self.snapshot();
        let built = build_tree_full(&root, &TemplateSelection::from_history(&self.template, &root.history))
            .unwrap_or_else(|e| panic!("{} at {}/{} does not materialize: {e:?}", self.template, self.pot, self.stack_oop.min(self.stack_ip)));
        let signature = tree_signature(&built.tree, built.pot);
        let perm = canonical_perm(&root.board, &self.ranges);
        (SolveInput { root, ranges: self.ranges.clone(), tree: built.tree, target_bp: self.target_bp }, signature, perm)
    }

    /// The production cache query for `actor` (`"oop"` / `"ip"`) at this decision: the legal menu is the rules
    /// engine's own replay of the street root (`core_model::replay_root`), the acting seat is `actor`'s seat.
    pub fn query(&self, actor: &str) -> CacheQuery {
        let (input, signature, perm) = self.solve_input();
        let mut derived = core_model::replay_root(&input.root).expect("the scenario's history replays");
        derived.to_act = Some(match actor {
            "oop" => self.oop,
            "ip" => self.ip,
            other => panic!("actor {other:?} is neither oop nor ip"),
        });
        make_cache_query(&input, &derived, self.bb_chips, &self.rake, &[], &signature, &perm, self.target_bp, BUDGET).expect("a well-formed scenario builds its query")
    }
}

/// Ruling P4.7-D6: `perm` comes from `core_iso::canonicalize(board, [blocked_oop, blocked_ip])` over the board-blocked
/// public ranges, never the unblocked ones, exactly as the bridge's callers and the entry writer compute it (Task 10
/// adds the shared helper; until then this is the rig's one place for it). `validate_entry` re-checks the fixed point.
fn canonical_perm(board: &[Card], ranges: &[Range1326; 2]) -> SuitPerm {
    let mut blocked = ranges.clone();
    for r in &mut blocked {
        core_ranges::block_public(r, board);
    }
    core_iso::canonicalize(board, &[&blocked[0], &blocked[1]]).1
}

/// The validated entry a street-root solve of `s` would store: the production key and source inputs, the scenario's
/// own effective tree, every root-street node exported with the synthetic matrices of the module doc, raw
/// exploitability `EXPLOITABILITY_OVER_P`, `f32` mode, no reasons.
pub fn entry_for(s: &Scenario) -> CacheEntry {
    let (input, signature, perm) = s.solve_input();
    let (key, source) = key_and_source(&input, s.bb_chips, &s.rake, &signature, &perm).expect("a well-formed scenario has a key");
    let tree = input.tree;
    let nodes = tree
        .materialized
        .iter()
        .filter(|m| m.street == tree.root_street)
        .enumerate()
        .map(|(n, m)| {
            let range = &source.ranges[if m.actor == "oop" { 0 } else { 1 }];
            let width = m.actions.len();
            let available = range.0.iter().map(|w| *w > 0.0).collect::<Vec<_>>();
            let probs = available.iter().map(|a| if *a { vec![1.0 / width as f32; width] } else { vec![0.0; width] }).collect();
            let ev_chips = available
                .iter()
                .map(|a| {
                    m.actions
                        .iter()
                        .enumerate()
                        .map(|(i, action)| if !*a || *action == Action::Fold { 0.0 } else { (10 * n + i) as f32 })
                        .collect()
                })
                .collect();
            proto::worker::NodeStrategy { path: cache::entry::chip_path(&tree.materialized, &m.path).unwrap(), actor: m.actor.clone(), actions: m.actions.clone(), probs, ev_chips, available }
        })
        .collect::<Vec<_>>();
    let solution = proto::worker::StreetSolution {
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        requested: 0,
        exploitability_chips: (EXPLOITABILITY_OVER_P * source.pot as f64) as f32,
        iterations: 1000,
        memory_bytes: 1 << 20,
        mode: "f32".into(),
        locks_applied: 0,
        export: "street".into(),
    };
    let nodes = cache::entry::normalize(&solution, &tree, source.pot).expect("the synthetic solution is valid");
    let fractions = tree
        .materialized
        .iter()
        .map(|m| m.actions.iter().map(|a| cache::lookup::action_to(a).map(|to| Rational::new(to as u64, source.pot as u64).unwrap())).collect())
        .collect();
    CacheEntry {
        key,
        source,
        tree,
        fractions,
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        exploitability_over_P: EXPLOITABILITY_OVER_P,
        target_bp: s.target_bp,
        iterations: 1000,
        elapsed_ms: 100,
        memory_bytes: 1 << 20,
        mode: "f32".into(),
        locks_applied: 0,
        export: "street".into(),
        reasons: vec![],
        created: 1,
        last_hit: 1,
    }
}

/// A per-rig temporary directory, unique per process and call, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("pokerai-cache-rig-{}-{n}", std::process::id()));
        // A dead process that had this id may have left this exact directory behind: never adopt its files.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The spec 13.1 T4 harness: one stored entry in its own real cache, and the queries asked of it.
pub struct CacheRig {
    /// The decision the entry was solved at; `query` rescales it.
    pub scenario: Scenario,
    /// The one entry this rig stored, exactly as it went to the writer.
    pub entry: CacheEntry,
    // Field order is drop order: the cache (and its reader) goes before its directory.
    cache: Cache,
    dir: TempDir,
}

impl CacheRig {
    /// Brief signature: `template` at pot `p`, effective stacks `eff`, rake cap `cap_mchips`, stored as described in the
    /// module doc.
    pub fn new(template: &str, p: u32, eff: u32, cap_mchips: u32) -> CacheRig {
        CacheRig::with_entry(Scenario::new(template, p, eff, cap_mchips), |_| {})
    }

    /// `new` for an entry solved after the street history `history` (from OOP): an observed off-menu size is inserted
    /// into the entry's tree and its signature exactly as a live solve's would be (section 10.2).
    pub fn new_at(template: &str, p: u32, eff: u32, cap_mchips: u32, history: &[Action]) -> CacheRig {
        CacheRig::with_entry(Scenario::new(template, p, eff, cap_mchips).with_history(history), |_| {})
    }

    /// A rig for `scenario` whose entry is first edited by `edit` (a stored reason, a raw accuracy, a storage mode, a
    /// locked model): the edited entry must still validate, and the writer must confirm it is on disk.
    pub fn with_entry(scenario: Scenario, edit: impl FnOnce(&mut CacheEntry)) -> CacheRig {
        install_templates();
        let mut entry = entry_for(&scenario);
        edit(&mut entry);
        cache::entry::validate_entry(&entry).expect("the rig's entry validates");
        let dir = TempDir::new();
        let cache = Cache::open(dir.0.clone(), cache::CACHE_QUOTA_BYTES);
        assert!(cache.store_tracked(&entry).wait(Duration::from_secs(60)), "the writer stores the rig's entry");
        CacheRig { scenario, entry, cache, dir }
    }

    /// The rig's decision rescaled to pot `p`, effective stacks `eff` and rake cap `cap_mchips`, same template, board,
    /// seats, ranges, rake rate, big blind and target.
    pub fn at(&self, p: u32, eff: u32, cap_mchips: u32) -> Scenario {
        let Rake::PotRake { rate, no_flop_no_drop, .. } = self.scenario.rake else { panic!("rigs are raked") };
        Scenario { pot: p, stack_oop: eff, stack_ip: eff, rake: Rake::PotRake { rate, cap_mchips, no_flop_no_drop }, ..self.scenario.clone() }
    }

    /// Brief signature: the production query for `actor` after the street history `path`, at the rescaled decision.
    pub fn query(&self, p: u32, eff: u32, cap_mchips: u32, path: &[Action], actor: &str) -> CacheQuery {
        self.at(p, eff, cap_mchips).with_history(path).query(actor)
    }

    /// The production lookup's whole outcome.
    pub fn lookup(&self, q: &CacheQuery) -> Lookup {
        self.cache.lookup(q)
    }

    /// Brief signature: any non-miss outcome's hit (Exact, Approximate or Provisional); a miss panics with its reason.
    pub fn hit(&self, q: &CacheQuery) -> CacheHit {
        match self.lookup(q) {
            Lookup::Exact { hit } | Lookup::Approximate { hit, .. } | Lookup::Provisional { hit, .. } => hit,
            Lookup::Miss { reason } => panic!("expected a hit for {:?} at {:?}, got Miss({reason:?})", q.actor, q.requested),
        }
    }

    /// Brief signature: whether the lookup is a miss, for any reason.
    pub fn miss(&self, q: &CacheQuery) -> bool {
        matches!(self.lookup(q), Lookup::Miss { .. })
    }

    /// The directory the rig's cache lives in.
    pub fn root(&self) -> &Path {
        &self.dir.0
    }
}

impl Drop for CacheRig {
    fn drop(&mut self) {
        self.cache.shutdown();
    }
}

/// Plan 4 Task 9: an `Engine` started by `Engine::with_core` over plan 2's rigs, a `FakeWorker` that never replies
/// (`FakeReply::Hang`) and a `FakeClock` at 0 ms, with the core's default session config and no hand in progress. Its
/// decision log goes to a per-process, per-call temporary directory nothing else writes.
pub fn engine_with_fake_worker() -> engine::Engine {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let clock = engine::testing::FakeClock::new();
    let identity = std::sync::Arc::new(std::sync::Mutex::new(engine::identity::IdentityState::new()));
    let (worker, _state) = engine::testing::FakeWorker::scripted(clock.clone(), identity.clone(), vec![engine::testing::FakeReply::Hang]);
    let log_dir = std::env::temp_dir().join(format!("pokerai-engine-fake-worker-{}-{n}", std::process::id()));
    let core = engine::core::EngineCore::new(worker, clock, identity, engine::log::DecisionLog::open(&log_dir));
    engine::Engine::with_core(core)
}

/// Plan 4 Task 9: the session `GameConfig` of plan 2's `testing::cfg_1_2()` (blinds 5/10 chips, 5% pot rake capped at 5
/// chips, no straddle, solver preferences 16 threads / 50 bp / `flop_budget_s` 10).
pub fn game_config() -> proto::GameConfig {
    engine::testing::cfg_1_2().0
}

// ===================== plan 4 Task 10: the flop-path harness =====================
//
// `FlopRig` is plan 2's fake worker and fake clock under the production `serve_request_with`, over a real cache in a
// temporary directory the rig owns. The cache is seeded through the production entry writer (`entry_from_solution`,
// `canonical_perm`, `Cache::store_tracked`), so every lookup a decision makes is the production `Cache::lookup`
// answering from disk; nothing of the engine's result is mocked. The root ranges are explicit and full (the board
// blocks them, hero's cards never do), so no reason is inherited unless a stored entry carries one. The fast-path
// equity routine is a stub that answers at once: the equity is not under test here, and a real one on a full flop range
// would run against a frozen fake clock.
//
// The table: `cfg_1_2` (blinds 5/10, 5% rake capped at 5000 mchips), six seats of 1000 chips, the button on seat 0. The
// button opens to 45, the small blind (seat 1) calls and the big blind folds: a 100-chip single-raised pot, stacks 955,
// with the small blind out of position (spec 13.3's `1.9` chips of a 100-chip pot is exactly 190 bp).

use engine::clock::Clock;
use engine::core::EngineCore;
use engine::identity::IdentityState;
use engine::log::{DecisionLog, DecisionRecord};
use engine::ranges::ExplicitRanges;
use engine::serve::{serve_request_with, EquityRoutine, LiveRequest, ServeSeams};
use engine::testing::{board, hand, play, FakeClock, FakeReply, FakeState, FakeWorker, IdRef, RecordingSink};
use proto::worker::{AckStatus, EngineMessage, ResultStatus, SolveRequest, StreetSolution, WorkerError};
use proto::{ApproxReason, DecisionIdentity, HandState, RecommendationEvent};
use std::sync::{Arc, Mutex};

/// The button (seat 0), IP on every postflop street of the rig's pot.
pub const BTN: Seat = Seat(0);
/// The small blind (seat 1), OOP on every postflop street of the rig's pot.
pub const SB: Seat = Seat(1);
/// The flop, turn and river of the rig's hand.
pub const FLOP: &str = "Kh 7d 2c";
pub const TURN: &str = "Kh 7d 2c 4d";
pub const RIVER: &str = "Kh 7d 2c 4d 9s";

/// Hero's cards: never in a public range, a solve input, a snapshot or a cache key.
pub fn hero_cards() -> [Card; 2] {
    [Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]
}

/// The rig's preflop with `hero` in `hero`'s seat: the button opens to 45, the small blind calls, the big blind folds
/// (two preflop wagers: a single-raised pot of 100 chips).
pub fn srp_preflop(hero: Seat) -> HandState {
    let s = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), BTN, hero, Some(hero_cards()));
    play(&s, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 45 }, Action::Call, Action::Fold])
}
/// The flop street root of the single-raised pot, the small blind (OOP) to act.
pub fn srp_flop(hero: Seat) -> HandState {
    board(&srp_preflop(hero), FLOP)
}
/// The turn street root after a checked-through flop, the small blind to act.
pub fn srp_turn(hero: Seat) -> HandState {
    board(&play(&srp_flop(hero), &[Action::Check, Action::Check]), TURN)
}
/// The river street root after a checked-through turn, the small blind to act.
pub fn srp_river(hero: Seat) -> HandState {
    board(&play(&srp_turn(hero), &[Action::Check, Action::Check]), RIVER)
}
/// A 3-bet pot's flop: the button opens to 25, the small blind 3-bets to 90, the big blind folds, the button calls
/// (three preflop wagers), the small blind to act.
pub fn three_bet_flop(hero: Seat) -> HandState {
    let s = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), BTN, hero, Some(hero_cards()));
    board(&play(&s, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 25 }, Action::Raise { to: 90 }, Action::Fold, Action::Call]), FLOP)
}

/// The rig's public root range for both seats: every combo, blocked by the board inside the range source.
pub fn full_range() -> Range1326 {
    Range1326([1.0; 1326])
}

/// The effective tree `template` materializes at `state`'s street root (the production builder), and the root.
pub fn live_tree(state: &HandState, template: &str) -> (proto::EffectiveTree, StreetRootSnapshot, u32) {
    let root = core_model::street_root(state).expect("a heads-up street root");
    let built = build_tree_full(&root, &TemplateSelection::from_history(template, &root.history)).unwrap_or_else(|e| panic!("{template}: {e:?}"));
    (built.tree, root, built.pot)
}

/// A valid solution of `tree` requested at `requested` with `expl` chips, whose rows differ by combo so a suit mapping
/// that moved a row to the wrong combo is observable: at exported node `n`, combo `c` plays its first action with
/// probability `(c % 7 + 1) / 8` and the others evenly, and every non-fold action `a` has EV `n * 100 + a * 10 + c % 5`
/// chips; a fold's EV is exactly 0.
pub fn varied_solution(tree: &proto::EffectiveTree, requested: &[Action], expl: f32) -> StreetSolution {
    let mut sol = engine::testing::uniform_solution(tree, requested, expl);
    for (n, node) in sol.nodes.iter_mut().enumerate() {
        let width = node.actions.len();
        for c in 0..proto::COMBOS {
            let first = (c % 7 + 1) as f32 / 8.0;
            let rest = if width > 1 { (1.0 - first) / (width - 1) as f32 } else { 0.0 };
            node.probs[c] = (0..width).map(|a| if width == 1 { 1.0 } else if a == 0 { first } else { rest }).collect();
            node.ev_chips[c] =
                node.actions.iter().enumerate().map(|(a, action)| if *action == Action::Fold { 0.0 } else { (n * 100 + a * 10) as f32 + (c % 5) as f32 }).collect();
        }
    }
    proto::worker::validate_solution(&sol, &tree.materialized).expect("the varied solution is valid");
    sol
}

/// The live worker's answer to one solve: `"ok"` / `"best_so_far"` with a varied solution at `raw` chips of the live
/// tree of `state` on `template`, or `"no_iteration"` (the worker's `error{no_iteration}` to the first attempt and to
/// its `_min` retry), or `"hang"` (no terminal ever).
pub fn live_script(state: &HandState, template: &str, raw: f32, status: &str) -> Vec<FakeReply> {
    let (tree, root, _) = live_tree(state, template);
    let history: Vec<Action> = root.history.iter().map(|(_, a)| *a).collect();
    let ack = || FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None };
    let answer = |status| FakeReply::Result { id: IdRef::Last, status, solution: Some(varied_solution(&tree, &history, raw)), error: None, elapsed_ms: 5 };
    let no_iteration = || FakeReply::Result { id: IdRef::Last, status: ResultStatus::Error, solution: None,
        error: Some(WorkerError { code: "no_iteration".into(), message: "no_iteration".into(), retryable: false, estimate_bytes: None }), elapsed_ms: 5 };
    match status {
        "ok" => vec![ack(), answer(ResultStatus::Ok)],
        "best_so_far" => vec![ack(), answer(ResultStatus::BestSoFar)],
        "no_iteration" => vec![ack(), no_iteration(), ack(), no_iteration()],
        "hang" => vec![ack(), FakeReply::Hang],
        other => panic!("live_script: unknown status {other:?}"),
    }
}

/// One entry the rig stores before a decision is asked: the solve of `template` at a street root on `board` with
/// reference pot `pot`, both stacks `eff` and rake cap `cap_mchips` (5% rake, the rig's ranges), whose raw stored
/// accuracy is `raw_over_p` and whose inherited reasons are `reasons`; `truncated` exports the root node alone.
#[derive(Clone, Debug)]
pub struct Seed {
    pub template: &'static str,
    pub board: &'static str,
    pub pot: u32,
    pub eff: u32,
    pub cap_mchips: u32,
    pub raw_over_p: f64,
    pub reasons: Vec<ApproxReason>,
    pub truncated: bool,
}

impl Seed {
    /// The rig's own flop decision (pot 100, stacks 955, cap 5000 mchips) on `template`, at 40 bp (inside the 50 bp
    /// target), no reasons, every root-street node exported: served `Exact`.
    pub fn exact(template: &'static str) -> Seed {
        Seed { template, board: FLOP, pot: 100, eff: 955, cap_mchips: 5000, raw_over_p: 0.004, reasons: vec![], truncated: false }
    }
    /// This seed at another reference state.
    pub fn at(self, pot: u32, eff: u32, cap_mchips: u32) -> Seed {
        Seed { pot, eff, cap_mchips, ..self }
    }
    /// This seed with raw stored accuracy `raw_over_p`.
    pub fn raw(self, raw_over_p: f64) -> Seed {
        Seed { raw_over_p, ..self }
    }
    /// This seed carrying `reasons` as its inherited reasons.
    pub fn reasons(self, reasons: Vec<ApproxReason>) -> Seed {
        Seed { reasons, ..self }
    }
    /// This seed exporting its root node only (`export: "truncated"`).
    pub fn truncated(self) -> Seed {
        Seed { truncated: true, ..self }
    }
    /// This seed on the turn street root of the rig's checked-through flop.
    pub fn on_turn(self) -> Seed {
        Seed { board: TURN, template: "turn_std_v1", ..self }
    }
}

/// The brief's `provisional_hit`: the rig's flop decision stored on the pre-solver template at raw `raw_over_p` (above
/// the 50 bp target when over 0.005), carrying `ChartRounded`.
pub fn provisional_hit(raw_over_p: f64) -> Seed {
    Seed::exact(engine::flop::PRESOLVER_TEMPLATE).raw(raw_over_p).reasons(vec![ApproxReason::ChartRounded])
}
/// The rig's flop decision stored at 40 bp with no reason: served `Exact`.
pub fn exact_hit() -> Vec<Seed> {
    vec![Seed::exact(engine::flop::PRESOLVER_TEMPLATE)]
}
/// The rig's flop decision stored at 40 bp carrying `ChartRounded`: served `Approximate{ChartRounded}`.
pub fn approximate_hit() -> Vec<Seed> {
    vec![Seed::exact(engine::flop::PRESOLVER_TEMPLATE).reasons(vec![ApproxReason::ChartRounded])]
}
/// The rig's flop decision stored at 190 bp: served `Provisional`, then refined live.
pub fn provisional_route() -> Vec<Seed> {
    vec![provisional_hit(0.019)]
}

/// The entry `seed` describes, written by the production entry writer (`entry_from_solution` over the shared
/// `canonical_perm`) from a varied solution at `raw_over_p * pot` chips, over the rig's full public ranges.
pub fn seed_entry(seed: &Seed) -> CacheEntry {
    seed_entry_with(seed, [full_range(), full_range()])
}

/// `seed_entry` over the public root ranges `ranges` (OOP then IP; blocked by the board, never by hero's cards).
pub fn seed_entry_with(seed: &Seed, ranges: [Range1326; 2]) -> CacheEntry {
    let cards = cards(seed.board);
    let street = if cards.len() == 3 { Street::Flop } else { Street::Turn };
    let root = StreetRootSnapshot { street, board: cards, oop: SB, ip: BTN, pot_root: seed.pot, stack_oop_root: seed.eff, stack_ip_root: seed.eff, dead_this_street: 0,
        projected_from: 2, history: vec![], bb_chips: 10 };
    let built = build_tree_full(&root, &TemplateSelection::from_history(seed.template, &[])).unwrap_or_else(|e| panic!("seed {seed:?}: {e:?}"));
    let signature = tree_signature(&built.tree, built.pot);
    let perm = engine::cache_bridge::canonical_perm(&root.board, &ranges[0], &ranges[1]);
    let mut sol = varied_solution(&built.tree, &[], (seed.raw_over_p * f64::from(built.pot)) as f32);
    if seed.truncated {
        sol.nodes.truncate(1);
        sol.covered_paths.truncate(1);
        sol.export = "truncated".into();
    }
    let input = SolveInput { root, ranges, tree: built.tree, target_bp: 50 };
    let rake = Rake::PotRake { rate: RATE, cap_mchips: seed.cap_mchips, no_flop_no_drop: false };
    engine::cache_bridge::entry_from_solution(&input, &sol, &seed.reasons, 10, &rake, &signature, &perm, 100, 50).unwrap_or_else(|e| panic!("seed {seed:?}: {e:?}"))
}

/// A fast-path equity routine that answers at once (the equity is not under test here).
pub fn stub_equity() -> EquityRoutine {
    Arc::new(|_clock: &dyn Clock, _hero: Option<[Card; 2]>, _hero_public: &Range1326, opponents: &[(Seat, Range1326)], _board: &[Card], _budget: Duration,
        _cancel: &std::sync::atomic::AtomicBool| {
        let ready = || proto::EquityEstimate { value: Some(0.5), availability: proto::Availability::Ready, method: Some(proto::EquityMethod::Exact) };
        proto::EquitySummary {
            hero_combo_vs_each: opponents.iter().map(|(s, _)| (*s, ready())).collect(),
            hero_range_vs_each: opponents.iter().map(|(s, _)| (*s, ready())).collect(),
            per_pot_shares: vec![],
        }
    })
}

/// A decision the rig served: its identity and its events in emission order, `Equity` and `Progress` left out (they
/// are not under test here, and `Equity` comes from the `fast-path` thread at a time of its own).
pub struct Served {
    pub id: DecisionIdentity,
    pub events: Vec<RecommendationEvent>,
}

impl Served {
    /// The one `Final`; panics naming the events if there is not exactly one.
    pub fn final_rec(&self) -> &proto::Recommendation {
        let finals: Vec<&proto::Recommendation> = self.events.iter().filter_map(|e| match e { RecommendationEvent::Final(r) => Some(r), _ => None }).collect();
        assert_eq!(finals.len(), 1, "exactly one Final: {:?}", self.kinds());
        finals[0]
    }
    /// The `Provisional`s, in order.
    pub fn provisionals(&self) -> Vec<&proto::Recommendation> {
        self.events.iter().filter_map(|e| match e { RecommendationEvent::Provisional(r) => Some(r), _ => None }).collect()
    }
    /// The events' kinds, in order.
    pub fn kinds(&self) -> Vec<&'static str> {
        self.events.iter().map(|e| match e {
            RecommendationEvent::Fast(_) => "Fast",
            RecommendationEvent::Provisional(_) => "Provisional",
            RecommendationEvent::Final(_) => "Final",
            RecommendationEvent::NoDecision { .. } => "NoDecision",
            RecommendationEvent::Equity { .. } => "Equity",
            RecommendationEvent::Progress { .. } => "Progress",
        }).collect()
    }
}

/// The spec 13.3 flop-path rig (see the section doc above).
pub struct FlopRig {
    pub core: EngineCore,
    pub clock: Arc<FakeClock>,
    pub identity: Arc<Mutex<IdentityState>>,
    pub worker: Arc<Mutex<FakeState>>,
    /// The key digest of the sentinel entry `flush` stores, left out of `stored`.
    sentinel: Option<[u8; 32]>,
    // Field order is drop order: the core (with its cache) goes before the directories.
    cache_dir: TempDir,
    log_dir: TempDir,
}

impl FlopRig {
    /// A rig over a fresh cache directory whose fake worker answers `script`, the session config `cfg_1_2`'s, the
    /// conservative flop policy, and a hand begun (hand 1, config revision 1: the hand builders' own).
    pub fn new(script: Vec<FakeReply>) -> FlopRig {
        let cache_dir = TempDir::new();
        let cache = Cache::open(cache_dir.0.clone(), cache::CACHE_QUOTA_BYTES);
        FlopRig::with_cache(script, cache, cache_dir)
    }

    /// A rig whose cache is `cache` (a disabled one, say), with `cache_dir` as the directory `stored` lists.
    ///
    /// Review P4T10-I5: the rig's cache measures every lookup deadline on the rig's fake clock
    /// (`cache::Cache::with_logical_clock`, the cache's `testing` seam), the clock the engine measures the cache phase
    /// on: the engine's budget and the cache's bound are then one logical limit, and the machine's load never turns an
    /// expected hit into a `BudgetExhausted` miss. The entries are real cells on disk and the lookup is otherwise the
    /// production one (reader, selection, reconstruction, label, touch).
    pub fn with_cache(script: Vec<FakeReply>, cache: Cache, cache_dir: TempDir) -> FlopRig {
        let clock = FakeClock::new();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let (worker_link, worker) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
        let log_dir = TempDir::new();
        let logical = clock.clone();
        let cache = cache.with_logical_clock(Arc::new(move || logical.now_ms()));
        let core = EngineCore::new(worker_link, clock.clone(), identity.clone(), DecisionLog::open(&log_dir.0)).with_cache(cache);
        core.set_config(game_config());
        *core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: Some(full_range()), ip: Some(full_range()) });
        {
            let mut ids = identity.lock().unwrap();
            ids.set_config();
            ids.begin_hand();
        }
        FlopRig { core, clock, identity, worker, sentinel: None, cache_dir, log_dir }
    }

    /// Stores `seeds` through the rig's own cache, each confirmed on disk by the writer before the next.
    pub fn seed(&self, seeds: &[Seed]) {
        for seed in seeds {
            assert!(self.core.cache.store_tracked(&seed_entry(seed)).wait(Duration::from_secs(60)), "the writer stores seed {seed:?}");
        }
    }

    /// Serves `state` as the next decision of the hand, with the stub equity and `seams`.
    pub fn serve_with(&mut self, state: &HandState, seams: ServeSeams) -> Served {
        let now = self.clock.now_ms();
        self.serve_at(state, now, seams)
    }

    /// Serves `state` with the stub equity and no other seam.
    pub fn serve(&mut self, state: &HandState) -> Served {
        self.serve_with(state, ServeSeams::default())
    }

    /// Admits `state` as the next decision at the clock's current time (its `t0`), then moves the fake clock to
    /// `now_ms` before `engine-main` serves it (a request that waited behind others), with `seams`.
    pub fn serve_at(&mut self, state: &HandState, now_ms: u64, seams: ServeSeams) -> Served {
        let id = self.identity.lock().unwrap().next_decision().expect("a hand is in progress");
        let (sink, events) = RecordingSink::new(self.clock.clone(), Some(self.worker.clone()));
        let sink: engine::watchdog::SharedSink = Arc::new(Mutex::new(Box::new(sink)));
        let req = LiveRequest::admitted(&self.core, id.clone(), state.clone(), self.clock.now_ms(), sink);
        self.clock.set_ms(now_ms);
        serve_request_with(&mut self.core, req, ServeSeams { equity: Some(seams.equity.clone().unwrap_or_else(stub_equity)), ..seams });
        let events = events
            .lock()
            .unwrap()
            .iter()
            .filter(|r| !matches!(r.event, RecommendationEvent::Equity { .. } | RecommendationEvent::Progress { .. }))
            .map(|r| r.event.clone())
            .collect();
        Served { id, events }
    }

    /// The snapshots of hand 1 registered for decision `decision_id`, in registration order.
    pub fn snapshots_of(&self, decision_id: u64) -> Vec<engine::snapshots::StreetSnapshot> {
        self.core.snapshots.lock().unwrap().for_hand(1).into_iter().filter(|s| s.provenance.identity_at_solve.decision_id == decision_id).cloned().collect()
    }

    /// Every `solve` the fake worker was sent, in order.
    pub fn solves(&self) -> Vec<SolveRequest> {
        self.worker.lock().unwrap().sent.iter().filter_map(|m| if let EngineMessage::Solve(q) = m { Some(q.clone()) } else { None }).collect()
    }

    /// Every decision record logged, in order.
    pub fn records(&self) -> Vec<DecisionRecord> {
        std::fs::read_to_string(self.log_dir.0.join("decisions.jsonl")).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default()
    }

    /// Every diagnostics record logged, in order, as JSON.
    pub fn diagnostics(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.log_dir.0.join("diagnostics.jsonl")).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default()
    }

    /// `(street, origin, decision id)` of every snapshot of hand 1, in registration order.
    pub fn origins(&self) -> Vec<(Street, String, u64)> {
        self.core.snapshots.lock().unwrap().for_hand(1).iter().map(|s| (s.key.street, s.provenance.origin.clone(), s.provenance.identity_at_solve.decision_id)).collect()
    }

    /// Every entry on disk once the writer has handled every command sent before this call, the sentinel of the
    /// barrier left out. The barrier is a tracked store of a sentinel entry on another board (the writer serves its
    /// queue in order, so its receipt comes after every store queued before it).
    pub fn stored(&mut self) -> Vec<CacheEntry> {
        let sentinel = seed_entry(&Seed { board: "As Qs 5h", ..Seed::exact("flop_fast_v1") });
        self.sentinel = Some(sentinel.key.digest());
        if self.core.cache.root().as_os_str().is_empty() {
            return list_entries(&self.cache_dir.0);
        }
        assert!(self.core.cache.store_tracked(&sentinel).wait(Duration::from_secs(60)), "the writer stores the sentinel");
        list_entries(&self.cache_dir.0).into_iter().filter(|e| Some(e.key.digest()) != self.sentinel).collect()
    }

    /// The rig's cache directory.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir.0
    }
}

/// Every entry of every cell under `dir` (`<key[0..2]>/<key>.bin`), in path order.
pub fn list_entries(dir: &Path) -> Vec<CacheEntry> {
    let mut cells = Vec::new();
    for shard in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        for file in std::fs::read_dir(shard.path()).into_iter().flatten().flatten() {
            if file.path().extension().is_some_and(|e| e == "bin") {
                cells.push(file.path());
            }
        }
    }
    cells.sort();
    cells.iter().flat_map(|p| cache::storage::read_cell(p).map(|c| c.entries).unwrap_or_default()).collect()
}

/// Spec 13.3's flop script: the rig's single-raised-pot flop decision (hero the small blind, OOP, to act at the street
/// root) asked once, over a cache holding `seeds` and a fake worker answering the live solve of its live template
/// (`flop_fast_v1`: V3 has admitted nothing) as `live_script(.., raw, status)` does. Returns its events (`Equity` and
/// `Progress` left out). The brief's `cache: Vec<Lookup>` becomes the entries on disk the production lookup answers
/// from, so a scripted route is the lookup's real outcome, never a mocked one.
pub fn run_flop_script(seeds: Vec<Seed>, raw: f32, status: &str) -> Vec<RecommendationEvent> {
    let state = srp_flop(SB);
    let mut rig = FlopRig::new(live_script(&state, "flop_fast_v1", raw, status));
    rig.seed(&seeds);
    let served = rig.serve(&state);
    rig.core.shutdown();
    served.events
}

/// How many `Final`s `events` hold.
pub fn final_count(events: &[RecommendationEvent]) -> usize {
    events.iter().filter(|e| matches!(e, RecommendationEvent::Final(_))).count()
}

// ---- Task 12 helpers ----
//
// Plan 4 Task 12 (cache snapshots replayed across streets): a single-raised pot with chosen stacks, a solution whose
// action columns differ by combo, node and action, and a stored entry exporting one of spec 9.2's three coverages,
// written by the production entry writer. Namespaced so they never meet another task's helpers in this shared file.

pub mod snapshot_replay {
    use super::*;

    /// Which decision nodes of its street a stored entry exports (spec 9.2's three export coverages).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Export {
        /// The requested node alone (`export: "truncated"`).
        RequestedOnly,
        /// The street root alone (`export: "truncated"`).
        RootOnly,
        /// Every decision node of the street (`export: "street"`).
        Complete,
    }

    /// The rig's single-raised pot with `stack` chips for the button and the small blind (1000 for the others) and
    /// hero in `hero`'s seat holding `cards`: the button opens to `open`, the small blind calls, the big blind folds.
    /// The flop pot is `2 * open + 10`, with `stack - open` behind each.
    pub fn srp_with(hero: Seat, cards: [Card; 2], stack: u32, open: u32) -> HandState {
        let stacks = (0..6).map(|i| (Seat(i), if i <= 1 { stack } else { 1000 })).collect::<Vec<_>>();
        let s = hand(&stacks, BTN, hero, Some(cards));
        play(&s, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: open }, Action::Call, Action::Fold])
    }

    /// A valid solution of `tree`'s root street requested at the chip path `requested`, at `exploitability_chips`:
    /// at the node of ordinal path `o`, combo `c` plays action `a` with weight `1 + (c (2a + 1) + 7a + 11 s) mod 13`
    /// (`s` a salt of `o`), normalized over the node's actions, so the columns differ by combo, by action and by node
    /// (a translated wager's two mapped sizes condition differently); every non-fold EV is `10a + c mod 5` chips and a
    /// fold's exactly 0. Every combo is available.
    pub fn leaning_solution(tree: &proto::EffectiveTree, requested: &[Action], exploitability_chips: f32) -> StreetSolution {
        let mut sol = engine::testing::uniform_solution(tree, requested, exploitability_chips);
        for node in &mut sol.nodes {
            let ordinal = proto::resolve_chip_path(&tree.materialized, &node.path).expect("an exported node resolves");
            let salt: usize = ordinal.iter().enumerate().map(|(k, i)| (k + 1) * (usize::from(*i) + 1)).sum();
            let width = node.actions.len();
            for c in 0..proto::COMBOS {
                let w: Vec<f32> = (0..width).map(|a| 1.0 + ((c * (2 * a + 1) + 7 * a + 11 * salt) % 13) as f32).collect();
                let total: f32 = w.iter().sum();
                node.probs[c] = w.iter().map(|x| x / total).collect();
                node.ev_chips[c] = node.actions.iter().enumerate().map(|(a, action)| if *action == Action::Fold { 0.0 } else { (10 * a + c % 5) as f32 }).collect();
            }
        }
        proto::worker::validate_solution(&sol, &tree.materialized).expect("the leaning solution is valid");
        sol
    }

    /// The entry the production writer (`cache_bridge::entry_from_solution`, over the shared `canonical_perm`) stores
    /// for a solve of `template` at `root` (its history cleared: an entry is keyed at the street root) over the public
    /// root ranges `ranges` (OOP then IP; hero's cards in neither) and `rake`: `leaning_solution` at raw accuracy
    /// `raw_over_p`, requested at the chip path `requested`, exporting `export`'s nodes, with no inherited reason.
    pub fn export_entry(root: &StreetRootSnapshot, template: &str, ranges: [Range1326; 2], rake: Rake, export: Export, requested: &[Action], raw_over_p: f64) -> CacheEntry {
        let root = StreetRootSnapshot { history: vec![], ..root.clone() };
        let built = build_tree_full(&root, &TemplateSelection::from_history(template, &[])).unwrap_or_else(|e| panic!("{template} at {root:?}: {e:?}"));
        let signature = tree_signature(&built.tree, built.pot);
        let perm = engine::cache_bridge::canonical_perm(&root.board, &ranges[0], &ranges[1]);
        let wanted = proto::resolve_chip_path(&built.tree.materialized, requested).expect("the requested node is in the tree");
        let mut sol = leaning_solution(&built.tree, requested, (raw_over_p * f64::from(built.pot)) as f32);
        let exported = |path: &[Action]| {
            let ordinal = proto::resolve_chip_path(&built.tree.materialized, path).expect("an exported node resolves");
            match export {
                Export::RequestedOnly => ordinal == wanted,
                Export::RootOnly => ordinal.is_empty(),
                Export::Complete => true,
            }
        };
        sol.nodes.retain(|n| exported(&n.path));
        sol.covered_paths = sol.nodes.iter().map(|n| n.path.clone()).collect();
        let requested_at = sol.nodes.iter().position(|n| n.path == requested).expect("the requested node is exported");
        sol.requested = u32::try_from(requested_at).expect("a node index fits in u32");
        if export != Export::Complete {
            sol.export = "truncated".into();
        }
        let input = SolveInput { root, ranges, tree: built.tree, target_bp: 50 };
        engine::cache_bridge::entry_from_solution(&input, &sol, &[], input.root.bb_chips, &rake, &signature, &perm, 100, 50)
            .unwrap_or_else(|e| panic!("{template} {export:?}: the writer refuses the entry: {e:?}"))
    }
}
