use super::*;

#[test]
fn refresh_gate_publishes_during_continuous_changes_and_coalesces_requests() {
    let mut gate = RefreshGate::default();
    let first = gate.request().expect("first refresh should start");
    assert!(gate.request().is_none());

    let completion = gate.finish(first).expect("active refresh should finish");
    assert!(completion.publish);
    let second = completion.next.expect("pending refresh should start");
    assert_ne!(first, second);

    assert!(gate.request().is_none());
    assert!(gate.request().is_none());
    let completion = gate.finish(second).expect("latest refresh should finish");
    assert!(completion.publish);
    let third = completion.next.expect("changes need one more refresh");
    let completion = gate.finish(third).expect("last refresh should finish");
    assert!(completion.publish);
    assert!(completion.next.is_none());
    assert!(gate.finish(first).is_none());
}

#[test]
fn display_equality_ignores_snapshot_capture_time() {
    let snapshot = WorkingCopySnapshot {
        location: RepositoryLocation {
            kind: crate::repository::RepositoryKind::Git,
            workspace_root: PathBuf::from("/workspace"),
            project_root: PathBuf::from("/workspace/project"),
        },
        identity: crate::repository::SnapshotIdentity::Git(Default::default()),
        changes: Vec::new(),
        captured_at: std::time::SystemTime::UNIX_EPOCH,
    };
    let mut later = snapshot.clone();
    later.captured_at = std::time::SystemTime::now();

    assert!(displayed_snapshot_eq(&snapshot, &later));
}

#[test]
fn display_equality_ignores_unrendered_jujutsu_operation_and_commit_ids() {
    let identity = crate::repository::JujutsuIdentity {
        operation_id: "operation-a".into(),
        commit_id: "commit-a".into(),
        change_id: "change".into(),
        description: "description".into(),
        bookmarks: vec!["main".into()],
        closest_bookmarks: vec!["main".into()],
        ahead: 0,
        conflicted_paths: Vec::new(),
        conflicted: false,
        empty: false,
    };
    let mut later = identity.clone();
    later.operation_id = "operation-b".into();
    later.commit_id = "commit-b".into();

    assert!(displayed_identity_eq(
        &crate::repository::SnapshotIdentity::Jujutsu(identity),
        &crate::repository::SnapshotIdentity::Jujutsu(later),
    ));
}

#[test]
fn invalidation_rejects_in_flight_work_without_starting_another_command() {
    let mut gate = RefreshGate::default();
    let generation = gate.request().expect("refresh should start");
    gate.invalidate();
    let completion = gate.finish(generation).expect("refresh should finish");
    assert!(!completion.publish);
    assert!(completion.next.is_none());
}

#[test]
fn project_change_rejects_old_scan_and_starts_pending_scan() {
    let mut gate = RefreshGate::default();
    let old = gate.request().unwrap();
    gate.invalidate();
    assert!(gate.request().is_none());
    let completion = gate.finish(old).unwrap();
    assert!(!completion.publish);
    let current = completion.next.unwrap();
    assert!(gate.finish(current).unwrap().publish);
}

#[test]
fn repository_preferences_are_project_specific() {
    let first = PathBuf::from("/first");
    let second = PathBuf::from("/second");
    let preferences = BTreeMap::from([
        (first.clone(), BackendPreference::Git),
        (second.clone(), BackendPreference::Jujutsu),
    ]);

    assert_eq!(preference_for(&preferences, &first), BackendPreference::Git);
    assert_eq!(
        preference_for(&preferences, &second),
        BackendPreference::Jujutsu
    );
    assert_eq!(
        preference_for(&preferences, std::path::Path::new("/other")),
        BackendPreference::Auto
    );
}
