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
    crate::agents::SessionResponse {
        id: Some(id.into()),
        operation: SessionOperation::Prompt(mode),
        success,
        data: Value::Null,
        error: (!success).then(|| "rejected by test harness".into()),
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
        RuntimeEvent::PromptResult { target, accepted: false, .. } if target == "draft:startup"
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

    owner.apply_response(crate::agents::SessionResponse {
        id: Some("request-2".into()),
        operation: SessionOperation::LoadState,
        success: true,
        data: empty_session_value(),
        error: None,
    });
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
fn failed_acknowledgement_commit_retains_prompt_and_reports_failure() -> Result<(), String> {
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
    let (message, state): (String, String) = connection
        .query_row("SELECT message, state FROM outbox", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(message, "keep me");
    assert_eq!(state, "failed");
    let results = events
        .try_iter()
        .filter_map(|event| match event {
            RuntimeEvent::PromptResult { accepted, .. } => Some(accepted),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(results, [false]);
    Ok(())
}
