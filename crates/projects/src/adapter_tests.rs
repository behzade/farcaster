use super::*;

#[test]
fn temporary_path_filter_is_scoped_to_the_configured_root() -> Result<(), std::io::Error> {
    let root = tempfile::tempdir()?;
    let project = root.path().join("project");
    fs::create_dir(&project)?;

    assert!(path_is_within(&project, root.path()));
    assert!(!path_is_within(
        root.path().parent().unwrap_or(root.path()),
        root.path()
    ));
    Ok(())
}
