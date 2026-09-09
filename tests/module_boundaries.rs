use std::path::{Path, PathBuf};

// Dependencies may point toward earlier modules, never back toward callers.
// Agents use session data and project trust; sessions remain backend-neutral.
const MODULE_ORDER: &[&str] = &["sessions", "access", "projects", "repository", "agents"];

#[test]
fn capability_modules_do_not_form_dependency_cycles() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/modules");
    assert_module_boundaries(&root)
}

fn assert_module_boundaries(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for (index, module) in MODULE_ORDER.iter().enumerate() {
        let mut forbidden = vec!["crate::app".to_owned()];
        for dependency in &MODULE_ORDER[index + 1..] {
            forbidden.push(format!("crate::{dependency}"));
            forbidden.push(format!("crate::modules::{dependency}"));
        }
        assert_tree_excludes(&root.join(module), &forbidden)?;
    }
    Ok(())
}

fn assert_tree_excludes(
    root: &Path,
    forbidden: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            // Integration tests can exercise both sides of a production boundary.
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs")
                || name == "tests.rs"
                || name.ends_with("_tests.rs")
            {
                continue;
            }
            let source = std::fs::read_to_string(&path)?;
            for dependency in forbidden {
                if source.contains(dependency) {
                    return Err(format!(
                        "{} imports forbidden boundary {dependency}",
                        path.display()
                    )
                    .into());
                }
            }
        }
    }
    Ok(())
}

#[test]
fn boundaries_allow_one_way_dependencies_but_reject_cycles_and_ui_imports()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    for module in MODULE_ORDER {
        std::fs::create_dir(root.path().join(module))?;
    }
    std::fs::write(
        root.path().join("agents/mod.rs"),
        "use crate::sessions::SessionTarget;",
    )?;
    std::fs::write(
        root.path().join("agents/bridge_tests.rs"),
        "use crate::app::FarcasterApp;",
    )?;
    assert_module_boundaries(root.path())?;

    let sessions = root.path().join("sessions/mod.rs");
    std::fs::write(&sessions, "use crate::modules::agents::SessionCommand;")?;
    assert!(assert_module_boundaries(root.path()).is_err());
    std::fs::write(sessions, "")?;
    std::fs::write(
        root.path().join("agents/mod.rs"),
        "use crate::app::FarcasterApp;",
    )?;
    assert!(assert_module_boundaries(root.path()).is_err());
    Ok(())
}
