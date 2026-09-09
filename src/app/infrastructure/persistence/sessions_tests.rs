use super::*;

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
fn live_metadata_preserves_archive_identity_and_other_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).unwrap();
    let mut update = metadata("parent");
    let parent = store.update_session_metadata(&update).unwrap();
    store.set_session_archived(&parent.path, true).unwrap();
    let other = store.update_session_metadata(&metadata("other")).unwrap();
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
    let updated = store.update_session_metadata(&update).unwrap();
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
        .unwrap();
    assert_eq!(saved, ("model".into(), Some("priority".into())));
    let cached = store.cached_sessions("").unwrap();
    assert_eq!(cached.len(), 2);
    let unchanged = cached.iter().find(|s| s.id == "other").unwrap();
    assert_eq!(unchanged.modified, other.modified);
    assert_eq!(unchanged.title, other.title);
}

#[test]
fn child_events_preserve_family_and_metadata_on_completion() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).unwrap();
    store.update_session_metadata(&metadata("parent")).unwrap();
    let mut child = metadata("child");
    child.parent_session = Some("parent".into());
    let started = store.update_session_metadata(&child).unwrap();
    assert!(started.is_running);
    child.is_running = false;
    child.title = None;
    child.message_count = None;
    let ended = store.update_session_metadata(&child).unwrap();
    assert!(!ended.is_running);
    assert_eq!(ended.app_session_id, started.app_session_id);
    assert_eq!(ended.title, "child");
    assert_eq!(ended.message_count, 1);
    assert_eq!(ended.parent_session.as_deref(), Some("parent"));
}

#[test]
fn failed_live_update_rolls_back_without_changing_archive_state() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3")).unwrap();
    let update = metadata("archived");
    store.update_session_metadata(&update).unwrap();
    store.set_session_archived(&update.path, true).unwrap();
    let before = store.cached_sessions("").unwrap();
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER fail_update BEFORE UPDATE ON sessions
        BEGIN SELECT RAISE(ABORT, 'test write failure'); END;",
        )
        .unwrap();
    assert!(store.update_session_metadata(&update).is_err());
    assert_eq!(store.cached_sessions("").unwrap(), before);
}
