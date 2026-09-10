use super::*;

#[cfg(unix)]
use std::{fs, os::unix::fs::symlink};

fn metadata(id: &str) -> crate::agents::SessionMetadata {
    crate::agents::SessionMetadata {
        harness: "codex-cli".into(),
        id: id.into(),
        path: PathBuf::from(format!("/locators/codex-cli/{id}")),
        project: PathBuf::from("/project"),
        title: Some(id.into()),
        first_user_message: None,
        parent_session: None,
        message_count: Some(1),
        model: None,
        thinking_level: None,
        service_tier: None,
        usage: None,
        is_running: true,
    }
}

#[test]
fn metadata_readback_failure_rolls_back_the_update() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let mut update = metadata("readback");
    let original = store.update_session_metadata(&update)?;
    // Inject a value accepted by SQLite but rejected by the summary decoder.
    // All writes succeed, so only the read-back can cause the rollback.
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER invalid_summary AFTER UPDATE OF title ON sessions
             BEGIN UPDATE sessions SET message_count=-1 WHERE id=NEW.id; END;",
        )
        .map_err(|error| error.to_string())?;
    update.title = Some("Must not commit".into());
    let error = store.update_session_metadata(&update).unwrap_err();
    assert!(error.contains("decode cached session"), "{error}");
    let restored = store.cached_sessions("")?;
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].title, original.title);
    assert_eq!(restored[0].message_count, original.message_count);
    Ok(())
}

#[test]
fn live_metadata_preserves_archive_identity_and_other_sessions() {
    let temp = tempfile::tempdir().expect("test operation should succeed");
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))
        .expect("test operation should succeed");
    let mut update = metadata("parent");
    let parent = store
        .update_session_metadata(&update)
        .expect("test operation should succeed");
    store
        .set_session_archived(&parent.path, true)
        .expect("test operation should succeed");
    let other = store
        .update_session_metadata(&metadata("other"))
        .expect("test operation should succeed");
    update.title = Some("Renamed".into());
    update.first_user_message = Some("First prompt".into());
    update.model = Some(("provider".into(), "model".into()));
    update.thinking_level = Some("high".into());
    update.service_tier = Some("priority".into());
    update.usage = Some(crate::agents::DiscoveredUsage {
        input: 12,
        total: 12,
        ..Default::default()
    });
    let updated = store
        .update_session_metadata(&update)
        .expect("test operation should succeed");
    assert!(updated.archived);
    assert_eq!(updated.app_session_id, parent.app_session_id);
    assert_eq!(updated.title, "Renamed");
    assert_eq!(updated.usage.total, 12);
    assert_eq!(updated.model, update.model);
    let saved: (String, Option<String>) = store
        .connection
        .query_row(
            "SELECT model,service_tier FROM session_models WHERE session_id=?1",
            [updated.app_session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("test operation should succeed");
    assert_eq!(saved, ("model".into(), Some("priority".into())));
    let cached = store
        .cached_sessions("")
        .expect("test operation should succeed");
    assert_eq!(cached.len(), 2);
    let unchanged = cached
        .iter()
        .find(|s| s.id == "other")
        .expect("test operation should succeed");
    assert_eq!(unchanged.modified, other.modified);
    assert_eq!(unchanged.title, other.title);
}

#[test]
fn child_events_preserve_family_and_metadata_on_completion() {
    let temp = tempfile::tempdir().expect("test operation should succeed");
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))
        .expect("test operation should succeed");
    store
        .update_session_metadata(&metadata("parent"))
        .expect("test operation should succeed");
    let mut child = metadata("child");
    child.parent_session = Some("parent".into());
    let started = store
        .update_session_metadata(&child)
        .expect("test operation should succeed");
    assert!(started.is_running);
    child.is_running = false;
    child.title = None;
    child.message_count = None;
    let ended = store
        .update_session_metadata(&child)
        .expect("test operation should succeed");
    assert!(!ended.is_running);
    assert_eq!(ended.app_session_id, started.app_session_id);
    assert_eq!(ended.title, "child");
    assert_eq!(ended.message_count, 1);
    assert_eq!(ended.parent_session.as_deref(), Some("parent"));
}

#[test]
fn failed_live_update_rolls_back_without_changing_archive_state() {
    let temp = tempfile::tempdir().expect("test operation should succeed");
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))
        .expect("test operation should succeed");
    let update = metadata("archived");
    store
        .update_session_metadata(&update)
        .expect("test operation should succeed");
    store
        .set_session_archived(&update.path, true)
        .expect("test operation should succeed");
    let before = store
        .cached_sessions("")
        .expect("test operation should succeed");
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER fail_update BEFORE UPDATE ON sessions
        BEGIN SELECT RAISE(ABORT, 'test write failure'); END;",
        )
        .expect("test operation should succeed");
    assert!(store.update_session_metadata(&update).is_err());
    assert_eq!(
        store
            .cached_sessions("")
            .expect("test operation should succeed"),
        before
    );
}

#[cfg(unix)]
#[test]
fn live_metadata_rekeys_a_legacy_project_alias() {
    let temp = tempfile::tempdir().expect("test operation should succeed");
    let database = temp.path().join("state.sqlite3");
    let project = temp.path().join("project");
    let alias = temp.path().join("project-alias");
    let session_path = temp.path().join("session.jsonl");
    let raw_locator = temp.path().join("synthetic/../session.jsonl");
    fs::create_dir(&project).expect("test operation should succeed");
    fs::write(&session_path, "{}").expect("test operation should succeed");
    symlink(&project, &alias).expect("test operation should succeed");
    let project = project
        .canonicalize()
        .expect("test operation should succeed");
    let mut update = metadata("legacy");
    update.project = project.clone();
    update.path = session_path
        .canonicalize()
        .expect("test operation should succeed");
    let mut store = StateStore::open_at(&database).expect("test operation should succeed");
    store
        .update_session_metadata(&update)
        .expect("test operation should succeed");
    store
        .connection
        .execute(
            "UPDATE sessions SET locator=?1",
            [raw_locator.to_string_lossy()],
        )
        .expect("test operation should succeed");
    store
        .connection
        .execute(
            "UPDATE projects SET path=?1 WHERE path=?2",
            params![alias.to_string_lossy(), project.to_string_lossy()],
        )
        .expect("test operation should succeed");

    assert_eq!(
        store
            .cached_sessions("")
            .expect("test operation should succeed")[0]
            .project,
        project
    );
    store
        .update_session_metadata(&update)
        .expect("test operation should succeed");
    let stored_project: String = store
        .connection
        .query_row(
            "SELECT p.path FROM sessions s JOIN projects p ON p.id=s.project_id",
            [],
            |row| row.get(0),
        )
        .expect("test operation should succeed");
    assert_eq!(stored_project, project.to_string_lossy());
    let stored_locator: String = store
        .connection
        .query_row("SELECT locator FROM sessions", [], |row| row.get(0))
        .expect("test operation should succeed");
    assert_eq!(stored_locator, update.path.to_string_lossy());
}

#[test]
fn legacy_synthetic_locator_mutators_use_canonical_identity() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let project = temp.path().join("project");
    std::fs::create_dir(&project).map_err(|error| error.to_string())?;
    let project = project.canonicalize().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let normal = ["archive", "relocate", "delete", "queued"].map(|id| {
        (
            id,
            temp.path().join(format!("{id}.jsonl")),
            temp.path().join(format!("legacy/../{id}.jsonl")),
        )
    });
    for (id, path, raw_locator) in &normal {
        let mut update = metadata(id);
        update.path = path.clone();
        update.project = project.clone();
        store.update_session_metadata(&update)?;
        store
            .connection
            .execute(
                "UPDATE sessions SET locator=?1 WHERE backend_id=?2",
                params![raw_locator.to_string_lossy(), id],
            )
            .map_err(|error| error.to_string())?;
    }

    let archive = &normal[0].1;
    let relocate_source = &normal[1].1;
    let delete = &normal[2].1;
    let queued = &normal[3].1;
    store.enqueue_prompt(
        &format!("session:{}", queued.display()),
        "codex-cli",
        &project,
        Some(queued),
        crate::protocol::PromptMode::Normal,
        "queued",
        &[],
    )?;
    assert!(store.has_queued_prompts_for(std::slice::from_ref(queued))?);

    store.set_session_archived(archive, true)?;
    let archived: (String, bool) = store
        .connection
        .query_row(
            "SELECT locator, archived_at IS NOT NULL FROM sessions WHERE backend_id='archive'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        archived,
        (
            crate::sessions::normalize_session_path(archive)
                .to_string_lossy()
                .into_owned(),
            true
        )
    );

    let relocated = temp.path().join("relocated.jsonl");
    store.relocate_session_paths(&[(relocate_source.clone(), relocated.clone())], &project)?;
    let moved: String = store
        .connection
        .query_row(
            "SELECT locator FROM sessions WHERE backend_id='relocate'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        moved,
        crate::sessions::normalize_session_path(&relocated).to_string_lossy()
    );

    store.delete_session_state(std::slice::from_ref(delete))?;
    let deleted: bool = store
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE backend_id='delete')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert!(!deleted);
    Ok(())
}

#[test]
fn ambiguous_legacy_synthetic_locator_does_not_mutate_a_session() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let locator = temp.path().join("session.jsonl");
    for id in ["first", "second"] {
        let mut update = metadata(id);
        update.path = temp.path().join(format!("seed-{id}.jsonl"));
        update.project = temp.path().to_path_buf();
        store.update_session_metadata(&update)?;
        store
            .connection
            .execute(
                "UPDATE sessions SET locator=?1 WHERE backend_id=?2",
                params![
                    temp.path()
                        .join(format!("legacy/{id}/../../session.jsonl"))
                        .to_string_lossy(),
                    id
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    assert!(store.set_session_archived(&locator, true).is_err());
    let archived: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE archived_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(archived, 0);
    Ok(())
}
