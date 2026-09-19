use super::*;

#[test]
fn git_sync_preserves_branch_and_upstream_mapping() {
    let identity = SnapshotIdentity::Git(GitIdentity {
        branch: Some("feature/sidebar".into()),
        upstream: Some("origin/review/sidebar".into()),
        ..GitIdentity::default()
    });

    assert_eq!(
        strings(&identity, RepositorySyncAction::PullOrFetch),
        ["pull", "--ff-only", "--", "origin", "review/sidebar"]
    );
    assert_eq!(
        strings(&identity, RepositorySyncAction::Push),
        ["push", "--", "origin", "feature/sidebar:review/sidebar"]
    );
}

#[test]
fn jj_push_requires_one_exact_bookmark() {
    let mut identity = jj_identity(vec!["feature/*".into()]);
    assert_eq!(
        strings(&identity, RepositorySyncAction::Push),
        [
            "--no-pager",
            "--color=never",
            "git",
            "push",
            "--bookmark",
            "exact:feature/*"
        ]
    );

    identity = jj_identity(Vec::new());
    assert!(!RepositorySyncAction::Push.is_available_for(&identity));
    identity = jj_identity(vec!["first".into(), "second".into()]);
    assert!(!RepositorySyncAction::Push.is_available_for(&identity));
}

fn jj_identity(bookmarks: Vec<String>) -> SnapshotIdentity {
    SnapshotIdentity::Jujutsu(JujutsuIdentity {
        operation_id: "operation".into(),
        commit_id: "commit".into(),
        change_id: "change".into(),
        description: String::new(),
        bookmarks,
        closest_bookmarks: Vec::new(),
        ahead: 0,
        conflicted_paths: Vec::new(),
        conflicted: false,
        empty: false,
    })
}

fn strings(identity: &SnapshotIdentity, action: RepositorySyncAction) -> Vec<String> {
    arguments(identity, action)
        .expect("sync arguments")
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}
