#[test]
fn tier_scenarios_and_flop_order_are_complete() {
    use cache::presolver::scenarios::*;
    let s = scenarios();
    assert_eq!(s.len(), 24);
    assert_eq!(s.iter().filter(|x| x.tier == 1).count(), 4);
    assert_eq!(s.iter().filter(|x| x.tier == 2).count(), 8);
    assert_eq!(s.iter().filter(|x| x.tier == 3).count(), 12);
    assert_eq!(1755 * 4, 7020);
    assert_eq!(1755 * 24, 42120);
    // the cheap half of the contract: the count is asserted from the frozen constant, and the
    // expensive enumeration runs only under `--features exhaustive` (review m5).
    assert_eq!(CANONICAL_FLOP_COUNT, 1755);
}

// Deviation (orchestrator pre-flight ruling, plan-mandated): the brief's Step 1 draft calls
// `core_iso::orbit_size(b)` on `b: &Vec<proto::Card>`, which does not compile because
// `core_iso::orbit_size` takes `&CanonicalBoard`. This uses `core_iso::orbit_size_of(b)` instead
// (`pub fn orbit_size_of(board: &[Card]) -> u8` in `crates/core-iso/src/lib.rs`), which accepts
// the `&[Card]` slice `canonical_flops_ordered()` actually produces.
#[cfg(feature = "exhaustive")]
#[test]
fn canonical_flop_enumeration_matches_the_frozen_count() {
    use cache::presolver::scenarios::*;
    let f = canonical_flops_ordered();
    assert_eq!(f.len(), CANONICAL_FLOP_COUNT);
    assert_eq!(raw_flops().count(), 22_100);
    let orbits = f.iter().map(|b| core_iso::orbit_size_of(b)).collect::<Vec<_>>();
    assert!(orbits.windows(2).all(|w| w[0] >= w[1]), "orbit 24 before 12 before 4");
    assert_eq!(orbits.iter().map(|&o| o as usize).sum::<usize>(), 22_100);
}
