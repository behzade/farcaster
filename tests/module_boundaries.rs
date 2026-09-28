use std::path::PathBuf;

#[test]
fn contracts_has_no_local_dependencies() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("crates/contracts/Cargo.toml"),
    )?;
    let dependencies = manifest
        .split_once("[dependencies]")
        .ok_or("contracts manifest has no dependency table")?
        .1
        .split("\n[")
        .next()
        .unwrap_or_default();
    assert!(
        !dependencies.contains("path ="),
        "contracts must not depend on another local crate"
    );
    Ok(())
}
