//! Actor-owned street payloads (spec section 10.4): a `CacheEntry` normalizes a worker
//! `StreetSolution` against the `EffectiveTree` it was solved on, converting each node's wire
//! chip path to the ordinal path the cache and replay identify nodes by (spec section 2), and
//! carrying EV as a pot-relative fraction (`ev_over_P`) rather than raw chips so the entry can
//! be reused across chip scales. `validate_entry` is the read-side counterpart: it reconstructs
//! a source `StreetSolution` from a persisted `CacheEntry` and re-runs the same structural
//! checks (`proto::worker::validate_solution`) plus the cache-specific invariants (board
//! blocking, canonical identity, SPR/fraction/hash agreement) that only apply to a stored
//! entry. Neither function re-implements the matrix/shape/path checks that already live in
//! `proto::worker::validate_solution` -- both call it and map its `Err` to
//! `CacheError::Invalid`.

use crate::CacheError;

// --- Numeric wire validation for cached matrices (standing ruling (a): validate wide, never
// clamp; fix round 1 R1) ---
//
// `CachedNode::probs`/`ev_over_P` are `Vec<Vec<f32>>` in memory (spec: EV is signed finite f32
// chips), matching `proto::worker::NodeStrategy`. Deriving plain `Deserialize` for those fields
// (as the brief's literal code did) admits any f32 the *narrow* decode happens to produce --
// e.g. a persisted probability row `[1.00000001, -1e-50]` narrows silently to `[1.0, -0.0]`,
// which is in-domain and passes `validate_entry` even though the wide value never was.
//
// `proto::worker` already solves this for the worker wire (spec 4.5) with private
// `narrow_checked`/`widen_checked`/`deserialize_matrix` helpers; they are not exported, so
// (per this fix's instruction) equivalent private helpers are defined here rather than
// weakening proto's visibility. Unlike proto::worker's codec, which only ever serializes to
// JSON-lines and can safely write narrow `f32` while reading wide `f64` (JSON is
// self-describing), `CachedNode` is also the type a bincode-encoded on-disk cache entry
// carries (spec 10.4). Bincode is *not* self-describing -- the deserializer reads exactly as
// many bytes as the field type it is asked for -- so if this codec wrote narrow `f32` (4
// bytes) but read wide `f64` (8 bytes), a bincode round trip of the very data this codec
// wrote would desync and fail to decode. Reading and writing `f64` on *both* sides keeps
// bincode's width symmetric while still performing the wide-before-narrow admission check
// against a value that may have arrived with more precision than f32 (the demonstrated JSON
// attack); the field type itself stays `Vec<Vec<f32>>`, so nothing about the in-memory or
// spec-mandated representation changes, and the wire cost is the same 8 bytes/value bincode
// would already spend on a raw `f64` field.

fn narrow_checked(raw: f64, domain: fn(f64) -> bool, what: &str) -> Result<f32, String> {
    if !raw.is_finite() || !domain(raw) {
        return Err(format!("{what} {raw} is outside its valid domain"));
    }
    let narrowed = raw as f32;
    if !narrowed.is_finite() || !domain(narrowed as f64) {
        return Err(format!("{what} {raw} narrows to {narrowed:e}, outside its valid domain"));
    }
    Ok(narrowed)
}

fn widen_checked(v: f32, domain: fn(f64) -> bool, what: &str) -> Result<f64, String> {
    if !v.is_finite() || !domain(v as f64) {
        return Err(format!("{what} {v} is outside its valid domain"));
    }
    Ok(v as f64)
}

fn domain_unit_interval(x: f64) -> bool {
    (0.0..=1.0).contains(&x)
}
fn domain_finite(_: f64) -> bool {
    true
}

fn deserialize_matrix<'de, D: serde::Deserializer<'de>>(d: D, domain: fn(f64) -> bool, what: &str) -> Result<Vec<Vec<f32>>, D::Error> {
    let raw: Vec<Vec<f64>> = serde::Deserialize::deserialize(d)?;
    raw.into_iter()
        .map(|row| row.into_iter().map(|x| narrow_checked(x, domain, what).map_err(serde::de::Error::custom)).collect())
        .collect()
}

fn serialize_matrix<S: serde::Serializer>(v: &[Vec<f32>], s: S, domain: fn(f64) -> bool, what: &str) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut outer = s.serialize_seq(Some(v.len()))?;
    for row in v {
        let mut checked_row = Vec::with_capacity(row.len());
        for x in row {
            checked_row.push(widen_checked(*x, domain, what).map_err(serde::ser::Error::custom)?);
        }
        outer.serialize_element(&checked_row)?;
    }
    outer.end()
}

fn deserialize_prob_matrix<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Vec<f32>>, D::Error> {
    deserialize_matrix(d, domain_unit_interval, "probability")
}
fn serialize_prob_matrix<S: serde::Serializer>(v: &Vec<Vec<f32>>, s: S) -> Result<S::Ok, S::Error> {
    serialize_matrix(v, s, domain_unit_interval, "probability")
}
fn deserialize_ev_matrix<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Vec<f32>>, D::Error> {
    deserialize_matrix(d, domain_finite, "ev_over_P")
}
fn serialize_ev_matrix<S: serde::Serializer>(v: &Vec<Vec<f32>>, s: S) -> Result<S::Ok, S::Error> {
    serialize_matrix(v, s, domain_finite, "ev_over_P")
}

/// The financial and range inputs a `CacheEntry` was solved from. Kept separately from
/// `KeyFields` because these are the raw, chip-scale values the key's normalized fields (SPR
/// rational, rake-over-pot fraction, range hashes) were derived from -- a reader needs them to
/// recompute and re-check that derivation (`validate_entry`).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceInputs {
    pub pot: u32,
    pub stack_oop: u32,
    pub stack_ip: u32,
    pub spr: crate::key::Rational,
    pub bb_chips: u32,
    pub quantum_over_p: crate::key::Rational,
    pub cap_mchips: u32,
    pub ranges: [proto::Range1326; 2],
}

/// One materialized decision node's normalized strategy: ordinal path (never a chip path, spec
/// section 2), the actor that owns it, and its probability/EV matrices in `proto::worker`'s
/// combo-row order. `ev_over_P` is EV chips divided by the source pot (`SourceInputs::pot`) so
/// the payload never embeds a raw chip scale.
#[allow(non_snake_case)]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CachedNode {
    pub path: proto::OrdinalPath,
    pub actor: String,
    #[serde(deserialize_with = "deserialize_prob_matrix", serialize_with = "serialize_prob_matrix")]
    pub probs: Vec<Vec<f32>>,
    #[serde(deserialize_with = "deserialize_ev_matrix", serialize_with = "serialize_ev_matrix")]
    pub ev_over_P: Vec<Vec<f32>>,
    pub available: Vec<bool>,
}

/// A normalized, persistable street solution (spec section 10.4): the structural key it was
/// solved under, the raw inputs that key was derived from, the effective tree, each menu
/// entry's pot-fraction (`fractions`, indexed the same way as `tree.materialized`), the
/// per-node strategies, and solver metadata. Never a `Recommendation` and never a request
/// identity (hand id, seat ids, hero's cards) -- those never enter this type.
#[allow(non_snake_case)]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct CacheEntry {
    pub key: crate::key::KeyFields,
    pub source: SourceInputs,
    pub tree: proto::EffectiveTree,
    pub fractions: Vec<Vec<Option<crate::key::Rational>>>,
    pub nodes: Vec<CachedNode>,
    pub covered_paths: Vec<proto::OrdinalPath>,
    pub exploitability_over_P: f64,
    pub target_bp: u16,
    pub iterations: u32,
    pub elapsed_ms: u32,
    pub memory_bytes: u64,
    pub mode: String,
    pub locks_applied: u16,
    pub export: String,
    pub reasons: Vec<proto::ApproxReason>,
    pub created: u64,
    pub last_hit: u64,
}

/// Walks an ordinal path `p` through materialized tree `t`, converting each ordinal index back
/// into the `Action` it names at that step -- the inverse of `proto::resolve_chip_path`. `None`
/// if any prefix along the path is not a materialized node or an index is out of range for that
/// node's action menu.
pub fn chip_path(t: &[proto::MaterializedNode], p: &[u8]) -> Option<proto::ChipPath> {
    p.iter()
        .enumerate()
        .map(|(i, &a)| t.iter().find(|n| n.path == p[..i])?.actions.get(a as usize).cloned())
        .collect()
}

/// Cross-plan M21/D2: the spec section 2 chip-path rule has exactly one implementation, in
/// `proto`. This crate re-exports it and never defines a second walk.
pub use proto::resolve_chip_path as resolve_path;

/// Materialized lists are compared position-by-position in Task 3, so the payload must carry
/// them in ordinal-path order with no duplicates (review m2).
pub fn sorted_by_path(t: &[proto::MaterializedNode]) -> bool {
    t.windows(2).all(|w| w[0].path < w[1].path)
}

/// Bit-exact range comparison (fix round 1 R2): unlike `Range1326`'s derived `PartialEq`
/// (ordinary float equality, which treats `+0.0 == -0.0`), this compares every weight's
/// `to_bits()` pattern -- matching the bit-exactness `core_iso::canonicalize`'s tie-break and
/// `core_ranges::hash_scaled` both already require (spec section 2).
fn ranges_bit_equal(a: &proto::Range1326, b: &proto::Range1326) -> bool {
    a.0.iter().zip(b.0.iter()).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// Normalizes a worker `StreetSolution` against the `EffectiveTree` it was solved on into the
/// entry's own actor-owned node list. `p` is the root pot (`SourceInputs::pot`, chips): EV is
/// divided by it to produce the pot-relative `ev_over_P` the entry stores. Delegates every
/// matrix/shape/path/actor structural check to `proto::worker::validate_solution` rather than
/// re-implementing it; the per-node re-derivation below only converts the already-validated
/// chip path to an ordinal path.
pub fn normalize(
    s: &proto::worker::StreetSolution,
    t: &proto::EffectiveTree,
    p: u32,
) -> Result<Vec<CachedNode>, CacheError> {
    if p == 0 {
        return Err(CacheError::Invalid("zero pot"));
    }
    proto::worker::validate_solution(s, &t.materialized).map_err(|_| CacheError::Invalid("invalid solution"))?;
    s.nodes
        .iter()
        .map(|n| {
            let path = resolve_path(&t.materialized, &n.path).ok_or(CacheError::Invalid("unresolved path"))?;
            let m = t.materialized.iter().find(|m| m.path == path).unwrap();
            if m.actor != n.actor || m.actions != n.actions {
                return Err(CacheError::Invalid("actor or source menu"));
            }
            Ok(CachedNode {
                path,
                actor: n.actor.clone(),
                probs: n.probs.clone(),
                ev_over_P: n.ev_chips.iter().map(|r| r.iter().map(|v| v / p as f32).collect()).collect(),
                available: n.available.clone(),
            })
        })
        .collect()
}

/// Re-validates a persisted `CacheEntry` end to end (spec section 10.4): checked source
/// pot/stacks, key/tree version agreement, SPR rational and bucket recomputed exactly, board
/// shape/blocking, rake and export/mode/model-lock agreement, menu fractions recomputed
/// exactly, per-range domain/mass/hash checks, canonical board/range identity (re-running
/// `core_iso::canonicalize` and requiring it reproduce the stored board and leave both ranges
/// fixed), unique-and-matching `covered_paths`, and -- reconstructing a source `StreetSolution`
/// from the stored nodes -- a final `proto::worker::validate_solution` pass. Board blocking,
/// canonical identity and fraction checks run before numeric matrix reconstruction so a
/// malformed board or range is rejected before it can panic a later combo-index computation.
pub fn validate_entry(e: &CacheEntry) -> Result<(), CacheError> {
    use crate::key::{spr_bucket, Rational};
    let bad = || CacheError::Invalid("invalid solution");
    let s = &e.source;
    if s.pot == 0
        || s.stack_oop == 0
        || s.stack_ip == 0
        || s.bb_chips == 0
        || s.pot as u64 + s.stack_oop as u64 + s.stack_ip as u64 >= 1_u64 << 31
        || e.key.schema_version != 3
        || e.key.rules_version != 3
        || e.tree.rules_version != 3
        || e.key.adapter_version != proto::worker::ADAPTER_VERSION
        || e.key.solver_commit != proto::worker::SOLVER_COMMIT
        || !matches!(e.key.root_street, proto::Street::Flop | proto::Street::Turn)
        || e.tree.root_street != e.key.root_street
        || !e.exploitability_over_P.is_finite()
        || e.exploitability_over_P < 0.0
        || !matches!(e.mode.as_str(), "f32" | "i16")
        || e.nodes.is_empty()
        || e.nodes.len() > 100_000
    {
        return Err(bad());
    }
    if s.spr != Rational::new(s.stack_oop.min(s.stack_ip) as u64, s.pot as u64)?
        || e.key.spr_bucket != spr_bucket(s.spr)
        || e.key.rake.cap_over_p != Rational::new(s.cap_mchips as u64, 1000 * s.pot as u64)?
    {
        return Err(bad());
    }
    // review m2: Task 3 zips the two materialized lists positionally, so ordering is a payload invariant.
    if !sorted_by_path(&e.tree.materialized) || e.tree.materialized.is_empty() {
        return Err(bad());
    }
    let board = &e.key.canonical_board;
    let length = if e.key.root_street == proto::Street::Flop { 3 } else { 4 };
    let rate = e.key.rake.clone().rate();
    if board.len() != length
        || board.iter().any(|c| c.0 >= 52)
        || board.iter().map(|c| c.0).collect::<std::collections::BTreeSet<_>>().len() != board.len()
        || !rate.is_finite()
        || !(0.0..=1.0).contains(&rate)
        || s.quantum_over_p.num() == 0
        || s.quantum_over_p.den() == 0
        || !matches!(e.export.as_str(), "street" | "truncated")
        || (matches!(e.key.model, crate::key::Model::Baseline) && e.locks_applied != 0)
    {
        return Err(bad());
    }
    let fractions = e
        .tree
        .materialized
        .iter()
        .map(|n| {
            n.actions
                .iter()
                .map(|a| match a {
                    proto::Action::Bet { to } | proto::Action::Raise { to } | proto::Action::AllIn { to } => {
                        Some(Rational::new(*to as u64, s.pot as u64).unwrap())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if fractions != e.fractions {
        return Err(bad());
    }
    for (range, hash) in s.ranges.iter().zip([e.key.range_hash_oop, e.key.range_hash_ip]) {
        if range.0.iter().any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
            || !range.0.iter().any(|x| *x > 0.0)
            || core_ranges::hash_scaled(range) != hash
        {
            return Err(bad());
        }
        for hi in 1_u8..52 {
            for lo in 0_u8..hi {
                if board.iter().any(|c| c.0 == hi || c.0 == lo) && range.0[hi as usize * (hi as usize - 1) / 2 + lo as usize] != 0.0 {
                    return Err(bad());
                }
            }
        }
    }
    let (_, perm) = core_iso::canonicalize(board, &[&s.ranges[0], &s.ranges[1]]);
    let mut canonical = board.iter().map(|c| core_iso::apply(&perm, *c)).collect::<Vec<_>>();
    canonical[..3].sort_by_key(|c| c.0);
    // Fix round 1 R2: `Range1326` derives ordinary float `PartialEq`, which treats `+0.0` and
    // `-0.0` as equal, while `canonicalize`'s own tie-break and `hash_scaled` both distinguish
    // the bit patterns (spec section 2: "every weight is divided ... and the resulting f32 bit
    // patterns are hashed"). A range that is numerically but not bit-exactly fixed under the
    // recomputed permutation must still be rejected, so compare `to_bits()` weight-by-weight.
    if canonical != *board || !ranges_bit_equal(&core_iso::apply_range(&perm, &s.ranges[0]), &s.ranges[0]) || !ranges_bit_equal(&core_iso::apply_range(&perm, &s.ranges[1]), &s.ranges[1]) {
        return Err(bad());
    }
    if e.covered_paths != e.nodes.iter().map(|n| n.path.clone()).collect::<Vec<_>>()
        || e.covered_paths.iter().collect::<std::collections::BTreeSet<_>>().len() != e.nodes.len()
    {
        return Err(bad());
    }
    let mut nodes = Vec::new();
    for n in &e.nodes {
        let m = e.tree.materialized.iter().find(|m| m.path == n.path).ok_or_else(bad)?;
        if m.actor != n.actor || m.street != e.key.root_street {
            return Err(bad());
        }
        nodes.push(proto::worker::NodeStrategy {
            path: chip_path(&e.tree.materialized, &n.path).ok_or_else(bad)?,
            actor: n.actor.clone(),
            actions: m.actions.clone(),
            probs: n.probs.clone(),
            ev_chips: n.ev_over_P.iter().map(|r| r.iter().map(|x| x * s.pot as f32).collect()).collect(),
            available: n.available.clone(),
        });
    }
    let sol = proto::worker::StreetSolution {
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        requested: 0,
        exploitability_chips: (e.exploitability_over_P * s.pot as f64) as f32,
        iterations: e.iterations,
        memory_bytes: e.memory_bytes,
        mode: e.mode.clone(),
        locks_applied: e.locks_applied,
        export: e.export.clone(),
    };
    proto::worker::validate_solution(&sol, &e.tree.materialized).map_err(|_| bad())?;
    Ok(())
}
