use super::*;
use crate::repository::JujutsuIdentity;

#[test]
fn file_labels_keep_names_and_parent_paths_distinct() {
    assert_eq!(
        file_path_labels(Path::new("src/app/theme.rs")),
        ("theme.rs".into(), "src/app".into())
    );
    assert_eq!(
        file_path_labels(Path::new("Cargo.toml")),
        ("Cargo.toml".into(), String::new())
    );
    assert_eq!(
        file_path_labels(Path::new("src/نام\n.rs")),
        ("نام\\n.rs".into(), "src".into())
    );
}

#[test]
fn unborn_and_detached_git_heads_are_explicit() {
    assert_eq!(
        git_identity(&GitIdentity {
            branch: Some("main".into()),
            ..GitIdentity::default()
        }),
        "main · unborn"
    );
    assert_eq!(
        git_identity(&GitIdentity {
            head_oid: Some("0123456789abcdef".into()),
            ..GitIdentity::default()
        }),
        "detached 01234567"
    );
}

#[test]
fn sync_metadata_uses_nearest_git_branch_and_jj_ancestor_bookmark() {
    assert_eq!(
        repository_sync_metadata(&SnapshotIdentity::Git(GitIdentity {
            nearest_branch: Some("main".into()),
            ahead: 2,
            ..GitIdentity::default()
        })),
        "main · 2 ahead"
    );
    assert_eq!(
        repository_sync_metadata(&SnapshotIdentity::Jujutsu(JujutsuIdentity {
            operation_id: String::new(),
            commit_id: "commit".into(),
            change_id: "change".into(),
            description: String::new(),
            bookmarks: Vec::new(),
            closest_bookmarks: vec!["main".into()],
            ahead: 2,
            conflicted_paths: Vec::new(),
            conflicted: false,
            empty: true,
        })),
        "main · 2 ahead"
    );
}

#[test]
fn unusual_paths_are_reduced_to_one_visible_line() {
    assert_eq!(
        visible_path(Path::new("old\nname\t.rs")),
        "old\\nname\\t.rs"
    );
}
