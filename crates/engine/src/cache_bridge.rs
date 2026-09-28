//! Plan 4 Task 7: the engine's side of the flop/turn cache (spec section 10.4) -- the shared
//! reference state (`key_and_source`) and the lookup query (`make_cache_query`) built from one
//! `SolveInput` at its street root.
//!
//! Only public street-root information enters a key: the canonical board, the two public ranges
//! (`SolveInput.ranges`, the replay's street-root ranges, blocked by the board here), the exact
//! SPR bucket, the tree signature, the normalized rake and the solver/adapter/rules versions.
//! Hero's cards never reach this module (a `SolveInput` has none), and hero's seat, `bb_chips`,
//! `target_bp` and the requested path travel on the query beside the key, never inside it. A
//! degenerate reference state is an `Unsupported{EngineError}` answer, never a panic. Building a
//! query performs no I/O. `serve` runs `cache::Cache::lookup` on it on `engine-main`, inside the
//! request's own budget: the shared 500 ms cache budget of spec 7 bounds it (plan 4 Task 10,
//! ruling 10-pre1), and the watchdog never calls it.
//!
//! Plan 4 Task 10 adds the other half: `canonical_perm`, the one producer of the suit permutation
//! for queries and stored entries alike (ruling 7-Q2/7-D6), and `entry_from_solution`, the
//! validated entry a live flop or turn solve stores.

use cache::entry::SourceInputs;
use cache::key::{spr_bucket, KeyFields, Model, RakeKey, Rational};
use cache::lookup::CacheQuery;
use core_iso::SuitPerm;
use proto::{Action, ApproxReason, Card, Derived, Rake, Range1326, SolveInput, Street, UnsupportedReason};

fn unsupported(message: &str) -> UnsupportedReason {
    UnsupportedReason::EngineError { message: message.into(), retryable: false }
}

/// Ruling 7-Q2/7-D6: the suit permutation of a street root's cache key, `core_iso::canonicalize`
/// of the board and both public ranges blocked by that board (OOP then IP). The one place it is
/// computed: `serve`'s queries and `entry_from_solution`'s entries both take it from here, and
/// `cache::entry::validate_entry` re-checks the fixed point it produces. The ranges are public
/// (hero's cards never enter them); blocking an already-blocked range changes nothing.
pub fn canonical_perm(board: &[Card], oop: &Range1326, ip: &Range1326) -> SuitPerm {
    let (mut oop, mut ip) = (oop.clone(), ip.clone());
    core_ranges::block_public(&mut oop, board);
    core_ranges::block_public(&mut ip, board);
    core_iso::canonicalize(board, &[&oop, &ip]).1
}

/// Plan 4 Task 10 Step 4b: the validated `CacheEntry` of a flop or turn solve (spec 10.4 payload).
/// `input` is the input actually solved: its tree is the answering attempt's (the `_min` retry's
/// when it answered), never the first attempt's template. `solution` is the validated terminal
/// solution of that tree; its combo rows are moved into canonical suits by `perm` (the same row
/// mover the lookup inverts), then `cache::entry::normalize` turns EV into `ev_over_P` and every
/// chip path into its ordinal path. The raw accuracy is `exploitability_chips / P`, never rounded;
/// `reasons` are the reasons the request inherited (never the solve's own `DeadlineBestSoFar`,
/// which certifies nothing about a later request's timing, spec 10.4); `target_bp` and
/// `elapsed_ms` are the solve's. The key is `key_and_source`'s, so the entry and a query of the
/// same decision can never disagree on it.
///
/// # Errors
/// Every `key_and_source` error (a river root among them: river solutions are never cached), and
/// a solution that does not normalize against `input.tree` or whose entry `validate_entry`
/// refuses: `Unsupported{EngineError{retryable: false}}`. A refused entry is never stored.
#[allow(clippy::too_many_arguments)]
pub fn entry_from_solution(
    input: &SolveInput,
    solution: &proto::worker::StreetSolution,
    reasons: &[ApproxReason],
    bb_chips: u32,
    rake: &Rake,
    signature: &str,
    perm: &SuitPerm,
    elapsed_ms: u32,
    target_bp: u16,
) -> Result<cache::entry::CacheEntry, UnsupportedReason> {
    let (key, source) = key_and_source(input, bb_chips, rake, signature, perm)?;
    let pot = source.pot;
    let mut canonical = solution.clone();
    for node in &mut canonical.nodes {
        if node.probs.len() != proto::COMBOS || node.ev_chips.len() != proto::COMBOS || node.available.len() != proto::COMBOS {
            return Err(unsupported("cache normalize: a node without its 1326 rows"));
        }
        node.probs = cache::lookup::map_rows(&node.probs, perm);
        node.ev_chips = cache::lookup::map_rows(&node.ev_chips, perm);
        node.available = cache::lookup::map_flags(&node.available, perm);
    }
    let nodes = cache::entry::normalize(&canonical, &input.tree, pot).map_err(|e| unsupported(&format!("cache normalize: {e}")))?;
    let fractions = input
        .tree
        .materialized
        .iter()
        .map(|n| {
            n.actions
                .iter()
                .map(|a| match a {
                    Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => Rational::new(u64::from(*to), u64::from(pot)).ok(),
                    Action::Fold | Action::Check | Action::Call => None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let entry = cache::entry::CacheEntry {
        key,
        source,
        tree: input.tree.clone(),
        fractions,
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        exploitability_over_P: f64::from(solution.exploitability_chips) / f64::from(pot),
        target_bp,
        iterations: solution.iterations,
        elapsed_ms,
        memory_bytes: solution.memory_bytes,
        mode: solution.mode.clone(),
        locks_applied: solution.locks_applied,
        export: solution.export.clone(),
        reasons: reasons.to_vec(),
        created: now,
        last_hit: now,
    };
    cache::entry::validate_entry(&entry).map_err(|e| unsupported(&format!("cache entry: {e}")))?;
    Ok(entry)
}

/// The shared §10.4 reference state: key fields and exact source inputs for one solve input at
/// its street root. `signature` is Plan 2's `engine::tree::tree_signature(&input.tree, pot)`;
/// this function never hashes raw materialized chips. `perm` is the permutation returned by the
/// joint canonicalization of the board and both public ranges (`core_iso::canonicalize(board,
/// [oop, ip])`, the ranges blocked by the board). Task 10's `entry_from_solution` calls the same
/// helper, so a stored entry and a query can never disagree on the key.
///
/// # Errors
/// `Unsupported{EngineError{retryable: false}}` for a street other than flop or turn, a pot that
/// overflows `u32`, a zero pot, effective stack or big blind, and a rake the key cannot carry.
pub fn key_and_source(input: &SolveInput, bb_chips: u32, rake: &Rake, signature: &str, perm: &SuitPerm) -> Result<(KeyFields, SourceInputs), UnsupportedReason> {
    let root = &input.root;
    if !matches!(root.street, Street::Flop | Street::Turn) {
        return Err(unsupported("cache covers flop and turn only"));
    }
    let pot = root.pot_root.checked_add(root.dead_this_street).ok_or_else(|| unsupported("pot overflow"))?;
    let eff = root.stack_oop_root.min(root.stack_ip_root);
    if pot == 0 || eff == 0 || bb_chips == 0 {
        return Err(unsupported("degenerate cache reference state"));
    }
    let spr = Rational::new(eff as u64, pot as u64).map_err(|_| unsupported("spr"))?;
    let (rate, cap_mchips, collection_rule_version) = match rake {
        Rake::TimeCharge => (0.0_f32, 0, 1),
        Rake::PotRake { rate, cap_mchips, .. } => (*rate, *cap_mchips, 1),
    };
    let cap_over_p = Rational::new(cap_mchips as u64, 1000 * pot as u64).map_err(|_| unsupported("cap"))?;
    let rake_key = RakeKey::new(rate, cap_over_p, collection_rule_version).map_err(|_| unsupported("rake rate"))?;
    let mut ranges = [input.ranges[0].clone(), input.ranges[1].clone()];
    for r in &mut ranges {
        core_ranges::block_public(r, &root.board);
    }
    let mut canonical_board = root.board.iter().map(|c| core_iso::apply(perm, *c)).collect::<Vec<_>>();
    canonical_board[..3].sort_by_key(|c| c.0);
    let mapped = [core_iso::apply_range(perm, &ranges[0]), core_iso::apply_range(perm, &ranges[1])];
    let key = KeyFields {
        schema_version: 3,
        solver_commit: proto::worker::SOLVER_COMMIT.into(),
        adapter_version: proto::worker::ADAPTER_VERSION,
        rules_version: 3,
        canonical_board,
        root_street: root.street,
        spr_bucket: spr_bucket(spr),
        tree_signature: signature.into(),
        rake: rake_key,
        range_hash_oop: core_ranges::hash_scaled(&mapped[0]),
        range_hash_ip: core_ranges::hash_scaled(&mapped[1]),
        model: Model::Baseline,
    };
    let source = SourceInputs {
        pot,
        stack_oop: root.stack_oop_root,
        stack_ip: root.stack_ip_root,
        spr,
        bb_chips,
        quantum_over_p: Rational::new(1, pot as u64).map_err(|_| unsupported("quantum"))?,
        cap_mchips,
        ranges: mapped,
    };
    Ok((key, source))
}

/// Builds the §10.4 query. The query actor comes from the OOP/IP seat mapping and `Derived.to_act`;
/// the requested node is the street root's history resolved in the query tree. Hero, `bb_chips`,
/// `target_bp` and the requested path never enter `KeyFields`. `budget` is the request's remaining
/// absolute budget (`Cache::lookup` waits at most `min(budget, 500 ms)`).
///
/// # Errors
/// Every `key_and_source` error, a history that does not resolve in `input.tree`, and a `to_act`
/// that is neither street-root seat: `Unsupported{EngineError{retryable: false}}`.
#[allow(clippy::too_many_arguments)]
pub fn make_cache_query(
    input: &SolveInput,
    derived: &Derived,
    bb_chips: u32,
    rake: &Rake,
    reasons: &[ApproxReason],
    signature: &str,
    perm: &SuitPerm,
    target_bp: u16,
    budget: std::time::Duration,
) -> Result<CacheQuery, UnsupportedReason> {
    let (key, source) = key_and_source(input, bb_chips, rake, signature, perm)?;
    let root = &input.root;
    let history = root.history.iter().map(|(_, a)| *a).collect::<Vec<_>>();
    let requested = proto::resolve_chip_path(&input.tree.materialized, &history).ok_or_else(|| unsupported("requested path is not in the query tree"))?;
    let actor = if derived.to_act == Some(root.oop) {
        "oop"
    } else if derived.to_act == Some(root.ip) {
        "ip"
    } else {
        return Err(unsupported("actor is neither street-root seat"));
    };
    Ok(CacheQuery {
        key,
        source,
        tree: input.tree.clone(),
        requested,
        actor: actor.into(),
        legal: derived.legal.clone(),
        target_bp,
        reasons: reasons.to_vec(),
        inverse_perm: core_iso::inverse(perm),
        budget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{build_tree_full, tree_signature, TemplateSelection};
    use proto::{Action, Card, Derived, LegalAction, Rake, Seat, SolveInput, Street, StreetRootSnapshot, UnsupportedReason};
    use std::time::Duration;

    const OOP: Seat = Seat(2);
    const IP: Seat = Seat(0);

    fn cards(s: &str) -> Vec<Card> {
        s.split(' ').map(|c| Card::parse(c).unwrap()).collect()
    }

    /// The flop street root of a single-raised pot on a monotone-free board: `oop` (seat 2) and
    /// `ip` (seat 0), pot 100 plus 10 dead, stacks 500/700 -- and `history` from the root.
    fn root(history: Vec<(Seat, Action)>) -> StreetRootSnapshot {
        StreetRootSnapshot { street: Street::Flop, board: cards("Kh 7d 2c"), oop: OOP, ip: IP, pot_root: 100, stack_oop_root: 500, stack_ip_root: 700, dead_this_street: 10,
            projected_from: 2, history, bb_chips: 2 }
    }

    /// Public street-root ranges as the replay produces them (spec 9.1): not yet blocked by the board.
    fn ranges() -> [proto::Range1326; 2] {
        [core_ranges::parse_range("AA,KK,QQ,AKs,K7s").unwrap(), core_ranges::parse_range("JJ,TT,AQs,KQs,72s").unwrap()]
    }

    /// The solve input at `root`, the signature the engine computes for its tree, and the suit
    /// permutation of the joint canonicalization of the board and the blocked public ranges.
    fn input(root: StreetRootSnapshot, target_bp: u16) -> (SolveInput, String, core_iso::SuitPerm) {
        let b = build_tree_full(&root, &TemplateSelection::from_history("flop_fast_v1", &root.history)).unwrap();
        let signature = tree_signature(&b.tree, b.pot);
        let mut blocked = ranges();
        for r in &mut blocked {
            core_ranges::block_public(r, &root.board);
        }
        let (_, perm) = core_iso::canonicalize(&root.board, &[&blocked[0], &blocked[1]]);
        (SolveInput { root, ranges: ranges(), tree: b.tree, target_bp }, signature, perm)
    }

    fn derived(to_act: Option<Seat>, legal: Vec<LegalAction>) -> Derived {
        Derived { street: Street::Flop, to_act, legal, ..Derived::default() }
    }

    fn rake() -> Rake {
        Rake::PotRake { rate: 0.05, cap_mchips: 3000, no_flop_no_drop: true }
    }

    fn query(input: &SolveInput, signature: &str, perm: &core_iso::SuitPerm, to_act: Seat, bb_chips: u32, target_bp: u16) -> Result<cache::lookup::CacheQuery, UnsupportedReason> {
        let legal = vec![LegalAction::Check, LegalAction::Bet { min_to: 2, max_to: 500 }, LegalAction::AllIn { to: 500 }];
        make_cache_query(input, &derived(Some(to_act), legal), bb_chips, &rake(), &[], signature, perm, target_bp, Duration::from_millis(300))
    }

    fn engine_error(r: Result<cache::lookup::CacheQuery, UnsupportedReason>) -> String {
        match r {
            Err(UnsupportedReason::EngineError { message, retryable: false }) => message,
            Err(other) => panic!("expected a non-retryable EngineError, got {other:?}"),
            Ok(_) => panic!("expected a non-retryable EngineError, got a query"),
        }
    }

    /// Spec 10.4 / plan 4 Task 7 Step 5: hero (which seat acts, and which seats the street root's
    /// two players sit in), `bb_chips`, `target_bp` and the requested path never enter
    /// `KeyFields`: every one of these queries shares one key digest, while the query itself
    /// carries each difference.
    #[test]
    fn hero_bb_chips_target_bp_and_the_requested_path_never_enter_the_key() {
        let (base, signature, perm) = input(root(vec![]), 50);
        let q = query(&base, &signature, &perm, OOP, 2, 50).unwrap();
        let digest = q.key.digest();
        assert_eq!((q.requested.clone(), q.actor.as_str(), q.target_bp, q.source.bb_chips), (vec![], "oop", 50, 2));

        let bigger_blind = query(&base, &signature, &perm, OOP, 5, 50).unwrap();
        assert_eq!(bigger_blind.key.digest(), digest, "bb_chips");
        assert_eq!(bigger_blind.source.bb_chips, 5);

        let (tighter, signature_t, perm_t) = input(root(vec![]), 30);
        let tighter = query(&tighter, &signature_t, &perm_t, OOP, 2, 30).unwrap();
        assert_eq!(tighter.key.digest(), digest, "target_bp");
        assert_eq!(tighter.target_bp, 30);

        let (checked, signature_c, perm_c) = input(root(vec![(OOP, Action::Check)]), 50);
        let after_check = query(&checked, &signature_c, &perm_c, IP, 2, 50).unwrap();
        assert_eq!(after_check.key.digest(), digest, "the requested path and the acting seat");
        assert_eq!((after_check.requested.clone(), after_check.actor.as_str()), (vec![0], "ip"));

        let mut reseated = root(vec![]);
        (reseated.oop, reseated.ip) = (Seat(4), Seat(1));
        let (reseated, signature_r, perm_r) = input(reseated, 50);
        let hero_elsewhere = query(&reseated, &signature_r, &perm_r, Seat(4), 2, 50).unwrap();
        assert_eq!(hero_elsewhere.key.digest(), digest, "the seats the two players sit in");
        assert_eq!(hero_elsewhere.actor, "oop");
    }

    /// The key and source are the §10.4 reference state of the street root: pot with the dead
    /// money, exact SPR, the canonical board (flop sorted) and both blocked public ranges under the
    /// joint canonicalization, which is its own fixed point exactly as `validate_entry` requires of
    /// a stored entry; `TimeCharge` is the unraked `0/0` rake key.
    #[test]
    fn key_and_source_are_the_canonical_reference_state_of_the_street_root() {
        let (input, signature, perm) = input(root(vec![]), 50);
        let (key, source) = key_and_source(&input, 2, &rake(), &signature, &perm).unwrap();
        assert_eq!((source.pot, source.stack_oop, source.stack_ip, source.bb_chips, source.cap_mchips), (110, 500, 700, 2, 3000));
        assert_eq!(source.spr, cache::key::Rational::new(500, 110).unwrap());
        assert_eq!(source.quantum_over_p, cache::key::Rational::new(1, 110).unwrap());
        assert_eq!(key.spr_bucket, cache::key::spr_bucket(source.spr));
        assert_eq!((key.schema_version, key.rules_version, key.adapter_version), (3, 3, proto::worker::ADAPTER_VERSION));
        assert_eq!(key.solver_commit, proto::worker::SOLVER_COMMIT);
        assert_eq!((key.root_street, key.tree_signature.as_str(), &key.model), (Street::Flop, signature.as_str(), &cache::key::Model::Baseline));
        assert_eq!(key.rake, cache::key::RakeKey::new(0.05, cache::key::Rational::new(3000, 110_000).unwrap(), 1).unwrap());

        let mut blocked = ranges();
        for r in &mut blocked {
            core_ranges::block_public(r, &input.root.board);
        }
        let mapped = [core_iso::apply_range(&perm, &blocked[0]), core_iso::apply_range(&perm, &blocked[1])];
        assert!(source.ranges[0] == mapped[0] && source.ranges[1] == mapped[1], "blocked by the board, then canonicalized");
        assert_eq!((key.range_hash_oop, key.range_hash_ip), (core_ranges::hash_scaled(&mapped[0]), core_ranges::hash_scaled(&mapped[1])));
        let (canonical, _) = core_iso::canonicalize(&input.root.board, &[&blocked[0], &blocked[1]]);
        assert_eq!(key.canonical_board, canonical.cards());

        // The stored-entry identity check of `cache::entry::validate_entry`, on this key and source.
        let (_, again) = core_iso::canonicalize(&key.canonical_board, &[&source.ranges[0], &source.ranges[1]]);
        let mut board = key.canonical_board.iter().map(|c| core_iso::apply(&again, *c)).collect::<Vec<_>>();
        board[..3].sort_by_key(|c| c.0);
        assert_eq!(board, key.canonical_board);
        for r in &source.ranges {
            let fixed = core_iso::apply_range(&again, r);
            assert!(fixed.0.iter().zip(r.0.iter()).all(|(x, y)| x.to_bits() == y.to_bits()), "the canonical ranges are their own fixed point");
        }

        let (unraked, _) = key_and_source(&input, 2, &Rake::TimeCharge, &signature, &perm).unwrap();
        assert_eq!(unraked.rake, cache::key::RakeKey::new(0.0, cache::key::Rational::new(0, 1).unwrap(), 1).unwrap());
        let q = query(&input, &signature, &perm, OOP, 2, 50).unwrap();
        assert_eq!((q.inverse_perm, q.budget, q.tree.clone()), (core_iso::inverse(&perm), Duration::from_millis(300), input.tree.clone()));
        assert_eq!(q.legal, vec![LegalAction::Check, LegalAction::Bet { min_to: 2, max_to: 500 }, LegalAction::AllIn { to: 500 }]);
    }

    /// Degenerate reference states are `Unsupported{EngineError{retryable: false}}`, never a panic
    /// (a zero SPR would otherwise reach `spr_bucket`'s always-on assertion).
    #[test]
    fn degenerate_reference_states_are_non_retryable_engine_errors() {
        let (input, signature, perm) = input(root(vec![]), 50);
        let with = |edit: &dyn Fn(&mut SolveInput)| {
            let mut i = input.clone();
            edit(&mut i);
            query(&i, &signature, &perm, OOP, 2, 50)
        };
        assert!(engine_error(with(&|i| (i.root.pot_root, i.root.dead_this_street) = (0, 0))).contains("degenerate"));
        assert!(engine_error(with(&|i| i.root.stack_oop_root = 0)).contains("degenerate"));
        assert!(engine_error(with(&|i| i.root.stack_ip_root = 0)).contains("degenerate"));
        assert!(engine_error(with(&|i| (i.root.pot_root, i.root.dead_this_street) = (u32::MAX, 1))).contains("overflow"));
        assert!(engine_error(with(&|i| i.root.street = Street::River)).contains("flop and turn"));
        assert!(engine_error(with(&|i| i.root.street = Street::Preflop)).contains("flop and turn"));
        assert!(engine_error(query(&input, &signature, &perm, OOP, 0, 50)).contains("degenerate"), "a zero big blind");
    }

    /// The query actor is the street-root seat that `Derived.to_act` names; anything else, and a
    /// history that does not resolve in the query tree, is a non-retryable engine error.
    #[test]
    fn the_actor_comes_from_the_street_root_seats_and_the_path_must_resolve() {
        let (input, signature, perm) = input(root(vec![]), 50);
        assert_eq!(query(&input, &signature, &perm, OOP, 2, 50).unwrap().actor, "oop");
        assert_eq!(query(&input, &signature, &perm, IP, 2, 50).unwrap().actor, "ip");
        assert!(engine_error(query(&input, &signature, &perm, Seat(5), 2, 50)).contains("street-root seat"));
        let nobody = make_cache_query(&input, &derived(None, vec![]), 2, &rake(), &[], &signature, &perm, 50, Duration::ZERO);
        assert!(engine_error(nobody).contains("street-root seat"));
        let mut off_tree = input.clone();
        off_tree.root.history = vec![(OOP, Action::Bet { to: 37 })];
        assert!(engine_error(query(&off_tree, &signature, &perm, IP, 2, 50)).contains("requested path"));
    }

    // ===================== plan 4 Task 10: the stored entry =====================

    /// A validated solution of `input`'s tree at `expl` chips whose rows differ by combo (so a suit mapping that moved
    /// a row to the wrong combo is observable): at node `n`, combo `c` bets/raises its first action with a probability
    /// that depends on `c`, and every non-fold EV names the node, the action and the combo; fold stays exactly 0.
    fn varied_solution(input: &SolveInput, expl: f32) -> proto::worker::StreetSolution {
        let mut sol = crate::testing::uniform_solution(&input.tree, &[], expl);
        for (n, node) in sol.nodes.iter_mut().enumerate() {
            let width = node.actions.len();
            for c in 0..proto::COMBOS {
                let first = (c % 7 + 1) as f32 / 8.0;
                let rest = if width > 1 { (1.0 - first) / (width - 1) as f32 } else { 0.0 };
                node.probs[c] = (0..width).map(|a| if width == 1 { 1.0 } else if a == 0 { first } else { rest }).collect();
                node.ev_chips[c] = node.actions.iter().enumerate().map(|(a, action)| if *action == Action::Fold { 0.0 } else { (n * 100 + a * 10) as f32 + (c % 5) as f32 }).collect();
            }
        }
        proto::worker::validate_solution(&sol, &input.tree.materialized).expect("the varied solution is valid");
        sol
    }

    /// Ruling 7-Q2/7-D6: `canonical_perm` is the one producer of the permutation, `core_iso::canonicalize` of the board
    /// and both public ranges blocked by the board (never the unblocked ones), for the query and the stored entry alike.
    #[test]
    fn canonical_perm_canonicalizes_the_board_blocked_public_ranges() {
        let (input, _, perm) = input(root(vec![]), 50);
        assert_eq!(canonical_perm(&input.root.board, &input.ranges[0], &input.ranges[1]), perm);
        let mut blocked = input.ranges.clone();
        for r in &mut blocked {
            core_ranges::block_public(r, &input.root.board);
        }
        assert_eq!(canonical_perm(&input.root.board, &blocked[0], &blocked[1]), perm, "blocking twice changes nothing");
    }

    /// Plan 4 Task 10 Step 4b: the stored entry of a solve is keyed exactly as the query of the same decision
    /// (`key_and_source` over the shared `canonical_perm`), carries the raw `exploitability_chips / P`, the solve's
    /// target, reasons, mode and elapsed time, and its rows are forward-mapped into canonical suits: the production
    /// lookup's reconstruction of the requested node in the query's suits gives back the solve's own rows.
    #[test]
    fn an_entry_from_a_solution_serves_back_the_solves_own_rows_in_the_querys_suits() {
        // The heart deuce is the lowest flop card: canonical suits move hearts to clubs and clubs to hearts, so a row left
        // in the query's suits would be served to another combo.
        let (input, signature, perm) = input(StreetRootSnapshot { board: cards("Kc 7d 2h"), ..root(vec![]) }, 50);
        assert_ne!(perm, core_iso::SuitPerm::IDENTITY, "the board is not its own canonical form");
        let sol = varied_solution(&input, 1.9);
        let reasons = vec![proto::ApproxReason::ChartRounded];
        let entry = entry_from_solution(&input, &sol, &reasons, 2, &rake(), &signature, &perm, 1234, 50).expect("a valid flop solution is storable");
        let (key, source) = key_and_source(&input, 2, &rake(), &signature, &perm).unwrap();
        assert_eq!(entry.key.digest(), key.digest());
        assert_eq!(entry.source.pot, source.pot);
        assert_eq!(entry.exploitability_over_P, f64::from(1.9f32) / 110.0, "raw, never rounded");
        assert_eq!((entry.target_bp, entry.elapsed_ms, entry.reasons.clone(), entry.mode.as_str(), entry.export.as_str()), (50, 1234, reasons, "f32", "street"));
        assert_eq!(entry.tree, input.tree, "the tree actually solved");
        cache::entry::validate_entry(&entry).expect("the entry validates");
        let q = query(&input, &signature, &perm, OOP, 2, 50).unwrap();
        let served = cache::lookup::reconstruct(&entry, &q).expect("the query serves the entry");
        assert_eq!(served.solution.nodes.len(), sol.nodes.len());
        for (k, (got, want)) in served.solution.nodes.iter().zip(&sol.nodes).enumerate() {
            assert_eq!((&got.path, &got.actions, &got.probs, &got.available), (&want.path, &want.actions, &want.probs, &want.available), "node {k}");
            for c in 0..proto::COMBOS {
                for (x, y) in got.ev_chips[c].iter().zip(&want.ev_chips[c]) {
                    assert!((x - y).abs() <= 1e-3 * y.abs().max(1.0), "node {k} combo {c}: EV {x} served for {y}");
                }
            }
        }
    }

    /// A solution the cache cannot keep is refused with a non-retryable engine error, never stored: a river root (river
    /// solutions are never cached), a solution of another tree than the one given, and one exporting a node of a later
    /// street (spec 2: exports are the current street's decision nodes), which `validate_entry` refuses although the
    /// solve client's own validation accepts it.
    #[test]
    fn an_unstorable_solution_is_refused_with_an_engine_error() {
        let (input, signature, perm) = input(root(vec![]), 50);
        let sol = varied_solution(&input, 0.4);
        let refused = |r: Result<cache::entry::CacheEntry, UnsupportedReason>| match r {
            Err(UnsupportedReason::EngineError { message, retryable: false }) => message,
            Err(other) => panic!("expected a non-retryable EngineError, got {other:?}"),
            Ok(_) => panic!("expected a refusal, got an entry"),
        };
        let mut river = input.clone();
        river.root.street = Street::River;
        assert!(refused(entry_from_solution(&river, &sol, &[], 2, &rake(), &signature, &perm, 1, 50)).contains("flop and turn"));
        let other_tree = build_tree_full(&input.root, &TemplateSelection::from_history("flop_min_v1", &[])).unwrap().tree;
        let wrong = SolveInput { tree: other_tree, ..input.clone() };
        assert!(refused(entry_from_solution(&wrong, &sol, &[], 2, &rake(), &signature, &perm, 1, 50)).contains("cache"));
        let mut later_street = sol.clone();
        let turn_node = input.tree.materialized.iter().find(|m| m.street == Street::Turn).expect("a turn node").clone();
        let mut exported = later_street.nodes[0].clone();
        exported.path = cache::entry::chip_path(&input.tree.materialized, &turn_node.path).unwrap();
        exported.actor = turn_node.actor.clone();
        exported.actions = turn_node.actions.clone();
        let width = exported.actions.len();
        exported.probs = vec![vec![1.0 / width as f32; width]; proto::COMBOS];
        exported.ev_chips = vec![exported.actions.iter().map(|a| if *a == Action::Fold { 0.0 } else { 1.0 }).collect(); proto::COMBOS];
        later_street.covered_paths.push(exported.path.clone());
        later_street.nodes.push(exported);
        proto::worker::validate_solution(&later_street, &input.tree.materialized).expect("the solve client's validation accepts a later-street node");
        assert!(refused(entry_from_solution(&input, &later_street, &[], 2, &rake(), &signature, &perm, 1, 50)).contains("cache entry"));
    }

    /// The reasons the request already incurred travel with the query, and the key never depends
    /// on them.
    #[test]
    fn the_querys_reasons_travel_with_it_and_stay_out_of_the_key() {
        let (input, signature, perm) = input(root(vec![]), 50);
        let reasons = [proto::ApproxReason::EvReferenceUnverified];
        let legal = vec![LegalAction::Check];
        let q = make_cache_query(&input, &derived(Some(OOP), legal), 2, &rake(), &reasons, &signature, &perm, 50, Duration::ZERO).unwrap();
        assert_eq!(q.reasons, reasons.to_vec());
        assert_eq!(q.key.digest(), query(&input, &signature, &perm, OOP, 2, 50).unwrap().key.digest());
    }
}
