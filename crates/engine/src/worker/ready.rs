use proto::worker::{Ready, ADAPTER_VERSION, PROTO_VERSION, SOLVER_COMMIT};

/// §4.5: proto_version 3, the pinned commit, the adapter version, `threads == requested`, `avx2` in build_features.
/// Every comparison is exact: the worker writes these values from the same `proto::worker` constants, so any
/// difference (a padded commit string, an `AVX2` spelled otherwise) is a different build and is never trusted.
/// `cpu_features` is not a refusal (§3.7: a CPU without AVX2 gets a startup banner, `cpu_lacks_avx2`).
pub fn validate_ready(r: &Ready, threads: u8) -> Result<(), String> {
    if r.proto_version != PROTO_VERSION { return Err(format!("proto_version {} != {}", r.proto_version, PROTO_VERSION)); }
    if r.solver_commit != SOLVER_COMMIT { return Err(format!("solver commit {:?} != pinned {}", r.solver_commit, SOLVER_COMMIT)); }
    if r.adapter_version != ADAPTER_VERSION { return Err(format!("adapter_version {} != {}", r.adapter_version, ADAPTER_VERSION)); }
    if r.threads != threads { return Err(format!("threads {} != requested {}", r.threads, threads)); }
    if !r.build_features.iter().any(|f| f == "avx2") { return Err("worker built without AVX2".into()); }
    Ok(())
}
pub fn cpu_lacks_avx2(r: &Ready) -> bool { !r.cpu_features.iter().any(|f| f == "avx2") }
