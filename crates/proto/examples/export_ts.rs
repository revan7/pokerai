fn main() -> std::io::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    proto::bindings::write_to(&root.join("apps/pokerai-ui/src/ipc/types.gen.ts"))
}
