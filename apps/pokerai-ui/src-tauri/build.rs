fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    proto::bindings::write_to(&manifest.join("../src/ipc/types.gen.ts")).unwrap();
    println!("cargo:rerun-if-changed=../../../crates/proto/src");
    tauri_build::build();
}
