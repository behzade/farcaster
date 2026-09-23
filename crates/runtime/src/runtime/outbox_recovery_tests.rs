use crate::agents::Backend;
use std::{cell::RefCell, rc::Rc};

use super::*;
use crate::runtime::tests::owner_without_process;

#[derive(Default)]
struct Recorder(Rc<RefCell<Vec<SessionCommand>>>);

impl SessionTransport for Recorder {
    fn send(&mut self, command: SessionCommand) -> Result<String, String> {
        let mut commands = self.0.borrow_mut();
        commands.push(command);
        Ok(format!("request-{}", commands.len()))
    }

    fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self) -> Option<SessionEvent> {
        None
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

fn empty_session() -> SessionState {
    serde_json::from_value(json!({
        "sessionId": "replay-session",
        "isStreaming": false,
        "isCompacting": false,
        "autoCompactionEnabled": true,
        "messageCount": 0,
        "pendingMessageCount": 0
    }))
    .expect("session fixture")
}

fn prompt_response(id: &str, mode: PromptMode, success: bool) -> crate::agents::SessionResponse {
    if success {
        crate::agents::SessionResponse::success(
            Some(id.into()),
            crate::agents::SessionResponsePayload::Prompt(mode),
        )
    } else {
        crate::agents::SessionResponse::failure(
            Some(id.into()),
            SessionOperation::Prompt(mode),
            "rejected by test harness".into(),
        )
    }
}

fn delivered(id: &str, message: &str) -> SessionEvent {
    SessionEvent::Activity(
        json!({
            "type":"prompt_delivery",
            "submissionId":id,
            "status":"delivered",
            "message":{"role":"user", "content":[{"type":"text", "text":message}]},
        })
        .into(),
    )
}

fn sent_messages(sent: &Rc<RefCell<Vec<SessionCommand>>>) -> Vec<String> {
    sent.borrow()
        .iter()
        .filter_map(|command| match command {
            SessionCommand::Prompt { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn outbox_rows(database: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    let connection = rusqlite::Connection::open(database).map_err(|error| error.to_string())?;
    connection
        .prepare("SELECT message, state FROM outbox ORDER BY id")
        .map_err(|error| error.to_string())?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn ready_owner(
    root: &std::path::Path,
    database: &std::path::Path,
) -> Result<(RuntimeOwner, Rc<RefCell<Vec<SessionCommand>>>), String> {
    let (mut owner, _) = owner_without_process(root.into());
    let sent = Rc::new(RefCell::new(Vec::new()));
    owner.process = Some(Box::new(Recorder(sent.clone())));
    owner.state = Some(SharedStateStore::open_at(database)?);
    owner.active_session = Some(root.join("session.jsonl"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    Ok((owner, sent))
}

#[test]
fn recovered_steer_and_follow_up_stay_out_of_the_transcript_until_delivery() -> Result<(), String> {
    for mode in [PromptMode::Steer, PromptMode::FollowUp] {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let database = temp.path().join("state.sqlite3");
        let store = StateStore::open_at(&database)?;
        store.enqueue_prompt(
            "session:recovered",
            Backend::Claude,
            temp.path(),
            Some(&temp.path().join("session.jsonl")),
            mode,
            "saved queued input",
            &[],
        )?;
        let prompt = store.queued_prompts()?.remove(0);
        let (mut owner, _) = ready_owner(temp.path(), &database)?;
        owner.harness = Some(Backend::Claude);
        owner.state = Some(store.into());

        owner.deliver_queued(prompt);

        assert!(owner.snapshot.conversation.items.is_empty(), "{mode:?}");
    }
    Ok(())
}

#[test]
fn abort_cancels_only_local_queue_work() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    for message in ["active task", "next task", "last task"] {
        store.enqueue_prompt(
            "draft:abort",
            Backend::Pi,
            temp.path(),
            None,
            PromptMode::Normal,
            message,
            &[],
        )?;
    }
    let recovered = store.queued_prompts()?;
    let active_id = recovered[0].id;
    let unrelated_id = store.enqueue_prompt(
        "draft:unrelated",
        Backend::Pi,
        temp.path(),
        None,
        PromptMode::Normal,
        "unrelated task",
        &[],
    )?;
    let (mut owner, sent) = ready_owner(temp.path(), &database)?;
    owner.state = Some(store.into());
    for prompt in recovered {
        owner.deliver_queued(prompt);
    }
    owner.apply_response(prompt_response("request-1", PromptMode::Normal, true));
    assert_eq!(sent_messages(&sent), ["active task"]);

    owner.apply_command(RuntimeCommand::Abort);
    owner.apply_response(crate::agents::SessionResponse::prompt_delivery_unknown(
        "request-1".into(),
        PromptMode::Normal,
        "abort ended before a model receipt".into(),
    ));
    owner.apply_process_item(SessionEvent::Activity(json!({"type":"agent_start"}).into()));
    owner.apply_process_item(SessionEvent::Activity(
        json!({"type":"agent_settled"}).into(),
    ));
    assert!(
        !owner.normal_prompt_in_flight,
        "settlement releases normal dispatch"
    );
    assert!(
        !owner.active_snapshot().conversation.running,
        "settlement ends the turn"
    );
    assert!(owner.pending_prompt_id.is_none());
    assert!(owner.pending_prompt_target.is_none());
    owner.maybe_send_deferred_prompt();
    assert_eq!(sent_messages(&sent), ["active task"]);
    drop(owner);

    let reopened = StateStore::open_at(&database)?;
    assert_eq!(
        reopened
            .queued_prompts()?
            .iter()
            .map(|prompt| prompt.id)
            .collect::<Vec<_>>(),
        [active_id, unrelated_id]
    );
    assert_eq!(
        outbox_rows(&database)?,
        [
            ("active task".into(), "pending".into()),
            ("next task".into(), "cancelled".into()),
            ("last task".into(), "cancelled".into()),
            ("unrelated task".into(), "pending".into()),
        ]
    );
    Ok(())
}

#[test]
fn abort_cancels_a_not_yet_dispatched_prompt_durably() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, sent) = ready_owner(temp.path(), &database)?;
    owner.startup_state_loaded = false;
    owner.startup_history_loaded = false;

    owner.send_prompt(
        "draft:startup".into(),
        PromptMode::Normal,
        "cancel before ready".into(),
        Vec::new(),
        false,
    );
    assert!(owner.deferred_prompt.is_some());
    owner.apply_command(RuntimeCommand::Abort);

    assert!(owner.deferred_prompt.is_none());
    assert!(owner.pending_prompt_item.is_none());
    assert!(
        owner
            .state
            .as_ref()
            .expect("state")
            .with(|store| store.queued_prompts())?
            .is_empty()
    );
    assert_eq!(
        outbox_rows(&database)?,
        [("cancel before ready".into(), "cancelled".into())]
    );
    assert!(matches!(sent.borrow().as_slice(), [SessionCommand::Abort]));
    Ok(())
}

#[test]
fn startup_replays_normal_prompts_in_order_after_delivery_and_settlement() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let target = "draft:replay";
    let first = store.enqueue_prompt(
        target,
        Backend::Pi,
        temp.path(),
        None,
        PromptMode::Normal,
        "first",
        &[],
    )?;
    let second = store.enqueue_prompt(
        target,
        Backend::Pi,
        temp.path(),
        None,
        PromptMode::Normal,
        "second",
        &[],
    )?;
    let third = store.enqueue_prompt(
        target,
        Backend::Pi,
        temp.path(),
        None,
        PromptMode::Normal,
        "third",
        &[],
    )?;
    let prompts = store.queued_prompts()?;
    let (mut owner, sent) = ready_owner(temp.path(), &database)?;
    owner.state = Some(store.into());
    for prompt in prompts {
        owner.deliver_queued(prompt);
    }
    owner.maybe_send_deferred_prompt();
    assert_eq!(sent_messages(&sent), ["first"]);

    owner.apply_response(prompt_response("request-1", PromptMode::Normal, true));
    assert_eq!(
        owner
            .state
            .as_ref()
            .expect("state")
            .with(|store| store.queued_prompts())?
            .iter()
            .map(|prompt| prompt.id)
            .collect::<Vec<_>>(),
        [first, second, third],
        "admission leaves every row pending"
    );
    owner.apply_process_item(delivered("request-1", "first"));
    assert_eq!(
        owner
            .state
            .as_ref()
            .expect("state")
            .with(|store| store.queued_prompts())?
            .iter()
            .map(|prompt| prompt.id)
            .collect::<Vec<_>>(),
        [second, third]
    );
    owner.apply_response(crate::agents::SessionResponse::success(
        Some("request-2".into()),
        crate::agents::SessionResponsePayload::LoadState(Box::new(empty_session())),
    ));
    owner.apply_process_item(SessionEvent::Activity(json!({"type":"agent_start"}).into()));
    owner.apply_process_item(SessionEvent::Activity(
        json!({"type":"agent_settled"}).into(),
    ));
    assert_eq!(sent_messages(&sent), ["first", "second"]);
    assert_eq!(owner.queued_prompts.len(), 1);
    Ok(())
}

#[test]
fn native_history_acks_recovered_prompt_before_it_can_replay() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let store = StateStore::open_at(&database)?;
    let outbox_id = store.enqueue_prompt(
        "session:recovered",
        Backend::Codex,
        temp.path(),
        Some(&session),
        PromptMode::Normal,
        "delivered before restart",
        &[],
    )?;
    store.record_prompt_dispatch(outbox_id, "codex-cli-prior-request")?;
    let prompt = store.queued_prompts()?.remove(0);
    let (mut owner, sent) = ready_owner(temp.path(), &database)?;
    owner.state = Some(store.into());
    owner.startup_history_loaded = false;
    owner.deliver_queued(prompt);
    assert!(sent_messages(&sent).is_empty());

    owner.apply_response(crate::agents::SessionResponse::success(
        Some("history-request".into()),
        crate::agents::SessionResponsePayload::LoadHistory(
            crate::agents::SessionHistory::Replace {
                messages: Vec::new(),
                prompt_deliveries: Some(crate::sessions::PromptDeliveryReconciliation {
                    delivered: vec!["codex-cli-prior-request".into()],
                    pending: Vec::new(),
                    absence_is_not_delivered: false,
                }),
            },
        ),
    ));
    owner.maybe_send_deferred_prompt();
    assert!(sent_messages(&sent).is_empty());
    assert!(StateStore::open_at(&database)?.queued_prompts()?.is_empty());
    Ok(())
}

#[test]
fn operational_rejection_rolls_back_the_transcript_but_retains_pending_work() -> Result<(), String>
{
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, sent) = ready_owner(temp.path(), &database)?;
    owner.send_prompt(
        "draft:retry".into(),
        PromptMode::Normal,
        "retry this input".into(),
        Vec::new(),
        false,
    );
    owner.apply_response(prompt_response("request-1", PromptMode::Normal, false));

    assert_eq!(sent_messages(&sent), ["retry this input"]);
    assert!(
        owner
            .state
            .as_ref()
            .expect("state")
            .with(|store| store.queued_prompts())?
            .iter()
            .any(|prompt| prompt.message == "retry this input")
    );
    assert!(
        owner
            .snapshot
            .conversation
            .items
            .iter()
            .all(|item| { item.kind != crate::conversation::TranscriptKind::User })
    );
    Ok(())
}

#[test]
fn normal_replay_waits_for_compaction_retry_or_pending_input() -> Result<(), String> {
    for blocker in ["compacting", "retrying", "pending input"] {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let database = temp.path().join("state.sqlite3");
        let store = StateStore::open_at(&database)?;
        store.enqueue_prompt(
            "draft:replay",
            Backend::Pi,
            temp.path(),
            None,
            PromptMode::Normal,
            "wait until available",
            &[],
        )?;
        let prompt = store.queued_prompts()?.remove(0);
        let (mut owner, sent) = ready_owner(temp.path(), &database)?;
        owner.state = Some(store.into());
        match blocker {
            "compacting" => Arc::make_mut(&mut owner.snapshot.conversation).compacting = true,
            "retrying" => Arc::make_mut(&mut owner.snapshot.conversation).retrying = true,
            "pending input" => {
                owner.snapshot.pending_question = Some(ExtensionUiRequest::Input {
                    id: "question".into(),
                    title: "Continue?".into(),
                    placeholder: None,
                    timeout: None,
                })
            }
            _ => unreachable!(),
        }
        owner.deliver_queued(prompt);
        owner.maybe_send_deferred_prompt();
        assert!(sent_messages(&sent).is_empty(), "{blocker}");
        let conversation = Arc::make_mut(&mut owner.snapshot.conversation);
        conversation.compacting = false;
        conversation.retrying = false;
        owner.snapshot.pending_question = None;
        owner.maybe_send_deferred_prompt();
        assert_eq!(sent_messages(&sent), ["wait until available"]);
    }
    Ok(())
}

#[test]
fn only_native_delivery_acknowledges_the_matching_outbox_row() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, _) = ready_owner(temp.path(), &database)?;
    owner.send_prompt(
        "draft:ack".into(),
        PromptMode::Normal,
        "model receipt required".into(),
        Vec::new(),
        false,
    );
    let id = owner.pending_prompt_id.clone().expect("request id");
    owner.apply_response(prompt_response("unrelated", PromptMode::Normal, true));
    owner.apply_response(prompt_response(&id, PromptMode::Normal, true));
    assert_eq!(
        outbox_rows(&database)?,
        [("model receipt required".into(), "pending".into())]
    );

    owner.apply_process_item(delivered(&id, "model receipt required"));
    assert_eq!(
        outbox_rows(&database)?,
        [("model receipt required".into(), "acked".into())]
    );
    Ok(())
}

#[test]
fn delivery_receipt_acks_prompt_and_persists_its_presentation() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let (mut owner, _) = ready_owner(temp.path(), &database)?;
    owner.active_session = Some(session.clone());
    let image = crate::protocol::PromptImage::new("aGVsbG8=".into(), "image/png".into());
    owner.send_prompt_with_presentation(
        "draft:presentation".into(),
        PromptMode::Normal,
        "resolved prompt".into(),
        Some("$review".into()),
        Some("review".into()),
        vec![image],
        false,
    );
    let id = owner.pending_prompt_id.clone().expect("request id");
    owner.apply_response(prompt_response(&id, PromptMode::Normal, true));
    let store = owner.state.as_ref().expect("state");
    assert_eq!(store.with(|store| store.queued_prompts())?.len(), 1);
    assert!(
        store
            .with(|store| store.accepted_prompt_history(&session))?
            .is_empty()
    );
    assert!(
        store
            .with(|store| store.prompt_presentations(&session))?
            .is_empty()
    );

    owner.apply_process_item(delivered(&id, "resolved prompt"));
    let store = owner.state.as_ref().expect("state");
    assert!(store.with(|store| store.queued_prompts())?.is_empty());
    assert_eq!(
        outbox_rows(&database)?,
        [("resolved prompt".into(), "acked".into())]
    );
    assert!(
        store
            .with(|store| store.accepted_prompt_history(&session))?
            .is_empty()
    );
    assert_eq!(
        store.with(|store| store.prompt_presentations(&session))?,
        [crate::agents::PromptPresentation {
            resolved_message: "resolved prompt".into(),
            display_message: "$review".into(),
            invocation: "review".into(),
        }]
    );
    Ok(())
}

#[test]
fn process_failure_after_dispatch_keeps_the_outbox_pending_for_retry() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, _) = ready_owner(temp.path(), &database)?;
    owner.send_prompt(
        "draft:fatal".into(),
        PromptMode::Normal,
        "retain after crash".into(),
        Vec::new(),
        false,
    );
    owner.apply_process_item(SessionEvent::Failure(
        "transport disconnected after dispatch".into(),
    ));

    assert!(
        owner
            .snapshot
            .conversation
            .items
            .iter()
            .all(|item| { item.kind != crate::conversation::TranscriptKind::User })
    );
    drop(owner);
    assert_eq!(
        outbox_rows(&database)?,
        [("retain after crash".into(), "pending".into())]
    );
    Ok(())
}

#[test]
fn late_receipt_never_acknowledges_a_new_same_text_submission() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, _) = ready_owner(temp.path(), &database)?;
    owner.send_prompt(
        "draft:old".into(),
        PromptMode::Normal,
        "same text".into(),
        Vec::new(),
        false,
    );
    let old_id = owner.pending_prompt_id.clone().expect("old request");
    let old_outbox = owner.pending_outbox_id.expect("old outbox");
    owner.apply_response(crate::agents::SessionResponse::prompt_delivery_unknown(
        old_id.clone(),
        PromptMode::Normal,
        "transport stopped before the model receipt".into(),
    ));
    assert_eq!(owner.retired_prompts[&old_id].outbox_id, Some(old_outbox));
    owner.apply_process_item(SessionEvent::Activity(
        json!({"type":"agent_settled"}).into(),
    ));
    owner.send_prompt(
        "draft:new".into(),
        PromptMode::Normal,
        "same text".into(),
        Vec::new(),
        false,
    );
    let new_id = owner.pending_prompt_id.clone().expect("new request");
    let new_outbox = owner.pending_outbox_id.expect("new outbox");

    owner.apply_response(prompt_response(&old_id, PromptMode::Normal, true));
    assert_eq!(owner.pending_prompt_id.as_deref(), Some(new_id.as_str()));
    assert_eq!(owner.pending_outbox_id, Some(new_outbox));
    owner.apply_process_item(delivered(&old_id, "same text"));
    assert_eq!(owner.pending_prompt_id.as_deref(), Some(new_id.as_str()));
    assert_eq!(owner.pending_outbox_id, Some(new_outbox));
    assert_eq!(
        outbox_rows(&database)?,
        [
            ("same text".into(), "acked".into()),
            ("same text".into(), "pending".into()),
        ]
    );
    owner.apply_process_item(delivered(&old_id, "same text"));
    assert_eq!(
        outbox_rows(&database)?,
        [
            ("same text".into(), "acked".into()),
            ("same text".into(), "pending".into()),
        ]
    );
    owner.apply_process_item(delivered(&new_id, "same text"));
    assert_eq!(
        outbox_rows(&database)?,
        [
            ("same text".into(), "acked".into()),
            ("same text".into(), "acked".into()),
        ]
    );
    Ok(())
}

#[test]
fn pending_prompt_is_not_safe_to_delete() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let path = temp.path().join("session.jsonl");
    let id = store.enqueue_prompt(
        &format!("session:{}", path.display()),
        Backend::Pi,
        temp.path(),
        Some(&path),
        PromptMode::Normal,
        "do not delete unresolved work",
        &[],
    )?;
    store.begin_prompt(id)?;
    assert!(store.has_queued_prompts_for(&[path])?);
    Ok(())
}

#[test]
fn failed_delivery_record_keeps_the_original_pending_row_retryable() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, _) = ready_owner(temp.path(), &database)?;
    owner.send_prompt(
        "draft:atomic".into(),
        PromptMode::Normal,
        "retry after storage failure".into(),
        Vec::new(),
        false,
    );
    let id = owner.pending_prompt_id.clone().expect("request id");
    owner.apply_response(prompt_response(&id, PromptMode::Normal, true));
    let connection = rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "CREATE TRIGGER reject_delivery_receipt BEFORE INSERT ON session_events
         WHEN json_extract(NEW.body, '$.type')='prompt_delivery_receipt'
         BEGIN SELECT RAISE(ABORT, 'delivery storage fixture'); END;",
        )
        .map_err(|error| error.to_string())?;

    owner.apply_process_item(delivered(&id, "retry after storage failure"));
    drop(owner);
    assert_eq!(
        outbox_rows(&database)?,
        [("retry after storage failure".into(), "pending".into())]
    );
    assert_eq!(StateStore::open_at(&database)?.queued_prompts()?.len(), 1);
    Ok(())
}
