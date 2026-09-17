//! Coverage inheritance and raw-accuracy filtering on cache lookup (spec section 10.4): whether a
//! matched `CacheEntry` may be disclosed as `Coverage::Exact` or must carry `Approximate` reasons,
//! and whether the result is `Phase::Provisional` because its raw stored exploitability -- never
//! rounded to whole basis points (spec section 13.1: "accuracy filter is raw, never rounded bp")
//! -- misses the *query's* accuracy target. Reasons only ever accumulate: an entry that already
//! carries an inherited reason never upgrades back to `Exact`, and reaching a looser target on a
//! later query never strips a reason the entry's own (possibly tighter) solve already incurred
//! (CLAUDE.md section 6 "Coverage labels": "a cached result NEVER upgrades its inherited
//! coverage").

use std::collections::BTreeSet;

/// Whether raw stored exploitability `raw` -- a pot-relative fraction, never rounded to whole
/// basis points before this comparison -- meets `target` basis points (spec section 10.4/13.1:
/// `exploitability_over_P <= target_bp / 10000`, raw). Non-finite or negative `raw` is never
/// accurate; both are checked explicitly (rather than relying on `is_finite()` alone masking a
/// finite-but-negative value, or a narrower `is_nan()` check missing +/-infinity).
pub fn accuracy_ok(raw: f64, target: u16) -> bool {
    raw.is_finite() && raw >= 0.0 && raw <= target as f64 / 10000.0
}

/// The union of `a` and `b`, in order, with exact-duplicate reasons (identical serialized form)
/// removed after their first occurrence. Reasons accumulate, never drop (spec section 13.1), so
/// this is strictly additive: every reason present in `a` or `b` appears in the result exactly
/// once, at the position it was first seen.
pub fn merge_reasons(a: &[proto::ApproxReason], b: &[proto::ApproxReason]) -> Vec<proto::ApproxReason> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for r in a.iter().chain(b) {
        if seen.insert(serde_json::to_vec(r).expect("validated reason")) {
            out.push(r.clone());
        }
    }
    out
}

/// Labels a matched `CacheEntry` for a live query: `spr` is the query's exact SPR rational, `c`
/// the entry/query `Comparison` (spec section 10.4 delta/dev), `target` the query's accuracy
/// target in basis points, and `query_reasons` any reasons already incurred translating or
/// mapping the live request before the cache was consulted. Returns the disclosed `Coverage` and
/// whether the result is `Phase::Provisional` (raw accuracy misses `target`).
///
/// Reasons accumulate only: inherited (`e.reasons`) and query reasons are merged and deduplicated
/// first (`merge_reasons`); `SprBucketed` is added iff the query's exact SPR differs from the
/// entry's own stored SPR (`spr != e.source.spr`, i.e. this lookup matched a neighboring bucket);
/// `MenuRounded` is added iff `c.max_dev > 0.0`. Coverage never upgrades an entry that already
/// carries an inherited reason back to `Exact`, and passing a looser target on this query never
/// removes a reason the entry's own solve already incurred (e.g. a stored `DeadlineBestSoFar`
/// from a tighter original target) -- reasons and accuracy are independent axes.
///
/// An entry whose raw exploitability misses `target` is `Provisional`, but that miss alone never
/// synthesizes a new reason: an above-target hit that incurred no reason is disclosed as
/// `Coverage::Exact` with `Phase::Provisional` plus the raw reached exploitability, never as an
/// `Approximate{reasons: []}` (review m3 / spec section 6: only reasons actually incurred are
/// emitted). Do not invent a new deadline reason for a source solve that met its own looser
/// target, and never certify current timing from an old, inherited `DeadlineBestSoFar` -- the
/// `provisional` bool is always computed from this call's own `target`, never from a reason's
/// stored `target_bp`.
pub fn label(
    e: &crate::entry::CacheEntry,
    spr: crate::key::Rational,
    c: &crate::lookup::Comparison,
    target: u16,
    query_reasons: &[proto::ApproxReason],
) -> (proto::Coverage, bool) {
    let mut reasons = merge_reasons(query_reasons, &e.reasons);
    if spr != e.source.spr {
        reasons.push(proto::ApproxReason::SprBucketed { actual: spr.value() as f32, used: e.source.spr.value() as f32 });
    }
    if c.max_dev > 0.0 {
        reasons.push(proto::ApproxReason::MenuRounded { max_delta_pct: (100.0 * c.max_dev) as f32 });
    }
    let provisional = !accuracy_ok(e.exploitability_over_P, target);
    (if reasons.is_empty() { proto::Coverage::Exact } else { proto::Coverage::Approximate { reasons } }, provisional)
}
