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
//! query performs no I/O: `cache::Cache::lookup` runs it on the request's own `fast-path` work.

use cache::entry::SourceInputs;
use cache::key::{spr_bucket, KeyFields, Model, RakeKey, Rational};
use cache::lookup::CacheQuery;
use core_iso::SuitPerm;
use proto::{ApproxReason, Derived, Rake, SolveInput, Street, UnsupportedReason};

fn unsupported(message: &str) -> UnsupportedReason {
    UnsupportedReason::EngineError { message: message.into(), retryable: false }
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
