use super::*;
use tempfile::tempdir;

#[test]
fn editor_completion_is_scoped_to_its_request_session_and_view() {
    assert!(editor_completion_is_current(
        1,
        1,
        11,
        Some(11),
        AppSurface::Editor
    ));
    for (generation, tab, surface) in [
        (2, Some(11), AppSurface::Editor),
        (1, Some(22), AppSurface::Editor),
        (1, None, AppSurface::Editor),
        (1, Some(11), AppSurface::Chat),
        (1, Some(11), AppSurface::Terminal),
        (1, Some(11), AppSurface::Work),
    ] {
        assert!(!editor_completion_is_current(
            1, generation, 11, tab, surface
        ));
    }
}

#[test]
fn editor_paths_allow_targets_outside_the_selected_project()
-> Result<(), Box<dyn std::error::Error>> {
    let project = tempdir()?;
    let file = project.path().join("src.rs");
    std::fs::write(&file, "fn main() {}")?;
    assert_eq!(
        resolve_editor_path(project.path(), Path::new("src.rs"))?,
        file.canonicalize()?
    );
    assert_eq!(
        resolve_editor_path(project.path(), Path::new("deleted.rs"))?,
        project.path().canonicalize()?.join("deleted.rs")
    );
    let outside = tempdir()?;
    let outside_file = outside.path().join("outside.rs");
    std::fs::write(&outside_file, "")?;
    assert_eq!(
        resolve_editor_path(project.path(), &outside_file)?,
        outside_file.canonicalize()?
    );
    let new_outside_file = outside.path().join("new.rs");
    assert_eq!(
        resolve_editor_path(project.path(), &new_outside_file)?,
        outside.path().canonicalize()?.join("new.rs")
    );
    #[cfg(unix)]
    {
        let dangling = project.path().join("dangling.rs");
        std::os::unix::fs::symlink(outside.path().join("missing.rs"), &dangling)?;
        assert!(resolve_editor_path(project.path(), &dangling).is_err());
    }
    Ok(())
}
