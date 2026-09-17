//! Semantic validation and bounded decoding of the wire `Envelope` (spec section 8.2):
//! the exact sum rule, bounds checked before indexing, and no silent renormalization.
//!
//! R1 (fix round 1): the per-class sibling-sum and unreachable-exact-zero rules run on
//! wide `f64` weights -- for `decode`, the raw wire values, before any narrowing to `f32`;
//! for `validate`, an already in-memory `Envelope`'s `f32` values, widened back to `f64`.
//! Narrowing each element to `f32` in isolation before these aggregate rules run can hide a
//! genuine violation: summing narrowed `f32`s can drift a true out-of-tolerance `f64` sum
//! back inside tolerance, and a declared-unreachable class holding a tiny nonzero `f64`
//! weight (e.g. `1e-50`) can narrow to exactly `0.0`, masking the nonzero source content.
//! `check_node_weights`/`check_node_evs`/`check_envelope` below are the one implementation
//! of these rules; `decode` and `validate` both call them (never a second, divergent copy),
//! feeding wide values from two different origins.

use crate::envelope::{Envelope, EnvelopeAction, EnvelopeNode};

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("bundle content: {0}")]
    Content(String),
    #[error("bundle exceeds 64 MiB")]
    TooLarge,
    #[error("bundle hash mismatch")]
    Hash,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;

/// A raise must carry a resolved positive size; every other token must carry no amount
/// at all (`Some(0)` is a malformed size field, not an omitted one).
pub fn valid_step(step: &str, amount: Option<u32>) -> bool {
    match step {
        "raise" => amount.is_some_and(|v| v > 0),
        "fold" | "check" | "call" | "allin" => amount.is_none(),
        _ => false,
    }
}

/// One node's fields in wide (`f64`) form, borrowed where the source representation allows
/// it and owned where it must be converted (from either raw wire `f64` or a narrowed
/// in-memory `f32`). The only thing `check_envelope`/`check_node_weights`/`check_node_evs`
/// ever see -- there is no code path that runs these rules directly against `f32`.
struct WideNode<'a> {
    history: &'a [(String, String, u32)],
    actor: &'a str,
    actions: &'a [EnvelopeAction],
    weights: Vec<Vec<f64>>,
    evs: Option<Vec<Vec<Option<f64>>>>,
    unreachable_classes: &'a [usize],
}

/// Spec 8.2's node-shape and per-class numeric rules: `weights` has one row per action, each
/// row exactly 169 wide; every element finite and in `[0, 1]`; each class's sibling weights
/// sum to `1 +- 1e-3`, or the class is declared unreachable and sums to *exactly* `0`. Takes
/// wide `f64` rows so a caller can run this before or after narrowing to `f32` -- the rule
/// itself never narrows or renormalizes.
fn check_node_weights(weights: &[Vec<f64>], actions_len: usize, unreachable_classes: &[usize]) -> Result<(), BundleError> {
    let bad = |s: &str| BundleError::Content(s.into());
    if weights.len() != actions_len || weights.iter().any(|r| r.len() != 169) || unreachable_classes.iter().any(|&c| c >= 169) {
        return Err(bad("shape"));
    }
    for c in 0..169 {
        let row: Vec<f64> = weights.iter().map(|a| a[c]).collect();
        if row.iter().any(|p| !p.is_finite() || !(0.0..=1.0).contains(p)) {
            return Err(bad("probability bound"));
        }
        let sum: f64 = row.iter().sum();
        let unreachable = unreachable_classes.contains(&c);
        if (unreachable && sum != 0.0) || (!unreachable && (sum - 1.0).abs() > 1e-3) {
            return Err(bad("sibling sum or unreachable declaration"));
        }
    }
    Ok(())
}

/// Spec 8.2's EV shape/finiteness rule (no sibling-sum rule applies to EVs, so this has no
/// wide-vs-narrow discrepancy the way `check_node_weights` does -- a per-cell finite check is
/// unaffected by exactly when narrowing happens -- but it takes wide rows for the same reason
/// `check_node_weights` does: one implementation, called from both `decode` and `validate`).
fn check_node_evs(evs: Option<&Vec<Vec<Option<f64>>>>, actions_len: usize) -> Result<(), BundleError> {
    let bad = |s: &str| BundleError::Content(s.into());
    if let Some(evs) = evs {
        if evs.len() != actions_len || evs.iter().any(|r| r.len() != 169) || evs.iter().flatten().flatten().any(|v| !v.is_finite()) {
            return Err(bad("EV shape or finite value"));
        }
    }
    Ok(())
}

/// Every non-numeric structural rule plus the numeric rules above, applied envelope-wide
/// (class order/depth, and, per node: actor validity, non-empty menu, unique history,
/// resolved sizes, unique action kinds, unique unreachable indices, then the numeric rules).
fn check_envelope(class_order: &str, depth_bb: u16, nodes: &[WideNode]) -> Result<(), BundleError> {
    let bad = |s: &str| BundleError::Content(s.into());
    if class_order != "A-2 row-major, section 4.1" || depth_bb == 0 {
        return Err(bad("class order or depth"));
    }
    let positions = ["UTG", "HJ", "CO", "BTN", "SB", "BB"];
    let mut keys = std::collections::BTreeSet::new();
    for n in nodes {
        if !positions.contains(&n.actor) || n.actions.is_empty() || !keys.insert(serde_json::to_string(n.history)?) {
            return Err(bad("actor, empty menu, or duplicate history"));
        }
        // History tuples are `(position, step, amount)` with 0 meaning "no amount",
        // which is why the history check maps 0 to None before calling `valid_step`.
        if n.history.iter().any(|(p, s, v)| {
            !positions.contains(&p.as_str()) || !valid_step(s, (*v != 0).then_some(*v))
        }) || n.actions.iter().any(|a| !valid_step(&a.step, a.to_bb_x1000))
        {
            return Err(bad("unresolved size or invalid token"));
        }
        let mut menu = std::collections::BTreeSet::new();
        if n.actions.iter().any(|a| !menu.insert((a.step.clone(), a.to_bb_x1000))) {
            return Err(bad("duplicate action kind and amount"));
        }
        let mut unreachable_seen = std::collections::BTreeSet::new();
        if n.unreachable_classes.iter().any(|c| !unreachable_seen.insert(*c)) {
            return Err(bad("duplicate unreachable class"));
        }
        check_node_weights(&n.weights, n.actions.len(), n.unreachable_classes)?;
        check_node_evs(n.evs.as_ref(), n.actions.len())?;
    }
    Ok(())
}

/// Validates an already-constructed in-memory `Envelope`: widens its `f32` weights/EVs back
/// to `f64` and runs the same rules `decode` runs on the raw wire values. This is what a
/// caller who builds an `Envelope` directly (not through `decode`) gets -- it can only check
/// the precision it's given, which is why `decode` runs these rules on the wire's original
/// wide values instead of narrowing first (see the module doc comment).
pub fn validate(e: &Envelope) -> Result<(), BundleError> {
    let nodes: Vec<WideNode> = e
        .nodes
        .iter()
        .map(|n| WideNode {
            history: &n.history,
            actor: &n.actor,
            actions: &n.actions,
            weights: n.weights.iter().map(|row| row.iter().map(|&x| x as f64).collect()).collect(),
            evs: n.evs.as_ref().map(|rows| rows.iter().map(|row| row.iter().map(|c| c.map(|x| x as f64)).collect()).collect()),
            unreachable_classes: &n.unreachable_classes,
        })
        .collect();
    check_envelope(&e.class_order, e.depth_bb, &nodes)
}

/// Private wide-form mirror of the wire shape, deserialized with plain (unchecked, `f64`)
/// numeric fields so `decode` can run `check_envelope` -- including the per-class sibling-sum
/// and unreachable-exact-zero rules -- against the *original* wire values before any of them
/// are narrowed to `f32` (R1). Never exposed outside this module.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEnvelope {
    bundle_id: String,
    depth_bb: u16,
    rake_profile: String,
    straddle: bool,
    class_order: String,
    nodes: Vec<WireNode>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNode {
    history: Vec<(String, String, u32)>,
    actor: String,
    actions: Vec<EnvelopeAction>,
    weights: Vec<Vec<f64>>,
    #[serde(default)]
    evs: Option<Vec<Vec<Option<f64>>>>,
    unreachable_classes: Vec<usize>,
}

/// Bounded decode: rejects an oversized manifest before ever handing it to `serde_json`,
/// checks every structural and numeric rule against the wire's original wide (`f64`) values
/// (R1), and only then narrows to the `f32` `Envelope` it returns -- narrowing happens once,
/// after every check has already passed, never before.
pub fn decode(bytes: &[u8]) -> Result<Envelope, BundleError> {
    if bytes.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(BundleError::TooLarge);
    }
    let wire: WireEnvelope = serde_json::from_slice(bytes)?;

    let wide_nodes: Vec<WideNode> = wire
        .nodes
        .iter()
        .map(|n| WideNode {
            history: &n.history,
            actor: &n.actor,
            actions: &n.actions,
            weights: n.weights.clone(),
            evs: n.evs.clone(),
            unreachable_classes: &n.unreachable_classes,
        })
        .collect();
    check_envelope(&wire.class_order, wire.depth_bb, &wide_nodes)?;

    let nodes = wire
        .nodes
        .into_iter()
        .map(|n| {
            let weights = n
                .weights
                .into_iter()
                .map(|row| row.into_iter().map(crate::numeric::narrow_weight).collect::<Result<Vec<f32>, String>>())
                .collect::<Result<Vec<Vec<f32>>, String>>()
                .map_err(BundleError::Content)?;
            let evs = n
                .evs
                .map(|rows| {
                    rows.into_iter()
                        .map(|row| {
                            row.into_iter()
                                .map(|c| c.map(crate::numeric::narrow_ev).transpose())
                                .collect::<Result<Vec<Option<f32>>, String>>()
                        })
                        .collect::<Result<Vec<Vec<Option<f32>>>, String>>()
                })
                .transpose()
                .map_err(BundleError::Content)?;
            Ok(EnvelopeNode {
                history: n.history,
                actor: n.actor,
                actions: n.actions,
                weights,
                evs,
                unreachable_classes: n.unreachable_classes,
            })
        })
        .collect::<Result<Vec<EnvelopeNode>, BundleError>>()?;

    Ok(Envelope {
        bundle_id: wire.bundle_id,
        depth_bb: wire.depth_bb,
        rake_profile: wire.rake_profile,
        straddle: wire.straddle,
        class_order: wire.class_order,
        nodes,
    })
}
