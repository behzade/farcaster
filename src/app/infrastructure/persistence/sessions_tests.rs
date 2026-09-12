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

#[test]
fn unavailable_project_round_trip_preserves_draft_composer_and_outbox() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let project = temp.path().join("project");
    let offline = temp.path().join("project-offline");
    std::fs::create_dir(&project).map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    let draft =
        crate::projects::DraftSession::new("pi".into(), "saved-work".into(), 0, project.clone(), 1);
    store.save_registry(&crate::projects::Registry {
        projects: vec![project.clone()],
        excluded_projects: Vec::new(),
        drafts: vec![draft],
    })?;
    store.save_composer_session(&ComposerRecord {
        target: "draft:saved-work".into(),
        text: "unsent text".into(),
        cursor: 4,
        selection_start: 2,
        selection_end: 4,
        history: vec!["previous draft".into()],
        ..Default::default()
    })?;
    store.enqueue_prompt(
        "draft:saved-work",
        "pi",
        &project,
        None,
        crate::protocol::PromptMode::Normal,
        "queued work",
        &[],
    )?;
    let before_registry = store.load_registry()?;
    let before_composer = store.load_composer_sessions()?;
    let before_queue = store.queued_prompts()?;
    drop(store);

    std::fs::rename(&project, &offline).map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&database)?;
    let registry = store.load_registry()?;
    store.save_registry(&registry)?;
    drop(store);
    std::fs::rename(&offline, &project).map_err(|error| error.to_string())?;

    let mut store = StateStore::open_at(&database)?;
    let restored = store.load_registry()?;
    assert_eq!(
        restored.projects,
        vec![project.canonicalize().map_err(|e| e.to_string())?]
    );
    assert_eq!(restored.drafts, before_registry.drafts);
    assert_eq!(store.load_composer_sessions()?, before_composer);
    assert_eq!(store.queued_prompts()?, before_queue);

    // Explicitly deleting the draft must still remove its dependent state.
    let mut deleted = restored;
    deleted.drafts.clear();
    store.save_registry(&deleted)?;
    drop(store);
    let reopened = StateStore::open_at(&database)?;
    assert!(reopened.load_registry()?.drafts.is_empty());
    assert!(reopened.load_composer_sessions()?.is_empty());
    assert!(reopened.queued_prompts()?.is_empty());
    Ok(())
}

#[test]
fn worker_family_native_ids_get_loadable_unique_locators() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    let mut parent = metadata("parent");
    parent.project = temp.path().to_path_buf();
    parent.path = temp.path().join("session-locators/codex-cli/parent");
    store.update_session_metadata(&parent)?;
    store.save_worker_family(&crate::agents::WorkerFamilyLink {
        project: temp.path().to_path_buf(),
        parent_backend: "codex-cli".into(),
        parent_session: "parent".into(),
        child_backend: "codex-cli".into(),
        child_session: "new-child-id".into(),
        execution: None,
    })?;

    let rows = store.cached_sessions("")?;
    let child = rows
        .iter()
        .find(|session| session.id == "new-child-id")
        .ok_or("missing child")?;
    assert_eq!(
        crate::agents::external_session_identity(&child.path),
        Some(("codex-cli", "new-child-id".into()))
    );
    crate::agents::validate_session_move(std::slice::from_ref(child))?;
    let other_project = temp.path().join("other-project");
    store.save_worker_family(&crate::agents::WorkerFamilyLink {
        project: other_project.clone(),
        parent_backend: "codex-cli".into(),
        parent_session: "parent".into(),
        child_backend: "codex-cli".into(),
        child_session: "new-child-id".into(),
        execution: None,
    })?;
    store.save_worker_family(&crate::agents::WorkerFamilyLink {
        project: temp.path().to_path_buf(),
        parent_backend: "codex-cli".into(),
        parent_session: "parent".into(),
        child_backend: "opencode2".into(),
        child_session: "new-child-id".into(),
        execution: None,
    })?;
    let identities: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE harness='codex-cli' AND project_id=(SELECT id FROM projects WHERE path=?1) AND backend_id='parent'",
            [crate::sessions::normalize_session_path(temp.path()).to_string_lossy()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(identities, 1);
    let scoped: Vec<(String, String)> = {
        let mut statement = store
            .connection
            .prepare(
                "SELECT p.path,s.locator FROM sessions s JOIN projects p ON p.id=s.project_id
                  WHERE s.harness='codex-cli' AND s.backend_id='new-child-id' ORDER BY p.path",
            )
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?
    };
    assert_eq!(scoped.len(), 2);
    assert_ne!(scoped[0].1, scoped[1].1);
    let families = store.load_worker_families()?;
    assert_eq!(families.len(), 3);
    assert!(
        families
            .iter()
            .any(|family| family.project == crate::sessions::normalize_session_path(&other_project))
    );
    assert!(families.iter().any(|family| {
        family.project == crate::sessions::normalize_session_path(temp.path())
            && family.parent_backend == "codex-cli"
            && family.child_backend == "opencode2"
            && family.child_session == "new-child-id"
    }));
    let backend_isolation: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sessions s JOIN projects p ON p.id=s.project_id
              WHERE p.path=?1 AND s.backend_id='new-child-id'",
            [crate::sessions::normalize_session_path(temp.path()).to_string_lossy()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(backend_isolation, 2);
    Ok(())
}

#[test]
fn live_metadata_merges_family_placeholder_without_losing_related_state() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let project = temp.path().to_path_buf();
    store.save_worker_family(&crate::agents::WorkerFamilyLink {
        project: project.clone(),
        parent_backend: "codex-cli".into(),
        parent_session: "parent".into(),
        child_backend: "codex-cli".into(),
        child_session: "child".into(),
        execution: None,
    })?;
    store
        .connection
        .execute(
            "UPDATE sessions SET locator='parent',client_key='legacy-draft',
                    submitted=1,rail_order=42,created_ms=1
              WHERE harness='codex-cli' AND backend_id='parent'",
            [],
        )
        .map_err(|error| error.to_string())?;
    let placeholder_id: i64 = store
        .connection
        .query_row(
            "SELECT id FROM sessions WHERE harness='codex-cli' AND backend_id='parent'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    store
        .connection
        .execute(
            "INSERT INTO session_events(session_id,seq,t,schema_version,body)
             VALUES(?1,1,11,1,'{\"type\":\"legacy_event\",\"value\":\"kept\"}')",
            [placeholder_id],
        )
        .map_err(|error| error.to_string())?;
    store
        .connection
        .execute(
            "INSERT INTO session_models(session_id,provider,model,effort,service_tier)
             VALUES(?1,'legacy-provider','legacy-model','high','priority')",
            [placeholder_id],
        )
        .map_err(|error| error.to_string())?;
    let placeholders = store.cached_sessions("")?;
    let placeholder_parent = placeholders
        .iter()
        .find(|session| session.id == "parent")
        .ok_or("missing parent placeholder")?;
    store.save_composer_session(&ComposerRecord {
        target: format!("session:{}", placeholder_parent.path.display()),
        text: "kept draft".into(),
        ..Default::default()
    })?;
    store.enqueue_prompt(
        &format!("session:{}", placeholder_parent.path.display()),
        "codex-cli",
        &project,
        Some(&placeholder_parent.path),
        crate::protocol::PromptMode::Normal,
        "kept prompt",
        &[],
    )?;
    let parent_path = temp.path().join("session-locators/codex-cli/parent");
    let child_path = temp.path().join("session-locators/codex-cli/child");
    let project_id: i64 = store
        .connection
        .query_row(
            "SELECT id FROM projects WHERE path=?1",
            [crate::sessions::normalize_session_path(&project).to_string_lossy()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    store
        .connection
        .execute(
            "INSERT INTO sessions(
               project_id,harness,locator,backend_id,title,modified_ms,archived_at,created_ms
             ) VALUES(?1,'codex-cli',?2,'parent','discovered title',2,5,2)",
            params![
                project_id,
                crate::sessions::normalize_session_path(&parent_path).to_string_lossy()
            ],
        )
        .map_err(|error| error.to_string())?;
    let mut parent = metadata("parent");
    parent.project = project.clone();
    parent.path = parent_path.clone();
    let merged_parent = store.update_session_metadata(&parent)?;
    assert!(merged_parent.archived);
    let mut child = metadata("child");
    child.project = project.clone();
    child.path = child_path.clone();
    child.parent_session = Some("parent".into());
    store.update_session_metadata(&child)?;

    let parents: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE harness='codex-cli' AND backend_id='parent'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(parents, 1);
    let composer = store.load_composer_sessions()?;
    assert_eq!(composer.len(), 1);
    assert_eq!(composer[0].text, "kept draft");
    assert_eq!(
        composer[0].target,
        format!("session:{}", merged_parent.path.display())
    );
    let queued = store.queued_prompts()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].message, "kept prompt");
    assert_eq!(
        queued[0].session.as_deref(),
        Some(merged_parent.path.as_path())
    );
    let cached = store.cached_sessions("")?;
    let saved_child = cached
        .iter()
        .find(|session| session.id == "child")
        .ok_or("missing child")?;
    assert_eq!(saved_child.parent_session.as_deref(), Some("parent"));
    assert_eq!(
        cached
            .iter()
            .find(|session| session.id == "parent")
            .ok_or("missing canonical parent")?
            .path,
        crate::sessions::normalize_session_path(&parent_path)
    );
    let family = store.load_worker_families()?;
    assert_eq!(family.len(), 1);
    assert_eq!(family[0].parent_session, "parent");
    assert_eq!(family[0].child_session, "child");
    let draft = store
        .load_registry()?
        .drafts
        .into_iter()
        .find(|draft| draft.id == "legacy-draft")
        .ok_or("merged draft identity was lost")?;
    assert_eq!(draft.app_session_id, merged_parent.app_session_id);
    let draft_state: (bool, i64, i64) = store
        .connection
        .query_row(
            "SELECT submitted,rail_order,created_ms FROM sessions WHERE id=?1",
            [merged_parent.app_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(draft_state, (true, 42, 1));
    let event_body: String = store
        .connection
        .query_row(
            "SELECT body FROM session_events WHERE session_id=?1 AND json_extract(body,'$.type')='legacy_event'",
            [merged_parent.app_session_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&event_body).map_err(|error| error.to_string())?,
        serde_json::json!({"type":"legacy_event","value":"kept"})
    );
    let model: (String, String, Option<String>, Option<String>) = store
        .connection
        .query_row(
            "SELECT provider,model,effort,service_tier FROM session_models WHERE session_id=?1",
            [merged_parent.app_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        model,
        (
            "legacy-provider".into(),
            "legacy-model".into(),
            Some("high".into()),
            Some("priority".into()),
        )
    );
    let foreign_key_errors: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(foreign_key_errors, 0);
    Ok(())
}

#[test]
fn interrupted_prompts_require_explicit_safe_disposition() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    let mut update = metadata("interrupted");
    update.project = temp.path().to_path_buf();
    update.path = temp.path().join("session-locators/codex-cli/interrupted");
    let session = store.update_session_metadata(&update)?;
    let ids = ["delivered", "discarded"]
        .map(|message| {
            store.enqueue_prompt(
                &format!("session:{}", session.path.display()),
                "codex-cli",
                &session.project,
                Some(&session.path),
                crate::protocol::PromptMode::Normal,
                message,
                &[],
            )
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    for id in &ids {
        store.begin_prompt(*id)?;
    }
    drop(store);

    let mut reopened = StateStore::open_at(&database)?;
    assert!(reopened.queued_prompts()?.is_empty());
    let interrupted = reopened.recover_interrupted_prompts()?;
    assert_eq!(
        interrupted
            .iter()
            .map(|prompt| prompt.message.as_str())
            .collect::<Vec<_>>(),
        vec!["delivered", "discarded"]
    );
    assert!(reopened.has_queued_prompts_for(std::slice::from_ref(&session.path))?);
    assert!(reopened.begin_prompt(ids[0]).is_err());

    let mut other = metadata("other-session");
    other.project = temp.path().to_path_buf();
    other.path = temp.path().join("session-locators/codex-cli/other-session");
    let other = reopened.update_session_metadata(&other)?;
    assert!(
        reopened
            .discard_unknown_prompt(
                ids[1],
                &format!("session:{}", other.path.display()),
                Some(&other.path),
            )
            .is_err()
    );
    assert_eq!(reopened.unknown_prompts()?.len(), 2);

    reopened.reconcile_unknown_prompt(
        ids[0],
        &format!("session:{}", session.path.display()),
        Some(&session.path),
    )?;
    reopened.discard_unknown_prompt(
        ids[1],
        &format!("session:{}", session.path.display()),
        Some(&session.path),
    )?;

    assert!(reopened.unknown_prompts()?.is_empty());
    assert!(!reopened.has_queued_prompts_for(std::slice::from_ref(&session.path))?);
    assert_eq!(reopened.accepted_prompt_history(&session.path)?.len(), 1);
    assert!(
        reopened
            .discard_unknown_prompt(
                ids[0],
                &format!("session:{}", session.path.display()),
                Some(&session.path),
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn cancelling_queued_prompts_is_atomic_and_scoped_to_exact_rows() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let project = temp.path();
    let ids = ["first queued", "second queued", "already sending"]
        .map(|message| {
            store.enqueue_prompt(
                "draft:cancel-scope",
                "codex-cli",
                project,
                None,
                crate::protocol::PromptMode::Normal,
                message,
                &[],
            )
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    store.begin_prompt(ids[2])?;
    store.cancel_queued_prompts(&ids[..2])?;

    let later = store.enqueue_prompt(
        "draft:cancel-scope",
        "codex-cli",
        project,
        None,
        crate::protocol::PromptMode::Normal,
        "must roll back",
        &[],
    )?;
    assert!(store.cancel_queued_prompts(&[ids[0], later]).is_err());

    let mut statement = store
        .connection
        .prepare("SELECT id,message,state,error FROM outbox ORDER BY id")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    assert_eq!(
        rows,
        vec![
            (
                ids[0],
                "first queued".into(),
                "failed".into(),
                Some("Prompt cancelled before delivery".into()),
            ),
            (
                ids[1],
                "second queued".into(),
                "failed".into(),
                Some("Prompt cancelled before delivery".into()),
            ),
            (ids[2], "already sending".into(), "sending".into(), None),
            (later, "must roll back".into(), "queued".into(), None),
        ]
    );
    Ok(())
}

#[test]
fn v12_migration_preserves_native_id_worker_family_links() -> Result<(), String> {
    let mut connection = Connection::open_in_memory().map_err(|error| error.to_string())?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    super::super::migrate_legacy::migrate_to_v11(&tx, 1)?;
    for (harness, id, locator, project) in [
        ("pi", "native-parent", "/sessions/parent.jsonl", "/project"),
        (
            "codex-cli",
            "native-child",
            "/locators/codex-cli/native-child",
            "/project",
        ),
        (
            "codex-cli",
            "native-child",
            "/other-project/codex-cli/native-child",
            "/other-project",
        ),
        (
            "opencode2",
            "native-child",
            "/locators/opencode2/native-child",
            "/project",
        ),
        (
            "pi",
            "native-parent",
            "/other-project/parent.jsonl",
            "/other-project",
        ),
    ] {
        tx.execute(
            "INSERT INTO sessions(path,id,project,title,first_user_message,timestamp,modified_ms,
             file_size,message_count,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,
             total_tokens,cost_micros,search_text,harness)
             VALUES(?1,?2,?3,'title','','',1,0,0,0,0,0,0,0,0,'',?4)",
            params![locator, id, project, harness],
        )
        .map_err(|error| error.to_string())?;
    }
    let link = crate::agents::WorkerFamilyLink {
        project: "/project".into(),
        parent_backend: "pi".into(),
        parent_session: "/sessions/parent.jsonl".into(),
        child_backend: "codex-cli".into(),
        child_session: "native-child".into(),
        execution: None,
    };
    tx.execute(
        "INSERT INTO meta(key,value) VALUES('worker_family:child',?1)",
        [serde_json::to_string(&link).map_err(|error| error.to_string())?],
    )
    .map_err(|error| error.to_string())?;
    let unresolved = crate::agents::WorkerFamilyLink {
        child_session: "missing-child".into(),
        ..link.clone()
    };
    tx.execute(
        "INSERT INTO meta(key,value) VALUES('worker_family:missing',?1)",
        [serde_json::to_string(&unresolved).map_err(|error| error.to_string())?],
    )
    .map_err(|error| error.to_string())?;

    super::super::migrate_v12::migrate_v11_to_v12(&tx)?;

    let family: (String, String, String, String, String) = tx
        .query_row(
            "SELECT child.backend_id,child.harness,parent.backend_id,parent.harness,p.path
             FROM worker_families f
             JOIN sessions child ON child.id=f.child_id
             JOIN sessions parent ON parent.id=child.parent_id
             JOIN projects p ON p.id=child.project_id",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        family,
        (
            "native-child".into(),
            "codex-cli".into(),
            "native-parent".into(),
            "pi".into(),
            "/project".into()
        )
    );
    let legacy_link: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key='worker_family:child')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert!(!legacy_link);
    let unresolved_json: String = tx
        .query_row(
            "SELECT value FROM meta WHERE key='worker_family:missing'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        serde_json::from_str::<crate::agents::WorkerFamilyLink>(&unresolved_json)
            .map_err(|error| error.to_string())?,
        unresolved
    );
    Ok(())
}

#[test]
fn schema_v14_upgrade_preserves_sending_outbox_and_adds_unknown_state() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    let mut update = metadata("v14-outbox");
    update.project = temp.path().to_path_buf();
    update.path = temp.path().join("session-locators/codex-cli/v14-outbox");
    let session = store.update_session_metadata(&update)?;
    let queued_id = store.enqueue_prompt(
        &format!("session:{}", session.path.display()),
        "codex-cli",
        &session.project,
        Some(&session.path),
        crate::protocol::PromptMode::Normal,
        "still queued",
        &[],
    )?;
    let image = PromptImage::new("aGVsbG8=".into(), "image/png".into());
    let id = store.enqueue_prompt_with_presentation(
        &format!("session:{}", session.path.display()),
        "codex-cli",
        &session.project,
        Some(&session.path),
        crate::protocol::PromptMode::FollowUp,
        "possibly delivered",
        Some("shown text"),
        Some("expanded text"),
        std::slice::from_ref(&image),
    )?;
    store.begin_prompt(id)?;
    let failed_id = store.enqueue_prompt(
        &format!("session:{}", session.path.display()),
        "codex-cli",
        &session.project,
        Some(&session.path),
        crate::protocol::PromptMode::Normal,
        "already failed",
        &[],
    )?;
    store.fail_prompt(failed_id, "prior failure")?;
    store
        .connection
        .execute_batch(&format!(
            "UPDATE outbox SET created_ms=101,provider='queued-provider' WHERE id={queued_id};
                 UPDATE outbox SET submission_event_seq=7,provider='provider',model='model',
                        effort='high',service_tier='priority',error='old error',created_ms=102
                  WHERE id={id};
                 UPDATE outbox SET created_ms=103,model='failed-model' WHERE id={failed_id};"
        ))
        .map_err(|error| error.to_string())?;
    let persisted_images: String = store
        .connection
        .query_row("SELECT images_json FROM outbox WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    drop(store);

    let connection = Connection::open(&database).map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "ALTER TABLE outbox RENAME TO outbox_v15;
             CREATE TABLE outbox (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               submission_event_seq INTEGER,
               mode TEXT NOT NULL,
               message TEXT NOT NULL,
               display_message TEXT,
               invocation TEXT,
               images_json TEXT NOT NULL DEFAULT '[]',
               provider TEXT,
               model TEXT,
               effort TEXT,
               service_tier TEXT,
               state TEXT NOT NULL DEFAULT 'queued'
                 CHECK (state IN ('queued', 'sending', 'failed')),
               error TEXT,
               created_ms INTEGER NOT NULL
             );
             INSERT INTO outbox SELECT * FROM outbox_v15;
             DROP TABLE outbox_v15;
             CREATE INDEX outbox_session_state ON outbox(session_id, state, id);
             UPDATE meta SET value='14' WHERE key='schema_version';",
        )
        .map_err(|error| error.to_string())?;
    drop(connection);

    let reopened = StateStore::open_at(&database)?;
    assert_eq!(
        reopened
            .queued_prompts()?
            .iter()
            .map(|prompt| (prompt.id, prompt.message.as_str()))
            .collect::<Vec<_>>(),
        vec![(queued_id, "still queued")]
    );
    let interrupted = reopened.recover_interrupted_prompts()?;
    assert_eq!(interrupted.len(), 1);
    assert_eq!(interrupted[0].id, id);
    assert_eq!(interrupted[0].mode, crate::protocol::PromptMode::FollowUp);
    assert_eq!(interrupted[0].message, "possibly delivered");
    assert_eq!(
        interrupted[0].display_message.as_deref(),
        Some("shown text")
    );
    assert_eq!(interrupted[0].invocation.as_deref(), Some("expanded text"));
    assert_eq!(interrupted[0].images[0].clone().into_inline()?, image);
    let migrated_images: String = reopened
        .connection
        .query_row("SELECT images_json FROM outbox WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(migrated_images, persisted_images);
    let preserved: (i64, String, String, String, String, String, String, i64) = reopened
        .connection
        .query_row(
            "SELECT submission_event_seq,provider,model,effort,service_tier,state,error,created_ms
               FROM outbox WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        preserved.0, 7,
        "submission event link must survive the table rebuild"
    );
    assert_eq!(
        (
            preserved.1.as_str(),
            preserved.2.as_str(),
            preserved.3.as_str(),
            preserved.4.as_str(),
            preserved.5.as_str(),
            preserved.6.as_str(),
        ),
        (
            "provider",
            "model",
            "high",
            "priority",
            "unknown",
            "Delivery status unknown after app interruption",
        )
    );
    assert_eq!(preserved.7, 102);
    let states: Vec<(i64, i64, String, String, Option<String>, i64)> = {
        let mut statement = reopened
            .connection
            .prepare("SELECT id,session_id,message,state,error,created_ms FROM outbox ORDER BY id")
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?
    };
    assert_eq!(
        states,
        vec![
            (
                queued_id,
                session.app_session_id,
                "still queued".into(),
                "queued".into(),
                None,
                101,
            ),
            (
                id,
                session.app_session_id,
                "possibly delivered".into(),
                "unknown".into(),
                Some("Delivery status unknown after app interruption".into()),
                102,
            ),
            (
                failed_id,
                session.app_session_id,
                "already failed".into(),
                "failed".into(),
                Some("prior failure".into()),
                103,
            ),
        ]
    );
    let failed_model: Option<String> = reopened
        .connection
        .query_row("SELECT model FROM outbox WHERE id=?1", [failed_id], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(failed_model.as_deref(), Some("failed-model"));
    let index_exists: bool = reopened
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='outbox_session_state')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert!(index_exists);
    let foreign_key_errors: i64 = reopened
        .connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(foreign_key_errors, 0);
    Ok(())
}
