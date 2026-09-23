use super::*;

#[test]
fn choices_follow_available_executables() -> Result<(), Box<dyn std::error::Error>> {
    let project = tempfile::tempdir()?;
    let bin = project.path().join("bin");
    std::fs::create_dir(&bin)?;
    assert!(!EditorChoice::VsCode.available(project.path(), Some(bin.as_os_str())));
    assert!(!EditorChoice::Helix.available(project.path(), Some(bin.as_os_str())));
    assert!(!EditorChoice::Zed.available(project.path(), Some(bin.as_os_str())));

    for program in ["code", "hx", "zeditor"] {
        let path = bin.join(program);
        std::fs::write(&path, "")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    assert!(EditorChoice::VsCode.available(project.path(), Some(bin.as_os_str())));
    assert!(EditorChoice::Helix.available(project.path(), Some(bin.as_os_str())));
    assert!(EditorChoice::Zed.available(project.path(), Some(bin.as_os_str())));
    assert_eq!(
        EditorChoice::Zed.program(project.path(), Some(bin.as_os_str())),
        PathBuf::from("zeditor")
    );
    Ok(())
}
