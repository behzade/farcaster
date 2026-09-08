use super::*;

#[test]
fn parses_identity_and_escaped_bookmarks() {
    let identity = parse_identity(
            b"\"commit\"\t\"change\"\t\"line\\tone\"\tfalse\ttrue\n\"topic\"\t\"quote\\\"name\"\n\"conflicted.rs\"\n",
        )
        .expect("identity should parse");
    assert_eq!(identity.description, "line\tone");
    assert_eq!(identity.bookmarks, ["topic", "quote\"name"]);
    assert!(identity.closest_bookmarks.is_empty());
    assert_eq!(identity.conflicted_paths, [PathBuf::from("conflicted.rs")]);
    assert!(!identity.conflicted);
    assert!(identity.empty);
}

#[test]
fn parses_status_and_preserves_rename_source() {
    let changes = parse_status(
            b"\"modified\"\t\"tab\\tname\"\t\"tab\\tname\"\tfalse\tfalse\n\"renamed\"\t\"old\"\t\"new name\"\tfalse\tfalse\n",
        )
        .expect("status should parse");
    assert_eq!(changes[0].relative_path, PathBuf::from("tab\tname"));
    assert_eq!(changes[1].kind, ChangeKind::Renamed);
    assert_eq!(
        changes[1].original_relative_path,
        Some(PathBuf::from("old"))
    );
}

#[test]
fn cross_project_renames_become_scoped_additions_or_deletions() {
    let project = std::path::Path::new("project");
    let moved_out = scope_change_to_project(
        ParsedChange {
            relative_path: PathBuf::from("outside/file.rs"),
            original_relative_path: Some(PathBuf::from("project/file.rs")),
            kind: ChangeKind::Renamed,
        },
        project,
    )
    .expect("in-project deletion");
    assert_eq!(moved_out.relative_path, PathBuf::from("project/file.rs"));
    assert_eq!(moved_out.original_relative_path, None);
    assert_eq!(moved_out.kind, ChangeKind::Deleted);

    let moved_in = scope_change_to_project(
        ParsedChange {
            relative_path: PathBuf::from("project/file.rs"),
            original_relative_path: Some(PathBuf::from("outside/file.rs")),
            kind: ChangeKind::Renamed,
        },
        project,
    )
    .expect("in-project addition");
    assert_eq!(moved_in.relative_path, PathBuf::from("project/file.rs"));
    assert_eq!(moved_in.original_relative_path, None);
    assert_eq!(moved_in.kind, ChangeKind::Added);
}

#[test]
fn literal_filesets_do_not_interpret_path_operators() {
    assert_eq!(
        literal_fileset(std::path::Path::new("a|b")).expect("fileset"),
        "root-file:\"a|b\""
    );
}

#[test]
fn decodes_surrogate_pairs() {
    assert_eq!(
        decode_json_string("\"smile: \\ud83d\\ude00\"").expect("JSON should parse"),
        "smile: 😀"
    );
}
