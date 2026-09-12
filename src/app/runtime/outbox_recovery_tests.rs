use std::{cell::RefCell, rc::Rc};

use super::*;
use crate::app::runtime::tests::owner_without_process;

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

fn empty_session_value() -> Value {
    json!({
        "sessionId": "replay-session",
        "isStreaming": false,
        "isCompacting": false,
        "autoCompactionEnabled": true,
        "messageCount": 0,
        "pendingMessageCount": 0
    })
}

fn empty_session() -> SessionState {
    serde_json::from_value(empty_session_value()).expect("session fixture")
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

fn sent_messages(sent: &Rc<RefCell<Vec<SessionCommand>>>) -> Vec<String> {
    sent.borrow()
        .iter()
        .filter_map(|command| match command {
            SessionCommand::Prompt { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn abort_cancels_all_recovered_prompts_without_replaying_them() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    for message in ["active task", "next task", "last task"] {
        store.enqueue_prompt(
            "draft:abort-replay",
            "pi",
            temp.path(),
            None,
            PromptMode::Normal,
            message,
            &[],
        )?;
    }
    let recovered = store.queued_prompts()?;
    let unrelated_id = store.enqueue_prompt(
        "draft:unrelated",
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "unrelated task",
        &[],
    )?;
    let (mut owner, _) = owner_without_process(temp.path().to_path_buf());
    let sent = Rc::new(RefCell::new(Vec::new()));
    owner.process = Some(Box::new(Recorder(sent.clone())));
    owner.state = Some(store);
    owner.active_session = Some(temp.path().join("session.jsonl"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    for prompt in recovered {
        owner.deliver_queued(prompt);
    }
    owner.apply_response(prompt_response("request-1", PromptMode::Normal, true));
    assert_eq!(sent_messages(&sent), ["active task"]);

    owner.apply_command(RuntimeCommand::Abort);
    owner.apply_process_item(SessionEvent::Activity(
        json!({"type":"agent_settled"}).into(),
    ));
    owner.maybe_send_deferred_prompt();
    assert_eq!(
        sent_messages(&sent),
        ["active task"],
        "Abort must not dispatch the next recovered task"
    );
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, SessionCommand::Abort))
    );
    assert!(owner.queued_prompts.is_empty());
    drop(owner);
    let reopened = StateStore::open_at(&database)?;
    assert_eq!(
        reopened
            .queued_prompts()?
            .iter()
            .map(|prompt| prompt.id)
            .collect::<Vec<_>>(),
        [unrelated_id],
        "cancelled work must not return on restart"
    );
    let connection = rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
    let rows = connection
        .prepare("SELECT message, state FROM outbox ORDER BY id")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    assert_eq!(
        rows,
        [
            ("next task".into(), "failed".into()),
            ("last task".into(), "failed".into()),
            ("unrelated task".into(), "queued".into())
        ],
        "keep cancelled payloads, but remove them from automatic delivery"
    );
    Ok(())
}

#[test]
fn abort_reports_failed_durable_cancellation_and_still_stops_this_run() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    store.enqueue_prompt(
        "draft:failed-cancel",
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "must not start this run",
        &[],
    )?;
    let connection = rusqlite::Connection::open(&database).map_err(|e| e.to_string())?;
    connection
        .execute_batch(
            "CREATE TRIGGER reject_cancellation BEFORE UPDATE OF state ON outbox
         WHEN NEW.state='failed' BEGIN SELECT RAISE(FAIL, 'storage failure fixture'); END;",
        )
        .map_err(|e| e.to_string())?;
    let (mut owner, _) = owner_without_process(temp.path().into());
    let sent = Rc::new(RefCell::new(Vec::new()));
    owner.process = Some(Box::new(Recorder(sent.clone())));
    owner.queued_prompts.extend(store.queued_prompts()?);
    owner.state = Some(store);
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.apply_command(RuntimeCommand::Abort);
    owner.maybe_send_deferred_prompt();
    assert!(sent_messages(&sent).is_empty());
    assert!(owner.queued_prompts.is_empty());
    assert!(
        owner
            .active_snapshot()
            .conversation
            .items
            .iter()
            .any(|item| {
                item.text.contains("They may return after restart")
                    && item.text.contains("storage failure fixture")
            }),
        "a persistence failure must show the restart risk"
    );
    drop(owner);
    let reopened = StateStore::open_at(&database)?;
    assert_eq!(
        reopened.queued_prompts()?.len(),
        1,
        "failed storage cannot be claimed as durable cancellation"
    );
    Ok(())
}

#[test]
fn abort_cancels_a_prompt_deferred_for_startup() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let (mut owner, events) = owner_without_process(temp.path().to_path_buf());
    let sent = Rc::new(RefCell::new(Vec::new()));
    owner.process = Some(Box::new(Recorder(sent.clone())));
    owner.state = Some(store);

    owner.send_prompt(
        "draft:startup".into(),
        PromptMode::Normal,
        "cancel before ready".into(),
        Vec::new(),
        false,
    );
    assert!(owner.deferred_prompt.is_some());
    assert!(owner.pending_prompt_item.is_some());
    assert!(owner.active_snapshot().conversation.running);

    owner.apply_command(RuntimeCommand::Abort);

    assert!(owner.deferred_prompt.is_none());
    assert!(owner.pending_prompt_item.is_none());
    assert!(owner.pending_prompt_target.is_none());
    assert!(!owner.active_snapshot().conversation.running);
    assert!(
        owner
            .state
            .as_ref()
            .expect("state")
            .queued_prompts()?
            .is_empty()
    );
    let connection = rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
    let outbox_state = connection
        .query_row("SELECT state FROM outbox", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(outbox_state, "failed");
    assert!(matches!(sent.borrow().as_slice(), [SessionCommand::Abort]));
    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            target,
            outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
            ..
        } if target == "draft:startup"
    )));

    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.maybe_send_deferred_prompt();
    assert!(matches!(sent.borrow().as_slice(), [SessionCommand::Abort]));
    Ok(())
}

#[test]
fn startup_replays_same_target_prompts_in_order_after_each_acknowledgement() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let target = "draft:replay";
    let first = store.enqueue_prompt(
        target,
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "first survives restart",
        &[],
    )?;
    let second = store.enqueue_prompt(
        target,
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "second survives restart",
        &[],
    )?;
    let third = store.enqueue_prompt(
        target,
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "third survives restart",
        &[],
    )?;
    let prompts = store.queued_prompts()?;
    assert_eq!(
        prompts.iter().map(|prompt| prompt.id).collect::<Vec<_>>(),
        [first, second, third]
    );

    let (mut owner, _events) = owner_without_process(temp.path().to_path_buf());
    let sent = Rc::new(RefCell::new(Vec::new()));
    owner.process = Some(Box::new(Recorder(sent.clone())));
    owner.state = Some(store);
    for prompt in prompts {
        owner.deliver_queued(prompt);
    }

    assert_eq!(
        owner
            .state
            .as_ref()
            .expect("state")
            .queued_prompts()?
            .iter()
            .map(|prompt| prompt.id)
            .collect::<Vec<_>>(),
        [first, second, third],
        "shutdown before startup must leave every prompt durable and queued"
    );

    owner.active_session = Some(temp.path().join("replay-session"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.maybe_send_deferred_prompt();

    assert_eq!(
        sent_messages(&sent),
        ["first survives restart"],
        "the first normal prompt starts after startup, while the next remains durable"
    );
    assert_eq!(
        owner
            .state
            .as_ref()
            .expect("state")
            .queued_prompts()?
            .iter()
            .map(|prompt| prompt.id)
            .collect::<Vec<_>>(),
        [second, third]
    );

    owner.apply_response(prompt_response("request-1", PromptMode::Normal, true));
    owner.maybe_send_deferred_prompt();

    assert_eq!(
        sent_messages(&sent),
        ["first survives restart"],
        "an RPC acknowledgement does not start a second normal prompt before the turn settles"
    );

    owner.apply_response(crate::agents::SessionResponse::success(
        Some("request-2".into()),
        crate::agents::SessionResponsePayload::LoadState(Box::new(empty_session())),
    ));
    assert!(
        owner.snapshot.conversation.running,
        "an idle state response must not hide a normal prompt awaiting its terminal event"
    );
    owner.maybe_send_deferred_prompt();
    assert_eq!(
        sent_messages(&sent),
        ["first survives restart"],
        "a stale idle state response cannot start a second normal prompt before the turn starts"
    );

    owner.apply_process_item(SessionEvent::Activity(
        json!({"type": "agent_start"}).into(),
    ));
    owner.maybe_send_deferred_prompt();
    assert_eq!(
        sent_messages(&sent),
        ["first survives restart"],
        "a late start event still keeps the next normal prompt queued"
    );

    owner.apply_process_item(SessionEvent::Activity(
        json!({"type": "agent_settled"}).into(),
    ));
    owner.maybe_send_deferred_prompt();
    assert_eq!(
        sent_messages(&sent),
        ["first survives restart", "second survives restart"],
        "odd state-poll pumps cannot rotate the next two queued normal prompts"
    );
    assert_eq!(
        owner
            .queued_prompts
            .iter()
            .map(|prompt| prompt.message.as_str())
            .collect::<Vec<_>>(),
        ["third survives restart"]
    );
    Ok(())
}

#[test]
fn rejected_replay_prompt_does_not_starve_later_prompts() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let target = "draft:replay";
    store.enqueue_prompt(
        target,
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "first is rejected",
        &[],
    )?;
    store.enqueue_prompt(
        target,
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "second is still delivered",
        &[],
    )?;
    let prompts = store.queued_prompts()?;
    let (mut owner, _events) = owner_without_process(temp.path().to_path_buf());
    let sent = Rc::new(RefCell::new(Vec::new()));
    owner.process = Some(Box::new(Recorder(sent.clone())));
    owner.state = Some(store);
    owner.active_session = Some(temp.path().join("replay-session"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    for prompt in prompts {
        owner.deliver_queued(prompt);
    }
    owner.maybe_send_deferred_prompt();
    assert_eq!(sent_messages(&sent), ["first is rejected"]);

    owner.apply_response(prompt_response("request-1", PromptMode::Normal, false));

    assert_eq!(
        sent_messages(&sent),
        ["first is rejected", "second is still delivered"],
        "a rejected row becomes failed without resending it or blocking the next row"
    );
    assert!(
        owner
            .state
            .as_ref()
            .expect("state")
            .queued_prompts()?
            .is_empty(),
        "the second row started and the rejected row did not return to queued"
    );
    Ok(())
}

#[test]
fn normal_replay_waits_for_compaction_retry_or_pending_input() -> Result<(), String> {
    for blocker in ["compacting", "retrying", "pending input"] {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
        store.enqueue_prompt(
            "draft:replay",
            "pi",
            temp.path(),
            None,
            PromptMode::Normal,
            "wait until the session is available",
            &[],
        )?;
        let prompt = store
            .queued_prompts()?
            .into_iter()
            .next()
            .expect("queued prompt");
        let (mut owner, _events) = owner_without_process(temp.path().to_path_buf());
        let sent = Rc::new(RefCell::new(Vec::new()));
        owner.process = Some(Box::new(Recorder(sent.clone())));
        owner.state = Some(store);
        owner.active_session = Some(temp.path().join("replay-session"));
        owner.snapshot.session = Some(empty_session());
        owner.startup_state_loaded = true;
        owner.startup_history_loaded = true;
        match blocker {
            "compacting" => Arc::make_mut(&mut owner.snapshot.conversation).compacting = true,
            "retrying" => Arc::make_mut(&mut owner.snapshot.conversation).retrying = true,
            "pending input" => {
                owner.snapshot.pending_question = Some(ExtensionUiRequest::Input {
                    id: "question".into(),
                    title: "Continue?".into(),
                    placeholder: None,
                    timeout: None,
                });
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
        assert_eq!(
            sent_messages(&sent),
            ["wait until the session is available"]
        );
    }
    Ok(())
}

#[test]
fn unrelated_acknowledgement_does_not_complete_the_outbox_row() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, _events) = owner_without_process(temp.path().to_path_buf());
    owner.state = Some(StateStore::open_at(&database)?);
    owner.process = Some(Box::new(Recorder::default()));
    owner.active_session = Some(temp.path().join("replay-session"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;

    owner.send_prompt(
        "draft:ack".into(),
        PromptMode::Normal,
        "keep this row until the backend accepts it".into(),
        Vec::new(),
        false,
    );
    let request_id = owner.pending_prompt_id.clone().expect("prompt request");
    owner.apply_process_item(SessionEvent::Response(prompt_response(
        "unrelated-request",
        PromptMode::Normal,
        true,
    )));

    let connection = rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
    let rows = connection
        .query_row("SELECT COUNT(*) FROM outbox", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(rows, 1, "an unrelated response must not delete a prompt");
    owner.apply_process_item(SessionEvent::Response(prompt_response(
        &request_id,
        PromptMode::Normal,
        true,
    )));
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM outbox", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        rows, 0,
        "the matching native acknowledgement completes the prompt"
    );
    Ok(())
}

#[test]
fn delivery_unknown_releases_the_request_and_late_acceptance_preserves_its_payload()
-> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, events) = owner_without_process(temp.path().to_path_buf());
    owner.state = Some(StateStore::open_at(&database)?);
    owner.process = Some(Box::new(Recorder::default()));
    owner.active_session = Some(temp.path().join("session.jsonl"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    let image = crate::protocol::PromptImage::new("aGVsbG8=".into(), "image/png".into());

    owner.send_prompt_with_presentation(
        "draft:unknown".into(),
        PromptMode::Normal,
        "resolved prompt".into(),
        Some("$review".into()),
        Some("review".into()),
        vec![image],
        false,
    );
    let request_id = owner.pending_prompt_id.clone().expect("prompt request id");
    assert_eq!(request_id, "request-1");
    owner.apply_response(crate::agents::SessionResponse::prompt_delivery_unknown(
        request_id.clone(),
        PromptMode::Normal,
        "socket closed after write".into(),
    ));

    let user = owner
        .snapshot
        .conversation
        .items
        .iter()
        .find(|item| item.kind == crate::app::views::transcript::conversation::TranscriptKind::User)
        .ok_or("unknown prompt was removed")?;
    assert_eq!(user.text, "$review");
    assert_eq!(user.label, "Delivery unknown");
    assert_eq!(user.images.len(), 1);
    assert!(owner.pending_prompt_id.is_none());
    assert!(owner.pending_prompt_target.is_none());
    assert!(!owner.normal_prompt_in_flight);

    let reopened = StateStore::open_at(&database)?;
    let unknown = reopened.unknown_prompts()?;
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].message, "resolved prompt");
    assert_eq!(unknown[0].display_message.as_deref(), Some("$review"));
    assert_eq!(unknown[0].invocation.as_deref(), Some("review"));
    assert_eq!(unknown[0].images.len(), 1);

    owner.apply_response(prompt_response(&request_id, PromptMode::Normal, true));
    assert!(owner.pending_prompt_id.is_none());
    assert!(owner.pending_prompt_target.is_none());
    assert_eq!(
        owner
            .snapshot
            .conversation
            .items
            .iter()
            .filter(|item| {
                item.kind == crate::app::views::transcript::conversation::TranscriptKind::User
            })
            .count(),
        1
    );
    let outcomes = events
        .try_iter()
        .filter_map(|event| match event {
            RuntimeEvent::PromptResult { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes,
        [crate::agents::PromptOutcome::DeliveryUnknown],
        "a late old receipt must not resolve a newer composer submission"
    );
    let accepted = owner
        .state
        .as_ref()
        .expect("state")
        .accepted_prompt_history(&temp.path().join("session.jsonl"))?;
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["submissionId"], request_id);
    assert_eq!(accepted[0]["deliveryStatus"], "accepted");
    owner.apply_process_item(SessionEvent::Activity(
        json!({
            "type":"prompt_delivery",
            "submissionId":request_id,
            "status":"delivered",
            "message":{"role":"user", "content":[{"type":"text", "text":"resolved prompt"}]},
        })
        .into(),
    ));
    drop(owner);
    let reopened = StateStore::open_at(&database)?;
    assert!(reopened.unknown_prompts()?.is_empty());
    let accepted = reopened.accepted_prompt_history(&temp.path().join("session.jsonl"))?;
    assert!(accepted.is_empty());
    assert_eq!(
        reopened.prompt_presentations(&temp.path().join("session.jsonl"))?,
        [crate::agents::PromptPresentation {
            resolved_message: "resolved prompt".into(),
            display_message: "$review".into(),
            invocation: "review".into(),
        }]
    );
    Ok(())
}

#[test]
fn fatal_transport_failure_after_dispatch_is_delivery_unknown_not_rejected() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, events) = owner_without_process(temp.path().to_path_buf());
    owner.state = Some(StateStore::open_at(&database)?);
    owner.process = Some(Box::new(Recorder::default()));
    owner.active_session = Some(temp.path().join("session.jsonl"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;

    owner.send_prompt(
        "draft:fatal".into(),
        PromptMode::Normal,
        "retain after uncertain write".into(),
        Vec::new(),
        false,
    );
    assert_eq!(owner.pending_prompt_id.as_deref(), Some("request-1"));
    owner.apply_process_item(SessionEvent::Failure(
        "transport disconnected after dispatch".into(),
    ));

    let user = owner
        .snapshot
        .conversation
        .items
        .iter()
        .find(|item| item.kind == crate::app::views::transcript::conversation::TranscriptKind::User)
        .ok_or("fatal transport failure rolled back the prompt")?;
    assert_eq!(user.text, "retain after uncertain write");
    assert_eq!(user.label, "Delivery unknown");
    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            outcome: crate::agents::PromptOutcome::DeliveryUnknown,
            ..
        }
    )));
    drop(owner);
    let reopened = StateStore::open_at(&database)?;
    let unknown = reopened.unknown_prompts()?;
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].message, "retain after uncertain write");
    assert!(reopened.queued_prompts()?.is_empty());
    Ok(())
}

#[test]
fn retired_unknown_receipts_never_resolve_a_new_submission() -> Result<(), String> {
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
    const GIF: &str = "R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==";
    for mode in [PromptMode::Normal, PromptMode::Steer, PromptMode::FollowUp] {
        for navigate in [false, true] {
            let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
            let database = temp.path().join("state.sqlite3");
            let first_session = temp.path().join("first.jsonl");
            let second_session = if navigate {
                temp.path().join("second.jsonl")
            } else {
                first_session.clone()
            };
            let (mut owner, events) = owner_without_process(temp.path().into());
            let sent = Rc::new(RefCell::new(Vec::new()));
            owner.process = Some(Box::new(Recorder(sent.clone())));
            owner.state = Some(StateStore::open_at(&database)?);
            owner.active_session = Some(first_session.clone());
            owner.snapshot.selected_session = Some(first_session.clone());
            owner.snapshot.session = Some(empty_session());
            owner.startup_state_loaded = true;
            owner.startup_history_loaded = true;
            if mode != PromptMode::Normal {
                conversation_mut(&mut owner.snapshot).running = true;
            }
            let target = format!("session:{}", first_session.display());
            owner.send_prompt(
                target,
                mode,
                "same text".into(),
                vec![crate::protocol::PromptImage::new(
                    PNG.into(),
                    "image/png".into(),
                )],
                true,
            );
            let old_id = owner.pending_prompt_id.clone().expect("first request");
            owner.apply_process_item(SessionEvent::Activity(json!({
                "type":"prompt_delivery", "submissionId":old_id, "status":"unknown",
                "message":{"role":"user", "queued":mode != PromptMode::Normal,
                    "content":[{"type":"text", "text":"same text"}, {"type":"image", "data":PNG, "mimeType":"image/png"}]}
            }).into()));
            owner.apply_process_item(SessionEvent::Response(
                crate::agents::SessionResponse::prompt_delivery_unknown(
                    old_id.clone(),
                    mode,
                    "cancelled without a receipt".into(),
                ),
            ));
            assert!(owner.pending_prompt_id.is_none());
            assert!(owner.pending_prompt_target.is_none());
            assert!(!owner.normal_prompt_in_flight);
            assert!(owner.process.is_some() && owner.snapshot.connected);
            assert_eq!(StateStore::open_at(&database)?.unknown_prompts()?.len(), 1);
            owner.apply_process_item(SessionEvent::Activity(
                json!({"type":"agent_settled"}).into(),
            ));
            if navigate {
                owner.active_session = Some(second_session.clone());
                owner.snapshot.selected_session = Some(second_session.clone());
                owner.snapshot.conversation = Arc::default();
            }
            let target = format!("session:{}", second_session.display());
            owner.send_prompt(
                target,
                PromptMode::Normal,
                "same text".into(),
                vec![crate::protocol::PromptImage::new(
                    GIF.into(),
                    "image/gif".into(),
                )],
                false,
            );
            let new_id = owner
                .pending_prompt_id
                .clone()
                .expect("new request must not be blocked");
            assert_ne!(old_id, new_id);
            let new_outbox = owner.pending_outbox_id.expect("new durable row");
            let rows_before = owner.snapshot.conversation.items.len();
            let new_image = owner
                .snapshot
                .conversation
                .items
                .iter()
                .filter(|item| item.kind == TranscriptKind::User)
                .last()
                .unwrap()
                .images[0]
                .clone();
            for _ in 0..2 {
                owner.apply_process_item(SessionEvent::Activity(json!({
                    "type":"prompt_delivery", "submissionId":old_id, "status":"accepted",
                    "message":{"role":"user", "queued":mode != PromptMode::Normal,
                        "content":[{"type":"text", "text":"same text"}, {"type":"image", "data":PNG, "mimeType":"image/png"}]}
                }).into()));
                owner.apply_process_item(SessionEvent::Response(prompt_response(
                    &old_id, mode, true,
                )));
                owner.apply_process_item(SessionEvent::Activity(json!({
                    "type":"prompt_delivery", "submissionId":old_id, "status":"delivered",
                    "message":{"role":"user", "content":[{"type":"text", "text":"same text"}, {"type":"image", "data":PNG, "mimeType":"image/png"}]}
                }).into()));
                owner.apply_process_item(SessionEvent::Response(prompt_response(
                    &old_id, mode, false,
                )));
            }
            assert_eq!(owner.pending_prompt_id.as_deref(), Some(new_id.as_str()));
            assert_eq!(owner.pending_outbox_id, Some(new_outbox));
            assert_eq!(owner.snapshot.conversation.items.len(), rows_before);
            assert!(Arc::ptr_eq(
                &owner
                    .snapshot
                    .conversation
                    .items
                    .iter()
                    .filter(|item| item.kind == TranscriptKind::User)
                    .last()
                    .unwrap()
                    .images[0],
                &new_image
            ));
            let outcomes = events
                .try_iter()
                .filter_map(|event| match event {
                    RuntimeEvent::PromptResult { outcome, .. } => Some(outcome),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(outcomes, [crate::agents::PromptOutcome::DeliveryUnknown]);
            assert_eq!(sent_messages(&sent), ["same text", "same text"]);
            // Backend history need not carry app receipt IDs. Replacing it
            // clears the delivered row's ledger; an old event must still not
            // append its payload again or bind the new optimistic row.
            Arc::make_mut(&mut owner.snapshot.conversation).replace_history(&[
                json!({"role":"user", "content":[{"type":"text", "text":"saved historical turn"}]}),
            ]);
            let history_rows = owner.snapshot.conversation.items.len();
            for status in ["accepted", "delivered", "unknown", "rejected"] {
                owner.apply_process_item(SessionEvent::Activity(json!({
                    "type":"prompt_delivery", "submissionId":old_id, "status":status,
                    "message":{"role":"user", "content":[{"type":"text", "text":"same text"}, {"type":"image", "data":PNG, "mimeType":"image/png"}]}
                }).into()));
            }
            assert_eq!(owner.snapshot.conversation.items.len(), history_rows);
            assert_eq!(owner.pending_prompt_id.as_deref(), Some(new_id.as_str()));
            assert_eq!(owner.pending_outbox_id, Some(new_outbox));
            owner.apply_process_item(SessionEvent::Response(prompt_response(
                &new_id,
                PromptMode::Normal,
                true,
            )));
            drop(owner);
            let reopened = StateStore::open_at(&database)?;
            assert!(reopened.unknown_prompts()?.is_empty());
            assert!(reopened.queued_prompts()?.is_empty());
            let saved = reopened.accepted_prompt_history(&second_session)?;
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0]["submissionId"], new_id);
            assert_eq!(saved[0]["content"][1]["data"], GIF);
        }
    }
    Ok(())
}

#[test]
fn failed_unknown_write_keeps_the_original_sending_row_recoverable() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let (mut owner, _) = owner_without_process(temp.path().into());
    let sent = Rc::new(RefCell::new(Vec::new()));
    owner.process = Some(Box::new(Recorder(sent.clone())));
    owner.state = Some(StateStore::open_at(&database)?);
    owner.active_session = Some(temp.path().join("session.jsonl"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.send_prompt(
        "draft:old".into(),
        PromptMode::Normal,
        "recover old payload".into(),
        Vec::new(),
        false,
    );
    let old_id = owner.pending_prompt_id.clone().expect("request");
    let old_outbox = owner.pending_outbox_id.expect("saved before dispatch");
    let connection = rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
    connection.execute_batch("CREATE TRIGGER reject_unknown BEFORE UPDATE OF state ON outbox WHEN NEW.state='unknown' BEGIN SELECT RAISE(FAIL, 'unknown storage fixture'); END;")
        .map_err(|error| error.to_string())?;
    owner.apply_process_item(SessionEvent::Response(
        crate::agents::SessionResponse::prompt_delivery_unknown(
            old_id.clone(),
            PromptMode::Normal,
            "receipt lost".into(),
        ),
    ));
    assert!(owner.pending_prompt_id.is_none());
    assert_eq!(owner.retired_prompts[&old_id].outbox_id, Some(old_outbox));
    assert!(
        owner
            .snapshot
            .conversation
            .items
            .iter()
            .any(|item| item.label == "Delivery state not saved"
                && item.text.contains("unknown storage fixture"))
    );
    let state: String = connection
        .query_row(
            "SELECT state FROM outbox WHERE id=?1",
            [old_outbox],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        state, "sending",
        "failed update must retain the durable pre-dispatch record"
    );
    owner.apply_process_item(SessionEvent::Activity(
        json!({"type":"agent_settled"}).into(),
    ));
    owner.send_prompt(
        "draft:new".into(),
        PromptMode::Normal,
        "new payload".into(),
        Vec::new(),
        false,
    );
    let new_id = owner.pending_prompt_id.clone().expect("new request");
    owner.apply_process_item(SessionEvent::Response(prompt_response(
        &new_id,
        PromptMode::Normal,
        true,
    )));
    assert_eq!(sent_messages(&sent), ["recover old payload", "new payload"]);
    drop(owner);
    connection
        .execute_batch("DROP TRIGGER reject_unknown;")
        .map_err(|error| error.to_string())?;
    let reopened = StateStore::open_at(&database)?;
    let recovered = reopened.recover_interrupted_prompts()?;
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, old_outbox);
    assert_eq!(recovered[0].message, "recover old payload");
    assert!(
        reopened.queued_prompts()?.is_empty(),
        "unknown work must never replay"
    );
    Ok(())
}

#[test]
fn sending_prompt_is_not_treated_as_safe_to_delete() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let path = temp.path().join("session.jsonl");
    let id = store.enqueue_prompt(
        &format!("session:{}", path.display()),
        "pi",
        temp.path(),
        Some(&path),
        PromptMode::Normal,
        "do not delete an unresolved prompt",
        &[],
    )?;
    store.begin_prompt(id)?;

    assert!(
        store.has_queued_prompts_for(&[path])?,
        "a sending row can still lack a native outcome and must guard deletion or moves"
    );
    Ok(())
}

#[test]
fn failed_acknowledgement_commit_keeps_accepted_prompt_visible_and_recovers_unknown()
-> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    store.enqueue_prompt(
        "draft:ack",
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "keep me",
        &[],
    )?;
    let prompt = store.queued_prompts()?.remove(0);
    let connection = rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "CREATE TRIGGER reject_ack BEFORE INSERT ON session_events
        BEGIN SELECT RAISE(ABORT, 'ack storage failed'); END;",
        )
        .map_err(|error| error.to_string())?;
    let (mut owner, events) = owner_without_process(temp.path().to_owned());
    owner.process = Some(Box::new(Recorder::default()));
    owner.state = Some(store);
    owner.active_session = Some(temp.path().join("resumed-session"));
    owner.snapshot.session = Some(empty_session());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.deliver_queued(prompt);
    owner.apply_response(prompt_response("request-1", PromptMode::Normal, true));
    assert!(owner.snapshot.conversation.items.iter().any(|item| {
        item.kind == crate::app::views::transcript::conversation::TranscriptKind::User
            && item.text == "keep me"
    }));
    let (message, state): (String, String) = connection
        .query_row("SELECT message, state FROM outbox", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(message, "keep me");
    assert_eq!(state, "unknown");
    let results = events
        .try_iter()
        .filter_map(|event| match event {
            RuntimeEvent::PromptResult { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(results, [crate::agents::PromptOutcome::Accepted]);
    drop(owner);
    let reopened = StateStore::open_at(&database)?;
    let recovered = reopened.recover_interrupted_prompts()?;
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].message, "keep me");
    let state = connection
        .query_row("SELECT state FROM outbox", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(state, "unknown");
    Ok(())
}
