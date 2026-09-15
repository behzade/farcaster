use super::*;
use serde_json::json;

fn caller(project: &Path, session: &str) -> crate::agents::CallerContext {
    crate::agents::CallerContext {
        worker_id: "worker".into(),
        worker_name: "Worker".into(),
        project: project.into(),
        session: session.into(),
        backend: Backend::Cursor,
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Auto,
        parent_worker_id: None,
    }
}
fn artifact(project: &Path, id: &str) -> serde_json::Value {
    json!({"farcaster_review":{"version":1,"id":id,"project":project,"review":{"title":"Review","items":[{"path":"README.md","note":"Inspect"}]}}})
}
fn execution(
    store: &StateStore,
    caller: &crate::agents::CallerContext,
    turn: &str,
) -> crate::agents::ExecutionBinding {
    let execution = crate::agents::ExecutionBinding {
        session_record: store
            .register_caller_session(caller)
            .expect("register session"),
        turn_id: turn.into(),
        prompt_id: Some(turn.into()),
    };
    store.register_execution(&execution).expect("register turn");
    execution
}

#[test]
fn review_submission_is_read_only_for_session_identity_and_uses_captured_turn() -> Result<(), String>
{
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let caller = caller(temp.path(), "native");
    let first = execution(&store, &caller, "first");
    let second = execution(&store, &caller, "second");
    let before: (i64, String) = store
        .connection
        .query_row("SELECT id,locator FROM sessions", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("session");
    // Newer execution registration cannot retarget a request captured earlier.
    store.save_review(&caller, &first, &artifact(temp.path(), "review"))?;
    let after: (i64, String) = store
        .connection
        .query_row("SELECT id,locator FROM sessions", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("session");
    assert_eq!(before, after);
    let saved = store.session_reviews(
        Backend::Cursor,
        temp.path(),
        &temp.path().join("session-locators/cursor-cli/native"),
    )?;
    assert_eq!(saved[0].turn_id.as_deref(), Some("first"));
    assert_eq!(saved[0].prompt_id.as_deref(), Some("first"));
    assert_ne!(saved[0].turn_id.as_deref(), Some(second.turn_id.as_str()));
    Ok(())
}

#[test]
fn bindings_and_reviews_survive_merges_reopen_and_delete() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let caller = caller(temp.path(), "native");
    let execution = execution(&store, &caller, "turn");
    let other = self::caller(temp.path(), "other");
    let keep = store.register_caller_session(&other)?;
    let tx = store
        .connection
        .unchecked_transaction()
        .expect("transaction");
    super::super::identity::merge_session(&tx, keep, execution.session_record)?;
    tx.commit().expect("merge");
    store.save_review(&caller, &execution, &artifact(temp.path(), "review"))?;
    drop(store);
    let store = StateStore::open_at(&database)?;
    let path = temp.path().join("session-locators/cursor-cli/native");
    assert_eq!(
        store
            .session_reviews(Backend::Cursor, temp.path(), &path)?
            .len(),
        1
    );
    assert!(
        store
            .session_reviews(Backend::Antigravity, temp.path(), &path)?
            .is_empty()
    );
    assert!(
        store
            .save_review(
                &self::caller(temp.path(), "stranger"),
                &execution,
                &artifact(temp.path(), "wrong")
            )
            .is_err()
    );
    store
        .connection
        .execute("DELETE FROM sessions", [])
        .expect("delete");
    assert!(
        store
            .session_reviews(Backend::Cursor, temp.path(), &path)?
            .is_empty()
    );
    Ok(())
}

#[test]
fn failed_review_write_does_not_provision_sessions_or_claim_success() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let caller = caller(temp.path(), "native");
    let execution = execution(&store, &caller, "turn");
    store.connection.execute_batch("CREATE TRIGGER reject_review BEFORE INSERT ON session_reviews BEGIN SELECT RAISE(FAIL,'disk failure'); END;").expect("trigger");
    assert!(
        store
            .save_review(&caller, &execution, &artifact(temp.path(), "review"))
            .is_err()
    );
    let count: i64 = store
        .connection
        .query_row("SELECT count(*) FROM session_reviews", [], |r| r.get(0))
        .expect("count");
    assert_eq!(count, 0);
    Ok(())
}

#[test]
fn migration_preserves_legacy_artifacts_without_guessing_a_turn() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let caller = caller(temp.path(), "native");
    let record = store.register_caller_session(&caller)?;
    let body = json!({"type":"review_submitted","submission":{"artifact":artifact(temp.path(),"legacy"),"prompt_id":"ambiguous"}});
    store
        .connection
        .execute(
            "INSERT INTO session_events VALUES(?1,1,1,1,?2)",
            params![record, body.to_string()],
        )
        .expect("legacy record");
    store.connection.execute_batch("DROP TABLE session_reviews; DROP TABLE session_turns; UPDATE meta SET value='16' WHERE key='schema_version';").expect("old schema");
    drop(store);
    let store = StateStore::open_at(&database)?;
    let saved = store.session_reviews(
        Backend::Cursor,
        temp.path(),
        &temp.path().join("session-locators/cursor-cli/native"),
    )?;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, "legacy");
    assert!(saved[0].turn_id.is_none());
    Ok(())
}

#[test]
fn review_lookup_uses_session_indexes_without_reading_prompt_journals() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let caller = caller(temp.path(), "native");
    let execution = execution(&store, &caller, "turn");
    store.save_review(&caller, &execution, &artifact(temp.path(), "review"))?;
    let mut statement = store
        .connection
        .prepare(&format!("EXPLAIN QUERY PLAN {SESSION_REVIEWS_SQL}"))
        .expect("query plan");
    let plan = statement
        .query_map(
            params![
                Backend::Cursor,
                temp.path().to_string_lossy(),
                "locator",
                "native"
            ],
            |row| row.get::<_, String>(3),
        )
        .expect("plan rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("decode plan")
        .join("\n");
    assert!(plan.contains("session_reviews_session"), "{plan}");
    assert!(plan.contains("sessions_native_identity"), "{plan}");
    assert!(!plan.contains("session_events"), "{plan}");
    store
        .connection
        .execute_batch("DROP TABLE session_events;")
        .expect("drop test journal");
    assert_eq!(
        store
            .session_reviews(
                Backend::Cursor,
                temp.path(),
                &temp.path().join("session-locators/cursor-cli/native")
            )?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn identical_backend_ids_in_different_projects_keep_distinct_review_owners() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("storage");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir_all(&first).expect("first project");
    std::fs::create_dir_all(&second).expect("second project");
    let a = caller(&first, "native");
    let b = caller(&second, "native");
    let turn_a = execution(&store, &a, "first-turn");
    let turn_b = execution(&store, &b, "second-turn");
    assert_ne!(turn_a.session_record, turn_b.session_record);
    assert!(
        store
            .save_review(&b, &turn_a, &artifact(&second, "wrong-project"))
            .is_err()
    );
    store.save_review(&a, &turn_a, &artifact(&first, "first-review"))?;
    store.save_review(&b, &turn_b, &artifact(&second, "second-review"))?;
    let path = temp.path().join("session-locators/cursor-cli/native");
    assert_eq!(
        store.session_reviews(Backend::Cursor, &first, &path)?[0].id,
        "first-review"
    );
    assert_eq!(
        store.session_reviews(Backend::Cursor, &second, &path)?[0].id,
        "second-review"
    );
    Ok(())
}
