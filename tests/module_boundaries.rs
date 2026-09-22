use std::path::PathBuf;

const EXTRACTED_CRATES: &[&str] = &[
    "access",
    "agent-protocol",
    "agents",
    "contracts",
    "conversation",
    "mcp-server",
    "projects",
    "repository",
    "reviews",
    "runtime",
    "sessions",
    "storage",
    "utility",
];

#[test]
fn each_extracted_module_is_an_independent_crate() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(!root.join("src/modules/mod.rs").exists());
    for name in EXTRACTED_CRATES {
        let crate_root = root.join("crates").join(name);
        assert!(
            crate_root.join("Cargo.toml").is_file(),
            "missing {name} manifest"
        );
        assert!(
            crate_root.join("src/lib.rs").is_file(),
            "missing {name} library"
        );
    }
}

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
