pub mod cards;
pub mod history;
pub mod memory;
pub mod solve_loop;
#[cfg(test)]
pub mod testutil;
pub mod tree_build;
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

/// Interprets raw CPUID/XCR0 register values into the runtime CPU feature list.
///
/// Factored out of `cpu_features` (which supplies the live register values) so it can be
/// unit-tested with synthetic register states -- including states where the real CPU lacks AVX2
/// -- independent of both the actual host and this crate's own mandatory `+avx2` compile flag.
/// `std::is_x86_feature_detected!("avx2")` cannot be used for this: because the crate is always
/// built with `-C target-feature=+avx2`, the compiler constant-folds that macro call to `true`
/// (documented behaviour; confirmed by the T7-R1 review's LLVM IR probe: `ret i1 true`) without
/// inspecting the running CPU at all, which defeats the spec 3.7 unsupported-CPU diagnostic.
///
/// AVX2 support additionally requires the OS to have enabled saving the extended AVX register
/// state (`OSXSAVE`, checked via `XGETBV(0)`), not just CPU silicon support: a CPU that supports
/// AVX2 but whose OS has not opted in cannot safely execute AVX2 instructions either.
fn features_from(leaf1_ecx: u32, leaf7_ebx: u32, xcr0: u64) -> Vec<String> {
    let mut v = Vec::new();
    let osxsave = (leaf1_ecx >> 27) & 1 != 0; // CPUID.(EAX=1):ECX.OSXSAVE[bit 27]
    let os_saves_avx_state = osxsave && (xcr0 & 0b110) == 0b110; // XGETBV(0): XMM[bit1] and YMM[bit2] state
    let cpu_avx2 = (leaf7_ebx >> 5) & 1 != 0; // CPUID.(EAX=7,ECX=0):EBX.AVX2[bit 5]
    if cpu_avx2 && os_saves_avx_state { v.push("avx2".to_string()); }
    v
}

/// Reads the live CPUID/XCR0 registers and interprets them via `features_from`. These intrinsics
/// are not constant-folded by the `+avx2` build flag (unlike `is_x86_feature_detected!`), so this
/// reports the real running CPU's AVX2 support.
#[cfg(target_arch = "x86_64")]
fn avx2_cpu_feature() -> Vec<String> {
    use std::arch::x86_64::{__cpuid, __cpuid_count};
    // CPUID leaves 1 and 7 are always valid to query on x86_64; `__cpuid`/`__cpuid_count` are safe fns.
    let leaf1 = __cpuid(1);
    let leaf7 = __cpuid_count(7, 0);
    let osxsave = (leaf1.ecx >> 27) & 1 != 0;
    // SAFETY: XGETBV is only executed once CPUID has confirmed the OS set OSXSAVE, per the Intel SDM.
    let xcr0 = if osxsave { unsafe { std::arch::x86_64::_xgetbv(0) } } else { 0 };
    features_from(leaf1.ecx, leaf7.ebx, xcr0)
}

pub fn cpu_features() -> Vec<String> {
    let mut v = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        v.extend(avx2_cpu_feature());
        // `fma`/`avx512f` are not forced on by `.cargo/config.toml` (only `avx2` is), so the
        // compiler cannot constant-fold these two -- the macro still queries the real CPU for them.
        if std::is_x86_feature_detected!("fma") { v.push("fma".to_string()); }
        if std::is_x86_feature_detected!("avx512f") { v.push("avx512f".to_string()); }
    }
    v
}

#[cfg(test)]
mod cpu_feature_tests {
    use super::features_from;

    #[test]
    fn avx2_absent_when_cpu_bit_clear() {
        // CPU does not report AVX2 in CPUID leaf 7 EBX bit 5, even though the OS fully supports it.
        let leaf1_ecx = 1 << 27; // OSXSAVE set
        let leaf7_ebx = 0; // AVX2 bit clear
        let xcr0 = 0b110; // XMM + YMM state saved
        assert!(features_from(leaf1_ecx, leaf7_ebx, xcr0).is_empty());
    }

    #[test]
    fn avx2_absent_when_cpu_bit_set_but_os_does_not_save_avx_state() {
        // CPU silicon supports AVX2 but OSXSAVE is clear: the OS never opted in, so AVX2 use is unsafe.
        let leaf1_ecx = 0; // OSXSAVE clear
        let leaf7_ebx = 1 << 5; // AVX2 bit set
        let xcr0 = 0; // irrelevant when OSXSAVE is clear
        assert!(features_from(leaf1_ecx, leaf7_ebx, xcr0).is_empty());
    }

    #[test]
    fn avx2_present_when_cpu_and_os_both_support_it() {
        let leaf1_ecx = 1 << 27; // OSXSAVE set
        let leaf7_ebx = 1 << 5; // AVX2 bit set
        let xcr0 = 0b110; // XMM + YMM state saved
        let features = features_from(leaf1_ecx, leaf7_ebx, xcr0);
        assert!(features.iter().any(|f| f == "avx2"));
    }
}

pub fn ready_message(threads: u8) -> WorkerMessage {
    WorkerMessage::Ready(Ready { proto_version: PROTO_VERSION, solver_commit: SOLVER_COMMIT.to_string(), adapter_version: ADAPTER_VERSION, threads,
        build_features: build_features(), cpu_features: cpu_features(), capabilities: CAPABILITIES.iter().map(|s| s.to_string()).collect() })
}
