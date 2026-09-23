use super::*;
use crate::agents::Backend;

#[test]
fn accepted_receipt_stays_pending_and_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        "draft:a",
        Backend::Codex,
        temp.path(),
        None,
        PromptMode::FollowUp,
        "same text",
        &[],
    )?;
    store.record_prompt_acceptance(id, "draft:a", Some(&session), "first", true)?;
    store.record_prompt_acceptance(id, "draft:a", Some(&session), "first", true)?;
    assert_eq!(store.queued_prompts()?.len(), 1);
    let accepted: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM session_events
          WHERE json_extract(body,'$.type')='accepted_prompt'
            AND json_extract(body,'$.submissionId')='first'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(accepted, 1);
    drop(store);
    let store = StateStore::open_at(&database)?;
    assert_eq!(store.queued_prompts()?.len(), 1);
    Ok(())
}

#[test]
fn late_delivery_receipt_acks_only_its_pending_outbox_row() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let first = store.enqueue_prompt(
        "draft:late",
        Backend::Codex,
        temp.path(),
        None,
        PromptMode::FollowUp,
        "first",
        &[],
    )?;
    let second = store.enqueue_prompt(
        "draft:late",
        Backend::Codex,
        temp.path(),
        None,
        PromptMode::FollowUp,
        "second",
        &[],
    )?;

    store.record_prompt_receipt_delivered("late-first", Some(first))?;

    let states = store
        .connection
        .prepare("SELECT id, state FROM outbox ORDER BY id")?
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        states,
        vec![(first, "acked".into()), (second, "pending".into())]
    );
    Ok(())
}

#[test]
fn native_history_acks_only_the_dispatched_row_after_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let store = StateStore::open_at(&database)?;
    let first = store.enqueue_prompt(
        "session:replay",
        Backend::Codex,
        temp.path(),
        Some(&session),
        PromptMode::Normal,
        "same text",
        &[],
    )?;
    let second = store.enqueue_prompt(
        "session:replay",
        Backend::Codex,
        temp.path(),
        Some(&session),
        PromptMode::Normal,
        "same text",
        &[],
    )?;
    store.record_prompt_dispatch(first, "codex-cli-first")?;
    store.record_prompt_dispatch(second, "codex-cli-second")?;
    drop(store);

    let mut store = StateStore::open_at(&database)?;
    store.reconcile_prompt_deliveries(
        &session,
        &crate::sessions::PromptDeliveryReconciliation {
            delivered: vec!["codex-cli-first".into()],
            pending: Vec::new(),
            absence_is_not_delivered: false,
        },
    )?;
    let queued = store.queued_prompts()?;
    assert_eq!(
        queued.iter().map(|prompt| prompt.id).collect::<Vec<_>>(),
        [second]
    );
    let states = store
        .connection
        .prepare("SELECT state FROM outbox ORDER BY id")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(states, ["acked", "pending"]);
    store.record_prompt_receipt_delivered("codex-cli-second", None)?;
    assert!(store.queued_prompts()?.is_empty());
    Ok(())
}

#[test]
fn delivered_prompt_completion_rolls_back_acceptance_if_delivery_write_fails()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        "draft:atomic",
        Backend::Codex,
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
    assert_eq!(saved, ("pending".into(), "exact queued payload".into()));
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
