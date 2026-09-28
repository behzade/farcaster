use crate::agents::Backend;
use std::{cell::RefCell, rc::Rc};

use super::*;
use crate::runtime::tests::owner_without_process;

#[derive(Default)]
struct Recorder(Rc<RefCell<Vec<SessionCommand>>>, Option<String>);

impl SessionTransport for Recorder {
    fn send(&mut self, command: SessionCommand) -> Result<String, String> {
        if let Some(error) = &self.1 {
            return Err(error.clone());
        }
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
    owner.process = Some(Box::new(Recorder(sent.clone(), None)));
    owner.state = Some(SharedStateStore::open_at(database)?);
    owner.active_session = Some(root.join("session.jsonl"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    Ok((owner, sent))
}

#[test]
fn recovered_prompt_waits_for_an_explicit_choice() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let target = "draft:saved";
    let first = store.enqueue_prompt(
        target,
        Backend::Pi,
        temp.path(),
        None,
        PromptMode::Normal,
        "send later",
        &[],
    )?;
    let second = store.enqueue_prompt(
        target,
        Backend::Pi,
        temp.path(),
        None,
        PromptMode::Normal,
        "remove me",
        &[],
    )?;
    let prompts = store.queued_prompts()?;
    let (mut owner, sent) = ready_owner(temp.path(), &database)?;
    owner.state = Some(store.into());
    for prompt in prompts {
        owner.apply_command(RuntimeCommand::RecoverPending(prompt));
    }
    assert!(sent_messages(&sent).is_empty());
    assert_eq!(owner.saved_prompts.len(), 2);
    owner.apply_command(RuntimeCommand::RemoveSaved {
        target: target.into(),
        id: second,
    });
    assert_eq!(owner.saved_prompts.len(), 1);
    assert_eq!(outbox_rows(&database)?[1].1, "cancelled");
    conversation_mut(&mut owner.snapshot).running = true;
    owner.apply_command(RuntimeCommand::SendSaved {
        target: target.into(),
        id: first,
    });
    assert!(sent_messages(&sent).is_empty());
    assert_eq!(owner.saved_prompts.len(), 1);
    conversation_mut(&mut owner.snapshot).running = false;
    owner.apply_command(RuntimeCommand::SendSaved {
        target: target.into(),
        id: first,
    });
    assert_eq!(sent_messages(&sent), ["send later"]);
    assert!(owner.saved_prompts.is_empty());
    Ok(())
}

#[test]
fn failed_manual_retry_stays_saved() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        "draft:retry",
        Backend::Pi,
        temp.path(),
        None,
        PromptMode::Normal,
        "keep on failure",
        &[],
    )?;
    let prompt = store.queued_prompts()?.remove(0);
    let (mut owner, _) = owner_without_process(temp.path().to_path_buf());
    owner.state = Some(store.into());
    owner.apply_command(RuntimeCommand::RecoverPending(prompt));
    owner.apply_command(RuntimeCommand::SendSaved {
        target: "draft:retry".into(),
        id,
    });
    assert_eq!(owner.saved_prompts.len(), 1);
    assert_eq!(
        outbox_rows(&database)?,
        [("keep on failure".into(), "pending".into())]
    );
    Ok(())
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
    assert_eq!(owner.saved_prompts.len(), 1);
    assert_eq!(owner.saved_prompts[0].message, "retry this input");
    assert_eq!(owner.snapshot.status, "Done");
    assert!(!owner.snapshot.conversation.running);
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
fn process_failure_after_dispatch_keeps_the_outbox_visible_for_manual_retry() -> Result<(), String>
{
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
    assert_eq!(owner.saved_prompts.len(), 1);
    assert_eq!(owner.saved_prompts[0].message, "retain after crash");

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
    assert!(owner.saved_prompts.is_empty());
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

#[test]
fn secondary_terminal_failure_releases_its_submission_without_blocking_later_results()
-> Result<(), String> {
    use agents::{PromptOutcome, SessionResponse, SessionResponsePayload};
    for uncertain in [false, true] {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let database = temp.path().join("state.sqlite3");
        let (mut owner, sent) = ready_owner(temp.path(), &database)?;
        let (sender, events) = mpsc::channel();
        owner.event_tx.sender = sender;
        owner
            .snapshot
            .selected_session
            .clone_from(&owner.active_session);
        let image = PromptImage::new("aW1hZ2UtYnl0ZXM=".into(), "image/png".into());
        for (id, text) in [
            ("first", "first input"),
            ("second", "same text"),
            ("third", "same text"),
        ] {
            owner.send_prompt_for_submission(
                id.into(),
                "session:one".into(),
                PromptMode::Normal,
                text.into(),
                if id == "second" {
                    vec![image.clone()]
                } else {
                    vec![]
                },
                false,
            );
        }
        let second = "request-2";
        let third = "request-3";
        let second_outbox = owner.pending_queued_prompts[second].outbox_id;
        let third_outbox = owner.pending_queued_prompts[third].outbox_id;
        owner.apply_response(SessionResponse::success(
            Some(second.into()),
            SessionResponsePayload::Prompt(PromptMode::FollowUp),
        ));
        assert!(
            events
                .try_iter()
                .all(|event| !matches!(event, RuntimeEvent::PromptResult { .. }))
        );
        assert!(owner.pending_queued_prompts.contains_key(second));
        owner.apply_process_item(delivered(third, "same text"));
        owner.apply_process_item(delivered("request-1", "first input"));
        let failure = if uncertain {
            SessionResponse::prompt_delivery_unknown(
                second.into(),
                PromptMode::FollowUp,
                "transport lost".into(),
            )
        } else {
            SessionResponse::failure(
                Some(second.into()),
                SessionOperation::Prompt(PromptMode::FollowUp),
                "transport unavailable".into(),
            )
        };
        owner.apply_response(failure);
        let published = events.try_iter().collect::<Vec<_>>();
        let mut results = published
            .iter()
            .filter_map(|event| match event {
                RuntimeEvent::PromptResult {
                    submission_id: Some(id),
                    outcome,
                    ..
                } => Some((id.clone(), *outcome)),
                _ => None,
            })
            .collect::<Vec<_>>();
        results.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            results,
            [
                ("first".into(), PromptOutcome::Accepted),
                ("second".into(), PromptOutcome::DeliveryUnknown),
                ("third".into(), PromptOutcome::Accepted),
            ]
        );
        assert!(!owner.pending_queued_prompts.contains_key(second));
        assert_eq!(owner.saved_prompts.len(), 1);
        assert_eq!(owner.saved_prompts[0].id, second_outbox);
        assert_eq!(owner.saved_prompts[0].images.len(), 1);
        assert_eq!(owner.saved_prompts[0].images[0].bytes()?, image.bytes()?);
        assert_eq!(owner.saved_prompts[0].images[0].mime_type, image.mime_type);
        assert_eq!(
            owner.saved_prompts[0].submission_id.as_deref(),
            Some("second")
        );
        assert_eq!(published_saved_ids(&published), [second_outbox]);
        assert_ne!(second_outbox, third_outbox);
        assert_eq!(owner.retired_prompts.contains_key(second), uncertain);
        assert_eq!(sent_messages(&sent).len(), 3, "no automatic retry");
        if uncertain {
            owner.apply_process_item(delivered(second, "same text"));
            assert!(owner.saved_prompts.is_empty());
            assert!(published_saved_ids(&events.try_iter().collect::<Vec<_>>()).is_empty());
        }
    }
    Ok(())
}

#[test]
fn current_and_secondary_recovery_uses_rejection_reason_not_wording() -> Result<(), String> {
    use agents::{PromptOutcome, RejectionReason, SessionResponse};
    for secondary in [false, true] {
        for (reason, message) in [
            (None, "cancelled before delivery"),
            (Some(RejectionReason::Authentication), "sign in again"),
            (Some(RejectionReason::Configuration), "choose another model"),
            (Some(RejectionReason::Permission), "operation denied"),
            (
                Some(RejectionReason::Other),
                "auth service transport closed; config unavailable",
            ),
        ] {
            let recover = reason == Some(RejectionReason::Other);
            let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
            let database = temp.path().join("state.sqlite3");
            let (mut owner, _) = ready_owner(temp.path(), &database)?;
            let (sender, events) = mpsc::channel();
            owner.event_tx.sender = sender;
            owner
                .snapshot
                .selected_session
                .clone_from(&owner.active_session);
            if secondary {
                owner.send_prompt_for_submission(
                    "first".into(),
                    "session:one".into(),
                    PromptMode::Normal,
                    "first input".into(),
                    vec![],
                    false,
                );
            }
            owner.send_prompt_for_submission(
                "rejected".into(),
                "session:one".into(),
                PromptMode::Normal,
                "restore me".into(),
                vec![],
                false,
            );
            let request = if secondary { "request-2" } else { "request-1" };
            let mode = if secondary {
                PromptMode::FollowUp
            } else {
                PromptMode::Normal
            };
            owner.apply_response(if reason.is_none() {
                SessionResponse::cancelled(
                    request.into(),
                    SessionOperation::Prompt(mode),
                    "cancelled before delivery".into(),
                )
            } else {
                SessionResponse::prompt_rejected(
                    request.into(),
                    mode,
                    agents::PromptRejection::new(reason.unwrap(), message),
                )
            });
            let results = events
                .try_iter()
                .filter_map(|event| match event {
                    RuntimeEvent::PromptResult {
                        submission_id: Some(id),
                        outcome,
                        ..
                    } if id == "rejected" => Some(outcome),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                results,
                [if recover {
                    PromptOutcome::DeliveryUnknown
                } else {
                    PromptOutcome::RejectedBeforeAcceptance
                }]
            );
            assert_eq!(owner.saved_prompts.is_empty(), !recover);
            assert_eq!(
                StateStore::open_at(&database)?
                    .queued_prompts()?
                    .iter()
                    .any(|prompt| prompt.message == "restore me"),
                recover
            );
            assert!(!owner.pending_queued_prompts.contains_key(request));
        }
    }
    Ok(())
}

#[test]
fn exact_history_delivery_removes_only_the_matching_saved_card() -> Result<(), String> {
    for cold_history in [false, true] {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let database = temp.path().join("state.sqlite3");
        let (mut owner, _) = ready_owner(temp.path(), &database)?;
        let (sender, events) = mpsc::channel();
        owner.event_tx.sender = sender;
        owner
            .snapshot
            .selected_session
            .clone_from(&owner.active_session);
        for id in ["first", "second"] {
            owner.send_prompt_for_submission(
                id.into(),
                "session:one".into(),
                PromptMode::Normal,
                "same text".into(),
                vec![],
                false,
            );
        }
        let second_outbox = owner.pending_queued_prompts["request-2"].outbox_id;
        for (id, mode) in [
            ("request-1", PromptMode::Normal),
            ("request-2", PromptMode::FollowUp),
        ] {
            owner.apply_response(agents::SessionResponse::prompt_delivery_unknown(
                id.into(),
                mode,
                "transport lost".into(),
            ));
        }
        assert_eq!(owner.saved_prompts.len(), 2);
        let evidence = sessions::PromptDeliveryReconciliation {
            delivered: vec!["request-1".into()],
            pending: vec![],
            absence_is_not_delivered: false,
        };
        if cold_history {
            let path = owner.active_session.take().expect("active session");
            owner.apply_history(HistoryResult {
                generation: owner.history_generation,
                path,
                project: temp.path().into(),
                kind: HistoryLoadKind::Selection,
                result: Ok(LoadedHistory {
                    messages: vec![],
                    model: None,
                    thinking_level: None,
                    pending_question: None,
                    prompt_deliveries: Some(evidence),
                }),
            });
        } else {
            owner.apply_response(agents::SessionResponse::success(
                Some("history".into()),
                agents::SessionResponsePayload::LoadHistory(agents::SessionHistory::Replace {
                    messages: vec![],
                    prompt_deliveries: Some(evidence),
                }),
            ));
        }
        assert_eq!(owner.saved_prompts.len(), 1);
        assert_eq!(owner.saved_prompts[0].id, second_outbox);
        assert_eq!(
            published_saved_ids(&events.try_iter().collect::<Vec<_>>()),
            [second_outbox]
        );
    }
    Ok(())
}

#[test]
fn exact_cancellation_removes_saved_recovery_and_preserves_other_input() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, _) = ready_owner(temp.path(), &database)?;
    let (sender, events) = mpsc::channel();
    owner.event_tx.sender = sender;
    owner
        .snapshot
        .selected_session
        .clone_from(&owner.active_session);
    owner.send_prompt_for_submission(
        "first".into(),
        "session:one".into(),
        PromptMode::Normal,
        "same text".into(),
        vec![],
        false,
    );
    owner.send_prompt_for_submission(
        "second".into(),
        "session:one".into(),
        PromptMode::Normal,
        "same text".into(),
        vec![],
        false,
    );
    owner.apply_response(agents::SessionResponse::prompt_delivery_unknown(
        "request-2".into(),
        PromptMode::FollowUp,
        "transport lost".into(),
    ));
    assert_eq!(owner.saved_prompts.len(), 1);
    assert_eq!(
        owner.apply_process_item(SessionEvent::Activity(
            json!({
                "type":"prompt_delivery", "submissionId":"request-2", "status":"cancelled"
            })
            .into()
        )),
        SnapshotChange::Immediate
    );
    owner.publish();
    assert!(owner.saved_prompts.is_empty());
    assert!(published_saved_ids(&events.try_iter().collect::<Vec<_>>()).is_empty());
    assert_eq!(owner.pending_prompt_id.as_deref(), Some("request-1"));
    assert_eq!(StateStore::open_at(&database)?.queued_prompts()?.len(), 1);
    Ok(())
}

fn published_saved_ids(events: &[RuntimeEvent]) -> Vec<i64> {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            RuntimeEvent::Snapshot { snapshot, .. } => Some(
                snapshot
                    .conversation
                    .queue
                    .saved
                    .iter()
                    .map(|prompt| prompt.id)
                    .collect(),
            ),
            _ => None,
        })
        .expect("published snapshot")
}

#[test]
fn secondary_recovery_keeps_payload_when_outbox_reads_fail() -> Result<(), String> {
    for transport_failure in [false, true] {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let database = temp.path().join("state.sqlite3");
        let (mut owner, sent) = ready_owner(temp.path(), &database)?;
        let (sender, events) = mpsc::channel();
        owner.event_tx.sender = sender;
        owner
            .snapshot
            .selected_session
            .clone_from(&owner.active_session);
        owner.send_prompt_for_submission(
            "first".into(),
            "session:one".into(),
            PromptMode::Normal,
            "first input".into(),
            vec![],
            false,
        );
        let image = PromptImage::new("aW1hZ2UtYnl0ZXM=".into(), "image/png".into());
        owner.send_prompt_with_presentation_for_submission(
            "second".into(),
            "session:one".into(),
            PromptMode::Normal,
            "expanded input".into(),
            Some("display input".into()),
            Some("invocation".into()),
            vec![image.clone()],
            false,
        );
        let outbox_id = owner.pending_queued_prompts["request-2"].outbox_id;
        let connection =
            rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
        connection
            .execute_batch("ALTER TABLE outbox RENAME TO unavailable_outbox")
            .map_err(|error| error.to_string())?;
        assert!(
            owner
                .state
                .as_ref()
                .unwrap()
                .with(|store| store.queued_prompts())
                .is_err()
        );
        if transport_failure {
            owner.fail_pending_queued_prompts("transport lost");
            owner.publish();
        } else {
            owner.apply_response(agents::SessionResponse::prompt_delivery_unknown(
                "request-2".into(),
                PromptMode::FollowUp,
                "transport lost".into(),
            ));
        }
        assert!(!owner.pending_queued_prompts.contains_key("request-2"));
        assert_eq!(
            owner.saved_prompts.len(),
            1,
            "saved queue must own recovery before composer release"
        );
        let prompt = &owner.saved_prompts[0];
        assert_eq!(prompt.id, outbox_id);
        assert_eq!(prompt.submission_id.as_deref(), Some("second"));
        assert_eq!(prompt.message, "expanded input");
        assert_eq!(prompt.display_message.as_deref(), Some("display input"));
        assert_eq!(prompt.invocation.as_deref(), Some("invocation"));
        assert_eq!(prompt.images[0].bytes()?, image.bytes()?);
        let published = events.try_iter().collect::<Vec<_>>();
        assert_eq!(published_saved_ids(&published), [outbox_id]);
        assert_eq!(published.iter().filter(|event| matches!(event,
            RuntimeEvent::PromptResult { submission_id: Some(id), outcome: agents::PromptOutcome::DeliveryUnknown, .. }
            if id == "second"
        )).count(), 1);

        connection
            .execute_batch("ALTER TABLE unavailable_outbox RENAME TO outbox")
            .map_err(|error| error.to_string())?;
        owner.apply_response(agents::SessionResponse::success(
            Some("history".into()),
            agents::SessionResponsePayload::LoadHistory(agents::SessionHistory::Replace {
                messages: vec![],
                prompt_deliveries: None,
            }),
        ));
        assert_eq!(
            published_saved_ids(&events.try_iter().collect::<Vec<_>>()),
            [outbox_id]
        );
        assert_eq!(
            sent_messages(&sent),
            ["first input", "expanded input"],
            "recovery never sends automatically"
        );
        assert!(owner.process.is_some(), "no runtime restart needed");
        assert!(
            StateStore::open_at(&database)?
                .queued_prompts()?
                .iter()
                .any(|prompt| prompt.id == outbox_id)
        );
        owner.apply_process_item(delivered("request-2", "expanded input"));
        assert!(owner.saved_prompts.is_empty());
        assert!(published_saved_ids(&events.try_iter().collect::<Vec<_>>()).is_empty());
    }
    Ok(())
}

#[test]
fn deferred_selection_rejection_cancels_outbox_and_settles_once() -> Result<(), String> {
    for operation in [
        SessionOperation::SelectModel,
        SessionOperation::SelectReasoning,
        SessionOperation::SelectServiceTier,
    ] {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let database = temp.path().join("state.sqlite3");
        let (mut owner, sent) = ready_owner(temp.path(), &database)?;
        let (sender, events) = mpsc::channel();
        owner.event_tx.sender = sender;
        let model: Model = serde_json::from_value(
            json!({"id":"selected", "name":"Selected", "provider":"openai"}),
        )
        .unwrap();
        let sent_selection = |owner: &mut RuntimeOwner, id: &str| match operation {
            SessionOperation::SelectModel => owner
                .pending_session_controls
                .model_sent(id.into(), (model.provider.clone(), model.id.clone())),
            SessionOperation::SelectReasoning => owner
                .pending_session_controls
                .thinking_sent(id.into(), Some("high".into())),
            SessionOperation::SelectServiceTier => owner
                .pending_session_controls
                .tier_sent(id.into(), "fast".into()),
            _ => unreachable!(),
        };
        sent_selection(&mut owner, "selection");
        owner.send_prompt_for_submission(
            "deferred".into(),
            "draft:deferred".into(),
            PromptMode::Normal,
            "unsent".into(),
            vec![],
            false,
        );
        assert!(owner.deferred_prompt.is_some());
        assert!(sent_messages(&sent).is_empty());
        for _ in 0..2 {
            owner.apply_response(agents::SessionResponse::failure(
                Some("selection".into()),
                operation,
                "selection unavailable".into(),
            ));
        }
        let outcomes = events
            .try_iter()
            .filter_map(|event| match event {
                RuntimeEvent::PromptResult {
                    submission_id: Some(id),
                    outcome,
                    ..
                } if id == "deferred" => Some(outcome),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(outcomes, [agents::PromptOutcome::RejectedBeforeAcceptance]);
        assert_eq!(
            outbox_rows(&database)?,
            [("unsent".into(), "cancelled".into())]
        );
        assert!(owner.saved_prompts.is_empty());
        assert!(owner.deferred_prompt.is_none());
        assert!(owner.pending_prompt_id.is_none());
        assert!(owner.pending_prompt_target.is_none());
        assert!(owner.pending_submission_id.is_none());
        assert!(owner.pending_outbox_id.is_none());
        assert!(!owner.snapshot.conversation.running);

        sent_selection(&mut owner, "replacement");
        let payload = match operation {
            SessionOperation::SelectModel => {
                agents::SessionResponsePayload::SelectModel(model.clone())
            }
            SessionOperation::SelectReasoning => agents::SessionResponsePayload::SelectReasoning,
            SessionOperation::SelectServiceTier => {
                agents::SessionResponsePayload::SelectServiceTier
            }
            _ => unreachable!(),
        };
        owner.apply_response(agents::SessionResponse::success(
            Some("replacement".into()),
            payload,
        ));
        owner.send_prompt_for_submission(
            "next".into(),
            "draft:deferred".into(),
            PromptMode::Normal,
            "next input".into(),
            vec![],
            false,
        );
        assert_eq!(sent_messages(&sent), ["next input"]);
    }
    Ok(())
}

#[test]
fn startup_failure_settles_deferred_outbox_with_or_without_controls() -> Result<(), String> {
    for controls in [false, true] {
        for operation in [SessionOperation::LoadState, SessionOperation::LoadHistory] {
            let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
            let database = temp.path().join("state.sqlite3");
            let (mut owner, sent) = ready_owner(temp.path(), &database)?;
            let (sender, events) = mpsc::channel();
            owner.event_tx.sender = sender;
            if controls {
                let process = owner.process.take();
                owner.set_thinking("high".into());
                owner.process = process;
                assert!(!owner.pending_session_controls.is_empty());
            }
            owner.startup_state_loaded = false;
            owner.startup_history_loaded = false;
            owner.send_prompt_for_submission(
                "deferred".into(),
                "draft:startup".into(),
                PromptMode::Normal,
                "never dispatched".into(),
                vec![],
                false,
            );
            assert!(owner.deferred_prompt.is_some());
            owner.apply_response(agents::SessionResponse::cancelled(
                "startup".into(),
                operation,
                "startup interrupted".into(),
            ));
            let outcomes = events
                .try_iter()
                .filter_map(|event| match event {
                    RuntimeEvent::PromptResult {
                        submission_id: Some(id),
                        outcome,
                        ..
                    } if id == "deferred" => Some(outcome),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(outcomes, [agents::PromptOutcome::RejectedBeforeAcceptance]);
            assert_eq!(
                outbox_rows(&database)?,
                [("never dispatched".into(), "cancelled".into())]
            );
            assert!(owner.saved_prompts.is_empty());
            assert!(owner.deferred_prompt.is_none());
            assert!(owner.pending_prompt_target.is_none());
            assert!(owner.pending_submission_id.is_none());
            assert!(owner.pending_outbox_id.is_none());
            assert!(owner.can_deliver_queued(PromptMode::Normal));
            owner.process = Some(Box::new(Recorder(sent.clone(), None)));
            owner.active_session = Some(temp.path().join("session.jsonl"));
            owner.startup_state_loaded = true;
            owner.startup_history_loaded = true;
            owner.send_prompt_for_submission(
                "next".into(),
                "draft:startup".into(),
                PromptMode::Normal,
                "next input".into(),
                vec![],
                false,
            );
            assert_eq!(sent_messages(&sent), ["next input"]);
        }
    }
    Ok(())
}

#[test]
fn deferred_cancellation_failure_keeps_one_recovery_payload() -> Result<(), String> {
    for startup_failure in [false, true] {
        for read_failure in [false, true] {
            let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
            let database = temp.path().join("state.sqlite3");
            let (mut owner, sent) = ready_owner(temp.path(), &database)?;
            let (sender, events) = mpsc::channel();
            owner.event_tx.sender = sender;
            if startup_failure {
                let process = owner.process.take();
                owner.set_thinking("high".into());
                owner.process = process;
                owner.startup_state_loaded = false;
                owner.startup_history_loaded = false;
            } else {
                owner
                    .pending_session_controls
                    .thinking_sent("effort".into(), Some("high".into()));
            }
            let image = PromptImage::new("aW1hZ2UtYnl0ZXM=".into(), "image/png".into());
            owner.send_prompt_with_presentation_for_submission(
                "deferred".into(),
                "draft:fault".into(),
                PromptMode::Normal,
                "expanded input".into(),
                Some("display input".into()),
                Some("invocation".into()),
                vec![image.clone()],
                false,
            );
            let outbox_id = owner.pending_outbox_id.unwrap();
            let connection =
                rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
            let fault = if read_failure {
                "ALTER TABLE outbox RENAME TO unavailable_outbox"
            } else {
                "CREATE TRIGGER fail_cancel BEFORE UPDATE OF state ON outbox BEGIN SELECT RAISE(ABORT, 'cancel unavailable'); END;"
            };
            connection
                .execute_batch(fault)
                .map_err(|error| error.to_string())?;
            if startup_failure {
                owner.apply_response(agents::SessionResponse::failure(
                    Some("startup".into()),
                    SessionOperation::LoadHistory,
                    "history unavailable".into(),
                ));
            } else {
                owner.apply_response(agents::SessionResponse::failure(
                    Some("effort".into()),
                    SessionOperation::SelectReasoning,
                    "effort unavailable".into(),
                ));
            }
            let outcomes = events
                .try_iter()
                .filter_map(|event| match event {
                    RuntimeEvent::PromptResult {
                        submission_id: Some(id),
                        outcome,
                        ..
                    } if id == "deferred" => Some(outcome),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(outcomes, [agents::PromptOutcome::DeliveryUnknown]);
            assert_eq!(owner.saved_prompts.len(), 1);
            let saved = &owner.saved_prompts[0];
            assert_eq!(saved.id, outbox_id);
            assert_eq!(saved.submission_id.as_deref(), Some("deferred"));
            assert_eq!(saved.message, "expanded input");
            assert_eq!(saved.display_message.as_deref(), Some("display input"));
            assert_eq!(saved.invocation.as_deref(), Some("invocation"));
            assert_eq!(saved.images[0].bytes()?, image.bytes()?);
            assert!(owner.deferred_prompt.is_none());
            assert!(owner.pending_prompt_target.is_none());
            assert!(owner.pending_submission_id.is_none());
            assert!(owner.pending_outbox_id.is_none());
            assert!(sent_messages(&sent).is_empty());
            connection
                .execute_batch(if read_failure {
                    "ALTER TABLE unavailable_outbox RENAME TO outbox"
                } else {
                    "DROP TRIGGER fail_cancel"
                })
                .map_err(|error| error.to_string())?;
            assert_eq!(
                outbox_rows(&database)?,
                [("expanded input".into(), "pending".into())]
            );
        }
    }
    Ok(())
}

#[test]
fn unclassified_secondary_send_error_keeps_the_prompt_recoverable() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, sent) = ready_owner(temp.path(), &database)?;
    let (sender, events) = mpsc::channel();
    owner.event_tx.sender = sender;
    owner.send_prompt_for_submission(
        "first".into(),
        "session:one".into(),
        PromptMode::Normal,
        "working".into(),
        vec![],
        false,
    );
    owner.process = Some(Box::new(Recorder(
        sent.clone(),
        Some("auth service config connection lost".into()),
    )));
    owner.send_prompt_for_submission(
        "second".into(),
        "session:one".into(),
        PromptMode::Normal,
        "keep me".into(),
        vec![],
        false,
    );
    assert!(events.try_iter().any(|event| matches!(event,
        RuntimeEvent::PromptResult { submission_id: Some(id), outcome: agents::PromptOutcome::DeliveryUnknown, .. } if id == "second"
    )));
    assert_eq!(owner.saved_prompts.len(), 1);
    assert!(
        StateStore::open_at(&database)?
            .queued_prompts()?
            .iter()
            .any(|prompt| prompt.message == "keep me")
    );
    assert_eq!(sent_messages(&sent), ["working"]);
    Ok(())
}
