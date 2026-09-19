use super::*;

fn review() -> Review {
    Review {
        title: "Check error handling".into(),
        items: vec![ReviewLocation {
            path: "src/main.rs".into(),
            start_line: Some(2),
            end_line: Some(4),
            note: "Check fallback".into(),
        }],
    }
}

#[test]
fn validates_review_boundaries() {
    assert!(review().validate().is_ok());
    for path in [
        "../outside",
        "/etc/passwd",
        "src/../../outside",
        "",
        "src/line\nfile",
    ] {
        let mut value = review();
        value.items[0].path = path.into();
        assert!(value.validate().is_err(), "{path}");
    }
    for (start, end) in [(Some(0), None), (None, Some(2)), (Some(4), Some(2))] {
        let mut value = review();
        value.items[0].start_line = start;
        value.items[0].end_line = end;
        assert!(value.validate().is_err());
    }
    let mut value = review();
    value.items = vec![value.items[0].clone(); 101];
    assert!(value.validate().is_err());
    value.items.clear();
    assert!(value.validate().is_err());
}

#[test]
fn resolves_missing_files_but_rejects_escaping_symlinks() {
    let project = tempfile::tempdir().expect("temporary project");
    let outside = tempfile::tempdir().expect("outside directory");
    std::fs::write(project.path().join("file.rs"), "code").expect("write fixture");
    let existing = resolve_path(project.path(), "file.rs").expect("resolve fixture");
    assert!(existing.is_file());
    assert_eq!(
        existing.as_os_str(),
        project
            .path()
            .canonicalize()
            .expect("canonical project")
            .join("file.rs")
            .as_os_str()
    );
    assert_eq!(
        resolve_path(project.path(), "missing/file.rs").expect("resolve missing file"),
        project
            .path()
            .canonicalize()
            .expect("canonical project")
            .join("missing/file.rs")
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), project.path().join("escape"))
            .expect("create escaping symlink");
        assert!(resolve_path(project.path(), "escape/missing.rs").is_err());
    }
}
