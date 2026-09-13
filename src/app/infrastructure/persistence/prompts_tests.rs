use super::*;

#[test]
fn delivered_prompt_completion_rolls_back_acceptance_if_delivery_write_fails()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        "draft:atomic",
        "codex-cli",
        temp.path(),
        None,
        PromptMode::FollowUp,
        "exact queued payload",
        &[],
    )?;
    store.begin_prompt(id)?;
    store.connection.execute_batch(
        "CREATE TRIGGER fail_delivery_write BEFORE INSERT ON session_events
          WHEN json_extract(NEW.body,'$.type')='prompt_delivery_receipt'
          BEGIN SELECT RAISE(ABORT, 'test delivery write failure'); END;",
    )?;
    let error = store
        .complete_delivered_prompt(id, "draft:atomic", Some(&session), "atomic-id", true)
        .expect_err("delivery write must fail");
    assert!(error.contains("test delivery write failure"));
    drop(store);

    let mut store = StateStore::open_at(&database)?;
    let saved: (String, String) = store.connection.query_row(
        "SELECT state,message FROM outbox WHERE id=?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(saved, ("sending".into(), "exact queued payload".into()));
    let accepted: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM session_events WHERE json_extract(body,'$.submissionId')='atomic-id'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        accepted, 0,
        "no half-committed acceptance survives reopening"
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_delivery_write")?;
    store.complete_delivered_prompt(id, "draft:atomic", Some(&session), "atomic-id", true)?;
    store.complete_delivered_prompt(id, "draft:atomic", Some(&session), "atomic-id", true)?;
    drop(store);

    let store = StateStore::open_at(&database)?;
    assert!(store.queued_prompts()?.is_empty());
    assert!(store.unknown_prompts()?.is_empty());
    assert!(
        store.accepted_prompt_history(&session)?.is_empty(),
        "delivered input is not pending history"
    );
    let events: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM session_events WHERE json_extract(body,'$.submissionId')='atomic-id'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(events, 2, "one accepted payload and one delivery receipt");
    Ok(())
}
