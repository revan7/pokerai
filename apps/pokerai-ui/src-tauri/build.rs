fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    proto::bindings::write_to(&manifest.join("../src/ipc/types.gen.ts")).unwrap();
    println!("cargo:rerun-if-changed=../../../crates/proto/src");

    // On Windows/MSVC, embed the Common-Controls-v6 manifest ourselves, via
    // plain linker flags applied uniformly to every link target this package
    // builds (the app `bin`, and the `--lib`/`--test` unittest harness),
    // instead of `tauri_build::build()`'s default (which embeds it as a
    // compiled resource scoped only to the `bin` target).
    //
    // Why: `tauri::Builder`/`tauri::test::MockRuntime` (wired in for *any*
    // `Builder`, not just a real Wry runtime -- menu/window plumbing lives
    // behind `muda`/`tauri-runtime-wry`, linked in regardless) statically
    // needs Common-Controls-v6-only `comctl32.dll` exports (e.g.
    // `TaskDialogIndirect`). tauri-build's default manifest embedding never
    // reaches Cargo's `--test`/unittest binaries (only the `bin` target), so
    // without it the loader binds the legacy (v5.82) `comctl32.dll` before
    // `main` runs and the process aborts with `STATUS_ENTRYPOINT_NOT_FOUND`
    // (0xc0000139) -- confirmed upstream, unfixed as of tauri 2.11.5:
    // tauri-apps/tauri#13419, #11028, #13954, #13948, discussion #11179.
    //
    // Fix pattern (ikenga-hq/ikenga#189, PR #205): disable tauri-build's own
    // resource-embedded manifest for the `bin` target
    // (`WindowsAttributes::new_without_app_manifest()`) and instead emit the
    // *same* manifest via MSVC's `/MANIFEST:EMBED` + `/MANIFESTINPUT:<file>`
    // linker flags with Cargo's unqualified `rustc-link-arg` instruction,
    // which (per the Cargo book) applies to every binary, cdylib, example and
    // test target the package builds -- including the unittest harness.
    //
    // Two scoped alternatives were tried and empirically rejected before
    // this one:
    //   - `cargo:rustc-link-arg-tests=...`: Cargo hard-rejects this
    //     ("does not have a test target") because this crate's tests are
    //     `#[cfg(test)] mod tests;` inside `src/lib.rs`, not a `[[test]]`
    //     integration-test target under `tests/`; adding a throwaway
    //     `tests/*.rs` file made Cargo accept the instruction, but it then
    //     only linked *that* file's own binary, never `src/lib.rs`'s
    //     unittest harness.
    //   - `cargo:rustc-link-arg-bins=...` while *also* leaving
    //     `tauri_build::build()`'s default manifest in place: this hits the
    //     `bin` target and fails the link with
    //     `CVTRES : fatal error CVT1100: duplicate resource. type:MANIFEST,
    //     name:1, language:0x0409` (two manifest resources on the same
    //     binary) -- exactly the duplicate-manifest risk to avoid. Disabling
    //     tauri-build's own manifest first (below) removes that duplicate,
    //     so the single, uniformly-applied manifest is safe for every
    //     target, `bin` included.
    let is_windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");

    if is_windows_msvc {
        let test_manifest = manifest.join("windows-test-manifest.xml");
        println!("cargo:rerun-if-changed=windows-test-manifest.xml");

        tauri_build::try_build(
            tauri_build::Attributes::new()
                .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
        )
        .expect("tauri_build::try_build");

        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
            test_manifest.display()
        );
    } else {
        tauri_build::build();
    }
}
