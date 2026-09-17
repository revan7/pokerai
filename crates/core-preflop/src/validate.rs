//! Semantic validation and bounded decoding of the wire `Envelope` (spec section 8.2):
//! the exact sum rule, bounds checked before indexing, and no silent renormalization.

use crate::envelope::Envelope;

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

/// Bounded decode: rejects an oversized manifest before ever handing it to `serde_json`,
/// then validates the decoded envelope before returning it.
pub fn decode(bytes: &[u8]) -> Result<Envelope, BundleError> {
    if bytes.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(BundleError::TooLarge);
    }
    let envelope: Envelope = serde_json::from_slice(bytes)?;
    validate(&envelope)?;
    Ok(envelope)
}

/// A raise must carry a resolved positive size; every other token must carry no amount
/// at all (`Some(0)` is a malformed size field, not an omitted one).
pub fn valid_step(step: &str, amount: Option<u32>) -> bool {
    match step {
        "raise" => amount.is_some_and(|v| v > 0),
        "fold" | "check" | "call" | "allin" => amount.is_none(),
        _ => false,
    }
}

pub fn validate(e: &Envelope) -> Result<(), BundleError> {
    let bad = |s: &str| BundleError::Content(s.into());
    if e.class_order != "A-2 row-major, section 4.1" || e.depth_bb == 0 {
        return Err(bad("class order or depth"));
    }
    let positions = ["UTG", "HJ", "CO", "BTN", "SB", "BB"];
    let mut keys = std::collections::BTreeSet::new();
    for n in &e.nodes {
        if !positions.contains(&n.actor.as_str())
            || n.actions.is_empty()
            || !keys.insert(serde_json::to_string(&n.history)?)
        {
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
        if n.weights.len() != n.actions.len()
            || n.weights.iter().any(|r| r.len() != 169)
            || n.unreachable_classes.iter().any(|&c| c >= 169)
        {
            return Err(bad("shape"));
        }
        for c in 0..169 {
            let row: Vec<f32> = n.weights.iter().map(|a| a[c]).collect();
            if row.iter().any(|p| !p.is_finite() || !(0.0..=1.0).contains(p)) {
                return Err(bad("probability bound"));
            }
            let sum: f64 = row.iter().map(|&p| p as f64).sum();
            let unreachable = n.unreachable_classes.contains(&c);
            if (unreachable && sum != 0.0) || (!unreachable && (sum - 1.0).abs() > 1e-3) {
                return Err(bad("sibling sum or unreachable declaration"));
            }
        }
        if let Some(evs) = &n.evs {
            if evs.len() != n.actions.len()
                || evs.iter().any(|r| r.len() != 169)
                || evs.iter().flatten().flatten().any(|v| !v.is_finite())
            {
                return Err(bad("EV shape or finite value"));
            }
        }
    }
    Ok(())
}
