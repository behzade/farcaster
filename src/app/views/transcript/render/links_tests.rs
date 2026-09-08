use super::*;

#[test]
fn file_links_resolve_paths_and_source_lines() {
    for (link, expected, line) in [
        ("src/main.rs", "/project/src/main.rs", None),
        ("../shared.rs:12", "/shared.rs", Some(12)),
        (
            "/project/src/main.rs:12:3",
            "/project/src/main.rs",
            Some(12),
        ),
        ("src/main.rs#L12-L18", "/project/src/main.rs", Some(12)),
        ("file:///project/a%20b.rs#L4", "/project/a b.rs", Some(4)),
        ("docs/a%20b.md:7", "/project/docs/a b.md", Some(7)),
        ("README.md#heading", "/project/README.md", None),
    ] {
        assert_eq!(
            local_file_target(link, Path::new("/project")),
            Some((PathBuf::from(expected), line)),
            "{link}"
        );
    }
}

#[test]
fn external_links_and_anchors_are_not_editor_targets() {
    for link in [
        "https://example.com/file.rs#L12",
        "http://localhost:3000",
        "mailto:user@example.com",
        "vscode://file/project/main.rs:12",
        "//example.com/path",
        "file://remote-server/project/file.rs",
        "#heading",
        "",
    ] {
        assert_eq!(
            local_file_target(link, Path::new("/project")),
            None,
            "{link}"
        );
    }
}
