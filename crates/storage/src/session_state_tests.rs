use super::*;

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir_in(std::env::temp_dir().canonicalize().expect("temp path")).expect("fixture")
}

fn draft(store: &mut StateStore, project: &Path, id: &str) -> DraftSession {
    let mut draft = DraftSession::with_id(Some(Backend::Pi), id.into(), project.into());
    draft.app_session_id = store
        .allocate_app_session_id(&draft)
        .expect("allocate draft");
    draft
}

#[test]
fn delayed_updates_do_not_restore_deleted_or_promoted_drafts_or_remove_new_ones() {
    let temp = tempdir();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
    let discarded = draft(&mut store, temp.path(), "discarded");
    let mut promoted = draft(&mut store, temp.path(), "promoted");
    promoted.session_path = Some(temp.path().join("session.jsonl"));
    promoted.submitted = true;
    store.allocate_app_session_id(&promoted).expect("associate");
    let changes = SessionStateChanges {
        drafts: [
            (discarded.id.clone(), Some(discarded.clone())),
            (promoted.id.clone(), Some(promoted.clone())),
        ]
        .into(),
        ..Default::default()
    };
    store.remove_draft(&discarded.id).expect("discard");
    store
        .remove_draft(&promoted.id)
        .expect("detach promoted draft");
    let created = draft(&mut store, temp.path(), "created-after-snapshot");
    store.save_session_changes(&changes).expect("late save");
    assert_eq!(store.load_drafts().expect("drafts"), [created]);
    let canonical = store.cached_sessions("").expect("chats");
    assert_eq!(canonical.len(), 1);
    assert_eq!(canonical[0].app_session_id, promoted.app_session_id);
}

#[test]
fn queued_draft_cannot_overwrite_canonical_metadata_or_archive_state() {
    let temp = tempdir();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
    let stale = draft(&mut store, temp.path(), "draft");
    let mut canonical = stale.clone();
    canonical.submitted = true;
    canonical.title = Some("Runtime title".into());
    canonical.harness = Some(Backend::Claude);
    canonical.profile_id = Some("00000000-0000-4000-8000-000000000001".into());
    canonical.project = temp.path().join("moved");
    std::fs::create_dir(&canonical.project).expect("project");
    canonical.session_path = Some(temp.path().join(
        "session-locators/profiles/00000000-0000-4000-8000-000000000001/claude/00000000-0000-4000-8000-000000000002",
    ));
    store
        .allocate_app_session_id(&canonical)
        .expect("runtime promotion");
    store
        .set_session_archived(canonical.session_path.as_ref().expect("path"), true)
        .expect("archive chat");
    canonical.archived = true;
    let mut stale = stale;
    stale.title = Some("Old UI title".into());
    stale.session_path = Some(temp.path().join("stale.jsonl"));
    store
        .save_session_changes(&SessionStateChanges {
            drafts: [(stale.id.clone(), Some(stale))].into(),
            ..Default::default()
        })
        .expect("late save");
    assert_eq!(store.load_drafts().expect("drafts"), [canonical]);
}

#[test]
fn a_batch_rolls_back_projects_draft_changes_and_deletions_if_folders_fail() {
    let temp = tempdir();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
    let existing = draft(&mut store, temp.path(), "existing");
    let deleted = draft(&mut store, temp.path(), "deleted");
    let mut changed = existing.clone();
    changed.title = Some("Unsaved title".into());
    let project = temp.path().join("new-project");
    std::fs::create_dir(&project).expect("project");
    let changes = SessionStateChanges {
        projects: Some(projects::ProjectList {
            projects: vec![project.clone()],
            excluded_projects: vec![],
        }),
        drafts: [
            (changed.id.clone(), Some(changed)),
            (deleted.id.clone(), None),
        ]
        .into(),
        folders: Some(Default::default()),
    };
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER fail_folders BEFORE INSERT ON meta
        WHEN NEW.key='session_folders' BEGIN SELECT RAISE(FAIL, 'fixture write failure'); END;",
        )
        .expect("failure trigger");
    assert!(store.save_session_changes(&changes).is_err());
    let drafts = store.load_drafts().expect("drafts");
    assert!(drafts.contains(&existing));
    assert!(drafts.contains(&deleted));
    assert!(
        !store
            .load_project_list()
            .expect("projects")
            .projects
            .contains(&project)
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_folders")
        .expect("repair");
    store.save_session_changes(&changes).expect("retry");
    let drafts = store.load_drafts().expect("drafts");
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].title.as_deref(), Some("Unsaved title"));
}
