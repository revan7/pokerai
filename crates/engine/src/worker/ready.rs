use proto::worker::{Ready, ADAPTER_VERSION, PROTO_VERSION, SOLVER_COMMIT};

/// Why a worker's `ready` was refused (§4.5): which check failed, with the value the worker reported (follow-up P2.W2).
/// Every one is permanent for that binary: the same build reports the same values, so it is never retried (§12,
/// "until rebuilt"). The message names the check, the reported value and the value required.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadyRefusal {
    #[error("proto_version {reported} != {PROTO_VERSION}")]
    ProtoVersion { reported: u16 },
    #[error("solver commit {reported:?} != pinned {SOLVER_COMMIT}")]
    SolverCommit { reported: String },
    #[error("adapter_version {reported} != {ADAPTER_VERSION}")]
    AdapterVersion { reported: u16 },
    #[error("threads {reported} != requested {requested}")]
    Threads { reported: u8, requested: u8 },
    /// §3.7: `EngineError("worker built without AVX2")`, with the build features the worker reported.
    #[error("worker built without AVX2 (build_features {build_features:?})")]
    NoAvx2 { build_features: Vec<String> },
}

/// §4.5: proto_version 3, the pinned commit, the adapter version, `threads == requested`, `avx2` in build_features.
/// Every comparison is exact: the worker writes these values from the same `proto::worker` constants, so any
/// difference (a padded commit string, an `AVX2` spelled otherwise) is a different build and is never trusted.
/// `cpu_features` is not a refusal (§3.7: a CPU without AVX2 gets a startup banner, `cpu_lacks_avx2`).
pub fn validate_ready(r: &Ready, threads: u8) -> Result<(), ReadyRefusal> {
    if r.proto_version != PROTO_VERSION { return Err(ReadyRefusal::ProtoVersion { reported: r.proto_version }); }
    if r.solver_commit != SOLVER_COMMIT { return Err(ReadyRefusal::SolverCommit { reported: r.solver_commit.clone() }); }
    if r.adapter_version != ADAPTER_VERSION { return Err(ReadyRefusal::AdapterVersion { reported: r.adapter_version }); }
    if r.threads != threads { return Err(ReadyRefusal::Threads { reported: r.threads, requested: threads }); }
    if !r.build_features.iter().any(|f| f == "avx2") { return Err(ReadyRefusal::NoAvx2 { build_features: r.build_features.clone() }); }
    Ok(())
}
pub fn cpu_lacks_avx2(r: &Ready) -> bool { !r.cpu_features.iter().any(|f| f == "avx2") }
