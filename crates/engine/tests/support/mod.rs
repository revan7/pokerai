//! Engine integration-test support: `CacheRig` (plan 4 Task 8), the spec section 13.1 T4 harness over the real cache.
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
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
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
