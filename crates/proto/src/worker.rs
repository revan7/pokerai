//! Worker protocol (spec 4.5): UTF-8 JSON Lines, `type`-tagged, lowercase tags, unknown fields rejected.
use serde::{Deserialize, Serialize};
use crate::cards::Card;
use crate::hand::Action;
use crate::range::Range1326;
use crate::tree::EffectiveTree;

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
    pub pot: u32, pub stack_oop: u32, pub stack_ip: u32, pub rake_rate: f32, pub rake_cap_mchips: u32,
    pub tree: EffectiveTree, pub history: Vec<Action>, pub target_bp: u16, pub deadline_ms: u32,
    pub extraction_margin_ms: u32, pub memory_limit_bytes: u64, pub background: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeLock { pub path: Vec<Action>, pub actor: String, pub probs: Vec<Vec<f32>> }

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
    pub probs: Vec<Vec<f32>>, pub ev_chips: Vec<Vec<f32>>, pub available: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreetSolution {
    pub nodes: Vec<NodeStrategy>, pub requested: u32, pub exploitability_chips: f32, pub iterations: u32,
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
    Progress { id: String, stage: Stage, iterations: u32, exploitability_chips: Option<f32>, elapsed_ms: u32, memory_bytes: u64 },
    Result {
        id: String, status: ResultStatus, elapsed_ms: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")] solution: Option<StreetSolution>,
        #[serde(default, skip_serializing_if = "Option::is_none")] error: Option<WorkerError>,
    },
}
