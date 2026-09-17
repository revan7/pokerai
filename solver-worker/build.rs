fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_FEATURE");
    let features = std::env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();
    if !features.split(',').any(|f| f == "avx2") {
        panic!("solver-worker requires AVX2 (spec 3.7): build with -C target-feature=+avx2 via .cargo/config.toml; CARGO_CFG_TARGET_FEATURE was {features:?}");
    }
}
