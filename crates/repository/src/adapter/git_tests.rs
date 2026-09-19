use super::*;

#[test]
fn parses_separate_layers_renames_untracked_and_conflicts() {
    let input = b"# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 MM N... 100644 100644 100644 aaaaaa bbbbbb src/lib.rs\0\
2 R. N... 100644 100644 100644 aaaaaa bbbbbb R100 new name.rs\0old.rs\0\
? untracked file\0\
u UU N... 100644 100644 100644 100644 aaaaaa bbbbbb cccccc conflict.txt\0";
    let (identity, changes) = parse_status(input).expect("status should parse");
    assert_eq!((identity.ahead, identity.behind), (2, 1));
    assert_eq!(changes.len(), 5);
    assert_eq!(changes[0].layer, ChangeLayer::GitIndex);
    assert_eq!(changes[1].layer, ChangeLayer::GitWorkingTree);
    assert_eq!(changes[2].kind, ChangeKind::Renamed);
    assert_eq!(
        changes[2].original_relative_path,
        Some(PathBuf::from("old.rs"))
    );
    assert_eq!(changes[3].kind, ChangeKind::Untracked);
    assert_eq!(changes[4].kind, ChangeKind::Conflict);
}

#[cfg(unix)]
#[test]
fn preserves_non_utf8_nul_paths() {
    use std::os::unix::ffi::OsStrExt as _;

    let (_, changes) = parse_status(b"? invalid-\xff-name\0").expect("status should parse");
    assert_eq!(
        changes[0].relative_path.as_os_str().as_bytes(),
        b"invalid-\xff-name"
    );
}
