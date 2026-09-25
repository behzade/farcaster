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
    let old = gate.request().expect("test operation should succeed");
    gate.invalidate();
    assert!(gate.request().is_none());
    let completion = gate.finish(old).expect("test operation should succeed");
    assert!(!completion.publish);
    let current = completion.next.expect("test operation should succeed");
    assert!(
        gate.finish(current)
            .expect("test operation should succeed")
            .publish
    );
}

fn cached_observation(preference: BackendPreference) -> RepositoryObservation {
    RepositoryObservation {
        preference,
        backend: None,
        snapshot: Some(WorkingCopySnapshot {
            location: RepositoryLocation {
                kind: crate::repository::RepositoryKind::Git,
                workspace_root: PathBuf::from("/workspace"),
                project_root: PathBuf::from("/workspace/project"),
            },
            identity: crate::repository::SnapshotIdentity::Git(Default::default()),
            changes: Vec::new(),
            captured_at: std::time::SystemTime::UNIX_EPOCH,
        }),
        additions: Some(2),
        deletions: Some(1),
    }
}

#[test]
fn switching_projects_reuses_the_working_copy_that_was_observed_for_them() {
    let mut cache = ObservationCache::default();
    let first = PathBuf::from("/first");
    let second = PathBuf::from("/second");
    cache.remember(first.clone(), cached_observation(BackendPreference::Auto));

    let reused = cache
        .reuse(&first, BackendPreference::Auto)
        .expect("the first project keeps its working copy");
    assert!(reused.snapshot.is_some());
    assert_eq!(reused.additions, Some(2));
    assert_eq!(reused.deletions, Some(1));

    assert!(cache.reuse(&first, BackendPreference::Auto).is_none());
    assert!(cache.reuse(&second, BackendPreference::Auto).is_none());
}

#[test]
fn a_scan_with_nothing_to_show_is_never_remembered() {
    assert!(
        RepositoryObservation::from_scan(BackendPreference::Auto, Ok(None)).is_none(),
        "a project without a working copy has nothing to show ahead of a switch"
    );
    assert!(
        RepositoryObservation::from_scan(
            BackendPreference::Auto,
            Err(crate::repository::RepositoryError::BackendUnavailable {
                kind: crate::repository::RepositoryKind::Git,
                project: PathBuf::from("/project"),
            }),
        )
        .is_none()
    );
}

#[test]
fn a_cached_working_copy_is_only_reused_by_the_backend_that_produced_it() {
    let mut cache = ObservationCache::default();
    let project = PathBuf::from("/project");
    cache.remember(project.clone(), cached_observation(BackendPreference::Git));

    assert!(
        cache.reuse(&project, BackendPreference::Jujutsu).is_none(),
        "a working copy from another backend is not reusable"
    );
    assert!(
        cache.reuse(&project, BackendPreference::Git).is_none(),
        "the mismatched working copy is dropped instead of kept stale"
    );
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

#[gpui::test]
fn returning_to_a_project_restores_counts_but_requires_fresh_validation(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::returning_to_a_project_restores_counts_but_requires_fresh_validation"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let first = project.to_path_buf();
                    let second = project.join("second");
                    let repository = &mut app.project.repository;
                    repository.execution_allowed = true;
                    let mut observed = cached_observation(repository.preference);
                    observed.snapshot.as_mut().unwrap().location.project_root = first.clone();
                    repository.apply_observation(observed);
                    repository.snapshot_validated = true;
                    assert!(repository.can_sync());
                    // Occupy the refresh gate without spawning a scan. Selection
                    // must queue its refresh behind this outstanding request.
                    let in_flight = repository.refresh.request().unwrap();

                    app.set_repository_project_execution(second.clone(), true, cx);
                    assert_eq!(app.project.repository.project, second);
                    assert!(app.project.repository.snapshot.is_none());
                    app.set_repository_project_execution(first.clone(), true, cx);

                    let repository = &mut app.project.repository;
                    assert_eq!(repository.project, first);
                    assert_eq!(
                        repository.snapshot.as_ref().unwrap().location.project_root,
                        first
                    );
                    assert_eq!(repository.additions, Some(2));
                    assert_eq!(repository.deletions, Some(1));
                    assert!(repository.initialized);
                    assert!(repository.loading);
                    assert_eq!(repository.refresh.in_flight, Some(in_flight));
                    assert!(repository.refresh.pending);
                    assert!(!repository.snapshot_validated);
                    assert!(!repository.can_sync());
                    let completion = repository.refresh.finish(in_flight).unwrap();
                    assert!(!completion.publish);
                    assert!(completion.next.is_some());
                });
            });
        },
    );
}

#[gpui::test]
fn selecting_an_untrusted_project_drops_its_cached_observation(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::selecting_an_untrusted_project_drops_its_cached_observation"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let target = project.join("untrusted");
                    app.project
                        .repository
                        .observations
                        .remember(target.clone(), cached_observation(BackendPreference::Auto));
                    app.set_repository_project_execution(target.clone(), false, cx);

                    let repository = &mut app.project.repository;
                    assert_eq!(repository.project, target);
                    assert!(!repository.execution_allowed);
                    assert!(repository.backend.is_none());
                    assert!(repository.snapshot.is_none());
                    assert!(repository.additions.is_none());
                    assert!(repository.deletions.is_none());
                    assert!(!repository.initialized);
                    assert!(!repository.loading);
                    assert!(!repository.can_sync());
                    assert!(repository.refresh.in_flight.is_none());
                    assert!(
                        repository
                            .observations
                            .reuse(&target, BackendPreference::Auto)
                            .is_none()
                    );
                });
            });
        },
    );
}

#[gpui::test]
fn selecting_and_leaving_a_project_rejects_its_old_background_scan(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::selecting_and_leaving_a_project_rejects_its_old_background_scan"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let target = project.join("target");
                    let repository = &mut app.project.repository;
                    let ticket = repository
                        .observations
                        .begin(target.clone(), BackendPreference::Auto)
                        .unwrap();
                    repository.refresh.request().unwrap();

                    app.set_repository_project_execution(target.clone(), true, cx);
                    assert!(app.project.repository.observations.busy());
                    // Supply the newer foreground result without executing Git.
                    let repository = &mut app.project.repository;
                    let mut newer = cached_observation(repository.preference);
                    newer.additions = Some(42);
                    repository.apply_observation(newer);
                    repository.snapshot_validated = true;
                    app.set_repository_project_execution(project.to_path_buf(), true, cx);

                    let repository = &mut app.project.repository;
                    assert!(!repository.observations.finish(&ticket));
                    assert!(!repository.observations.busy());
                    let kept = repository
                        .observations
                        .reuse(&target, BackendPreference::Auto)
                        .unwrap();
                    assert_eq!(kept.additions, Some(42));
                });
            });
        },
    );
}

#[gpui::test]
fn changing_repository_preference_away_and_back_rejects_pending_scan(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::changing_repository_preference_away_and_back_rejects_pending_scan"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    let repository = &mut app.project.repository;
                    assert!(repository.select_preference(BackendPreference::Git));
                    repository.execution_allowed = true;
                    repository.apply_observation(cached_observation(BackendPreference::Git));
                    repository.snapshot_validated = true;
                    let ticket = repository
                        .observations
                        .begin(project.to_path_buf(), BackendPreference::Git)
                        .unwrap();

                    assert!(repository.select_preference(BackendPreference::Jujutsu));
                    assert!(repository.observations.busy());
                    assert!(repository.snapshot.is_none());
                    assert!(!repository.can_sync());
                    assert!(repository.select_preference(BackendPreference::Git));
                    assert_eq!(repository.preference, ticket.preference);
                    assert_eq!(
                        repository.preferences.get(project),
                        Some(&BackendPreference::Git)
                    );
                    assert!(!repository.observations.finish(&ticket));
                    assert!(!repository.observations.busy());
                    assert!(!repository.snapshot_validated);
                });
            });
        },
    );
}
