use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = manifest_dir.join("../src/lib/ipc.ts");

    syncup_lib::specta_builder().export(specta_typescript::Typescript::default(), output)?;

    println!("Generated src/lib/ipc.ts");
    Ok(())
}
