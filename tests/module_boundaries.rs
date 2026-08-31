use std::path::{Path, PathBuf};

#[test]
fn capability_modules_do_not_form_dependency_cycles() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/modules");
    assert_tree_excludes(
        &root.join("agents"),
        &["crate::sessions", "crate::modules::sessions", "crate::app"],
    )?;
    assert_tree_excludes(
        &root.join("sessions"),
        &["crate::agents", "crate::modules::agents", "crate::app"],
    )?;
    assert_tree_excludes(&root.join("access"), &["crate::app"])?;
    assert_tree_excludes(&root.join("projects"), &["crate::app"])?;
    assert_tree_excludes(&root.join("repository"), &["crate::app"])?;
    Ok(())
}

fn assert_tree_excludes(root: &Path, forbidden: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path)?;
            for dependency in forbidden {
                assert!(
                    !source.contains(dependency),
                    "{} imports forbidden boundary {dependency}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}
