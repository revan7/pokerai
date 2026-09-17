//! Coverage inheritance and raw-accuracy filtering on cache lookup (spec section 10.4/3.5):
//! whether a matched `CacheEntry` may be disclosed as `Label::Exact`, must carry
//! `Label::Approximate` reasons, or is `Label::Provisional` because its raw stored
//! exploitability -- never rounded to whole basis points (spec section 13.1: "accuracy filter is
//! raw, never rounded bp") -- misses the *query's* accuracy target. Reasons only ever accumulate:
//! an entry that already carries an inherited reason never upgrades back to `Exact`, and reaching
//! a looser target on a later query never strips a reason the entry's own solve already incurred
//! (CLAUDE.md section 6 "Coverage labels": "a cached result NEVER upgrades its inherited
//! coverage").
//!
//! Fix round 1 (review R1): the task brief's original pseudocode returned `(proto::Coverage,
//! bool)`, pairing `proto::Coverage::Exact` with a separate `provisional` flag for an
//! above-target hit. Spec section 3.5 defines the cache lookup outcome itself as a three-way
//! `Exact / Approximate{reasons} / Provisional{reasons}` (plus `Miss`, not this module's
//! concern), and `constraints.md:11` requires raw accuracy to pass for `Exact`. Reusing
//! `proto::Coverage` (which has no `Provisional` variant -- that axis lives on `proto::Phase`
//! downstream of this module) would either violate that `Exact` condition or invent a fourth,
//! unspec'd shape; `Label` is this module's own three-way type, named per the orchestrator's
//! ruling, distinct from `proto::Coverage`/`proto::Phase`.

use std::collections::BTreeSet;

/// The lookup-side disclosure for a matched `CacheEntry` against a live query (spec section 3.5).
/// `Exact` iff the query's SPR rational equals the entry's own, every realized menu fraction
/// agreed (zero deviation), raw accuracy passed, and no reason -- inherited or newly incurred --
/// applies. `Approximate` carries every reason that does apply while raw accuracy still passed.
/// `Provisional` is raw accuracy failing to meet the query's target; it always carries the same
/// merged reason list `Approximate` would have carried (possibly empty) -- the accuracy shortfall
/// itself is expressed by being `Provisional`, never by inventing a reason (e.g. a synthesized
/// `DeadlineBestSoFar`, which names a live solve stopped by its own deadline, not a raw-accuracy
/// comparison performed here).
#[derive(Clone, Debug, PartialEq)]
pub enum Label {
    Exact,
    Approximate { reasons: Vec<proto::ApproxReason> },
    Provisional { reasons: Vec<proto::ApproxReason> },
}

impl Label {
    /// Every reason this label carries (empty for `Exact`, and for `Approximate`/`Provisional`
    /// with an empty list).
    pub fn reasons(&self) -> &[proto::ApproxReason] {
        match self {
            Label::Exact => &[],
            Label::Approximate { reasons } | Label::Provisional { reasons } => reasons,
        }
    }

    /// Whether raw accuracy missed the query's target (`Label::Provisional`).
    pub fn is_provisional(&self) -> bool {
        matches!(self, Label::Provisional { .. })
    }
}

/// Whether raw stored exploitability `raw` -- a pot-relative fraction, never rounded to whole
/// basis points before this comparison -- meets `target` basis points (spec section 10.4/13.1:
/// `exploitability_over_P <= target_bp / 10000`, raw). Non-finite or negative `raw` is never
/// accurate; both are checked explicitly for defensiveness/clarity (matching this crate's other
/// numeric-validation call sites, e.g. `entry.rs`'s `narrow_checked`/`widen_checked`), though in
/// this particular expression a non-finite `raw` is already rejected by the surrounding
/// comparisons too: IEEE-754 makes every ordered comparison against `NaN` false, and `+inf`
/// already fails `raw <= target as f64 / 10000.0` on its own.
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
/// mapping the live request before the cache was consulted. Returns the disclosed `Label`.
///
/// Reasons accumulate only: inherited (`e.reasons`) and query reasons are merged and deduplicated
/// first (`merge_reasons`, query first -- fix round 1 R2: this is the one call site that must not
/// drop `query_reasons`, since a standalone `merge_reasons` unit test cannot catch a caller
/// dropping its argument); `SprBucketed` is added iff the query's exact SPR differs from the
/// entry's own stored SPR (`spr != e.source.spr`, i.e. this lookup matched a neighboring bucket);
/// `MenuRounded` is added iff `c.max_dev > 0.0`.
///
/// Raw accuracy is checked last and decides the variant, never the reason list: a miss returns
/// `Label::Provisional { reasons }` carrying the exact same merged reasons an accuracy pass would
/// have disclosed (possibly empty -- fix round 1 R1's frozen regression: raw 0.004 at a 30bp
/// target with no other reasons is `Provisional { reasons: vec![] }`, never `Exact`, because
/// `Exact` requires raw accuracy to pass). A pass with a nonempty reason list is `Approximate`; a
/// pass with an empty one is `Exact`. Passing a looser target on a later query never removes a
/// reason the entry's own (possibly tighter) solve already incurred (e.g. a stored
/// `DeadlineBestSoFar`) -- reasons and accuracy are independent axes, both folded into `Label`.
pub fn label(
    e: &crate::entry::CacheEntry,
    spr: crate::key::Rational,
    c: &crate::lookup::Comparison,
    target: u16,
    query_reasons: &[proto::ApproxReason],
) -> Label {
    let mut reasons = merge_reasons(query_reasons, &e.reasons);
    if spr != e.source.spr {
        reasons.push(proto::ApproxReason::SprBucketed { actual: spr.value() as f32, used: e.source.spr.value() as f32 });
    }
    if c.max_dev > 0.0 {
        reasons.push(proto::ApproxReason::MenuRounded { max_delta_pct: (100.0 * c.max_dev) as f32 });
    }
    if !accuracy_ok(e.exploitability_over_P, target) {
        return Label::Provisional { reasons };
    }
    if reasons.is_empty() {
        Label::Exact
    } else {
        Label::Approximate { reasons }
    }
}
