//! Plan 3 Task 18: the replay bridge. The replayed public root ranges feed the turn and river solves, and every accepted
//! solution is registered as a replay snapshot through one rule (spec 9, 9.2, 9.3, 10.2).
//!
//! # The root ranges (`ReplayRanges`)
//!
//! `RangeSource::ranges_at_root` is the only root-range provider, and `serve_request` has already derived the street
//! root and classified its errors (`coverage::classify` maps all five `RootError` variants: `Multiway` to the multiway
//! row, `ProjectionNotReproducing` and `Inconsistent` to their `Unsupported` reasons, `NoDecision`, `Preflop`) before it
//! asks. [`ReplayRanges`] only produces the ranges: it reads the active decision (the engine is the identity authority:
//! `ReplayInput` carries no model revision), takes that decision's snapshots and misses (`for_identity`, the hand's
//! config revision included, ruling F-I4), replays the public history over the loaded preflop store, and answers the
//! replay's published ranges of the root's two seats, whole, validated by `ranges.rs`'s own validators (finite `[0, 1]`
//! weights before board blocking, support and joint compatibility after it; the replay published them blocked by this
//! very board, so the blocking changes nothing). The replay's reasons are the root's reasons. Hero's cards enter none of
//! it: they reach only the hero-conditioned copies [`opposing_equity_ranges`] builds after the replay.
//!
//! Locks and panics (brief decisions 3 and 4). The identity lock and the snapshot store's are each held for one step and
//! released before the replay runs; neither is held across a sink callback or a solve. `serve_request` holds the range
//! source's lock while this answers; `ReplayRanges` has no state of its own to leave half-changed (its three handles
//! never change), so a panic of the replay, contained on `engine-main`, leaves a poisoned lock around a source that is
//! as it was. A contained panic may also have interrupted an invalidation of the snapshot store, leaving entries its
//! rules would have dropped: every snapshot and miss read here is validated against the request's state first
//! (`core_replay::snapshot_root`, `SnapshotMiss::in_history`: the rules `SnapshotStore::invalidate` keeps a record by),
//! so a stale entry is never walked and never changes a cause.
//!
//! Missing snapshots (spec 9.3, Task 15 Q2). A completed postflop street with no valid snapshot is disclosed with the
//! engine's concrete cause: the cause the latest request on that street recorded when it ended without a solution (an
//! engine error, the watchdog's deadline: `serve` records a `SnapshotMiss` for the decision whose `Final` was
//! delivered), or `no request` when no request of hero's on the street left anything (a request superseded before any
//! `Final` leaves nothing). A street that opened multiway stays `multiway prior street`
//! (replay's own rule), and a street whose valid snapshots do not match the replayed ranges stays `no compatible
//! snapshot`. The selected snapshots' provenance, their `origin` included, is in `ReplayOutput::snapshots_used`, which
//! `RootRanges::snapshots_used` hands to `serve`: it discloses each in the current result's assumptions
//! ([`snapshot_note`], fix round 1, ruling 18-I3). A snapshot's stored reasons are its solve's full coverage reasons,
//! the solve's own accuracy (`DeadlineBestSoFar`) included (ruling 18-I1), and the walk carries them into the result.
//!
//! Equity (brief decision 6). A turn or river request's `Equity` runs hero's public range against the opponent's, both
//! the replay's published ranges exactly as the solve received them (`serve` hands them to the equity routine). The
//! routine's hero-combo population conditions its own private copy of the opponent's range on hero's cards, which is
//! exactly the copy [`opposing_equity_ranges`] builds (`core_ranges::hero_conditioned` of the replayed range). The
//! routine is given the public ranges, never these copies: its range-versus-range population must read the opponent's
//! public range (spec 4.4), and a copy handed in would enter that population as well.
//!
//! # Registration
//!
//! [`snapshot_from_solution`] builds the snapshot of a resolved, validated solution from the exact solve input (the
//! public root ranges, hashed by `hash_scaled`; the street root and its history, the solved prefix; the tree the solution
//! was validated on). The crate-private `register_accepted` is the one registration rule: `SnapshotStore::register`'s
//! identity gate, and a `Provisional` that never replaces the `Final` of the same decision and street. `serve` applies
//! it inside the accepted delivery of the engine's own `Final` (live `ok` and `best_so_far`), and
//! `Engine::register_snapshot` applies it for plan 4's cache route.

use crate::identity::IdentityState;
use crate::ranges::{jointly_compatible, weights_valid, RangeSource, RootRanges};
use crate::snapshots::{SnapshotKey, SnapshotProvenance, SnapshotStore, StreetSnapshot};
use core_preflop::PreflopStore;
use core_ranges::{block_public, hash_scaled, hero_conditioned, mass, range_to_string};
use core_replay::{snapshot_root, ReplayInput, ReplayOutput, SnapshotMiss};
use proto::worker::StreetSolution;
use proto::{
    index_materialized, resolve_chip_path_indexed, Action, ApproxReason, Coverage, DecisionIdentity, HandState, OrdinalPath, Range1326, Recommendation, Seat,
    SolveInput, Street, StreetRootSnapshot, UnsupportedReason,
};
use std::sync::{Arc, Mutex, MutexGuard};

/// Spec 9.1's four snapshot origins: a live solve, and plan 4's three cache routes.
pub const ORIGINS: [&str; 4] = ["live", "cache_exact", "cache_approximate", "cache_provisional"];

/// The origin a later `Final` of the same decision replaces, and that never replaces one (spec 9.2).
const PROVISIONAL: &str = "cache_provisional";

/// Spec 9.3's cause of a completed street no request of hero's reached (or none that left anything behind).
pub const NO_REQUEST: &str = "no request";

/// A lock that survives a panic elsewhere (see `serve`'s `lock`): the identity state and the snapshot store stay
/// consistent at every point a panic could interrupt them, and every snapshot read here is validated anyway.
fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn engine_error(message: String) -> UnsupportedReason {
    UnsupportedReason::EngineError { message, retryable: false }
}

/// The replay-backed root-range source (see the module doc): the preflop store the engine loaded once, and the snapshot
/// store and decision identity the engine shares with `engine-main`.
pub struct ReplayRanges {
    pub store: Arc<PreflopStore>,
    pub snapshots: Arc<Mutex<SnapshotStore>>,
    pub identity: Arc<Mutex<IdentityState>>,
}

impl RangeSource for ReplayRanges {
    /// The replay's public ranges of `root`'s two seats at `state` (see the module doc). A non-retryable `EngineError`
    /// when no decision is active, or the active one is not of `state`'s hand and config revision (a request superseded
    /// meanwhile: nothing of it is delivered); the replay's own `unsupported` reason when the replay has one;
    /// `InvalidRanges` for a range the validators refuse.
    fn ranges_at_root(&self, state: &HandState, root: &StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason> {
        let active = lock(&self.identity).active().cloned().ok_or_else(|| engine_error("no active decision".into()))?;
        if active.hand_id != state.hand_id || active.config_revision != state.config.config_revision {
            return Err(engine_error(format!("the active decision is not of hand {} under config revision {}", state.hand_id, state.config.config_revision)));
        }
        let (snapshots, misses) = {
            let store = lock(&self.snapshots);
            (store.for_identity(&active), store.misses_for_identity(&active))
        };
        let replayed = replay_for(&self.store, state, snapshots, misses);
        if let Some(reason) = &replayed.unsupported {
            return Err(reason.clone());
        }
        let published = |seat: Seat| replayed.ranges.get(usize::from(seat.0)).cloned().flatten().ok_or(UnsupportedReason::InvalidRanges);
        let (mut oop, mut ip) = (published(root.oop)?, published(root.ip)?);
        if !weights_valid(&oop) || !weights_valid(&ip) {
            return Err(UnsupportedReason::InvalidRanges);
        }
        block_public(&mut oop, &root.board);
        block_public(&mut ip, &root.board);
        if mass(&oop) <= 0.0 || mass(&ip) <= 0.0 || !jointly_compatible(&oop, &ip) {
            return Err(UnsupportedReason::InvalidRanges);
        }
        let ranges_used = vec![(root.oop, range_to_string(&oop), mass(&oop)), (root.ip, range_to_string(&ip), mass(&ip))];
        Ok(RootRanges { oop, ip, reasons: replayed.reasons, ranges_used, snapshots_used: replayed.snapshots_used })
    }
}

/// Spec 9.3's disclosure, in the current result's assumptions, of one snapshot a prior `street` was conditioned through
/// (fix round 1, ruling 18-I3): its origin (`live`, or one of the cache routes), the decision it was validated for (hand,
/// hand revision, config and model revision) and the street history it was solved at. Its stored reasons, the solve's
/// accuracy included, are in the result's coverage already (the walk inherits them); this note says which snapshot, and
/// so which street, they came from.
pub fn snapshot_note(street: Street, provenance: &SnapshotProvenance) -> String {
    let id = &provenance.identity_at_solve;
    format!(
        "{street:?} conditioned through the {} snapshot of decision {} (hand {}, hand revision {}, config revision {}, model revision {}), {}",
        provenance.origin, id.decision_id, id.hand_id, id.hand_revision, id.config_revision, id.model_revision, solved_clause(&provenance.solved_prefix)
    )
}

/// The `solved at`/`solved after` clause of [`snapshot_note`]'s provenance text (fix round 2, ruling 18-N2): `solved at
/// the street root` for an empty solved prefix, otherwise `solved after` a readable, comma-separated list of its
/// `(Seat, Action)` entries (`Seat 2 Check, Seat 0 Bet 50`). Neither `proto::Action` nor `Seat` derives `Display`, so
/// this formats each entry itself rather than falling back to `{:?}` (which read `[(Seat(2), Check)]`).
fn solved_clause(prefix: &[(Seat, Action)]) -> String {
    if prefix.is_empty() {
        "solved at the street root".to_string()
    } else {
        format!("solved after {}", prefix.iter().map(format_prefix_entry).collect::<Vec<_>>().join(", "))
    }
}

/// One `(Seat, Action)` entry of a solved prefix, e.g. `Seat 2 Check` or `Seat 0 Bet 50`.
fn format_prefix_entry((seat, action): &(Seat, Action)) -> String {
    let action = match action {
        Action::Fold => "Fold".to_string(),
        Action::Check => "Check".to_string(),
        Action::Call => "Call".to_string(),
        Action::Bet { to } => format!("Bet {to}"),
        Action::Raise { to } => format!("Raise {to}"),
        Action::AllIn { to } => format!("AllIn {to}"),
    };
    format!("Seat {} {}", seat.0, action)
}

/// The replay behind a decision's root ranges: `state` over `store`, with the snapshots and misses of the decision's
/// hand, config and model revision (`SnapshotStore::for_identity`, `misses_for_identity`), each validated against
/// `state` first (a store whose invalidation a contained panic interrupted is never trusted), and the engine's cause
/// for every completed street without a snapshot (see the module doc).
pub(crate) fn replay_for(store: &PreflopStore, state: &HandState, snapshots: Vec<StreetSnapshot>, misses: Vec<SnapshotMiss>) -> ReplayOutput {
    let snapshots: Vec<StreetSnapshot> = snapshots.into_iter().filter(|s| snapshot_root(state, s).is_ok()).collect();
    let misses: Vec<SnapshotMiss> = misses.into_iter().filter(|m| m.in_history(state)).collect();
    let missing = missing_causes(state, &snapshots, &misses);
    core_replay::replay(ReplayInput { cfg: &state.config, state, store, snapshots: &snapshots, missing: &missing })
}

/// Spec 9.3's concrete cause of every completed postflop street of `state` (the flop on the turn, the flop and the turn
/// on the river) with no snapshot among `snapshots`: the cause of the latest miss recorded on it, or [`NO_REQUEST`].
fn missing_causes(state: &HandState, snapshots: &[StreetSnapshot], misses: &[SnapshotMiss]) -> Vec<(Street, String)> {
    let completed: &[Street] = match state.board.len() {
        4 => &[Street::Flop],
        5 => &[Street::Flop, Street::Turn],
        _ => &[],
    };
    completed
        .iter()
        .filter(|street| !snapshots.iter().any(|s| s.key.street == **street))
        .map(|&street| {
            let latest = misses.iter().filter(|m| m.street == street).max_by_key(|m| m.identity.decision_id);
            (street, latest.map_or_else(|| NO_REQUEST.to_string(), |m| m.cause.clone()))
        })
        .collect()
}

/// Every dealt seat still in the hand but hero, with a hero-conditioned copy of its replayed public range: the
/// populations hero's actual combo meets in equity and terminal calculations (spec 2, 4.4; built after the replay, the
/// public ranges untouched). None without hero's cards.
pub fn opposing_equity_ranges(replayed: &ReplayOutput, state: &HandState) -> Vec<(Seat, Range1326)> {
    let Some(hero) = state.hero_cards else { return vec![] };
    state
        .dealt
        .iter()
        .copied()
        .filter(|&s| s != state.hero && !state.derived.folded[usize::from(s.0)])
        .filter_map(|seat| replayed.ranges[usize::from(seat.0)].as_ref().map(|r| (seat, hero_conditioned(r, hero))))
        .collect()
}

/// The snapshot of a resolved, validated solution `s` of decision `id` (spec 9.1): keyed by the identity's hand, config
/// and model revision, the root's street and board, the scaled hashes of the exact public input ranges (OOP then IP;
/// never a hero-conditioned or current-node range) and `signature`; provenanced by `id`, the root's history (the solved
/// prefix) and `origin`; holding `input.tree` (the tree the solution was validated on), the solution's nodes and
/// exploitability, `paths` (the ordinal paths of its nodes) and `reasons`.
///
/// Only for a solution that passed `proto::worker::validate_solution` against `input.tree.materialized`, the requested
/// node and actor checks, the active-identity check and the expiry check (the solve client's `validate` and its checks
/// after the terminal; plan 4's cache validation).
///
/// # Panics
/// Always, on an origin outside [`ORIGINS`], a tree rooted on another street than the root, or a `paths` that is not the
/// resolution of every node's chip path on `input.tree` by spec 2's single rule (`proto::resolve_chip_path_indexed`,
/// which `proto::resolve_chip_path` is), node by node, the wire's covered paths being the nodes' paths.
pub fn snapshot_from_solution(id: &DecisionIdentity, input: &SolveInput, s: &StreetSolution, paths: Vec<OrdinalPath>, signature: String, origin: &str,
    reasons: Vec<ApproxReason>) -> StreetSnapshot {
    assert!(ORIGINS.contains(&origin), "snapshot_from_solution: origin {origin:?} is not live, cache_exact, cache_approximate or cache_provisional");
    assert!(input.tree.root_street == input.root.street, "snapshot_from_solution: a {:?} tree for a {:?} root", input.tree.root_street, input.root.street);
    assert!(paths.len() == s.nodes.len() && s.covered_paths.len() == s.nodes.len(),
        "snapshot_from_solution: {} ordinal and {} wire covered paths for {} nodes", paths.len(), s.covered_paths.len(), s.nodes.len());
    let index = index_materialized(&input.tree.materialized);
    for (i, (node, path)) in s.nodes.iter().zip(&paths).enumerate() {
        assert!(s.covered_paths[i] == node.path, "snapshot_from_solution: wire covered path {i} is {:?}, node {i}'s path {:?}", s.covered_paths[i], node.path);
        let resolved = resolve_chip_path_indexed(&index, &node.path);
        assert!(resolved.as_ref() == Some(path), "snapshot_from_solution: covered path {i} is {path:?}, but node {i}'s chip path resolves to {resolved:?} on the input tree");
    }
    StreetSnapshot {
        key: SnapshotKey {
            hand_id: id.hand_id,
            config_revision: id.config_revision,
            model_revision: id.model_revision,
            street: input.root.street,
            root_board: input.root.board.clone(),
            root_range_hashes: [hash_scaled(&input.ranges[0]), hash_scaled(&input.ranges[1])],
            tree_signature: signature,
        },
        provenance: SnapshotProvenance { identity_at_solve: id.clone(), solved_prefix: input.root.history.clone(), origin: origin.into() },
        tree: input.tree.clone(),
        nodes: s.nodes.clone(),
        covered_paths: paths,
        exploitability_chips: s.exploitability_chips,
        reasons,
    }
}

/// Spec 9.2's one registration rule, applied by `serve` inside the accepted delivery of the engine's own `Final` and by
/// `Engine::register_snapshot` (plan 4's cache route): `SnapshotStore::register` (only the `active` decision's snapshot,
/// the one its caller read as active under the identity lock; a later snapshot of the same decision and street replaces
/// its earlier one), except that a `Provisional` never replaces a snapshot of the same decision and street that is not
/// one (the engine permits only Provisional-then-Final). Returns whether `snapshot` was stored.
pub(crate) fn register_accepted(store: &mut SnapshotStore, active: &DecisionIdentity, snapshot: StreetSnapshot) -> bool {
    let finalized = snapshot.provenance.origin == PROVISIONAL
        && store.for_hand(active.hand_id).iter().any(|s| s.provenance.identity_at_solve == *active && s.key.street == snapshot.key.street && s.provenance.origin != PROVISIONAL);
    !finalized && store.register(active, snapshot)
}

/// The miss of decision `id`, asked at `root` (its street, board and street-root history), with `cause`.
pub(crate) fn miss_for(id: &DecisionIdentity, root: &StreetRootSnapshot, cause: String) -> SnapshotMiss {
    SnapshotMiss { identity: id.clone(), street: root.street, root_board: root.board.clone(), prefix: root.history.clone(), cause }
}

/// Spec 9.3's wording of why a request registered no solution: `engine error: <message>`, `deadline exceeded` (with
/// the stage reached, when known), or `unsupported: <reason>`.
pub(crate) fn miss_cause(reason: &UnsupportedReason) -> String {
    match reason {
        UnsupportedReason::EngineError { message, .. } => format!("engine error: {message}"),
        UnsupportedReason::DeadlineExceeded { stage } if stage.is_empty() => "deadline exceeded".into(),
        UnsupportedReason::DeadlineExceeded { stage } => format!("deadline exceeded at stage {stage}"),
        other => format!("unsupported: {other:?}"),
    }
}

/// [`miss_cause`] of a `Final` the watchdog delivered in the engine's place: its `Unsupported` reason, or a deadline
/// for a payload it delivered at its fire (plan 4's retained `Provisional`).
pub(crate) fn miss_cause_of_final(rec: &Recommendation) -> String {
    match &rec.coverage {
        Coverage::Unsupported { reason, .. } => miss_cause(reason),
        Coverage::Exact | Coverage::Approximate { .. } => "deadline exceeded".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{board, hand, play, uniform_solution};
    use crate::tree::{build_tree_full, tree_signature, TemplateSelection};
    use proto::{Action, Card, Street};

    fn cards(s: &str) -> [Card; 2] {
        core_model::parse_hand(s).unwrap()
    }

    /// Three players see the flop (the small blind hero, the big blind and the cutoff); the others folded preflop.
    fn three_way_flop(hero_cards: Option<[Card; 2]>) -> HandState {
        let s = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(5), Seat(0), hero_cards);
        let s = play(&s, &[Action::Fold, Action::Fold, Action::Call, Action::Fold, Action::Call, Action::Check]);
        board(&s, "Kh 7d 2c")
    }

    fn replayed(state: &HandState) -> ReplayOutput {
        replayed_with(state, &[])
    }

    /// The replay of `state` over an empty store and no snapshot, with the engine's causes `missing`.
    fn replayed_with(state: &HandState, missing: &[(Street, String)]) -> ReplayOutput {
        let store = PreflopStore::from_sources(vec![]);
        core_replay::replay(ReplayInput { cfg: &state.config, state, store: &store, snapshots: &[], missing })
    }

    /// `opposing_equity_ranges`: every dealt seat still in the hand but hero, each with a hero-conditioned copy of its
    /// replayed public range (hero's combos removed from the copy only); none without hero's cards.
    #[test]
    fn opposing_equity_ranges_are_hero_conditioned_copies_of_every_other_seat_in_the_hand() {
        let aa = cards("AhAd");
        let state = three_way_flop(Some(aa));
        let out = replayed(&state);
        let opposing = opposing_equity_ranges(&out, &state);
        assert_eq!(opposing.iter().map(|o| o.0).collect::<Vec<_>>(), [Seat(1), Seat(4)], "the big blind and the cutoff; the folded seats and hero are not");
        let hero_combo = usize::from(proto::combo_index(aa[0], aa[1]));
        for (seat, copy) in &opposing {
            let public = out.ranges[usize::from(seat.0)].clone().unwrap();
            assert!(public.0[hero_combo] > 0.0, "hero's combo stays in the public range of seat {seat:?}");
            assert!(*copy == core_ranges::hero_conditioned(&public, aa) && copy.0[hero_combo] == 0.0, "seat {seat:?}");
        }
        assert!(opposing_equity_ranges(&out, &three_way_flop(None)).is_empty(), "no hero cards, no hero-conditioned copy");
    }

    /// The river decision of `crate::testing`'s shared hand (hero in the big blind checks it down), and its street root.
    fn river() -> (HandState, StreetRootSnapshot) {
        let s = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), Some(cards("AhAd")));
        let s = play(&s, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call]);
        let s = board(&play(&board(&play(&board(&s, "Kh 7d 2c"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d 9s");
        let root = core_model::street_root(&s).unwrap();
        (s, root)
    }

    fn source(identity: Arc<Mutex<IdentityState>>) -> ReplayRanges {
        ReplayRanges { store: Arc::new(PreflopStore::from_sources(vec![])), snapshots: Arc::new(Mutex::new(SnapshotStore::new())), identity }
    }

    /// The engine is the identity authority: with no active decision, or with the active decision of another hand or
    /// config revision, the replay range source reads no snapshot and answers a non-retryable `EngineError`; for the
    /// active decision of the state's hand it answers the replay's public ranges, whole.
    #[test]
    fn the_replay_range_source_reads_the_active_decision_of_the_states_hand_only() {
        let (state, root) = river();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let ranges = source(identity.clone());
        let refused = |r: Result<RootRanges, UnsupportedReason>| matches!(r, Err(UnsupportedReason::EngineError { retryable: false, .. }));
        assert!(refused(ranges.ranges_at_root(&state, &root)), "no active decision");
        {
            let mut ids = identity.lock().unwrap();
            ids.set_config();
            ids.begin_hand();
            ids.begin_hand();
            ids.next_decision().unwrap();
        }
        assert!(refused(ranges.ranges_at_root(&state, &root)), "the active decision is of hand 2, the state of hand 1");
        {
            let mut ids = identity.lock().unwrap();
            ids.set_config();
            ids.begin_hand();
        }
        let mut other_config = state.clone();
        other_config.hand_id = 3;
        identity.lock().unwrap().next_decision().unwrap();
        assert!(refused(ranges.ranges_at_root(&other_config, &root)), "the hand began under config revision 2, the state carries 1");
        let fresh = Arc::new(Mutex::new(IdentityState::new()));
        {
            let mut ids = fresh.lock().unwrap();
            ids.set_config();
            ids.begin_hand();
            ids.next_decision().unwrap();
        }
        let answered = source(fresh).ranges_at_root(&state, &root).expect("the active decision of the state's hand");
        // Neither completed street has a snapshot or a recorded miss: the engine's cause for both is "no request".
        let out = replayed_with(&state, &[(Street::Flop, "no request".into()), (Street::Turn, "no request".into())]);
        assert!(answered.oop == out.ranges[usize::from(root.oop.0)].clone().unwrap() && answered.ip == out.ranges[usize::from(root.ip.0)].clone().unwrap());
        assert_eq!(answered.reasons, out.reasons);
        assert!(answered.reasons.contains(&ApproxReason::UnconditionedPriorStreet { street: Street::Turn, seat: root.ip, cause: "no request".into() }), "{:?}", answered.reasons);
    }

    /// Brief decision 4: `engine-main` uses a poisoned lock as it stands after a contained panic. The replay range source
    /// keeps no state of its own (its store, snapshot store and identity handles never change), so poisoned identity and
    /// snapshot locks change nothing it answers.
    #[test]
    fn poisoned_identity_and_snapshot_locks_change_nothing_the_replay_range_source_answers() {
        let (state, root) = river();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        {
            let mut ids = identity.lock().unwrap();
            ids.set_config();
            ids.begin_hand();
            ids.next_decision().unwrap();
        }
        let ranges = source(identity.clone());
        let before = ranges.ranges_at_root(&state, &root).unwrap();
        let (i, s) = (identity.clone(), ranges.snapshots.clone());
        let _ = std::thread::spawn(move || {
            let _held = (i.lock().unwrap(), s.lock().unwrap());
            panic!("a contained engine panic (deliberate)");
        })
        .join();
        assert!(identity.is_poisoned() && ranges.snapshots.is_poisoned());
        let after = ranges.ranges_at_root(&state, &root).unwrap();
        assert!(after.oop == before.oop && after.ip == before.ip && after.reasons == before.reasons && after.ranges_used == before.ranges_used);
    }

    /// The solve input and the solution of the river root above: its public ranges, its tree, every node exported.
    fn solve_of(state: &HandState, root: &StreetRootSnapshot) -> (SolveInput, StreetSolution, Vec<OrdinalPath>, String) {
        let b = build_tree_full(root, &TemplateSelection::from_history("river_std_v1", &root.history)).unwrap();
        let sol = uniform_solution(&b.tree, &b.history, 0.2);
        let paths = proto::worker::validate_solution(&sol, &b.tree.materialized).unwrap();
        let out = replayed(state);
        let ranges = [out.ranges[usize::from(root.oop.0)].clone().unwrap(), out.ranges[usize::from(root.ip.0)].clone().unwrap()];
        let signature = tree_signature(&b.tree, b.pot);
        (SolveInput { root: root.clone(), ranges, tree: b.tree, target_bp: 50 }, sol, paths, signature)
    }

    fn identity() -> DecisionIdentity {
        DecisionIdentity { hand_id: 1, hand_revision: 4, decision_id: 7, config_revision: 1, model_revision: 0 }
    }

    /// `snapshot_from_solution`: the key is the identity's hand, config and model revision, the root's street and board,
    /// the hashes of the exact public input ranges (OOP then IP) and the signature given; the provenance is the identity,
    /// the root's history and the origin; the tree is the input's, the nodes and exploitability the solution's, the covered
    /// paths the ordinal paths given, and the reasons the ones given.
    #[test]
    fn a_snapshot_is_built_from_the_input_the_solution_and_its_resolved_paths() {
        let (state, root) = river();
        let (input, sol, paths, signature) = solve_of(&state, &root);
        let reasons = vec![ApproxReason::UnconditionedCurrentStreet];
        let s = snapshot_from_solution(&identity(), &input, &sol, paths.clone(), signature.clone(), "cache_approximate", reasons.clone());
        let id = identity();
        assert_eq!(s.key, SnapshotKey { hand_id: 1, config_revision: 1, model_revision: 0, street: Street::River, root_board: root.board.clone(),
            root_range_hashes: [core_ranges::hash_scaled(&input.ranges[0]), core_ranges::hash_scaled(&input.ranges[1])], tree_signature: signature });
        assert_eq!(s.provenance, SnapshotProvenance { identity_at_solve: id, solved_prefix: root.history.clone(), origin: "cache_approximate".into() });
        assert!(s.tree == input.tree && s.nodes == sol.nodes && s.covered_paths == paths && s.exploitability_chips == 0.2 && s.reasons == reasons);
    }

    #[test]
    #[should_panic(expected = "origin \"replayed\" is not live, cache_exact, cache_approximate or cache_provisional")]
    fn a_snapshot_origin_outside_the_four_is_a_bug() {
        let (state, root) = river();
        let (input, sol, paths, signature) = solve_of(&state, &root);
        let _ = snapshot_from_solution(&identity(), &input, &sol, paths, signature, "replayed", vec![]);
    }

    #[test]
    #[should_panic(expected = "covered path 1 is [], but node 1's chip path resolves to")]
    fn a_covered_path_that_is_not_its_nodes_resolution_is_a_bug() {
        let (state, root) = river();
        let (input, sol, mut paths, signature) = solve_of(&state, &root);
        assert!(paths[0].is_empty() && !paths[1].is_empty(), "the root is exported first");
        paths[1] = paths[0].clone();
        let _ = snapshot_from_solution(&identity(), &input, &sol, paths, signature, "live", vec![]);
    }

    /// `snapshot_note`'s `solved at`/`solved after` clause (fix round 2, ruling 18-N2): the street root for an empty
    /// solved prefix, otherwise a readable, comma-separated `Seat <n> <Action>` list, never `proto::Action`/`Seat`'s
    /// `Debug` output.
    #[test]
    fn snapshot_note_names_the_solved_prefix_readably() {
        let id = identity();
        let at_root = SnapshotProvenance { identity_at_solve: id.clone(), solved_prefix: vec![], origin: "live".into() };
        let note = snapshot_note(Street::Turn, &at_root);
        assert!(note.ends_with("solved at the street root"), "{note:?}");
        let mixed = SnapshotProvenance {
            identity_at_solve: id,
            solved_prefix: vec![(Seat(2), Action::Check), (Seat(0), Action::Bet { to: 50 })],
            origin: "live".into(),
        };
        let note = snapshot_note(Street::Turn, &mixed);
        assert!(note.ends_with("solved after Seat 2 Check, Seat 0 Bet 50"), "{note:?}");
    }
}
