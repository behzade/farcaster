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
fn metadata_merge_preserves_unarchive_intent_on_the_discarded_draft() {
    let temp = tempdir();
    let database = temp.path().join("state.sqlite3");
    let mut store = StateStore::open_at(&database).expect("store");
    let mut metadata = crate::agents::SessionMetadata {
        harness: Backend::Pi,
        profile_id: None,
        id: "native".into(),
        path: temp.path().join("placeholder.jsonl"),
        project: temp.path().into(),
        title: None,
        first_user_message: None,
        parent_session: None,
        message_count: None,
        model: None,
        thinking_level: None,
        service_tier: None,
        access_mode: None,
        usage: None,
        is_running: false,
    };
    let native = store
        .update_session_metadata(&metadata)
        .expect("native row");
    store
        .set_session_archived(&metadata.path, true)
        .expect("archive native row");
    let mut pending = draft(&mut store, temp.path(), "pending");
    pending.submitted = true;
    pending.session_path = Some(temp.path().join("real.jsonl"));
    store.allocate_app_session_id(&pending).expect("bind draft");
    let mut stale = pending.clone();
    stale.archived = true;
    store
        .set_app_session_archived(
            sessions::AppSessionId::new(pending.app_session_id).expect("identity"),
            false,
        )
        .expect("explicit unarchive");
    metadata.path = pending.session_path.clone().expect("path");
    let merged = store.update_session_metadata(&metadata).expect("merge");
    assert_eq!(merged.app_session_id, native.app_session_id);
    assert_ne!(merged.app_session_id, pending.app_session_id);
    assert!(!merged.archived);
    store
        .save_session_changes(&SessionStateChanges {
            drafts: [(stale.id.clone(), Some(stale))].into(),
            ..Default::default()
        })
        .expect("late snapshot");
    drop(store);
    let store = StateStore::open_at(&database).expect("reopen");
    let drafts = store.load_drafts().expect("drafts");
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].app_session_id, native.app_session_id);
    assert!(!drafts[0].archived);
    assert!(!store.cached_sessions("").expect("sessions")[0].archived);
}

#[test]
fn merge_orders_explicit_archive_intents_by_time_then_durable_id() {
    for reverse in [false, true] {
        for (first_time, second_time) in [(100, 200), (200, 100), (100, 100)] {
            for first_archived in [false, true] {
                let temp = tempdir();
                let mut store =
                    StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
                let first = draft(&mut store, temp.path(), "first");
                let second = draft(&mut store, temp.path(), "second");
                for archived in [!first_archived, first_archived] {
                    store
                        .set_app_session_archived(
                            sessions::AppSessionId::new(first.app_session_id).expect("first"),
                            archived,
                        )
                        .expect("first intent");
                }
                store
                    .set_app_session_archived(
                        sessions::AppSessionId::new(second.app_session_id).expect("second"),
                        !first_archived,
                    )
                    .expect("second intent");
                for (id, time) in [
                    (first.app_session_id, first_time),
                    (second.app_session_id, second_time),
                ] {
                    store
                        .connection
                        .execute(
                            "UPDATE session_events SET t=?2 WHERE session_id=?1",
                            params![id, time],
                        )
                        .expect("fixture intent time");
                }
                let (keep, other) = if reverse {
                    (second.app_session_id, first.app_session_id)
                } else {
                    (first.app_session_id, second.app_session_id)
                };
                let expected = if first_time > second_time {
                    first_archived
                } else {
                    !first_archived
                };
                let tx = store.connection.transaction().expect("begin merge");
                super::super::identity::merge_session(&tx, keep, other).expect("merge");
                tx.commit().expect("commit merge");
                assert_eq!(store.load_drafts().expect("drafts")[0].archived, expected);

                let third = draft(&mut store, temp.path(), "third");
                let tx = store.connection.transaction().expect("begin next merge");
                super::super::identity::merge_session(&tx, third.app_session_id, keep)
                    .expect("next merge");
                tx.commit().expect("commit next merge");
                assert_eq!(store.load_drafts().expect("drafts")[0].archived, expected);

                let path = temp.path().join("bound.jsonl");
                let tx = store.connection.transaction().expect("begin bind");
                bind_locator(&tx, &third.id, &path).expect("bind survivor");
                tx.commit().expect("commit bind");
                store
                    .set_session_archived(&path, !expected)
                    .expect("later path archive change");
                let fourth = draft(&mut store, temp.path(), "fourth");
                let tx = store.connection.transaction().expect("begin final merge");
                super::super::identity::merge_session(
                    &tx,
                    fourth.app_session_id,
                    third.app_session_id,
                )
                .expect("merge changed survivor");
                tx.commit().expect("commit final merge");
                assert_eq!(store.load_drafts().expect("drafts")[0].archived, !expected);
            }
        }
    }
}

#[test]
fn newer_path_unarchive_wins_over_another_rows_older_intent() {
    for reverse in [false, true] {
        let temp = tempdir();
        let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
        let first = draft(&mut store, temp.path(), "first");
        let second = draft(&mut store, temp.path(), "second");
        let path = temp.path().join("session.jsonl");
        let tx = store.connection.transaction().expect("begin bind");
        bind_locator(&tx, &first.id, &path).expect("bind first");
        tx.commit().expect("commit bind");
        for (id, time) in [(first.app_session_id, 100), (second.app_session_id, 200)] {
            store
                .set_app_session_archived(sessions::AppSessionId::new(id).expect("identity"), true)
                .expect("old archive intent");
            store
                .connection
                .execute(
                    "UPDATE session_events SET t=?2 WHERE session_id=?1",
                    params![id, time],
                )
                .expect("fixture time");
        }
        store
            .set_session_archived(&path, false)
            .expect("new path unarchive");
        let (keep, other) = if reverse {
            (second.app_session_id, first.app_session_id)
        } else {
            (first.app_session_id, second.app_session_id)
        };
        let tx = store.connection.transaction().expect("begin merge");
        super::super::identity::merge_session(&tx, keep, other).expect("merge");
        tx.commit().expect("commit merge");
        assert!(!store.load_drafts().expect("drafts")[0].archived);
    }
}

#[test]
fn path_archive_intent_is_atomic_with_legacy_binding_and_ignores_missing_paths() {
    let temp = tempdir();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
    let path = temp.path().join("session.jsonl");
    store
        .set_session_archived(&path, true)
        .expect("missing path is a no-op");
    assert!(store.load_drafts().expect("no allocation").is_empty());
    let pending = draft(&mut store, temp.path(), "pending");
    let legacy = temp.path().join("old/../session.jsonl");
    store
        .connection
        .execute(
            "UPDATE sessions SET locator=?2 WHERE id=?1",
            params![pending.app_session_id, legacy.to_string_lossy()],
        )
        .expect("legacy locator fixture");
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER fail_path_archive_event BEFORE INSERT ON session_events
         WHEN json_extract(NEW.body,'$.type')='session_archive_intent'
         BEGIN SELECT RAISE(FAIL, 'fixture event failure'); END;",
        )
        .expect("failure trigger");
    assert!(store.set_session_archived(&path, true).is_err());
    let saved: (String, bool) = store
        .connection
        .query_row(
            "SELECT locator,archived_at IS NOT NULL FROM sessions WHERE id=?1",
            [pending.app_session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("unchanged row");
    assert_eq!(saved, (legacy.to_string_lossy().into_owned(), false));
    store
        .connection
        .execute_batch("DROP TRIGGER fail_path_archive_event")
        .expect("repair");
    store.set_session_archived(&path, true).expect("retry");
    let saved: (String, bool, i64) = store
        .connection
        .query_row(
            "SELECT locator,archived_at IS NOT NULL,
                (SELECT COUNT(*) FROM session_events WHERE session_id=s.id)
           FROM sessions s WHERE id=?1",
            [pending.app_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("bound row and event");
    assert_eq!(saved, (path.to_string_lossy().into_owned(), true, 1));
}

#[test]
fn stale_index_snapshot_cannot_replace_explicit_unarchive() {
    for by_path in [false, true] {
        let temp = tempdir();
        let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
        let pending = draft(&mut store, temp.path(), "pending");
        let path = temp.path().join("session.jsonl");
        let tx = store.connection.transaction().expect("begin bind");
        bind_locator(&tx, &pending.id, &path).expect("bind");
        tx.commit().expect("commit bind");
        store.set_session_archived(&path, true).expect("archive");
        let stale = store.cached_sessions("").expect("archived snapshot");
        assert!(stale[0].archived);
        if by_path {
            store
                .set_session_archived(&path, false)
                .expect("path unarchive");
        } else {
            store
                .set_app_session_archived(
                    sessions::AppSessionId::new(pending.app_session_id).expect("identity"),
                    false,
                )
                .expect("identity unarchive");
        }
        store
            .index_sessions(&stale, false)
            .expect("stale index snapshot");
        let saved = store.cached_sessions("").expect("sessions");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].app_session_id, pending.app_session_id);
        assert!(!saved[0].archived);
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
