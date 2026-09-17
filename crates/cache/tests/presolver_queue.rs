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
