//! Plan 4 Task 12 (spec 8.4, 9.1, 9.2, 9.3, 10.4; spec 13.1 T4): a cache hit's snapshot is registered through the one
//! plan-3 registration path, and later streets replay through it, prior-street bets translated.
//!
//! As built (plan-3 final review carry (d); plan 4 Task 10): there is no `cache_bridge::snapshot_from_hit` and no
//! `snapshots::CACHE_ORIGINS`. A cache hit's snapshot is `replay_bridge::snapshot_from_solution` over the hit's
//! query-sized solution, its query tree, its ordinal covered paths and the query key's tree signature, with an origin of
//! `replay_bridge::ORIGINS`; `serve` registers it only inside the accepted delivery of its `Final` (or of its
//! `Provisional`), under the identity lock, through `replay_bridge::register_accepted`. Every registration this file
//! relies on is the engine's own: `support::FlopRig` is the production `serve_request_with` over plan 2's fake worker,
//! the fake clock and a real cache seeded through the production entry writer. The snapshot store is called directly
//! only for the brief's register-rule unit assertions and for the prefix invalidation `Engine::undo` applies to it.
//!
//! The hand. The button (seat 0) and the small blind (seat 1) start with 145 chips; the button opens to 45, the small
//! blind calls and the big blind folds: a 100-chip flop with 100 behind, where `flop_fast_v1` offers `[Check, Bet 50,
//! AllIn 100]` at the root and after a check (the brief's snapshot menus 50/100). A 73-chip bet into 100 is `s = 0.73`,
//! translated with `f_A = 81/173` onto the 50 and `f_B = 92/173` onto the all-in. After a checked-through flop the turn
//! is 100/100 again, where `turn_std_v1` offers `[Check, Bet 33, AllIn 100]`. The replay's preflop store is empty, so
//! the preflop is unconditioned and both seats reach the flop uniform: the expected ranges below are closed forms of the
//! snapshot's own columns (spec 8.4's kernel: an on-menu action multiplies the actor's masses by its column once, a
//! translated wager splits the branch with weights `f_X` times the integrated likelihood of each mapped size).
//!
//! The golden `golden/cache_snapshot_replay.json` freezes, per case, the cache decision and its registered snapshot, the
//! next street's `Final` and solve input (both 1326-combo root ranges) and the direct replay's `log_reach`, branch
//! weights and reasons. It is recorded once with `POKERAI_RECORD_GOLDENS=1`, inspected, and then only compared.

// `pub`, so the shared helpers this binary never calls are reachable rather than dead code, with no lint filter.
pub mod support;

use cache::entry::CacheEntry;
use cache::lookup::CacheHit;
use core_ranges::hash_scaled;
use core_replay::{board_mask, snapshot_node_at, HistoryBranch, ReplayInput, ReplayOutput, SnapshotProvenance, SnapshotStore, StreetSnapshot};
use engine::ranges::ExplicitRanges;
use engine::replay_bridge::{snapshot_from_solution, snapshot_note, NO_REQUEST, ORIGINS};
use engine::testing::{board, play};
use engine::tree::{build_tree_full, tree_signature, TemplateSelection};
use proto::worker::SolveRequest;
use proto::{
    Action, ApproxReason, Card, Coverage, DecisionIdentity, HandState, Rake, Range1326, Recommendation, RecommendationEvent, Seat, SolveInput, Street,
    StreetRootSnapshot, COMBOS,
};
use serde_json::{json, Value};
use std::time::Duration;
use support::snapshot_replay::{export_entry, srp_with, Export};
use support::{CacheRig, FlopRig, Served, BTN, SB};

const CHECK: Action = Action::Check;
const CALL: Action = Action::Call;

fn bet(to: u32) -> Action {
    Action::Bet { to }
}

fn all_in(to: u32) -> Action {
    Action::AllIn { to }
}

fn hole(text: &str) -> [Card; 2] {
    core_model::parse_hand(text).unwrap()
}

// ===================================== the scenario runner =====================================

/// One scenario: hero's decision on `street` is answered from a stored entry exporting `export`; the street is played
/// out (`after`), the next board comes, and hero's next decision (after `next_before`) is solved live.
#[derive(Clone, Debug)]
struct Case {
    /// The street whose decision the cache answers (flop or turn).
    street: Street,
    hero: Seat,
    cards: [Card; 2],
    /// The button's and the small blind's starting stacks, and the button's open: the flop pot is `2 * open + 10`.
    stack: u32,
    open: u32,
    export: Export,
    /// The stored raw accuracy: 0.004 is inside the 50 bp target (served as the `Final`), 0.019 above it (served as the
    /// `Provisional`; the live refinement then fails and the retained payload is the `Final`).
    raw_over_p: f64,
    /// The cache decision is served over the explicit full ranges (no inherited reason: a `cache_exact` hit), the replay
    /// range source installed only afterwards; otherwise the replay's ranges from the start.
    explicit: bool,
    /// The street's actions before hero's decision, and the rest of the street after it.
    before: Vec<Action>,
    after: Vec<Action>,
    /// The next street's actions before hero's decision there.
    next_before: Vec<Action>,
    /// The reference state `(pot, eff, cap_mchips)` the entry was solved at, when it is not the decision's own.
    entry_at: Option<(u32, u32, u32)>,
}

/// The off-menu line on `street` of the 100/100 pot: the small blind checks, the button bets 73 into 100, the small
/// blind calls. Hero is the button for a requested-node-only export (its decision after the check is the requested
/// node, the only one exported) and the small blind otherwise (its decision is the street root).
fn off_menu(street: Street, export: Export) -> Case {
    let hero = if export == Export::RequestedOnly { BTN } else { SB };
    let (before, after) = if hero == BTN { (vec![CHECK], vec![bet(73), CALL]) } else { (vec![], vec![CHECK, bet(73), CALL]) };
    let next_before = if hero == BTN { vec![CHECK] } else { vec![] };
    Case { street, hero, cards: support::hero_cards(), stack: 145, open: 45, export, raw_over_p: 0.004, explicit: false, before, after, next_before, entry_at: None }
}

/// Brief Step 1's on-menu line on the flop of pot `2 * open + 10` with `stack - open` behind: hero (the small blind)
/// bets half the pot and the button calls, the entry complete and, with `entry_at`, stored at another reference state.
fn on_menu(stack: u32, open: u32, entry_at: Option<(u32, u32, u32)>) -> Case {
    let pot = 2 * open + 10;
    Case { street: Street::Flop, hero: SB, cards: support::hero_cards(), stack, open, export: Export::Complete, raw_over_p: 0.004, explicit: false, before: vec![],
        after: vec![bet(pot / 2), CALL], next_before: vec![], entry_at }
}

/// The cached street's template and the next street's live template.
fn templates(street: Street) -> (&'static str, &'static str) {
    match street {
        Street::Flop => ("flop_fast_v1", "turn_std_v1"),
        Street::Turn => ("turn_std_v1", "river_std_v1"),
        other => panic!("no cache case on the {other:?}"),
    }
}

fn next_street(street: Street) -> Street {
    match street {
        Street::Flop => Street::Turn,
        Street::Turn => Street::River,
        other => panic!("no street after the {other:?} here"),
    }
}

/// Hero's decision on the case's street, and hero's decision on the next street, on the boards `flop`, `turn`, `river`
/// (a turn case checks the flop through first, with no request).
fn states_on(case: &Case, flop: &str, turn: &str, river: &str) -> (HandState, HandState) {
    let preflop = srp_with(case.hero, case.cards, case.stack, case.open);
    let (lead, here, next) = match case.street {
        Street::Flop => (preflop, flop, turn),
        Street::Turn => (play(&board(&preflop, flop), &[CHECK, CHECK]), turn, river),
        other => panic!("no cache case on the {other:?}"),
    };
    let cached = play(&board(&lead, here), &case.before);
    let next = play(&board(&play(&cached, &case.after), next), &case.next_before);
    (cached, next)
}

fn states(case: &Case) -> (HandState, HandState) {
    states_on(case, support::FLOP, support::TURN, support::RIVER)
}

/// Spec 9.3's cause the engine names for every completed postflop street of `state` without a snapshot among
/// `snapshots`: no request of hero's left anything on it in these scenarios, so `no request`.
fn missing(state: &HandState, snapshots: &[StreetSnapshot]) -> Vec<(Street, String)> {
    let completed: &[Street] = match state.board.len() {
        4 => &[Street::Flop],
        5 => &[Street::Flop, Street::Turn],
        _ => &[],
    };
    completed.iter().filter(|s| !snapshots.iter().any(|x| x.key.street == **s)).map(|s| (*s, NO_REQUEST.to_string())).collect()
}

/// `core_replay::replay` of `state` over the rig's preflop store and `snapshots`, with the engine's causes: the replay
/// `replay_bridge::ReplayRanges` runs for a decision at `state`.
fn direct(rig: &FlopRig, state: &HandState, snapshots: &[StreetSnapshot]) -> ReplayOutput {
    let missing = missing(state, snapshots);
    core_replay::replay(ReplayInput { cfg: &state.config, state, store: &rig.core.preflop, snapshots, missing: &missing })
}

fn seat_range(out: &ReplayOutput, seat: Seat) -> Range1326 {
    out.ranges[usize::from(seat.0)].clone().expect("a dealt seat's published range")
}

/// Stores `case`'s entry for hero's decision `cached` through the rig's own cache: keyed by the replay's public root
/// ranges there (OOP then IP), at the decision's own reference state or at `case.entry_at`, exporting `case.export`,
/// requested at hero's node. Returns the entry and those ranges.
fn seed(rig: &FlopRig, case: &Case, cached: &HandState) -> (CacheEntry, [Range1326; 2]) {
    let root = core_model::street_root(cached).expect("hero's decision is a heads-up street root");
    let at_root = direct(rig, cached, &[]);
    let ranges = [seat_range(&at_root, root.oop), seat_range(&at_root, root.ip)];
    let own = (root.pot_root + root.dead_this_street, root.stack_oop_root.min(root.stack_ip_root), 5000);
    let (pot, eff, cap) = case.entry_at.unwrap_or(own);
    let entry_root = StreetRootSnapshot { pot_root: pot, dead_this_street: 0, stack_oop_root: eff, stack_ip_root: eff, ..root.clone() };
    let rake = Rake::PotRake { rate: support::RATE, cap_mchips: cap, no_flop_no_drop: false };
    let requested: Vec<Action> = root.history.iter().map(|(_, a)| *a).collect();
    let entry = export_entry(&entry_root, templates(case.street).0, ranges.clone(), rake, case.export, &requested, case.raw_over_p);
    assert!(rig.core.cache.store_tracked(&entry).wait(Duration::from_secs(60)), "the writer stores the case's entry");
    (entry, ranges)
}

/// What one case left behind.
struct Outcome {
    cached: HandState,
    next: HandState,
    /// The cache decision and the next street's decision, as the rig served them.
    first: Served,
    second: Served,
    entry: CacheEntry,
    /// The replay's public root ranges at the cache decision (OOP then IP): the ranges the entry was keyed by.
    root_ranges: [Range1326; 2],
    /// The one snapshot the cache decision registered.
    snapshot: StreetSnapshot,
    /// Every solve the worker was sent, the next street's last.
    solves: Vec<SolveRequest>,
    /// The direct replay of the next decision over the snapshots its request read (every one registered before it).
    replay: ReplayOutput,
}

impl Outcome {
    fn solve(&self) -> &SolveRequest {
        self.solves.last().expect("the next street's solve")
    }
}

/// Runs `case` on a fresh rig and returns it with the outcome; the caller shuts the rig down.
fn run(case: &Case) -> (FlopRig, Outcome) {
    let (cached, next) = states(case);
    let (template, next_template) = templates(case.street);
    // An above-target hit is refined live: both attempts (the template and its `_min` retry) fail with `no_iteration`,
    // so the retained cache payload is the `Final`.
    let refinement = if case.raw_over_p > 0.005 { support::live_script(&cached, template, 0.0, "no_iteration") } else { vec![] };
    let mut rig = FlopRig::new([refinement, support::live_script(&next, next_template, 0.4, "ok")].concat());
    if !case.explicit {
        rig.core.install_replay_ranges();
    }
    let (entry, root_ranges) = seed(&rig, case, &cached);
    let first = rig.serve(&cached);
    let snapshots = rig.snapshots_of(first.id.decision_id);
    assert_eq!(snapshots.len(), 1, "the cache decision registered one snapshot: {:?}", rig.origins());
    if case.explicit {
        rig.core.install_replay_ranges();
    }
    // The rest of the street and the next board are entered: mutations, each a fresh hand revision.
    rig.identity.lock().unwrap().mutate();
    let second = rig.serve(&next);
    let seen: Vec<StreetSnapshot> =
        rig.core.snapshots.lock().unwrap().for_identity(&second.id).into_iter().filter(|s| s.provenance.identity_at_solve != second.id).collect();
    let replay = direct(&rig, &next, &seen);
    let solves = rig.solves();
    let snapshot = snapshots[0].clone();
    (rig, Outcome { cached, next, first, second, entry, root_ranges, snapshot, solves, replay })
}

// ===================================== reading results =====================================

/// Every reason a result's coverage lists.
fn reasons(rec: &Recommendation) -> Vec<ApproxReason> {
    match &rec.coverage {
        Coverage::Exact => vec![],
        Coverage::Approximate { reasons } => reasons.clone(),
        Coverage::Unsupported { partial, .. } => partial.clone(),
    }
}

/// `(seat, cause)` of every `UnconditionedPriorStreet` of `street` among `rs`, in order.
fn unconditioned(rs: &[ApproxReason], street: Street) -> Vec<(Seat, String)> {
    rs.iter()
        .filter_map(|r| match r {
            ApproxReason::UnconditionedPriorStreet { street: s, seat, cause } if *s == street => Some((*seat, cause.clone())),
            _ => None,
        })
        .collect()
}

fn sorted(mut v: Vec<(Seat, String)>) -> Vec<(Seat, String)> {
    v.sort();
    v
}

/// Every `BetTranslation` of `street` by `by` among `rs`.
fn translations(rs: &[ApproxReason], street: Street, by: Seat) -> Vec<ApproxReason> {
    rs.iter().filter(|r| matches!(r, ApproxReason::BetTranslation { street: s, seat, .. } if *s == street && *seat == by)).cloned().collect()
}

fn fast_of(served: &Served) -> &Recommendation {
    served.events.iter().find_map(|e| match e { RecommendationEvent::Fast(r) => Some(r), _ => None }).expect("the Fast")
}

/// The solved probability of `action` for every combo at the exported node of ordinal path `path` of `snapshot`.
fn column(snapshot: &StreetSnapshot, path: &[u8], action: Action) -> Vec<f64> {
    let node = snapshot_node_at(snapshot, path).unwrap_or_else(|| panic!("node {path:?} is exported"));
    let a = node.actions.iter().position(|x| *x == action).unwrap_or_else(|| panic!("{action:?} is on node {path:?}'s menu {:?}", node.actions));
    node.probs.iter().map(|row| f64::from(row[a])).collect()
}

/// `f(c)` on every combo `board` leaves, 0 on the others.
fn on_board(board: &[Card], f: impl Fn(usize) -> f64) -> Vec<f64> {
    let mask = board_mask(board);
    (0..COMBOS).map(|c| if mask.0[c] == 0.0 { 0.0 } else { f(c) }).collect()
}

/// A published range against `expected` (per-combo weights before normalization, 0 where the board blocks): equal once
/// `expected` is normalized to its maximum, within `f32` rounding.
fn assert_proportional(range: &Range1326, expected: &[f64], what: &str) {
    let max = expected.iter().copied().fold(0.0_f64, f64::max);
    assert!(max > 0.0, "{what}: an expected range with support");
    for c in 0..COMBOS {
        let (got, want) = (f64::from(range.0[c]), expected[c] / max);
        assert!((got - want).abs() <= 1e-6, "{what}: combo {c} has weight {got}, expected {want}");
    }
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// The weight `q` of the one live branch whose translated history took `wager` by `seat`.
fn branch_q(out: &ReplayOutput, seat: Seat, wager: Action) -> f64 {
    let hits: Vec<&HistoryBranch> = out.branches.iter().filter(|b| !b.residual && b.translated.contains(&(seat, wager))).collect();
    assert_eq!(hits.len(), 1, "one live branch took {wager:?}: {:?}", out.branches.iter().map(|b| (b.id, b.q, &b.translated)).collect::<Vec<_>>());
    hits[0].q
}

// ===================================== Step 1: the translation weights =====================================

/// Brief Step 1, verbatim: spec 8.4's pseudo-harmonic weights at `A = 0.5`, `B = 1.0`, `s = 0.73`, and a two-combo
/// marginal through them. The engine's own interpolation (`core_preflop::interpolate`, which the postflop walk calls at
/// the mapped financial parent) gives the same split; its prominence is the integer rule of plan-3 F-M2, asserted where
/// the engine discloses it (the `BetTranslation`'s `prominent` below).
#[test]
fn prior_street_translation_weights_are_fixed() {
    let (a, b, s) = (0.5_f64, 1.0_f64, 0.73_f64);
    let fa = (b - s) * (1.0 + a) / ((b - a) * (1.0 + s));
    let fb = 1.0 - fa;
    assert!((fa - 0.4682080924855491).abs() < 1e-12);
    let marginal = [fa * 0.9 + fb * 0.1, fa * 0.3 + fb * 0.5];
    assert!((marginal[0] - 0.4745664739884393).abs() < 1e-12);
    assert!((marginal[1] - 0.4063583815028902).abs() < 1e-12);
    assert!((s - a).min((b - s).abs()) > 0.10);
    assert!((fa - 81.0 / 173.0).abs() < 1e-15 && (fb - 92.0 / 173.0).abs() < 1e-15);
    let t = core_preflop::interpolate(73.0 / 100.0, &[(1, 50.0 / 100.0), (2, 100.0 / 100.0)]).expect("0.73 lies between the two sizes");
    assert_eq!(t.choices.iter().map(|c| c.0).collect::<Vec<_>>(), [1, 2]);
    assert!((t.choices[0].1 - 81.0 / 173.0).abs() < 1e-12 && (t.choices[1].1 - 92.0 / 173.0).abs() < 1e-12, "{t:?}");
}

// ===================================== the snapshot of a hit =====================================

/// The solve input a cache hit of `rig`'s decision at `p`/`eff`/`cap` answers (as `serve` builds it): the street root,
/// the public root ranges as the range source publishes them (blocked by the board; OOP then IP) and the query tree the
/// hit was rebuilt on.
fn rig_input(rig: &CacheRig, p: u32, eff: u32, cap: u32, hit: &CacheHit) -> SolveInput {
    let scenario = rig.at(p, eff, cap);
    let mut ranges = scenario.ranges.clone();
    for r in &mut ranges {
        core_ranges::block_public(r, &scenario.board);
    }
    SolveInput { root: scenario.snapshot(), ranges, tree: hit.tree.clone(), target_bp: scenario.target_bp }
}

/// Brief Step 1's second test, on the as-built seam: `snapshot_from_solution` over a real cache hit (spec 13.1 T4's
/// `CacheRig`: one entry stored at 100/500 by the real writer, answered by the production lookup) keeps the query's
/// identity and paths. Its signature is the query key's own (never parsed from a display note), its root hashes the
/// public root ranges' (OOP then IP). The snapshot is query-sized: asked at 200/1000 the same entry serves the query
/// tree in the query's chips (the 50-chip bet is a 100-chip one, EV and exploitability double) with the stored rows;
/// and on a board that is not its own canonical form the rows are back in the query's original suits.
#[test]
fn cache_hit_snapshot_keeps_query_identity_and_paths() {
    let rig = CacheRig::new("flop_fast_v1", 100, 500, 5000);
    let query = rig.query(100, 500, 5000, &[], "oop");
    let hit = rig.hit(&query);
    let id = DecisionIdentity { hand_id: 1, hand_revision: 7, decision_id: 9, config_revision: 1, model_revision: 0 };
    let input = rig_input(&rig, 100, 500, 5000, &hit);
    let snapshot = snapshot_from_solution(&id, &input, &hit.solution, hit.covered_paths.clone(), hit.tree_signature.clone(), "cache_exact", vec![]);
    assert_eq!(snapshot.provenance.identity_at_solve, id);
    assert_eq!(snapshot.covered_paths, hit.covered_paths);
    assert_eq!(snapshot.tree, input.tree);
    // Brief Step 4's snapshot assertions.
    let expected_origins = ["cache_exact", "cache_approximate", "cache_provisional"];
    assert!(expected_origins.contains(&snapshot.provenance.origin.as_str()));
    assert_eq!(snapshot.covered_paths.len(), snapshot.nodes.len());
    // The query's own tree, signature and paths; the public root ranges' hashes; the hit's nodes.
    assert_eq!(snapshot.tree, query.tree, "the query tree the hit was rebuilt on");
    assert_eq!(snapshot.key.tree_signature, query.key.tree_signature);
    assert_eq!(snapshot.key.tree_signature, tree_signature(&query.tree, 100), "the engine's signature of the query tree");
    assert_eq!(snapshot.key.root_range_hashes, [hash_scaled(&input.ranges[0]), hash_scaled(&input.ranges[1])]);
    assert_eq!((snapshot.key.hand_id, snapshot.key.config_revision, snapshot.key.model_revision, snapshot.key.street), (1, 1, 0, Street::Flop));
    assert_eq!(snapshot.key.root_board, support::cards(support::FLOP));
    assert_eq!(snapshot.covered_paths, rig.entry.covered_paths, "the entry's ordinal paths");
    assert_eq!(snapshot.nodes, hit.solution.nodes);
    assert!(snapshot.provenance.solved_prefix.is_empty(), "solved at the street root");

    // Query-sized: the same entry asked at 200/1000 (cap doubled with the pot).
    let doubled = rig.query(200, 1000, 10_000, &[], "oop");
    let big = rig.hit(&doubled);
    let big_input = rig_input(&rig, 200, 1000, 10_000, &big);
    let sized = snapshot_from_solution(&id, &big_input, &big.solution, big.covered_paths.clone(), big.tree_signature.clone(), "cache_exact", vec![]);
    assert_eq!(sized.tree, doubled.tree);
    let root_menu = |s: &StreetSnapshot| snapshot_node_at(s, &[]).expect("the root is exported").actions.clone();
    assert_eq!((root_menu(&snapshot), root_menu(&sized)), (vec![CHECK, bet(50)], vec![CHECK, bet(100)]), "the query's chips");
    assert_eq!(snapshot_node_at(&sized, &[1]).expect("the node after the bet").path, vec![bet(100)], "the query's chip path");
    assert_eq!(sized.covered_paths, snapshot.covered_paths, "the same ordinal paths");
    assert_eq!(sized.key.root_range_hashes, snapshot.key.root_range_hashes);
    for (x, y) in sized.nodes.iter().zip(&snapshot.nodes) {
        assert!(x.probs == y.probs && x.available == y.available, "the stored rows, node {:?}", x.path);
        for c in (0..COMBOS).filter(|c| y.available[*c]) {
            for (big_ev, ev) in x.ev_chips[c].iter().zip(&y.ev_chips[c]) {
                assert!((big_ev - 2.0 * ev).abs() <= 1e-3, "node {:?} combo {c}: EV {big_ev} at 200 for {ev} at 100", x.path);
            }
        }
    }
    assert!((f64::from(sized.exploitability_chips) - 2.0 * f64::from(snapshot.exploitability_chips)).abs() < 1e-5);

    // Original suits: on Kc 7d 2h the canonical suits differ from the query's, and every exported node's availability
    // is its actor's public range support in the query's own suits (KK without the club king, not without another).
    let scenario = support::Scenario { board: support::cards("Kc 7d 2h"), ..support::Scenario::new("flop_fast_v1", 100, 500, 5000) };
    let offsuit = CacheRig::with_entry(scenario, |_| {});
    let q = offsuit.query(100, 500, 5000, &[], "oop");
    assert_ne!(q.inverse_perm, core_iso::SuitPerm::IDENTITY, "the board is not its own canonical form");
    let h = offsuit.hit(&q);
    let inp = rig_input(&offsuit, 100, 500, 5000, &h);
    let s = snapshot_from_solution(&id, &inp, &h.solution, h.covered_paths.clone(), h.tree_signature.clone(), "cache_exact", vec![]);
    assert!(s.nodes.iter().any(|n| n.actor == "ip"), "an IP node is exported");
    for (path, node) in s.covered_paths.iter().zip(&s.nodes) {
        let range = &inp.ranges[if node.actor == "oop" { 0 } else { 1 }];
        assert!(node.available.iter().zip(range.0.iter()).all(|(a, w)| *a == (*w > 0.0)), "node {path:?}: rows in the query's own suits");
    }
}

// ===================================== one store, one rule =====================================

/// Brief Step 3 on the as-built seams. The register-rule unit assertions of the brief, on `SnapshotStore` directly: for
/// every origin of `replay_bridge::ORIGINS` (a live solve and the three cache routes) the active decision's snapshot
/// registers and a stale identity's never does, and a later registration of the same decision and street replaces the
/// earlier one. Then through the engine's own deliveries: a live flop solve and, at the next decision, the cache hit that
/// serves the stored terminal back register in the one store the replay reads, each with its own accepted `Final`, and
/// compete in the turn replay's one selection (equal coverage and accuracy: the later decision wins, spec 9.2).
#[test]
fn cache_and_live_registrations_share_one_store_and_one_rule() {
    let rig = CacheRig::new("flop_fast_v1", 100, 500, 5000);
    let query = rig.query(100, 500, 5000, &[], "oop");
    let hit = rig.hit(&query);
    let input = rig_input(&rig, 100, 500, 5000, &hit);
    let active = DecisionIdentity { hand_id: 1, hand_revision: 7, decision_id: 9, config_revision: 1, model_revision: 0 };
    let stale = DecisionIdentity { decision_id: 8, ..active.clone() };
    let of = |id: &DecisionIdentity, origin: &str| snapshot_from_solution(id, &input, &hit.solution, hit.covered_paths.clone(), hit.tree_signature.clone(), origin, vec![]);
    assert_eq!(ORIGINS, ["live", "cache_exact", "cache_approximate", "cache_provisional"]);
    let mut store = SnapshotStore::new();
    for origin in ORIGINS {
        assert!(store.register(&active, of(&active, origin)), "{origin}");
        assert!(!store.register(&active, of(&stale, origin)), "a stale identity never registers ({origin})");
    }
    let kept = store.for_identity(&active);
    assert_eq!(kept.len(), 1, "a later origin replaces the earlier one");
    assert_eq!(kept[0].provenance.origin, "cache_provisional");

    // Through the engine: the 100/100 flop solved live, then served back from its stored terminal.
    let flop = board(&srp_with(SB, support::hero_cards(), 145, 45), support::FLOP);
    let turn = board(&play(&flop, &[bet(50), CALL]), support::TURN);
    let mut rig = FlopRig::new([support::live_script(&flop, "flop_fast_v1", 0.4, "ok"), support::live_script(&turn, "turn_std_v1", 0.4, "ok")].concat());
    rig.core.install_replay_ranges();
    let live = rig.serve(&flop);
    assert_eq!(live.final_rec().assumptions.cache, "miss");
    assert_eq!(rig.stored().len(), 1, "the live terminal is stored");
    let cached = rig.serve(&flop);
    let f = cached.final_rec();
    assert!(f.assumptions.source.starts_with("cache@") && rig.solves().len() == 1, "the second decision is the cache's: {:?}", f.assumptions);
    let (d_live, d_cache) = (live.id.decision_id, cached.id.decision_id);
    assert_eq!(rig.origins(), vec![(Street::Flop, "live".to_string(), d_live), (Street::Flop, "cache_approximate".to_string(), d_cache)],
        "both registered in the one store, each with its own Final");
    let (s_live, s_cache) = (rig.snapshots_of(d_live).remove(0), rig.snapshots_of(d_cache).remove(0));
    assert_eq!(s_cache.key.root_range_hashes, s_live.key.root_range_hashes, "one public root");
    assert_eq!(s_cache.key.tree_signature, s_live.key.tree_signature);
    assert!(s_cache.tree == s_live.tree && s_cache.covered_paths == s_live.covered_paths);
    assert!(s_cache.nodes.iter().zip(&s_live.nodes).all(|(c, l)| c.probs == l.probs && c.available == l.available), "the cache serves back the live solve's rows");
    rig.identity.lock().unwrap().mutate();
    let on_turn = rig.serve(&turn);
    let seen: Vec<StreetSnapshot> = rig.core.snapshots.lock().unwrap().for_identity(&on_turn.id).into_iter().filter(|s| s.key.street == Street::Flop).collect();
    assert_eq!(seen.len(), 2);
    let replay = direct(&rig, &turn, &seen);
    assert_eq!(replay.snapshots_used, vec![(Street::Flop, s_cache.provenance.clone())], "equal coverage and accuracy: the later decision");
    let notes = &on_turn.final_rec().assumptions.notes;
    assert!(notes.contains(&snapshot_note(Street::Flop, &s_cache.provenance)) && !notes.contains(&snapshot_note(Street::Flop, &s_live.provenance)), "{notes:?}");
    let solve = rig.solves().last().cloned().unwrap();
    assert!(solve.oop_range == seat_range(&replay, SB) && solve.ip_range == seat_range(&replay, BTN));
    rig.core.shutdown();
}

// ===================================== Step 1: the flop hit, then the turn =====================================

/// Brief Step 1 (spec 9.2, 10.4): a cache-hit `Final` at the flop root (100/500), then hero bets 50, the button calls
/// and the turn 4d comes. The hit registers its query-sized snapshot with its `Final`: keyed by the replay's public root
/// ranges and the query's signature, holding the query tree and the flop's nodes only. The turn is one live solve at the
/// turn street root, from the replay's published ranges (no turn strategy is ever taken from the flop entry: its own
/// probe misses), where the root's bet and the button's call each condition once, through the snapshot's own columns.
/// The turn `Final` discloses the snapshot with its cache origin, and nothing of the flop is unconditioned.
#[test]
fn a_flop_cache_hit_conditions_the_turn_root_once_per_action() {
    let (mut rig, o) = run(&on_menu(545, 45, None));
    let f1 = o.first.final_rec();
    assert_eq!((o.first.kinds(), f1.assumptions.cache.as_str(), f1.assumptions.source.starts_with("cache@")), (vec!["Fast", "Final"], "approximate", true));
    assert_eq!(rig.origins(), vec![(Street::Flop, "cache_approximate".to_string(), o.first.id.decision_id), (Street::Turn, "live".to_string(), o.second.id.decision_id)]);
    // The snapshot.
    let s = &o.snapshot;
    let root = core_model::street_root(&o.cached).unwrap();
    let query = build_tree_full(&root, &TemplateSelection::from_history("flop_fast_v1", &[])).unwrap();
    assert_eq!(s.provenance, SnapshotProvenance { identity_at_solve: o.first.id.clone(), solved_prefix: vec![], origin: "cache_approximate".into() });
    assert_eq!(s.key.root_range_hashes, [hash_scaled(&o.root_ranges[0]), hash_scaled(&o.root_ranges[1])], "the query's public root ranges");
    assert_eq!((&s.tree, &s.key.tree_signature), (&query.tree, &tree_signature(&query.tree, query.pot)));
    let flop_nodes: Vec<Vec<u8>> = s.tree.materialized.iter().filter(|m| m.street == Street::Flop).map(|m| m.path.clone()).collect();
    assert_eq!(s.covered_paths, flop_nodes, "every flop decision node, and nothing of a later street");
    assert!(s.nodes.iter().zip(&o.entry.nodes).all(|(n, e)| n.probs == e.probs), "the stored rows in the query's suits");
    // The turn: one live solve at the turn street root.
    let solve = o.solve();
    assert_eq!(o.solves.len(), 1, "the flop is never solved");
    assert_eq!((solve.tree.root_street, solve.tree.template_id.as_str(), solve.pot, solve.stack_oop, solve.stack_ip, solve.history.clone()),
        (Street::Turn, "turn_std_v1", 200, 450, 450, vec![]));
    let f2 = o.second.final_rec();
    assert_eq!((f2.assumptions.cache.as_str(), f2.assumptions.source.starts_with("solver-worker@")), ("miss", true), "the turn's own probe missed");
    assert!(solve.oop_range == seat_range(&o.replay, SB) && solve.ip_range == seat_range(&o.replay, BTN), "the replay's published marginals");
    // Each observed action conditions once.
    let (bet50, call) = (column(s, &[], bet(50)), column(s, &[1], CALL));
    assert_proportional(&solve.oop_range, &on_board(&o.next.board, |c| bet50[c]), "the small blind, through the root's Bet 50");
    assert_proportional(&solve.ip_range, &on_board(&o.next.board, |c| call[c]), "the button, through its call at [1]");
    assert_eq!(o.replay.branches.iter().filter(|b| !b.residual).count(), 1, "on-menu actions never split a branch");
    // The flop is conditioned through the snapshot, disclosed with its origin.
    let note = snapshot_note(Street::Flop, &s.provenance);
    assert!(note.contains("the cache_approximate snapshot of decision"), "{note}");
    assert!(fast_of(&o.second).assumptions.notes.contains(&note) && f2.assumptions.notes.contains(&note), "{:?}", f2.assumptions.notes);
    assert_eq!(o.replay.snapshots_used, vec![(Street::Flop, s.provenance.clone())]);
    let rs = reasons(f2);
    assert!(unconditioned(&rs, Street::Flop).is_empty(), "{rs:?}");
    assert!(o.replay.reasons.iter().all(|r| rs.contains(r)), "the Final inherits every replay reason");
    rig.core.shutdown();
}

/// Brief Step 1: equivalent 100/500 and 200/1000 paths produce equal normalized turn ranges. The 200/1000 flop is served
/// from the entry stored at 100/500 (its rake cap scaled with the pot), as a query-sized snapshot: the query tree in the
/// query's chips, the stored rows, the exploitability over the query's pot. The turn replays hero's 100-chip bet and the
/// call through it exactly as the 50-chip line through the 100/500 snapshot: equal ranges, `log_reach` and weights. The
/// two trees agree within spec 10.4's menu tolerance, not exactly: the 2.5x re-raise over the 125-chip raise is 312.5
/// chips, 313 once rounded at pot 100 but exactly 625 at pot 200, so the 200/1000 hit discloses `MenuRounded{0.5}` (the
/// one reason the two replays differ by: its snapshot carries it into the turn).
#[test]
fn equivalent_100_500_and_200_1000_paths_replay_equal_normalized_ranges() {
    let (mut small_rig, small) = run(&on_menu(545, 45, None));
    let (mut big_rig, big) = run(&on_menu(1095, 95, Some((100, 500, 2500))));
    assert_eq!((big.entry.source.pot, big.entry.source.stack_oop, big.entry.source.cap_mchips), (100, 500, 2500), "stored at 100/500");
    assert_eq!(big.first.final_rec().assumptions.cache, "approximate");
    let menu_at = |s: &StreetSnapshot, path: &[u8]| s.tree.materialized.iter().find(|m| m.path == path).expect("a materialized node").actions.clone();
    let reraise = |to: u32| vec![Action::Fold, CALL, Action::Raise { to }];
    assert_eq!((menu_at(&small.snapshot, &[1, 2]), menu_at(&big.snapshot, &[1, 2])), (reraise(313), reraise(625)), "313/100 against 625/200: 0.5% of the pot");
    let rounded = ApproxReason::MenuRounded { max_delta_pct: 0.5 };
    assert!(reasons(big.first.final_rec()).contains(&rounded) && big.snapshot.reasons.contains(&rounded), "{:?}", big.snapshot.reasons);
    assert!(!small.snapshot.reasons.contains(&rounded), "{:?}", small.snapshot.reasons);
    assert_eq!(big.solves.len(), 1, "the 200/1000 flop is never solved");
    let root_menu = |s: &StreetSnapshot| snapshot_node_at(s, &[]).expect("the root is exported").actions.clone();
    assert_eq!((root_menu(&small.snapshot), root_menu(&big.snapshot)), (vec![CHECK, bet(50)], vec![CHECK, bet(100)]), "each snapshot in its query's chips");
    assert_eq!(big.snapshot.covered_paths, small.snapshot.covered_paths);
    assert!(big.snapshot.nodes.iter().zip(&small.snapshot.nodes).all(|(b, s)| b.probs == s.probs), "the same stored rows");
    assert!((f64::from(big.snapshot.exploitability_chips) - 2.0 * f64::from(small.snapshot.exploitability_chips)).abs() < 1e-5, "over the query's pot");
    assert_eq!(big.snapshot.key.root_range_hashes, small.snapshot.key.root_range_hashes);
    // The turn roots differ in chips only.
    assert_eq!((small.solve().pot, small.solve().stack_oop, big.solve().pot, big.solve().stack_oop), (200, 450, 400, 900));
    assert!(big.solve().oop_range == small.solve().oop_range && big.solve().ip_range == small.solve().ip_range, "equal normalized turn ranges");
    for (x, y) in big.replay.log_reach.iter().zip(&small.replay.log_reach) {
        assert!((x - y).abs() <= 1e-12, "log_reach {x} against {y}");
    }
    assert_eq!(big.replay.branches.len(), small.replay.branches.len());
    for (x, y) in big.replay.branches.iter().zip(&small.replay.branches) {
        assert!((x.q - y.q).abs() <= 1e-12, "q {} against {}", x.q, y.q);
    }
    assert_eq!(big.replay.reasons, [small.replay.reasons.clone(), vec![rounded]].concat(), "the one extra disclosure");
    small_rig.core.shutdown();
    big_rig.core.shutdown();
}

// ===================================== Step 4: the three export coverages =====================================

/// Spec 9.2's three export coverages on `street` (the flop, or the turn after a checked-through flop), each a cache hit
/// of hero's decision followed by the off-menu line (the small blind checks, the button bets 73 into 100, the small blind
/// calls) and a live solve of hero's next decision:
///
/// - requested-node-only covering `[Check]` (hero the button): the missing root does not condition the small blind, the
///   button's translated wager does (split `f_A`/`f_B` over the two mapped sizes, each through its own column), the
///   uncovered call does not;
/// - root-only: the check conditions; the wager and the call do not (the path freezes at the uncovered node), and the
///   check's conditioned mass is kept;
/// - complete street: all three condition, the call in each translated branch through that branch's own node.
///
/// The published next-street ranges are the closed forms of spec 8.4's kernel over the snapshot's columns, the branch
/// weights their split, the `BetTranslation` discloses the weights and its integer prominence, and no prior street is
/// ever re-solved. Hero's cards enter no public range, snapshot or cache key.
fn check_off_menu(street: Street) {
    for export in [Export::RequestedOnly, Export::RootOnly, Export::Complete] {
        let case = off_menu(street, export);
        let (mut rig, o) = run(&case);
        let what = format!("{street:?} {export:?}");
        let s = &o.snapshot;
        let root = core_model::street_root(&o.cached).unwrap();
        // The cache decision: hero's, answered by the hit and registered with its Final.
        assert_eq!(o.first.kinds(), ["Fast", "Final"], "{what}");
        assert_eq!((s.provenance.origin.as_str(), &s.provenance.solved_prefix, s.key.street), ("cache_approximate", &root.history, street), "{what}");
        let street_nodes: Vec<Vec<u8>> = s.tree.materialized.iter().filter(|m| m.street == street).map(|m| m.path.clone()).collect();
        let exported = match export {
            Export::RequestedOnly => vec![vec![0]],
            Export::RootOnly => vec![vec![]],
            Export::Complete => street_nodes,
        };
        assert_eq!(s.covered_paths, exported, "{what}");
        // The next street: one live solve at its own street root, from the replay's published ranges, conditioned through
        // the snapshot, which the Final discloses with its origin.
        let solve = o.solve();
        assert_eq!((o.solves.len(), solve.tree.root_street), (1, next_street(street)), "{what}: no prior street is re-solved");
        let (oop, ip) = (seat_range(&o.replay, SB), seat_range(&o.replay, BTN));
        assert!(solve.oop_range == oop && solve.ip_range == ip, "{what}: the solve's root ranges are the replay's");
        assert_eq!(o.replay.snapshots_used, vec![(street, s.provenance.clone())], "{what}");
        let f2 = o.second.final_rec();
        let rs = reasons(f2);
        assert!(o.replay.reasons.iter().all(|r| rs.contains(r)), "{what}: the Final inherits every replay reason");
        assert!(f2.assumptions.notes.contains(&snapshot_note(street, &s.provenance)), "{what}: {:?}", f2.assumptions.notes);
        if street == Street::Turn {
            assert_eq!(unconditioned(&rs, Street::Flop), [(SB, NO_REQUEST.to_string()), (BTN, NO_REQUEST.to_string())], "{what}: no flop request");
        }
        // The node after the check, where the button bets 73: two wager sizes, the translation's `A` and `B`.
        let at0 = s.tree.materialized.iter().find(|m| m.path == [0]).expect("the node after the check");
        let wagers: Vec<(usize, Action)> = at0.actions.iter().copied().enumerate().filter(|(_, a)| matches!(a, Action::Bet { .. } | Action::AllIn { .. })).collect();
        let menu: Vec<Action> = wagers.iter().map(|w| w.1).collect();
        assert_eq!(menu, if street == Street::Flop { [bet(50), all_in(100)] } else { [bet(33), all_in(100)] }, "{what}: the snapshot's menu");
        let to = |a: Action| match a {
            Action::Bet { to } | Action::AllIn { to } => f64::from(to),
            other => panic!("{other:?} is not a wager"),
        };
        let t = core_preflop::interpolate(0.73, &[(wagers[0].0, to(wagers[0].1) / 100.0), (wagers[1].0, to(wagers[1].1) / 100.0)]).expect("0.73 is bracketed");
        let (fa, fb) = (t.choices[0].1, t.choices[1].1);
        if street == Street::Flop {
            assert!((fa - 81.0 / 173.0).abs() < 1e-12 && (fb - 92.0 / 173.0).abs() < 1e-12, "{what}: {t:?}");
        }
        let translated = translations(&rs, street, BTN);
        let uncovered = |path: &[u8]| format!("uncovered path {path:?}");
        let board = &o.next.board;
        let uniform = on_board(board, |_| 1.0);
        match export {
            Export::RequestedOnly => {
                let causes = sorted(unconditioned(&rs, street));
                assert_eq!(causes, sorted(vec![(SB, uncovered(&[])), (SB, uncovered(&[0, 1])), (SB, uncovered(&[0, 2]))]), "{what}");
                assert_eq!(translated.len(), 1, "{what}: {rs:?}");
                let (pa, pb) = (column(s, &[0], menu[0]), column(s, &[0], menu[1]));
                assert_proportional(&oop, &uniform, &format!("{what}: the small blind, unconditioned"));
                assert_proportional(&ip, &on_board(board, |c| fa * pa[c] + fb * pb[c]), &format!("{what}: the button, through f_A P_A + f_B P_B"));
                let ratio = branch_q(&o.replay, BTN, menu[0]) / branch_q(&o.replay, BTN, menu[1]);
                assert!((ratio - fa * mean(&pa) / (fb * mean(&pb))).abs() <= 1e-9 * ratio, "{what}: the branch weights");
            }
            Export::RootOnly => {
                let causes = sorted(unconditioned(&rs, street));
                assert_eq!(causes, sorted(vec![(BTN, uncovered(&[0])), (SB, uncovered(&[0]))]), "{what}");
                assert!(translated.is_empty(), "{what}: nothing is translated at an unexported node");
                let check = column(s, &[], CHECK);
                assert_proportional(&oop, &on_board(board, |c| check[c]), &format!("{what}: the small blind keeps its check's conditioning"));
                assert_proportional(&ip, &uniform, &format!("{what}: the button, unconditioned"));
                assert_eq!(o.replay.branches.iter().filter(|b| !b.residual).count(), 1, "{what}: no split");
            }
            Export::Complete => {
                assert!(unconditioned(&rs, street).is_empty(), "{what}: {rs:?}");
                assert_eq!(translated.len(), 1, "{what}: {rs:?}");
                let check = column(s, &[], CHECK);
                let (pa, pb) = (column(s, &[0], menu[0]), column(s, &[0], menu[1]));
                let (ia, ib) = (u8::try_from(wagers[0].0).unwrap(), u8::try_from(wagers[1].0).unwrap());
                let (ca, cb) = (column(s, &[0, ia], CALL), column(s, &[0, ib], CALL));
                // Each branch's call integrates over the small blind's checked masses.
                let through = |call: &[f64]| (0..COMBOS).map(|c| check[c] * call[c]).sum::<f64>() / check.iter().sum::<f64>();
                let (ka, kb) = (through(&ca), through(&cb));
                let (ma, mb) = (mean(&pa), mean(&pb));
                assert_proportional(&ip, &on_board(board, |c| fa * ka * pa[c] + fb * kb * pb[c]), &format!("{what}: the button"));
                assert_proportional(&oop, &on_board(board, |c| check[c] * (fa * ma * ca[c] + fb * mb * cb[c])), &format!("{what}: the small blind"));
                let ratio = branch_q(&o.replay, BTN, menu[0]) / branch_q(&o.replay, BTN, menu[1]);
                assert!((ratio - fa * ma * ka / (fb * mb * kb)).abs() <= 1e-9 * ratio, "{what}: the branch weights");
            }
        }
        if let Some(ApproxReason::BetTranslation { observed_pct, mapped, deviation, prominent, .. }) = translated.first() {
            assert!((f64::from(*observed_pct) - 0.73).abs() < 1e-6, "{what}");
            assert_eq!(mapped.len(), 2, "{what}");
            let sizes = [to(menu[0]) / 100.0, to(menu[1]) / 100.0];
            for (k, (size, f)) in mapped.iter().enumerate() {
                assert!((f64::from(*size) - sizes[k]).abs() < 1e-6 && (f64::from(*f) - t.choices[k].1).abs() < 1e-6, "{what}: mapped {mapped:?}");
            }
            assert!((f64::from(*deviation) - t.deviation).abs() < 1e-6, "{what}");
            // Decided in exact chips at the mapped parent (plan-3 F-M2): 10 * 23 > 100 + 0 on the flop, 10 * 27 on the turn.
            assert!(*prominent, "{what}");
            assert!(f2.assumptions.translations.contains(translated.first().unwrap()), "{what}: listed in the assumptions");
        }
        // Hero's cards: hero's own combo keeps its public weight, and other hero cards change no key and no range.
        let hero_combo = usize::from(proto::combo_index(case.cards[0], case.cards[1]));
        assert!(solve.oop_range.0[hero_combo] > 0.0 && solve.ip_range.0[hero_combo] > 0.0, "{what}");
        if export == Export::RequestedOnly {
            let (mut other_rig, other) = run(&Case { cards: hole("QsQc"), ..case.clone() });
            assert_eq!(other.entry.key.digest(), o.entry.key.digest(), "{what}: hero's cards enter no cache key");
            assert_eq!(other.snapshot.key, o.snapshot.key, "{what}: nor a snapshot key");
            assert!(other.solve().oop_range == solve.oop_range && other.solve().ip_range == solve.ip_range, "{what}: nor a public range");
            other_rig.core.shutdown();
        }
        rig.core.shutdown();
    }
}

#[test]
fn a_prior_street_off_menu_wager_is_translated_across_the_three_export_coverages() {
    check_off_menu(Street::Flop);
}

/// Brief Step 4: the same three cases one street later. A turn cache hit conditions the river root the same way (the
/// 73 translated over `turn_std_v1`'s 33 and the all-in), the flop without a request stays unconditioned with the
/// engine's cause, and only the river is solved.
#[test]
fn turn_to_river_translation_through_a_turn_cache_snapshot() {
    check_off_menu(Street::Turn);
}

// ===================================== the three cache origins =====================================

/// Spec 9.1/9.3: every cache origin reaches the turn `Final`. The complete flop entry is served as `cache_exact` (the
/// explicit full ranges: no inherited reason), `cache_approximate` (the replay's ranges: its preflop reasons) and
/// `cache_provisional` (stored above the target: the `Provisional`, a failed live refinement, the retained payload as
/// the `Final`); each registers its snapshot with its delivery, and the turn's `Fast` and `Final` name it with its
/// origin. The snapshot's reasons ride along (ruling 18-I1: the provisional's accuracy shortfall included), and the
/// origin changes the disclosure only, never a mass.
#[test]
fn every_cache_origin_reaches_the_turn_final() {
    let base = off_menu(Street::Flop, Export::Complete);
    let mut ranges = vec![];
    for (origin, case) in [
        ("cache_exact", Case { explicit: true, ..base.clone() }),
        ("cache_approximate", base.clone()),
        ("cache_provisional", Case { raw_over_p: 0.019, ..base.clone() }),
    ] {
        let (mut rig, o) = run(&case);
        let f1 = o.first.final_rec();
        let (d1, d2) = (o.first.id.decision_id, o.second.id.decision_id);
        assert_eq!(o.snapshot.provenance.origin, origin);
        assert_eq!(rig.origins(), vec![(Street::Flop, origin.to_string(), d1), (Street::Turn, "live".to_string(), d2)], "{origin}");
        let shortfall = ApproxReason::DeadlineBestSoFar { reached_bp: 190, target_bp: 50 };
        match origin {
            "cache_exact" => {
                assert_eq!((o.first.kinds(), &f1.coverage, f1.assumptions.cache.as_str(), o.solves.len()), (vec!["Fast", "Final"], &Coverage::Exact, "exact", 1));
                assert!(o.snapshot.reasons.is_empty());
            }
            "cache_approximate" => {
                assert_eq!((o.first.kinds(), f1.assumptions.cache.as_str(), o.solves.len()), (vec!["Fast", "Final"], "approximate", 1));
                assert!(matches!(f1.coverage, Coverage::Approximate { .. }));
            }
            _ => {
                assert_eq!((o.first.kinds(), f1.assumptions.cache.as_str()), (vec!["Fast", "Provisional", "Final"], "provisional"));
                let sent: Vec<&str> = o.solves.iter().map(|q| q.tree.template_id.as_str()).collect();
                assert_eq!(sent, ["flop_fast_v1", "flop_min_v1", "turn_std_v1"], "the failed refinement and its retry, then the turn");
                assert!(o.snapshot.reasons.contains(&shortfall) && reasons(o.second.final_rec()).contains(&shortfall), "{:?}", o.snapshot.reasons);
            }
        }
        assert_eq!(o.snapshot.key.root_range_hashes, [hash_scaled(&o.root_ranges[0]), hash_scaled(&o.root_ranges[1])], "{origin}: the replay's root hashes");
        let note = snapshot_note(Street::Flop, &o.snapshot.provenance);
        assert!(note.contains(&format!("the {origin} snapshot of decision {d1}")), "{note}");
        let f2 = o.second.final_rec();
        assert!(fast_of(&o.second).assumptions.notes.contains(&note) && f2.assumptions.notes.contains(&note), "{origin}: {:?}", f2.assumptions.notes);
        assert_eq!(o.replay.snapshots_used, vec![(Street::Flop, o.snapshot.provenance.clone())]);
        assert!(o.snapshot.reasons.iter().all(|r| reasons(f2).contains(r)), "{origin}: the snapshot's reasons reach the turn");
        ranges.push((o.solve().oop_range.clone(), o.solve().ip_range.clone()));
        rig.core.shutdown();
    }
    assert!(ranges.windows(2).all(|w| w[0] == w[1]), "the origin changes no mass");
}

// ===================================== the current street =====================================

/// Brief Step 4 (spec 10.2): a current-street 73 is inserted exactly alongside the 50, never translated. After the flop
/// cache hit at the root, hero checks and the button bets 73: hero's decision facing it builds the tree with the 73
/// inserted, which no stored entry answers, so it is solved live from the flop root ranges (the current street is never
/// replayed) with no translation disclosed. On the turn both flop snapshots are compatible and cover the whole line;
/// the live one (the 73 on its own menu) is the more accurate and is selected, so the flop conditions with no
/// translation at all.
#[test]
fn a_current_street_off_menu_wager_is_inserted_exactly_never_translated() {
    let case = off_menu(Street::Flop, Export::Complete);
    let (cached, _) = states(&case);
    let facing = play(&cached, &[CHECK, bet(73)]);
    let turn = board(&play(&facing, &[CALL]), support::TURN);
    let mut rig = FlopRig::new([support::live_script(&facing, "flop_fast_v1", 0.3, "ok"), support::live_script(&turn, "turn_std_v1", 0.4, "ok")].concat());
    rig.core.install_replay_ranges();
    let (_, root_ranges) = seed(&rig, &case, &cached);
    let first = rig.serve(&cached);
    assert_eq!(first.final_rec().assumptions.cache, "approximate");
    rig.identity.lock().unwrap().mutate();
    let at_73 = rig.serve(&facing);
    let f = at_73.final_rec();
    let solve = rig.solves()[0].clone();
    assert_eq!(f.assumptions.cache, "miss", "the inserted size is another tree: no entry answers it");
    let at0 = solve.tree.materialized.iter().find(|m| m.path == [0]).expect("the node after the check");
    assert_eq!(at0.actions, [CHECK, bet(50), bet(73), all_in(100)], "inserted exactly alongside the 50");
    assert_eq!(solve.tree.inserted, vec![(vec![CHECK], "ip".to_string(), bet(73))]);
    assert_eq!(solve.history, vec![CHECK, bet(73)]);
    assert!(solve.oop_range == root_ranges[0] && solve.ip_range == root_ranges[1], "the current street never enters the root ranges");
    assert!(translations(&reasons(f), Street::Flop, BTN).is_empty(), "never translated: {:?}", f.coverage);
    // The turn.
    rig.identity.lock().unwrap().mutate();
    let on_turn = rig.serve(&turn);
    let live = rig.snapshots_of(at_73.id.decision_id).remove(0);
    let hit = rig.snapshots_of(first.id.decision_id).remove(0);
    assert_eq!((live.provenance.origin.as_str(), hit.provenance.origin.as_str()), ("live", "cache_approximate"));
    let seen: Vec<StreetSnapshot> = vec![hit, live.clone()];
    let replay = direct(&rig, &turn, &seen);
    assert_eq!(replay.snapshots_used, vec![(Street::Flop, live.provenance.clone())], "the more accurate snapshot of the two");
    let f2 = on_turn.final_rec();
    assert!(translations(&reasons(f2), Street::Flop, BTN).is_empty() && unconditioned(&reasons(f2), Street::Flop).is_empty(), "{:?}", f2.coverage);
    assert!(f2.assumptions.notes.contains(&snapshot_note(Street::Flop, &live.provenance)));
    let solve = rig.solves().last().cloned().unwrap();
    assert!(solve.oop_range == seat_range(&replay, SB) && solve.ip_range == seat_range(&replay, BTN));
    rig.core.shutdown();
}

// ===================================== undo and reuse =====================================

/// Brief Step 4 (spec 9.2, plan 3's retention rule, applied by `Engine::apply_action`, `set_board` and `undo` as a fresh
/// revision followed by `SnapshotStore::invalidate` against the new state): the requested-node-only case's cache
/// snapshot, solved after the small blind's check, and the turn's live one. An append-only mutation keeps both. Undoing
/// to the flop facing the 73 drops the later street's snapshot and keeps the prefix-compatible cache snapshot, with its
/// original provenance: redoing the line replays through it exactly as before. Undoing past the check drops it.
#[test]
fn undo_keeps_prefix_compatible_cache_snapshots_and_discards_the_rest() {
    let case = off_menu(Street::Flop, Export::RequestedOnly);
    let (mut rig, o) = run(&case);
    let listed = |rig: &FlopRig| rig.origins().into_iter().map(|(street, origin, _)| (street, origin)).collect::<Vec<_>>();
    let both = vec![(Street::Flop, "cache_approximate".to_string()), (Street::Turn, "live".to_string())];
    assert_eq!(listed(&rig), both);
    let mutate_to = |rig: &FlopRig, state: &HandState| {
        rig.identity.lock().unwrap().mutate();
        rig.core.snapshots.lock().unwrap().invalidate(state);
    };
    // Append-only: the button checks behind on the turn.
    mutate_to(&rig, &play(&o.next, &[CHECK]));
    assert_eq!(listed(&rig), both, "an append-only mutation keeps every decision's snapshot");
    // Undo to the flop, facing the button's 73.
    mutate_to(&rig, &play(&o.cached, &[bet(73)]));
    assert_eq!(listed(&rig), vec![(Street::Flop, "cache_approximate".to_string())], "the later street's snapshot is dropped");
    let kept = rig.core.snapshots.lock().unwrap().for_hand(1)[0].clone();
    assert_eq!(kept, o.snapshot, "retained with its original identity and provenance");
    let redone = direct(&rig, &o.next, &[kept]);
    assert!(seat_range(&redone, SB) == o.solve().oop_range && seat_range(&redone, BTN) == o.solve().ip_range, "the redone line replays as before");
    // Undo before the small blind's check: the decision the cache snapshot was solved for is undone.
    mutate_to(&rig, &board(&srp_with(case.hero, case.cards, case.stack, case.open), support::FLOP));
    assert!(listed(&rig).is_empty(), "the incompatible snapshot is dropped");
    rig.core.shutdown();
}

/// Spec 9.2's compatibility: a cache snapshot is reused only by its own hand, config revision, model revision, public
/// root ranges and board. Replayed at the turn it conditions the flop; for another hand or config revision, under other
/// root hashes, or on another flop it is never compatible (`no compatible snapshot`); the engine's identity slices hand
/// the replay nothing of another hand, config or model revision; and another board invalidates it outright.
#[test]
fn a_cache_snapshot_is_reused_only_by_its_own_hand_config_model_ranges_and_board() {
    let case = off_menu(Street::Flop, Export::Complete);
    let (mut rig, o) = run(&case);
    let flop_only = [o.snapshot.clone()];
    let reused = direct(&rig, &o.next, &flop_only);
    assert_eq!(reused.snapshots_used, vec![(Street::Flop, o.snapshot.provenance.clone())]);
    assert!(seat_range(&reused, SB) == o.solve().oop_range && seat_range(&reused, BTN) == o.solve().ip_range);
    let refused = |out: &ReplayOutput, what: &str| {
        assert!(out.snapshots_used.is_empty(), "{what}: {:?}", out.snapshots_used);
        let cause = "no compatible snapshot".to_string();
        assert_eq!(unconditioned(&out.reasons, Street::Flop), [(SB, cause.clone()), (BTN, cause)], "{what}");
    };
    refused(&direct(&rig, &HandState { hand_id: 2, ..o.next.clone() }, &flop_only), "another hand");
    let mut other_config = o.next.clone();
    other_config.config.config_revision = 2;
    refused(&direct(&rig, &other_config, &flop_only), "another config revision");
    let mut other_ranges = o.snapshot.clone();
    other_ranges.key.root_range_hashes[1][0] ^= 1;
    refused(&direct(&rig, &o.next, &[other_ranges]), "other public root ranges");
    let (other_flop, other_turn) = states_on(&case, "Kh 7d 3c", "Kh 7d 3c 4d", "Kh 7d 3c 4d 9s");
    refused(&direct(&rig, &other_turn, &flop_only), "another flop");
    {
        let store = rig.core.snapshots.lock().unwrap();
        assert_eq!(store.for_identity(&o.second.id).len(), 2);
        let id = &o.second.id;
        for (what, other) in [
            ("hand", DecisionIdentity { hand_id: 2, ..id.clone() }),
            ("config revision", DecisionIdentity { config_revision: 2, ..id.clone() }),
            ("model revision", DecisionIdentity { model_revision: 1, ..id.clone() }),
        ] {
            assert!(store.for_identity(&other).is_empty(), "another {what} reads nothing");
        }
    }
    rig.identity.lock().unwrap().mutate();
    rig.core.snapshots.lock().unwrap().invalidate(&other_flop);
    assert!(rig.origins().is_empty(), "another board drops both: {:?}", rig.origins());
    rig.core.shutdown();
}

/// Review P4T12-M1 (ruling 12-M1): the delivery path keeps the public root ranges in order from the range source to the
/// snapshot key. Every other delivery scenario here has equal OOP and IP root ranges, so an exchange of the pair between
/// the range source and the snapshot input would pass them unseen. Here the range source (`core.range_source`, explicit
/// ranges) gives OOP every combo at 1 and IP every combo at 1 but one unblocked combo at 0.5; the entry is seeded with that
/// ordered pair, board-blocked as the source publishes it; the flop decision is a cache hit delivered through
/// `rig.serve`, and its registered snapshot is keyed by the two distinct hashes in OOP-then-IP order.
#[test]
fn a_cache_hit_keys_its_snapshot_by_the_ordered_root_ranges_it_was_served() {
    let case = off_menu(Street::Flop, Export::Complete);
    let (cached, _) = states(&case);
    let root = core_model::street_root(&cached).expect("hero's flop decision");
    assert_eq!((root.oop, root.ip), (SB, BTN));
    let oop_given = support::full_range();
    let mut ip_given = support::full_range();
    let marked = usize::from(proto::combo_index(Card::parse("Qs").unwrap(), Card::parse("Js").unwrap()));
    ip_given.0[marked] = 0.5;
    let mut rig = FlopRig::new(vec![]);
    *rig.core.range_source.lock().unwrap() = Box::new(ExplicitRanges { oop: Some(oop_given.clone()), ip: Some(ip_given.clone()) });
    let (mut oop, mut ip) = (oop_given, ip_given);
    core_ranges::block_public(&mut oop, &root.board);
    core_ranges::block_public(&mut ip, &root.board);
    assert!(ip.0[marked] == 0.5, "the marked combo is not blocked by the board");
    let expected = [hash_scaled(&oop), hash_scaled(&ip)];
    assert_ne!(expected[0], expected[1], "the two root ranges hash apart");
    let rake = Rake::PotRake { rate: support::RATE, cap_mchips: 5000, no_flop_no_drop: false };
    let entry = export_entry(&root, "flop_fast_v1", [oop, ip], rake, Export::Complete, &[], 0.004);
    assert!(rig.core.cache.store_tracked(&entry).wait(Duration::from_secs(60)), "the writer stores the entry");
    let served = rig.serve(&cached);
    let f = served.final_rec();
    assert_eq!((served.kinds(), f.assumptions.cache.as_str(), &f.coverage), (vec!["Fast", "Final"], "exact", &Coverage::Exact), "a cache hit at target");
    assert!(f.assumptions.source.starts_with("cache@") && rig.solves().is_empty(), "delivered from the cache: {:?}", f.assumptions);
    let snapshots = rig.snapshots_of(served.id.decision_id);
    assert_eq!(snapshots.len(), 1, "registered with its Final");
    assert_eq!(snapshots[0].provenance.origin, "cache_exact");
    assert_eq!(snapshots[0].key.root_range_hashes, expected, "OOP then IP, as served");
    rig.core.shutdown();
}

// ===================================== the golden =====================================

/// A 1326-combo range as JSON numbers, each the shortest decimal that reads back as the same `f32`.
fn weights(r: &Range1326) -> Value {
    Value::Array(r.0.iter().map(|w| json!(w.to_string().parse::<f64>().expect("an f32 prints as a number"))).collect())
}

/// One case's canonical projection (see the module doc). `arrays` writes both next-street root ranges whole; otherwise
/// their hashes (the cases whose ranges other tests assert equal to a case written whole).
fn project(o: &Outcome, arrays: bool) -> Value {
    let s = &o.snapshot;
    let f1 = o.first.final_rec();
    let f2 = o.second.final_rec();
    let solve = o.solve();
    let ranges = if arrays {
        json!({ "oop": weights(&solve.oop_range), "ip": weights(&solve.ip_range) })
    } else {
        json!({ "oop_hash": hex::encode(hash_scaled(&solve.oop_range)), "ip_hash": hex::encode(hash_scaled(&solve.ip_range)) })
    };
    json!({
        "cached": {
            "kinds": o.first.kinds(),
            "coverage": f1.coverage,
            "cache": f1.assumptions.cache,
            "entry": { "pot": o.entry.source.pot, "eff": o.entry.source.stack_oop, "cap_mchips": o.entry.source.cap_mchips, "export": o.entry.export,
                "exploitability_over_P": o.entry.exploitability_over_P },
            "snapshot": {
                "street": s.key.street,
                "root_board": s.key.root_board,
                "root_range_hashes": [hex::encode(s.key.root_range_hashes[0]), hex::encode(s.key.root_range_hashes[1])],
                "tree_signature": s.key.tree_signature,
                "template": s.tree.template_id,
                "origin": s.provenance.origin,
                "decision_id": s.provenance.identity_at_solve.decision_id,
                "solved_prefix": s.provenance.solved_prefix,
                "exploitability_chips": s.exploitability_chips,
                "reasons": s.reasons,
                "nodes": s.covered_paths.iter().zip(&s.nodes).map(|(p, n)| json!({ "path": p, "actor": n.actor, "actions": n.actions })).collect::<Vec<_>>(),
            },
        },
        "next": {
            "kinds": o.second.kinds(),
            "coverage": f2.coverage,
            "cache": f2.assumptions.cache,
            "notes": f2.assumptions.notes,
            "translations": f2.assumptions.translations,
            "solve": { "street": solve.tree.root_street, "template": solve.tree.template_id, "pot": solve.pot, "stack_oop": solve.stack_oop,
                "stack_ip": solve.stack_ip, "history": solve.history, "ranges": ranges },
        },
        "replay": {
            "log_reach": o.replay.log_reach,
            "branches": o.replay.branches.iter().map(|b| json!({ "id": b.id, "q": b.q, "residual": b.residual, "stopped": b.stopped, "translated": b.translated }))
                .collect::<Vec<_>>(),
            "reasons": o.replay.reasons,
            "snapshots_used": o.replay.snapshots_used,
        },
    })
}

/// `v` as pretty JSON (two-space indent), except that an array of numbers is written on one line, so a 1326-combo range
/// is one line of the file.
fn render(v: &Value, indent: usize, out: &mut String) {
    let pad = |n: usize| " ".repeat(n);
    match v {
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Array(items) if items.iter().all(Value::is_number) => out.push_str(&serde_json::to_string(v).expect("a JSON value prints")),
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&pad(indent + 2));
                render(item, indent + 2, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(indent));
            out.push(']');
        }
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Object(map) => {
            out.push_str("{\n");
            for (i, (key, item)) in map.iter().enumerate() {
                out.push_str(&pad(indent + 2));
                out.push_str(&serde_json::to_string(key).expect("a key prints"));
                out.push_str(": ");
                render(item, indent + 2, out);
                out.push_str(if i + 1 < map.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(indent));
            out.push('}');
        }
        other => out.push_str(&serde_json::to_string(other).expect("a JSON value prints")),
    }
}

/// Every golden case: its name, the case, and whether its next-street ranges are written whole.
fn golden_cases() -> Vec<(&'static str, Case, bool)> {
    let complete = off_menu(Street::Flop, Export::Complete);
    vec![
        ("on_menu_100_500", on_menu(545, 45, None), true),
        ("on_menu_200_1000_from_a_100_500_entry", on_menu(1095, 95, Some((100, 500, 2500))), false),
        ("flop_requested_only", off_menu(Street::Flop, Export::RequestedOnly), true),
        ("flop_root_only", off_menu(Street::Flop, Export::RootOnly), true),
        ("flop_complete", complete.clone(), true),
        ("flop_complete_cache_exact", Case { explicit: true, ..complete.clone() }, false),
        ("flop_complete_cache_provisional", Case { raw_over_p: 0.019, ..complete }, false),
        ("turn_requested_only", off_menu(Street::Turn, Export::RequestedOnly), true),
        ("turn_root_only", off_menu(Street::Turn, Export::RootOnly), true),
        ("turn_complete", off_menu(Street::Turn, Export::Complete), true),
    ]
}

/// Brief Step 4: every case's canonical projection, frozen in `golden/cache_snapshot_replay.json` (recorded once with
/// `POKERAI_RECORD_GOLDENS=1` and inspected, then compared; never re-recorded to pass).
#[test]
fn cache_snapshot_replay_golden() {
    let mut cases = serde_json::Map::new();
    for (name, case, arrays) in golden_cases() {
        let (mut rig, o) = run(&case);
        cases.insert(name.into(), project(&o, arrays));
        rig.core.shutdown();
    }
    let mut text = String::new();
    render(&Value::Object(cases), 0, &mut text);
    text.push('\n');
    // Compared as text parsed back by the same parser on both sides (see `flop_path_golden`): a value built in memory is
    // never compared with one read from the file directly.
    let frozen: Value = serde_json::from_str(&text).expect("the rendering is JSON");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/cache_snapshot_replay.json");
    if std::env::var_os("POKERAI_RECORD_GOLDENS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("the committed golden {} is missing ({e}); record it once with POKERAI_RECORD_GOLDENS=1 and inspect it", path.display()));
    let expected: Value = serde_json::from_slice(&bytes).unwrap();
    for (name, value) in frozen.as_object().unwrap() {
        assert_eq!(Some(value), expected.get(name), "golden case {name}");
    }
    assert_eq!(frozen, expected);
}
