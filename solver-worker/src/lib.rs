pub mod cards;
pub mod win;

use proto::worker::{Ready, WorkerMessage, ADAPTER_VERSION, PROTO_VERSION};

pub const CAPABILITIES: [&str; 5] = ["solve", "lock", "cancel", "street_export", "i16"];

/// The pinned commit is owned by `proto::worker` (plan 1 Task 7) because the engine validates `ready`
/// against it and may not depend on this crate (spec §3.2). Re-exported, never redefined.
pub use proto::worker::SOLVER_COMMIT;

/// What is actually vendored on disk, read at build time. A mismatch with `SOLVER_COMMIT` is a build error:
/// the `ready` message must never claim a commit the binary was not built from.
const VENDORED_COMMIT: &str = include_str!("../../third_party/postflop-solver/PINNED_COMMIT");
const _: () = {
    // `const` string comparison: same length and same bytes.
    let (a, b) = (VENDORED_COMMIT.as_bytes(), SOLVER_COMMIT.as_bytes());
    assert!(a.len() >= b.len(), "third_party/postflop-solver/PINNED_COMMIT is shorter than proto::worker::SOLVER_COMMIT");
    let mut i = 0;
    while i < b.len() { assert!(a[i] == b[i], "vendored commit differs from proto::worker::SOLVER_COMMIT"); i += 1; }
};

pub fn build_features() -> Vec<String> {
    let mut v = Vec::new();
    if cfg!(target_feature = "avx2") { v.push("avx2".to_string()); }
    if cfg!(target_feature = "fma") { v.push("fma".to_string()); }
    if cfg!(target_feature = "avx512f") { v.push("avx512f".to_string()); }
    v
}

pub fn cpu_features() -> Vec<String> {
    let mut v = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") { v.push("avx2".to_string()); }
        if std::is_x86_feature_detected!("fma") { v.push("fma".to_string()); }
        if std::is_x86_feature_detected!("avx512f") { v.push("avx512f".to_string()); }
    }
    v
}

pub fn ready_message(threads: u8) -> WorkerMessage {
    WorkerMessage::Ready(Ready { proto_version: PROTO_VERSION, solver_commit: SOLVER_COMMIT.to_string(), adapter_version: ADAPTER_VERSION, threads,
        build_features: build_features(), cpu_features: cpu_features(), capabilities: CAPABILITIES.iter().map(|s| s.to_string()).collect() })
}
