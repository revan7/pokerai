//! Worker protocol (spec 4.5): UTF-8 JSON Lines, `type`-tagged, lowercase tags, unknown fields rejected.
use serde::de::Error as DeError;
use serde::ser::Error as SerError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use crate::cards::Card;
use crate::hand::Action;
use crate::range::Range1326;
use crate::tree::EffectiveTree;

// --- Numeric wire validation (spec 4.5, standing ruling: validate wide, never clamp) ---
//
// The machinery itself -- the domain predicates and the `narrow_checked` / `widen_checked` pair --
// lives in `crate::numeric`, which is where every other wire in this crate also draws it from
// (review S1). What stays here is the mapping from *this* wire's field names to those domains, so
// a worker-protocol error still names the worker-protocol field.

use crate::numeric::{
    deserialize_matrix, domain_finite, domain_unit_interval, narrow_checked, serialize_matrix, widen_checked,
};

/// Scalar, finite-only float (no domain bound beyond finiteness): used for
/// `StreetSolution::exploitability_chips`, which is always present and never null.
fn deserialize_finite_f32<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    let raw = f64::deserialize(d)?;
    narrow_checked(raw, domain_finite, "exploitability_chips").map_err(DeError::custom)
}
fn serialize_finite_f32<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    let checked = widen_checked(*v, domain_finite, "exploitability_chips").map_err(SerError::custom)?;
    s.serialize_f32(checked)
}

/// Required-but-nullable scalar float: used for `WorkerMessage::Progress::exploitability_chips`
/// (spec 4.5: always present on the wire, `null` until measured). Unlike a plain `Option<f32>`
/// field, this does not default a missing key to `None` -- the key itself is required, and only
/// its value may legitimately be `null`.
fn deserialize_required_nullable_f32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    let raw = Option::<f64>::deserialize(d)?;
    match raw {
        None => Ok(None),
        Some(x) => narrow_checked(x, domain_finite, "exploitability_chips").map(Some).map_err(DeError::custom),
    }
}
fn serialize_required_nullable_f32<S: Serializer>(v: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        None => s.serialize_none(),
        Some(x) => {
            let checked = widen_checked(*x, domain_finite, "exploitability_chips").map_err(SerError::custom)?;
            s.serialize_some(&checked)
        }
    }
}

fn deserialize_prob_matrix<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<f32>>, D::Error> {
    deserialize_matrix(d, domain_unit_interval, "probability")
}
fn serialize_prob_matrix<S: Serializer>(v: &Vec<Vec<f32>>, s: S) -> Result<S::Ok, S::Error> {
    serialize_matrix(v, s, domain_unit_interval, "probability")
}
fn deserialize_finite_matrix<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<f32>>, D::Error> {
    deserialize_matrix(d, domain_finite, "ev_chips")
}
fn serialize_finite_matrix<S: Serializer>(v: &Vec<Vec<f32>>, s: S) -> Result<S::Ok, S::Error> {
    serialize_matrix(v, s, domain_finite, "ev_chips")
}

pub const REQUEST_LINE_MAX: usize = 1 << 20;
pub const RESULT_LINE_MAX: usize = 16 << 20;
pub const MAX_EXPORTED_NODES: usize = 100_000;
pub const FAILURE_CODES: [&str; 7] = ["invalid_request", "tree_mismatch", "tree_too_large", "out_of_memory", "lock_mismatch", "no_iteration", "internal"];

/// Wire protocol version, re-exported here so consumers import one `proto::worker::*` set.
pub use crate::PROTO_VERSION;
/// Pinned upstream solver commit (spec 3.7). The single definition in the workspace:
/// `solver-worker` re-exports it and checks its vendored `PINNED_COMMIT` file against it.
pub const SOLVER_COMMIT: &str = "9d1509fe5077d019825f833eed04b16d342dfda1";
/// Version of the worker's library adapter (spec 4.5 `ready`, spec 10.4 cache key).
pub const ADAPTER_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolveRequest {
    pub id: String, pub spot: String, pub board: Vec<Card>, pub oop_range: Range1326, pub ip_range: Range1326,
    pub pot: u32, pub stack_oop: u32, pub stack_ip: u32,
    #[serde(with = "crate::numeric::rake_rate")]
    pub rake_rate: f32,
    pub rake_cap_mchips: u32,
    pub tree: EffectiveTree, pub history: Vec<Action>, pub target_bp: u16, pub deadline_ms: u32,
    pub extraction_margin_ms: u32, pub memory_limit_bytes: u64, pub background: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeLock {
    pub path: Vec<Action>, pub actor: String,
    #[serde(deserialize_with = "deserialize_prob_matrix", serialize_with = "serialize_prob_matrix")]
    pub probs: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum EngineMessage {
    Solve(SolveRequest),
    Lock { id: String, spot: String, locks: Vec<NodeLock> },
    Cancel { id: String, target: String },
    Shutdown { id: String },
}

/// The `ready` payload (spec 4.5). Named `Ready` because plans 2 and 5 consume it under that name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    pub proto_version: u16, pub solver_commit: String, pub adapter_version: u16, pub threads: u8,
    pub build_features: Vec<String>, pub cpu_features: Vec<String>, pub capabilities: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckStatus { Accepted, Staged, Rejected, AlreadyFinished, UnknownTarget }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage { Building, Solving, Extracting }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus { Ok, BestSoFar, Cancelled, Error }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerError {
    pub code: String, pub message: String, pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeStrategy {
    pub path: Vec<Action>, pub actor: String, pub actions: Vec<Action>,
    #[serde(deserialize_with = "deserialize_prob_matrix", serialize_with = "serialize_prob_matrix")]
    pub probs: Vec<Vec<f32>>,
    #[serde(deserialize_with = "deserialize_finite_matrix", serialize_with = "serialize_finite_matrix")]
    pub ev_chips: Vec<Vec<f32>>,
    pub available: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreetSolution {
    pub nodes: Vec<NodeStrategy>, pub requested: u32,
    #[serde(deserialize_with = "deserialize_finite_f32", serialize_with = "serialize_finite_f32")]
    pub exploitability_chips: f32,
    pub iterations: u32,
    pub memory_bytes: u64, pub mode: String, pub locks_applied: u16, pub export: String, pub covered_paths: Vec<Vec<Action>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum WorkerMessage {
    Ready(Ready),
    Ack {
        id: String, status: AckStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")] reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] replaced: Option<bool>,
    },
    Progress {
        id: String, stage: Stage, iterations: u32,
        #[serde(deserialize_with = "deserialize_required_nullable_f32", serialize_with = "serialize_required_nullable_f32")]
        exploitability_chips: Option<f32>,
        elapsed_ms: u32, memory_bytes: u64,
    },
    Result {
        id: String, status: ResultStatus, elapsed_ms: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")] solution: Option<StreetSolution>,
        #[serde(default, skip_serializing_if = "Option::is_none")] error: Option<WorkerError>,
    },
}

// --- Structural validation (spec 4.5), deliberately outside the wire codecs above ---
//
// The `narrow_checked`/`widen_checked`/`deserialize_matrix` machinery already guarantees every
// float on the wire is finite and, for probability matrices, in the unit interval -- on both
// serde directions, independent of how the value was constructed. What is left for this task is
// structural: matrix shape against `actions.len()` and against `COMBOS` rows, per-row probability
// sums, `available`/lock free-combo consistency, node-count and `requested` bounds, and path/actor
// agreement against the materialized tree. Validation is always done in wide form (f64 for the
// row sum) before any comparison against tolerance, per the standing ruling; the individual `f32`
// entries are already domain-checked by the wire codecs, so this module reads them, never clamps.

use crate::cards::COMBOS;
use crate::tree::{index_materialized, resolve_chip_path_indexed, MaterializedIndex, MaterializedNode, OrdinalPath};

const ROW_TOLERANCE: f32 = 1e-3;

/// Resolves one node against an index built **once** per validation call (review S2).
///
/// Both the resolution and the lookup of the resolved node go through that one index: rebuilding it
/// per node, and then scanning the tree linearly for the resolved ordinal, made validation
/// Theta(nodes x materialized) against a declared cap of `MAX_EXPORTED_NODES = 100_000` nodes,
/// inside the spec section 7 fast-path budget.
fn resolve_node<'a>(index: &MaterializedIndex<'a>, what: &str, k: usize, path: &[Action], actor: &str) -> Result<(&'a MaterializedNode, OrdinalPath), String> {
    let ordinal = resolve_chip_path_indexed(index, path).ok_or_else(|| format!("{what} {k}: chip path does not resolve against the materialized tree"))?;
    let node = *index.get(ordinal.as_slice()).expect("resolved paths are materialized");
    if node.actor != actor { return Err(format!("{what} {k}: actor {actor:?} differs from the materialized actor {:?}", node.actor)); }
    Ok((node, ordinal))
}

fn check_row(what: &str, k: usize, combo: usize, row: &[f32], width: usize, allow_all_zero: bool, must_be_zero: bool) -> Result<(), String> {
    if row.len() != width { return Err(format!("{what} {k} combo {combo}: row has {} entries, expected {width}", row.len())); }
    // Standing ruling: validate in wide form (f64) before narrowing or comparing sums. Each
    // entry is already an in-domain f32 (checked by the wire codecs); widening to f64 for the
    // accumulation and the tolerance comparison avoids compounding f32 rounding error across a
    // long row before it is checked against `ROW_TOLERANCE`.
    let mut sum = 0.0f64;
    for x in row {
        if !x.is_finite() { return Err(format!("{what} {k} combo {combo}: non-finite entry")); }
        if *x < 0.0 || *x > 1.0 { return Err(format!("{what} {k} combo {combo}: probability {x} outside [0, 1]")); }
        sum += *x as f64;
    }
    if must_be_zero {
        if sum != 0.0 { return Err(format!("{what} {k} combo {combo}: unavailable combo has a non-zero probability row")); }
        return Ok(());
    }
    if allow_all_zero && sum == 0.0 { return Ok(()); }
    if (sum - 1.0).abs() > ROW_TOLERANCE as f64 { return Err(format!("{what} {k} combo {combo}: probability row sums to {sum}, expected 1")); }
    Ok(())
}

/// Spec 4.5 matrix and structure validation; returns the ordinal path of every node.
pub fn validate_solution(sol: &StreetSolution, materialized: &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String> {
    if sol.nodes.is_empty() { return Err("solution has no nodes".into()); }
    if sol.nodes.len() > MAX_EXPORTED_NODES { return Err(format!("{} nodes exceed the export limit {MAX_EXPORTED_NODES}", sol.nodes.len())); }
    if sol.requested as usize >= sol.nodes.len() { return Err(format!("requested {} is not below nodes.len() {}", sol.requested, sol.nodes.len())); }
    if sol.covered_paths.len() != sol.nodes.len() { return Err(format!("covered_paths has {} entries for {} nodes", sol.covered_paths.len(), sol.nodes.len())); }
    if !sol.exploitability_chips.is_finite() || sol.exploitability_chips < 0.0 { return Err("exploitability_chips must be finite and non-negative".into()); }
    if sol.mode != "f32" && sol.mode != "i16" { return Err(format!("unknown mode {:?}", sol.mode)); }
    if sol.export != "street" && sol.export != "truncated" { return Err(format!("unknown export {:?}", sol.export)); }
    // One index for the whole call, not one per node (review S2).
    let index = index_materialized(materialized);
    let mut out = Vec::with_capacity(sol.nodes.len());
    for (k, node) in sol.nodes.iter().enumerate() {
        if sol.covered_paths[k] != node.path { return Err(format!("covered_paths[{k}] differs from nodes[{k}].path")); }
        let (m, ordinal) = resolve_node(&index, "node", k, &node.path, &node.actor)?;
        if m.actions != node.actions { return Err(format!("node {k}: actions differ from the materialized menu")); }
        let width = node.actions.len();
        if node.probs.len() != COMBOS || node.ev_chips.len() != COMBOS || node.available.len() != COMBOS {
            return Err(format!("node {k}: matrices must have exactly 1326 rows"));
        }
        // Spec section 2: `EV(fold) = 0` exactly -- chips already in the pot are sunk. Enforced
        // as a bit-exact comparison to positive zero (never `-0.0`, never a rounding-error
        // near-zero), on every available row; unavailable rows are already required all-zero above.
        let fold_cols: Vec<usize> = node.actions.iter().enumerate().filter(|(_, a)| **a == Action::Fold).map(|(i, _)| i).collect();
        for c in 0..COMBOS {
            check_row("node", k, c, &node.probs[c], width, false, !node.available[c])?;
            let ev = &node.ev_chips[c];
            if ev.len() != width { return Err(format!("node {k} combo {c}: ev row has {} entries, expected {width}", ev.len())); }
            if ev.iter().any(|x| !x.is_finite()) { return Err(format!("node {k} combo {c}: non-finite ev")); }
            if !node.available[c] && ev.iter().any(|x| *x != 0.0) { return Err(format!("node {k} combo {c}: unavailable combo has a non-zero ev row")); }
            if node.available[c] {
                for &fi in &fold_cols {
                    if ev[fi].to_bits() != 0.0f32.to_bits() {
                        return Err(format!("node {k} combo {c} action {fi}: fold EV must be exactly 0.0 (chips already in the pot are sunk), got {}", ev[fi]));
                    }
                }
            }
        }
        out.push(ordinal);
    }
    Ok(out)
}

/// Lock validation (spec 4.5): every entry in [0, 1]; a row is all zero (the free-combo sentinel) or sums to 1 +- 1e-3.
pub fn validate_locks(locks: &[NodeLock], materialized: &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String> {
    // One index for the whole call, not one per lock (review S2).
    let index = index_materialized(materialized);
    let mut out = Vec::with_capacity(locks.len());
    for (k, lock) in locks.iter().enumerate() {
        let (m, ordinal) = resolve_node(&index, "lock", k, &lock.path, &lock.actor)?;
        if lock.probs.len() != COMBOS { return Err(format!("lock {k}: probs must have exactly 1326 rows")); }
        for (c, row) in lock.probs.iter().enumerate() { check_row("lock", k, c, row, m.actions.len(), true, false)?; }
        out.push(ordinal);
    }
    Ok(out)
}
