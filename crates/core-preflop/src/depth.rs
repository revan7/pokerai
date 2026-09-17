//! Depth bucketing, stack-asymmetry labelling and rake-profile ranking (spec section 8.3).
//!
//! Every function here is pure and reads only public state: starting stacks, the live rake rule and
//! a candidate bundle's manifest. Hero's cards reach none of them, and none of them can reach a
//! public range, a solve input or a cache key (spec section 2).

use crate::envelope::{BundleInfo, SourceKind};
use proto::{ApproxReason, HandState, Seat};

/// One seat's starting stack.
///
/// `HandState.stacks_start` is aligned with `HandState.dealt` (Plan 1, spec section 4.3 S6), while
/// `Derived`'s per-seat vectors are indexed by `Seat.0`. Indexing `stacks_start` by `seat.0`
/// silently returns another seat's stack, which would give a wrong depth bucket, wrong
/// `AsymmetricStacks` and a wrong `MissingPreflopNode` rate. Every depth and eligibility
/// computation in this crate goes through this function; none indexes `stacks_start` directly.
///
/// # Panics
/// Panics (in every build profile, per the standing ruling) if `seat` is not a dealt seat, naming
/// the seat: the alternative is reading an unrelated seat's stack.
pub fn start_stack(state: &HandState, seat: Seat) -> u32 {
    let i = state.dealt.iter().position(|&s| s == seat).unwrap_or_else(|| panic!("start_stack: seat {} is not dealt", seat.0));
    assert!(i < state.stacks_start.len(), "start_stack: no starting stack for dealt seat {}", seat.0);
    state.stacks_start[i]
}

/// Effective depth in source units: the actor's starting stack capped by the deepest other
/// eligible seat's, divided by the unit (spec section 8.3). `others` is empty when the actor is the
/// only eligible seat, which leaves the actor's own stack as the depth.
///
/// # Panics
/// Panics (in every build profile, per the standing ruling) on a zero unit: the caller is
/// `source_unit`, whose value is a validated blind or straddle and therefore at least one chip, and
/// a zero here would silently produce an infinite or NaN depth that no bucket could label.
pub fn depth_for(actor: u32, others: &[u32], unit: u32) -> f64 {
    assert!(unit > 0, "depth_for: the source unit is at least one chip");
    actor.min(others.iter().copied().max().unwrap_or(actor)) as f64 / unit as f64
}

/// Nearest acquired depth, ties deeper. The upper filter implements section 8.3's "above 200 clamps
/// to 200" only while 200 is itself an acquired depth: with 200 present, the nearest-under-200
/// choice for any actual above 200 is 200 exactly. That holds for the chart set and for the eight
/// PokerData depths; a future bundle set whose deepest acquired depth is below 200 must clamp
/// explicitly instead of relying on this filter.
///
/// The lower bound of the filter is the one deviation from the brief's `d <= 200`: a declared
/// acquired depth of zero is not a depth, and admitting it would divide by zero in
/// [`prominent_depth`] and [`asymmetric`]. `validate` rejects a zero `depth_bb` in an envelope, but
/// nothing validates a manifest's `depths` list, so a zero there is skipped here and the bundle
/// simply offers no acquired depth (`MissingPreflopNode`) rather than panicking mid-query.
pub fn bucket(actual: f64, available: &[u16]) -> Option<u16> {
    available
        .iter()
        .copied()
        .filter(|&d| (1..=200).contains(&d))
        .min_by(|a, b| (actual - *a as f64).abs().total_cmp(&(actual - *b as f64).abs()).then_with(|| b.cmp(a)))
}

/// Whether the depth bucket's distance is worth surfacing: `DepthBucket` is emitted whenever
/// `actual != used`, and this only decides its `prominent` flag (spec section 8.3).
///
/// # Panics
/// Panics (in every build profile) on a zero `used` depth; see [`bucket`].
pub fn prominent_depth(actual: f64, used: u16) -> bool {
    assert!(used > 0, "prominent_depth: the used depth is at least one bb");
    (actual - used as f64).abs() / used as f64 > 0.05
}

/// Returns `(label, prominent)` for `ApproxReason::AsymmetricStacks { stacks_bb, prominent }`
/// (section 4.4 as amended by revision 6 S11): label on any difference, prominent above 5%.
///
/// # Panics
/// Panics (in every build profile) on a zero `used` depth; see [`bucket`].
pub fn asymmetric(stacks: &[f64], used: u16) -> (bool, bool) {
    assert!(used > 0, "asymmetric: the used depth is at least one bb");
    (
        stacks.iter().any(|&s| s != used as f64),
        stacks.iter().any(|&s| (s - used as f64).abs() / used as f64 > 0.05),
    )
}

/// The rake cap in source units: `cap_mchips` is thousandths of a chip and `unit` is chips per
/// source unit, so the cap in units is `cap_mchips / (1000 * unit)`.
///
/// # Panics
/// Panics (in every build profile) on a zero unit, which would make every cap distance NaN and the
/// candidate ordering non-deterministic.
pub fn cap_bb(cap_mchips: u32, unit: u32) -> f64 {
    assert!(unit > 0, "cap_bb: the source unit is at least one chip");
    cap_mchips as f64 / (1000.0 * unit as f64)
}

/// Rake rank, smaller is better: `(undocumented, cap distance, rate distance, collection-rule
/// mismatch, cap)` against a pot rake, and `(raked, cap, rate, 0, cap)` against a time charge,
/// which takes an unraked bundle ahead of any raked one and otherwise the smallest cap (spec
/// section 8.3). An undocumented source rake (`BundleInfo.rake == None`) sorts after every
/// documented profile at the same source and depth.
pub fn rake_rank(actual: &proto::Rake, candidate: &BundleInfo, unit: u32) -> (u8, f64, f64, u8, f64) {
    let Some(c) = &candidate.rake else { return (2, f64::INFINITY, f64::INFINITY, 1, f64::INFINITY) };
    match actual {
        proto::Rake::TimeCharge => (
            if c.rate == 0.0 || c.cap_bb == 0.0 { 0 } else { 1 },
            c.cap_bb as f64,
            c.rate as f64,
            0,
            c.cap_bb as f64,
        ),
        proto::Rake::PotRake { rate, cap_mchips, no_flop_no_drop } => (
            0,
            (c.cap_bb as f64 - cap_bb(*cap_mchips, unit)).abs(),
            (c.rate as f64 - *rate as f64).abs(),
            u8::from(c.no_flop_no_drop != *no_flop_no_drop),
            c.cap_bb as f64,
        ),
    }
}

/// `RakeProfileMapped` on any difference (section 8.3: "Any difference is `RakeProfileMapped`").
/// An undocumented source rake always maps, with the fixed `used` string.
pub fn rake_reason(actual: &proto::Rake, candidate: &BundleInfo, unit: u32) -> Option<ApproxReason> {
    let used = match &candidate.rake {
        None => "undocumented chart rake".to_string(),
        Some(c) => {
            let exact = match actual {
                proto::Rake::TimeCharge => c.rate == 0.0 && c.cap_bb == 0.0,
                proto::Rake::PotRake { rate, cap_mchips, no_flop_no_drop } => {
                    c.rate == *rate
                        && (c.cap_bb as f64 - cap_bb(*cap_mchips, unit)).abs() < 1e-9
                        && c.no_flop_no_drop == *no_flop_no_drop
                }
            };
            if exact {
                return None;
            }
            candidate.rake_profile.clone()
        }
    };
    Some(ApproxReason::RakeProfileMapped { actual: format!("{actual:?}"), used })
}

/// Lexicographic candidate ranking: source kind (PokerData before charts), nearest depth with the
/// deeper tie, then rake rank. The caller breaks a remaining tie with the lexicographically smaller
/// `bundle_id` (spec section 8.3, plan-3 global constraint "Source order").
pub fn rank_key(
    candidate: &BundleInfo,
    actual_depth: f64,
    actual_rake: &proto::Rake,
    unit: u32,
) -> (u8, f64, u8, (u8, f64, f64, u8, f64)) {
    let kind = match candidate.source {
        SourceKind::PokerDataJson => 0,
        SourceKind::ChartTranscription => 1,
    };
    let used = bucket(actual_depth, &candidate.depths);
    let distance = used.map(|d| (actual_depth - d as f64).abs()).unwrap_or(f64::INFINITY);
    // 0 for the deeper of two equidistant depths, so ties resolve deeper.
    let shallower = used.map(|d| u8::from((d as f64) < actual_depth)).unwrap_or(1);
    (kind, distance, shallower, rake_rank(actual_rake, candidate, unit))
}
