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
fn archive_intent_survives_binding_in_either_order_and_late_snapshots() {
    for archived in [false, true] {
        for bind_first in [false, true] {
            let temp = tempdir();
            let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
            let mut pending = draft(&mut store, temp.path(), "pending");
            pending.submitted = true;
            store.allocate_app_session_id(&pending).expect("submit");
            store
                .set_app_session_archived(
                    sessions::AppSessionId::new(pending.app_session_id).expect("identity"),
                    !archived,
                )
                .expect("prior intent");
            pending.archived = !archived;
            let late = SessionStateChanges {
                drafts: [(pending.id.clone(), Some(pending.clone()))].into(),
                ..Default::default()
            };
            let path = temp.path().join("session.jsonl");
            let bind = |store: &mut StateStore| {
                let tx = store.connection.transaction().expect("begin bind");
                bind_locator(&tx, &pending.id, &path).expect("bind");
                tx.commit().expect("commit bind");
            };
            if bind_first {
                bind(&mut store);
            }
            store
                .set_app_session_archived(
                    sessions::AppSessionId::new(pending.app_session_id).expect("identity"),
                    archived,
                )
                .expect("explicit intent");
            store.save_session_changes(&late).expect("stale snapshot");
            assert_eq!(store.load_drafts().expect("drafts")[0].archived, archived);
            if !bind_first {
                bind(&mut store);
            }
            store
                .save_session_changes(&late)
                .expect("stale bound snapshot");
            store.remove_draft(&pending.id).expect("promote");
            store
                .save_session_changes(&late)
                .expect("stale promoted snapshot");
            assert!(store.load_drafts().expect("drafts").is_empty());
            let canonical = store.cached_sessions("").expect("sessions");
            assert_eq!(canonical.len(), 1);
            assert_eq!(canonical[0].app_session_id, pending.app_session_id);
            assert_eq!(canonical[0].archived, archived);
            // A command queued while the draft existed also works after its
            // draft key has gone. No locator was captured by that command.
            store
                .set_app_session_archived(
                    sessions::AppSessionId::new(pending.app_session_id).expect("identity"),
                    !archived,
                )
                .expect("intent after promotion");
            assert_eq!(
                store.cached_sessions("").expect("sessions")[0].archived,
                !archived
            );
        }
    }
}

#[test]
fn stale_unsubmitted_snapshot_cannot_replace_archive_intent() {
    let temp = tempdir();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
    let stale = draft(&mut store, temp.path(), "draft");
    store
        .set_app_session_archived(
            sessions::AppSessionId::new(stale.app_session_id).expect("identity"),
            true,
        )
        .expect("intent");
    store
        .save_session_changes(&SessionStateChanges {
            drafts: [(stale.id.clone(), Some(stale))].into(),
            ..Default::default()
        })
        .expect("late snapshot");
    assert!(store.load_drafts().expect("drafts")[0].archived);
}

#[test]
fn binding_an_archived_discovered_row_distinguishes_unarchive_intent_from_snapshot() {
    for explicit_unarchive in [false, true] {
        let temp = tempdir();
        let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
        let path = temp.path().join("discovered.jsonl");
        let mut discovered = draft(&mut store, temp.path(), "discovered");
        discovered.submitted = true;
        discovered.session_path = Some(path.clone());
        store
            .allocate_app_session_id(&discovered)
            .expect("discover");
        store.remove_draft(&discovered.id).expect("catalog row");
        store
            .set_session_archived(&path, true)
            .expect("archived discovered chat");

        let mut pending = draft(&mut store, temp.path(), "pending");
        pending.submitted = true;
        store
            .save_session_changes(&SessionStateChanges {
                drafts: [(pending.id.clone(), Some(pending.clone()))].into(),
                ..Default::default()
            })
            .expect("ordinary unarchived snapshot");
        if explicit_unarchive {
            store
                .set_app_session_archived(
                    sessions::AppSessionId::new(pending.app_session_id).expect("identity"),
                    false,
                )
                .expect("explicit unarchive");
        }
        let tx = store.connection.transaction().expect("begin bind");
        bind_locator(&tx, &pending.id, &path).expect("bind discovered row");
        tx.commit().expect("commit bind");
        let canonical = store.cached_sessions("").expect("sessions");
        assert_eq!(canonical.len(), 1);
        assert_eq!(canonical[0].app_session_id, pending.app_session_id);
        assert_eq!(canonical[0].archived, !explicit_unarchive);
    }
}

#[test]
fn archive_intent_for_deleted_draft_fails_without_touching_another_chat() {
    let temp = tempdir();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
    let deleted = draft(&mut store, temp.path(), "deleted");
    store.remove_draft(&deleted.id).expect("delete");
    let remaining = draft(&mut store, temp.path(), "remaining");
    assert!(
        store
            .set_app_session_archived(
                sessions::AppSessionId::new(deleted.app_session_id).expect("identity"),
                true
            )
            .is_err()
    );
    assert_eq!(store.load_drafts().expect("drafts"), [remaining]);
}

#[test]
fn archive_intent_and_its_event_commit_together() {
    let temp = tempdir();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
    let pending = draft(&mut store, temp.path(), "pending");
    let id = sessions::AppSessionId::new(pending.app_session_id).expect("identity");
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER fail_archive_event BEFORE INSERT ON session_events
         WHEN json_extract(NEW.body,'$.type')='session_archive_intent'
         BEGIN SELECT RAISE(FAIL, 'fixture event failure'); END;",
        )
        .expect("failure trigger");
    assert!(store.set_app_session_archived(id, true).is_err());
    assert!(!store.load_drafts().expect("drafts")[0].archived);
    store
        .connection
        .execute_batch("DROP TRIGGER fail_archive_event")
        .expect("repair");
    store.set_app_session_archived(id, true).expect("archive");
    store
        .set_app_session_archived(id, false)
        .expect("unarchive");
    let events: Vec<(i64, bool)> = store
        .connection
        .prepare(
            "SELECT seq,json_extract(body,'$.archived') FROM session_events
          WHERE session_id=?1 AND json_extract(body,'$.type')='session_archive_intent'
          ORDER BY seq",
        )
        .expect("prepare events")
        .query_map([id.get()], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("events")
        .collect::<rusqlite::Result<_>>()
        .expect("decode events");
    assert_eq!(events, [(1, true), (2, false)]);
    assert!(!store.load_drafts().expect("drafts")[0].archived);
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
