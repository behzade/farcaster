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

#[derive(Clone, Copy, Debug)]
enum DeliveryPath {
    Live,
    History,
    ReceiptWithOutbox,
    ReceiptFromDispatch,
}

impl DeliveryPath {
    fn complete(self, store: &mut StateStore, id: i64, session: &Path) -> Result<(), String> {
        match self {
            Self::Live => store.complete_delivered_prompt(
                id,
                "session:expanded",
                Some(session),
                "expanded-receipt",
                true,
            ),
            Self::History => store.reconcile_prompt_deliveries(
                session,
                &crate::sessions::PromptDeliveryReconciliation {
                    delivered: vec!["expanded-receipt".into()],
                    pending: Vec::new(),
                    absence_is_not_delivered: false,
                },
            ),
            Self::ReceiptWithOutbox => {
                store.record_prompt_receipt_delivered("expanded-receipt", Some(id))
            }
            Self::ReceiptFromDispatch => {
                store.record_prompt_receipt_delivered("expanded-receipt", None)
            }
        }
    }
}

#[test]
fn every_delivery_path_preserves_expanded_prompt_after_restart_and_repeated_completion()
-> Result<(), Box<dyn std::error::Error>> {
    for path in [
        DeliveryPath::Live,
        DeliveryPath::History,
        DeliveryPath::ReceiptWithOutbox,
        DeliveryPath::ReceiptFromDispatch,
    ] {
        for accepted_before_restart in [false, true] {
            let temp = tempfile::tempdir()?;
            let database = temp.path().join("state.sqlite3");
            let session = temp.path().join("session");
            let mut store = StateStore::open_at(&database)?;
            let image = PromptImage::new("aGVsbG8=".into(), "image/png".into());
            let enqueue = || {
                store.enqueue_prompt_with_presentation(
                    "session:expanded",
                    Backend::Codex,
                    temp.path(),
                    Some(&session),
                    PromptMode::FollowUp,
                    "Expanded instructions that should stay collapsed",
                    Some("$prompt:commit"),
                    Some("commit"),
                    std::slice::from_ref(&image),
                )
            };
            let id = enqueue()?;
            let other = enqueue()?;
            store.record_prompt_dispatch(id, "expanded-receipt")?;
            store.record_prompt_dispatch(other, "other-receipt")?;
            if accepted_before_restart {
                store.record_prompt_acceptance(
                    id,
                    "session:expanded",
                    Some(&session),
                    "expanded-receipt",
                    false,
                )?;
            }
            drop(store);

            let mut store = StateStore::open_at(&database)?;
            path.complete(&mut store, id, &session)?;
            drop(store);
            let mut store = StateStore::open_at(&database)?;
            let presentations = store.prompt_presentations(&session)?;
            assert_eq!(
                presentations,
                vec![PromptPresentation {
                    resolved_message: "Expanded instructions that should stay collapsed".into(),
                    display_message: "$prompt:commit".into(),
                    invocation: "commit".into(),
                }],
                "{path:?}, previously accepted: {accepted_before_restart}"
            );
            let queued = store.queued_prompts()?;
            assert_eq!(queued.len(), 1);
            assert_eq!(queued[0].id, other);
            assert_eq!(queued[0].images[0].clone().into_inline()?, image);
            assert!(store.accepted_prompt_history(&session)?.is_empty());
            let accepted: String = store.connection.query_row(
                "SELECT body FROM session_events WHERE json_extract(body,'$.type')='accepted_prompt'",
                [],
                |row| row.get(0),
            )?;
            let accepted: serde_json::Value = serde_json::from_str(&accepted)?;
            assert_eq!(accepted["message"], presentations[0].resolved_message);
            assert_eq!(accepted["promptMode"], "follow_up");
            assert_eq!(accepted["deliveryTracked"], !accepted_before_restart);
            let images = store.decode_prompt_images(&accepted["images"].to_string())?;
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].clone().into_inline()?, image);
            let submitted: bool = store.connection.query_row(
                "SELECT submitted FROM sessions WHERE id=(SELECT session_id FROM outbox WHERE id=?1)",
                [id],
                |row| row.get(0),
            )?;
            assert!(submitted);
            let events_before: i64 =
                store
                    .connection
                    .query_row("SELECT COUNT(*) FROM session_events", [], |row| row.get(0))?;
            path.complete(&mut store, id, &session)?;
            DeliveryPath::Live.complete(&mut store, id, &session)?;
            DeliveryPath::History.complete(&mut store, id, &session)?;
            DeliveryPath::ReceiptFromDispatch.complete(&mut store, id, &session)?;
            drop(store);
            let store = StateStore::open_at(&database)?;
            assert_eq!(store.prompt_presentations(&session)?, presentations);
            let events_after: i64 =
                store
                    .connection
                    .query_row("SELECT COUNT(*) FROM session_events", [], |row| row.get(0))?;
            assert_eq!(events_after, events_before, "{path:?} must be idempotent");
            let receipts: i64 = store.connection.query_row(
                "SELECT COUNT(*) FROM session_events WHERE json_extract(body,'$.type')='prompt_delivery_receipt'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(receipts, 1);
        }
    }
    Ok(())
}

#[test]
fn every_delivery_path_rolls_back_if_presentation_write_fails()
-> Result<(), Box<dyn std::error::Error>> {
    for path in [
        DeliveryPath::Live,
        DeliveryPath::History,
        DeliveryPath::ReceiptWithOutbox,
        DeliveryPath::ReceiptFromDispatch,
    ] {
        let temp = tempfile::tempdir()?;
        let database = temp.path().join("state.sqlite3");
        let session = temp.path().join("session");
        let mut store = StateStore::open_at(&database)?;
        let id = store.enqueue_prompt_with_presentation(
            "session:expanded",
            Backend::Codex,
            temp.path(),
            Some(&session),
            PromptMode::Normal,
            "expanded",
            Some("display"),
            Some("invocation"),
            &[],
        )?;
        store.record_prompt_dispatch(id, "expanded-receipt")?;
        store.connection.execute_batch(
            "CREATE TRIGGER fail_presentation BEFORE INSERT ON session_events
              WHEN json_extract(NEW.body,'$.type')='prompt_presentation'
              BEGIN SELECT RAISE(ABORT, 'test presentation failure'); END;",
        )?;
        let error = path
            .complete(&mut store, id, &session)
            .expect_err("presentation write must fail");
        assert!(error.contains("test presentation failure"), "{path:?}");
        drop(store);
        let mut store = StateStore::open_at(&database)?;
        assert_eq!(store.queued_prompts()?.len(), 1);
        assert!(store.prompt_presentations(&session)?.is_empty());
        let events: i64 = store.connection.query_row(
            "SELECT COUNT(*) FROM session_events WHERE json_extract(body,'$.type')!='prompt_dispatch'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(events, 0, "{path:?} must roll back all delivery effects");
        let submitted: bool = store.connection.query_row(
            "SELECT submitted FROM sessions WHERE id=(SELECT session_id FROM outbox WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?;
        assert!(!submitted);
        store
            .connection
            .execute_batch("DROP TRIGGER fail_presentation")?;
        path.complete(&mut store, id, &session)?;
        assert!(store.queued_prompts()?.is_empty());
        assert_eq!(store.prompt_presentations(&session)?.len(), 1);
    }
    Ok(())
}

#[test]
fn delivery_evidence_does_not_restore_cancelled_outbox_payloads()
-> Result<(), Box<dyn std::error::Error>> {
    for path in [
        DeliveryPath::Live,
        DeliveryPath::History,
        DeliveryPath::ReceiptWithOutbox,
        DeliveryPath::ReceiptFromDispatch,
    ] {
        let temp = tempfile::tempdir()?;
        let session = temp.path().join("session");
        let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
        let id = store.enqueue_prompt_with_presentation(
            "session:expanded",
            Backend::Codex,
            temp.path(),
            Some(&session),
            PromptMode::Normal,
            "expanded",
            Some("display"),
            Some("invocation"),
            &[],
        )?;
        store.record_prompt_dispatch(id, "expanded-receipt")?;
        store.cancel_queued_prompts(&[id])?;
        path.complete(&mut store, id, &session)?;
        let state: String =
            store
                .connection
                .query_row("SELECT state FROM outbox WHERE id=?1", [id], |row| {
                    row.get(0)
                })?;
        assert_eq!(state, "cancelled", "{path:?}");
        assert!(store.prompt_presentations(&session)?.is_empty());
        assert!(store.accepted_prompt_history(&session)?.is_empty());
    }
    Ok(())
}

#[test]
fn late_receipt_resolves_accepted_history_without_an_outbox_row()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let id = store.enqueue_prompt(
        "session:expanded",
        Backend::Codex,
        temp.path(),
        Some(&session),
        PromptMode::Normal,
        "accepted payload",
        &[],
    )?;
    store.record_prompt_acceptance(
        id,
        "session:expanded",
        Some(&session),
        "expanded-receipt",
        true,
    )?;
    store
        .connection
        .execute("DELETE FROM outbox WHERE id=?1", [id])?;
    assert_eq!(store.accepted_prompt_history(&session)?.len(), 1);
    store.record_prompt_receipt_delivered("expanded-receipt", None)?;
    store.record_prompt_receipt_delivered("expanded-receipt", None)?;
    assert!(store.accepted_prompt_history(&session)?.is_empty());
    let receipts: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM session_events WHERE json_extract(body,'$.type')='prompt_delivery_receipt'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(receipts, 1);
    Ok(())
}
