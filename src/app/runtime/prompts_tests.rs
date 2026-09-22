use super::*;
use crate::agents::Backend;
use crate::agents::{SessionEvent, SessionResponse, SessionResponsePayload};
use crate::app::persistence::SharedStateStore;
use crate::app::runtime::{RuntimeCommand, tests::owner_without_process};
use serde_json::json;
use std::{cell::RefCell, rc::Rc, sync::mpsc};

const HARNESSES: [&str; 6] = [
    "codex-cli",
    "pi",
    "opencode",
    "cursor-cli",
    "claude",
    "antigravity-acp",
];

struct HeldAcks {
    commands: Rc<RefCell<Vec<SessionCommand>>>,
    cancelled: Rc<RefCell<Vec<String>>>,
    next_id: usize,
}

impl HeldAcks {
    fn new() -> Self {
        Self {
            commands: Rc::new(RefCell::new(Vec::new())),
            cancelled: Rc::new(RefCell::new(Vec::new())),
            next_id: 0,
        }
    }
}

impl crate::agents::SessionTransport for HeldAcks {
    fn cancel_prompt(&mut self, id: &str) -> Result<(), String> {
        self.cancelled.borrow_mut().push(id.to_owned());
        Ok(())
    }
    fn send(&mut self, command: SessionCommand) -> Result<String, String> {
        self.commands.borrow_mut().push(command);
        self.next_id += 1;
        Ok(format!("held-{}", self.next_id))
    }

    fn respond(&mut self, _: crate::agents::extensions::ExtensionUiResponse) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        None
    }

    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

type HeldAcksRuntime = (
    RuntimeOwner,
    mpsc::Receiver<RuntimeEvent>,
    Rc<RefCell<Vec<SessionCommand>>>,
);

/// Running Claude session with a held-ack transport and a durable state store.
/// The first dispatched input receives the `held-1` request id.
fn held_acks_runtime(temp: &std::path::Path) -> Result<HeldAcksRuntime, String> {
    let (mut owner, events) = owner_without_process(temp.to_path_buf());
    let transport = HeldAcks::new();
    let commands = transport.commands.clone();
    owner.process = Some(Box::new(transport));
    owner.state = Some(SharedStateStore::open_at(&temp.join("state.sqlite3"))?);
    owner.harness = Some(Backend::Claude);
    owner.active_session = Some(temp.join("session"));
    owner.snapshot.selected_session = owner.active_session.clone();
    owner.snapshot.session = Some(empty_session());
    conversation_mut(&mut owner.snapshot).running = true;
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    Ok((owner, events, commands))
}

fn fail_delivery_receipt_writes(connection: &rusqlite::Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TRIGGER fail_delivery_receipt BEFORE INSERT ON session_events
             WHEN json_extract(NEW.body,'$.type')='prompt_delivery_receipt'
             BEGIN SELECT RAISE(FAIL,'delivery receipt fixture'); END;",
        )
        .map_err(|error| error.to_string())
}

fn allow_delivery_receipt_writes(connection: &rusqlite::Connection) -> Result<(), String> {
    connection
        .execute_batch("DROP TRIGGER fail_delivery_receipt;")
        .map_err(|error| error.to_string())
}

fn empty_session_json() -> serde_json::Value {
    serde_json::json!({
        "sessionId": "test-session",
        "isStreaming": false,
        "isCompacting": false,
        "autoCompactionEnabled": true,
        "messageCount": 0,
        "pendingMessageCount": 0
    })
}

fn empty_session() -> crate::protocol::SessionState {
    serde_json::from_value(empty_session_json()).expect("test operation should succeed")
}

#[test]
fn normal_receipt_after_steering_settles_releases_the_next_send() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let (mut owner, events, commands) = held_acks_runtime(temp.path())?;
    owner.harness = Some(Backend::OpenCode);
    conversation_mut(&mut owner.snapshot).running = false;
    let submit = |id: &str, mode, message: &str| RuntimeCommand::Prompt {
        submission_id: id.into(),
        target: "session:held".into(),
        mode,
        message: message.into(),
        display_message: None,
        invocation: None,
        images: Vec::new(),
        allow_while_running: false,
    };
    owner.apply_command(submit("normal", PromptMode::Normal, "original"));
    let normal = owner
        .pending_prompt_id
        .clone()
        .expect("normal was dispatched");
    owner.apply_process_item(SessionEvent::Activity(
        json!({
            "type":"prompt_delivery", "submissionId":normal, "status":"accepted"
        })
        .into(),
    ));
    owner.apply_command(submit("steer", PromptMode::Steer, "steer"));
    let steer = owner
        .pending_queued_prompts
        .keys()
        .next()
        .expect("steer was dispatched")
        .clone();
    owner.apply_command(RuntimeCommand::ApplySteering);

    // Settlement alone cannot acknowledge Normal. The native Escape E2E
    // requires its delivery too; delay that receipt here to check that the
    // runtime releases the send guard even if settlement arrives first.
    owner.apply_process_item(SessionEvent::Activity(
        json!({
            "type":"prompt_delivery", "submissionId":steer, "status":"delivered"
        })
        .into(),
    ));
    owner.apply_process_item(SessionEvent::Response(SessionResponse::success(
        Some(steer),
        SessionResponsePayload::Prompt(PromptMode::Steer),
    )));
    owner.apply_process_item(SessionEvent::Activity(
        json!({"type":"agent_settled"}).into(),
    ));
    assert!(!owner.snapshot.conversation.running);
    assert_eq!(owner.pending_prompt_id.as_deref(), Some(normal.as_str()));
    assert!(owner.pending_queued_prompts.is_empty());
    owner.apply_process_item(SessionEvent::Activity(
        json!({
            "type":"prompt_delivery", "submissionId":normal, "status":"delivered"
        })
        .into(),
    ));
    owner.apply_process_item(SessionEvent::Response(SessionResponse::success(
        Some(normal),
        SessionResponsePayload::Prompt(PromptMode::Normal),
    )));
    assert!(owner.pending_prompt_id.is_none());
    assert!(owner.pending_prompt_target.is_none());
    events.try_iter().for_each(drop);

    owner.apply_command(submit("next", PromptMode::Normal, "next input"));
    let rejected = events.try_iter().any(|event| {
        matches!(event,
            RuntimeEvent::PromptResult {
                submission_id: Some(id),
                outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance, ..
            } if id == "next"
        )
    });
    assert!(
        !rejected,
        "the delivered normal input must no longer block the next send"
    );
    let saved: i64 = rusqlite::Connection::open(temp.path().join("state.sqlite3"))
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT COUNT(*) FROM outbox WHERE message='next input'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        saved, 1,
        "the next input must remain durably owned, not disappear"
    );
    assert!(
        commands
            .borrow()
            .iter()
            .any(|command| matches!(command, SessionCommand::ApplySteering))
    );
    Ok(())
}

#[test]
fn admission_before_delivery_retains_the_outbox_and_composer_identity() -> Result<(), String> {
    for mode in [PromptMode::Normal, PromptMode::Steer, PromptMode::FollowUp] {
        let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let (mut owner, _, _) = held_acks_runtime(temp.path())?;
        conversation_mut(&mut owner.snapshot).running = false;
        owner.send_prompt_for_submission(
            "composer-id".into(),
            "session:held".into(),
            mode,
            "same".into(),
            vec![],
            false,
        );
        let id = owner.pending_prompt_id.clone().expect("sent");
        let response =
            SessionResponse::success(Some(id.clone()), SessionResponsePayload::Prompt(mode));
        owner.apply_response(response.clone());
        assert_eq!(owner.pending_prompt_id.as_deref(), Some(id.as_str()));
        assert!(!owner.pending_prompt_result_emitted);
        assert_eq!(
            owner
                .state
                .as_ref()
                .unwrap()
                .with(|store| store.queued_prompts())?
                .len(),
            1
        );
        owner.apply_process_item(SessionEvent::Activity(
            json!({
                "type":"prompt_delivery", "submissionId":id, "status":"delivered",
                "message":{"role":"user", "content":"same", "queued":mode != PromptMode::Normal}
            })
            .into(),
        ));
        owner.apply_response(response);
        assert!(
            owner
                .state
                .as_ref()
                .unwrap()
                .with(|store| store.queued_prompts())?
                .is_empty()
        );
        assert!(owner.pending_prompt_id.is_none());
        assert!(owner.snapshot.conversation.pending_receipts().is_empty());
    }
    Ok(())
}

#[test]
fn live_cancel_routes_composer_and_native_ids_to_the_shared_owner() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let (mut owner, _, _) = held_acks_runtime(temp.path())?;
    let transport = HeldAcks::new();
    let cancelled = transport.cancelled.clone();
    owner.process = Some(Box::new(transport));
    owner.send_prompt_for_submission(
        "composer-id".into(),
        "session:held".into(),
        PromptMode::Steer,
        "same".into(),
        vec![],
        false,
    );
    for id in ["composer-id", "held-1"] {
        owner.apply_command(RuntimeCommand::CancelQueued {
            target: "session:held".into(),
            id: id.into(),
        });
    }
    owner.apply_command(RuntimeCommand::CancelQueued {
        target: "another-session".into(),
        id: "held-1".into(),
    });
    assert_eq!(*cancelled.borrow(), ["held-1", "held-1"]);
    assert_eq!(
        owner
            .state
            .as_ref()
            .unwrap()
            .with(|store| store.queued_prompts())?
            .len(),
        1,
        "only the owner's cancelled event can change durable state"
    );
    Ok(())
}

#[test]
fn command_entry_sends_all_unacknowledged_inputs_before_first_escape() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let (mut owner, events, commands) = held_acks_runtime(temp.path())?;

    for (submission_id, mode, message) in [
        ("steer-id", PromptMode::Steer, "steer now"),
        ("follow-id", PromptMode::FollowUp, "then follow"),
        ("later-id", PromptMode::FollowUp, "then later"),
    ] {
        owner.apply_command(RuntimeCommand::Prompt {
            submission_id: submission_id.into(),
            target: "session:held".into(),
            mode,
            message: message.into(),
            display_message: None,
            invocation: None,
            images: Vec::new(),
            allow_while_running: false,
        });
    }
    owner.apply_command(RuntimeCommand::ApplySteering);

    assert!(owner.pending_prompt_id.is_some());
    assert_eq!(owner.pending_queued_prompts.len(), 2);
    assert!(matches!(
        commands.borrow().as_slice(),
        [
            SessionCommand::Prompt {
                mode: PromptMode::Steer,
                ..
            },
            SessionCommand::Prompt {
                mode: PromptMode::FollowUp,
                ..
            },
            SessionCommand::Prompt {
                mode: PromptMode::FollowUp,
                ..
            },
            SessionCommand::ApplySteering,
        ]
    ));
    owner.apply_process_item(SessionEvent::Activity(
        json!({
            "type":"prompt_delivery",
            "submissionId":"held-2",
            "status":"delivered"
        })
        .into(),
    ));
    let delivered = events
        .try_iter()
        .filter(|event| {
            matches!(
                event,
                RuntimeEvent::PromptResult {
                    submission_id: Some(id),
                    outcome: crate::agents::PromptOutcome::Accepted,
                    ..
                } if id == "follow-id"
            )
        })
        .count();
    assert_eq!(delivered, 1);
    assert!(owner.pending_queued_prompts["held-2"].result_emitted);
    let delivery_rows: i64 = rusqlite::Connection::open(temp.path().join("state.sqlite3"))
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT COUNT(*) FROM session_events
              WHERE json_extract(body,'$.type')='prompt_delivery_receipt'
                AND json_extract(body,'$.submissionId')='held-2'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(delivery_rows, 1, "delivery must bind the queued outbox row");

    let failure_connection = rusqlite::Connection::open(temp.path().join("state.sqlite3"))
        .map_err(|error| error.to_string())?;
    fail_delivery_receipt_writes(&failure_connection)?;
    owner.apply_process_item(SessionEvent::Activity(
        json!({
            "type":"prompt_delivery",
            "submissionId":"held-3",
            "status":"delivered"
        })
        .into(),
    ));
    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            submission_id: Some(id),
            outcome: crate::agents::PromptOutcome::Accepted,
            ..
        } if id == "later-id"
    )));
    allow_delivery_receipt_writes(&failure_connection)?;
    owner.apply_response(SessionResponse::success(
        Some("held-3".into()),
        SessionResponsePayload::Prompt(PromptMode::FollowUp),
    ));
    assert!(
        events
            .try_iter()
            .all(|event| !matches!(event, RuntimeEvent::PromptResult { .. }))
    );
    assert_eq!(owner.pending_prompt_id.as_deref(), Some("held-1"));
    fail_delivery_receipt_writes(&failure_connection)?;
    owner.apply_process_item(SessionEvent::Activity(
        json!({
            "type":"prompt_delivery",
            "submissionId":"held-1",
            "status":"delivered"
        })
        .into(),
    ));
    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            submission_id: Some(id),
            outcome: crate::agents::PromptOutcome::Accepted,
            ..
        } if id == "steer-id"
    )));
    allow_delivery_receipt_writes(&failure_connection)?;
    owner.apply_response(SessionResponse::success(
        Some("held-1".into()),
        SessionResponsePayload::Prompt(PromptMode::Steer),
    ));
    assert!(
        events
            .try_iter()
            .all(|event| !matches!(event, RuntimeEvent::PromptResult { .. }))
    );
    owner.reset_process_runtime();
    assert!(
        events
            .try_iter()
            .all(|event| !matches!(event, RuntimeEvent::PromptResult { .. }))
    );
    let connection = rusqlite::Connection::open(temp.path().join("state.sqlite3"))
        .map_err(|error| error.to_string())?;
    let reset_delivery: (i64, i64, i64, i64) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM outbox WHERE message='then follow' AND state='pending'),
                (SELECT COUNT(*) FROM session_events
                  WHERE json_extract(body,'$.type')='accepted_prompt'
                    AND json_extract(body,'$.submissionId')='held-2'),
                (SELECT COUNT(*) FROM outbox WHERE message='steer now' AND state='pending'),
                (SELECT COUNT(*) FROM session_events
                  WHERE json_extract(body,'$.type')='accepted_prompt'
                    AND json_extract(body,'$.submissionId')='held-1')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(reset_delivery, (0, 1, 0, 1));
    let reopened =
        crate::app::persistence::StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    assert!(reopened.queued_prompts()?.is_empty());
    let hardened: (i64, i64, i64, i64) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM outbox WHERE message='then later' AND state='pending'),
                (SELECT COUNT(*) FROM session_events
                  WHERE json_extract(body,'$.type')='accepted_prompt'
                    AND json_extract(body,'$.submissionId')='held-3'),
                (SELECT COUNT(*) FROM session_events
                  WHERE json_extract(body,'$.type')='prompt_delivery_receipt'
                    AND json_extract(body,'$.submissionId')='held-1'),
                (SELECT COUNT(*) FROM session_events
                  WHERE json_extract(body,'$.type')='prompt_delivery_receipt'
                    AND json_extract(body,'$.submissionId')='held-3')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(hardened, (0, 1, 1, 1));
    assert!(
        reopened
            .accepted_prompt_history(&temp.path().join("session"))?
            .is_empty(),
        "delivered held-1 and held-3 must not return as pending receipt history"
    );
    Ok(())
}

/// A transport that fails after a known delivery must keep the accepted
/// outcome for the composer and leave the durable row in the recoverable
/// `pending` state (the delivery receipt cannot be persisted because the
/// injected trigger fails).
#[test]
fn transport_failure_after_delivery_keeps_the_accepted_outcome_and_recoverable_row()
-> Result<(), String> {
    let failure_temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let (mut failure_owner, failure_events, _commands) = held_acks_runtime(failure_temp.path())?;
    let cleanup_connection = rusqlite::Connection::open(failure_temp.path().join("state.sqlite3"))
        .map_err(|error| error.to_string())?;
    fail_delivery_receipt_writes(&cleanup_connection)?;
    failure_owner.apply_command(RuntimeCommand::Prompt {
        submission_id: "delivered-primary".into(),
        target: "session:failure".into(),
        mode: PromptMode::Steer,
        message: "delivered before failure".into(),
        display_message: None,
        invocation: None,
        images: Vec::new(),
        allow_while_running: false,
    });
    failure_owner.apply_process_item(SessionEvent::Activity(
        json!({
            "type":"prompt_delivery",
            "submissionId":"held-1",
            "status":"delivered"
        })
        .into(),
    ));
    failure_owner.fail("transport failed after delivery".into());
    let outcomes = failure_events
        .try_iter()
        .filter_map(|event| match event {
            RuntimeEvent::PromptResult {
                submission_id: Some(id),
                outcome,
                ..
            } if id == "delivered-primary" => Some(outcome),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(outcomes, [crate::agents::PromptOutcome::Accepted]);
    let reopened =
        crate::app::persistence::StateStore::open_at(&failure_temp.path().join("state.sqlite3"))?;
    assert_eq!(reopened.queued_prompts()?.len(), 1);
    let state: String = cleanup_connection
        .query_row(
            "SELECT state FROM outbox WHERE message='delivered before failure'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        state, "pending",
        "unpersisted delivery must remain retryable"
    );
    Ok(())
}

#[test]
fn rejected_submission_keeps_the_process_and_accepts_the_next_message() -> Result<(), String> {
    use crate::agents::extensions::ExtensionUiResponse;
    use crate::agents::{SessionEvent, SessionResponse, SessionResponsePayload, SessionTransport};
    use std::{cell::RefCell, rc::Rc};

    struct RejectOnce {
        calls: Rc<RefCell<Vec<SessionCommand>>>,
        closes: Rc<RefCell<usize>>,
        events: Rc<RefCell<std::collections::VecDeque<SessionEvent>>>,
    }
    impl SessionTransport for RejectOnce {
        fn send(&mut self, command: SessionCommand) -> Result<String, String> {
            self.calls.borrow_mut().push(command);
            if self.calls.borrow().len() == 1 {
                Err("unsupported prompt command".into())
            } else {
                Ok("accepted".into())
            }
        }
        fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<SessionEvent> {
            self.events.borrow_mut().pop_front()
        }
        fn close(&mut self) -> Result<(), String> {
            *self.closes.borrow_mut() += 1;
            Ok(())
        }
    }

    for running in [false, true] {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let (mut owner, events) = owner_without_process(temp.path().into());
        let calls = Rc::new(RefCell::new(Vec::new()));
        let closes = Rc::new(RefCell::new(0));
        let transport_events = Rc::new(RefCell::new(std::collections::VecDeque::new()));
        owner.process = Some(Box::new(RejectOnce {
            calls: calls.clone(),
            closes: closes.clone(),
            events: transport_events.clone(),
        }));
        let database = temp.path().join("state.sqlite3");
        owner.state = Some(SharedStateStore::open_at(&database)?);
        owner.harness = Some(Backend::Claude);
        owner.active_session = Some(temp.path().join("session"));
        let mut state = empty_session();
        state.is_streaming = running;
        owner.snapshot.session = Some(state);
        conversation_mut(&mut owner.snapshot).running = running;
        owner.startup_state_loaded = true;
        owner.startup_history_loaded = true;
        let mode = if running {
            PromptMode::Steer
        } else {
            PromptMode::Normal
        };
        owner.send_prompt(
            "draft:reject-once".into(),
            mode,
            "bad input".into(),
            vec![],
            false,
        );
        assert_eq!(
            *closes.borrow(),
            0,
            "a submission error must not close the healthy process"
        );
        assert!(owner.process.is_some());
        assert!(owner.snapshot.connected);
        assert_eq!(owner.snapshot.conversation.running, running);
        assert!(owner.pending_prompt_target.is_none());
        assert!(owner.pending_prompt_id.is_none());
        assert!(
            !owner.snapshot.conversation.items.iter().any(|item| {
                item.kind == crate::conversation::TranscriptKind::User && item.text == "bad input"
            }),
            "rejected optimistic text must roll back"
        );
        let connection = rusqlite::Connection::open(&database).map_err(|e| e.to_string())?;
        let rejected: (String, String) = connection
            .query_row("SELECT message, state FROM outbox", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|e| e.to_string())?;
        assert_eq!(rejected, ("bad input".into(), "pending".into()));
        let reopened = crate::app::persistence::StateStore::open_at(&database)?;
        assert_eq!(reopened.queued_prompts()?.len(), 1);
        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::PromptResult {
                outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                ..
            }
        )));

        let next_mode = if running {
            PromptMode::FollowUp
        } else {
            PromptMode::Normal
        };
        owner.send_prompt(
            "draft:reject-once".into(),
            next_mode,
            "valid input".into(),
            vec![],
            false,
        );
        assert_eq!(
            calls.borrow().len(),
            2,
            "the same process must accept another submission"
        );
        owner.apply_response(SessionResponse::success(
            Some("accepted".into()),
            SessionResponsePayload::Prompt(next_mode),
        ));
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, RuntimeEvent::PromptResult { .. }))
        );
        owner.apply_process_item(SessionEvent::Activity(
            json!({
                "type":"prompt_delivery", "submissionId":"accepted", "status":"delivered",
                "message":{"role":"user", "content":"valid input", "queued":running}
            })
            .into(),
        ));
        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::PromptResult {
                outcome: crate::agents::PromptOutcome::Accepted,
                ..
            }
        )));
        assert_eq!(*closes.borrow(), 0);
        transport_events
            .borrow_mut()
            .push_back(SessionEvent::Failure("connection closed".into()));
        while let Some(event) = owner.process.as_mut().and_then(|process| process.poll()) {
            owner.apply_process_item(event);
        }
        assert_eq!(
            *closes.borrow(),
            1,
            "an actual transport failure still closes the session"
        );
        assert!(owner.process.is_none());
        assert!(!owner.snapshot.connected);
        assert!(owner.pending_prompt_target.is_none());
        assert!(owner.pending_prompt_id.is_none());
    }
    Ok(())
}

#[test]
fn automatic_title_does_not_treat_unloaded_resume_as_new() {
    for harness in HARNESSES {
        let (mut owner, _events) = owner_without_process(std::env::temp_dir());
        owner.harness = Some(harness.parse().expect("fixture backend"));
        owner.snapshot.selected_session = Some(std::env::temp_dir().join("existing-session"));
        owner.snapshot.history_preview = true;
        assert!(owner.snapshot.session.is_none());

        assert!(
            !owner.should_generate_automatic_title(PromptMode::Normal, false),
            "{harness} must not generate a title while resuming unloaded history"
        );
    }
}

#[test]
fn automatic_title_requires_a_loaded_new_unnamed_session() {
    for harness in HARNESSES {
        let (mut owner, _events) = owner_without_process(std::env::temp_dir());
        owner.harness = Some(harness.parse().expect("fixture backend"));
        owner.title_generation.new_session = true;
        owner.snapshot.session = Some(empty_session());
        assert!(!owner.should_generate_automatic_title(PromptMode::Normal, false));
        owner.startup_state_loaded = true;
        assert!(!owner.should_generate_automatic_title(PromptMode::Normal, false));
        owner.startup_history_loaded = true;
        assert_eq!(
            owner.should_generate_automatic_title(PromptMode::Normal, false),
            matches!(harness, "codex-cli" | "pi"),
            "{harness} fresh-session title support"
        );
        owner
            .snapshot
            .session
            .as_mut()
            .expect("test operation should succeed")
            .session_name = Some("Keep this title".into());
        assert!(!owner.should_generate_automatic_title(PromptMode::Normal, false));
        owner
            .snapshot
            .session
            .as_mut()
            .expect("test operation should succeed")
            .session_name = None;
        owner
            .snapshot
            .session
            .as_mut()
            .expect("test operation should succeed")
            .message_count = 2;
        assert!(!owner.should_generate_automatic_title(PromptMode::Normal, false));
        owner
            .snapshot
            .session
            .as_mut()
            .expect("test operation should succeed")
            .message_count = 0;
        assert!(!owner.should_generate_automatic_title(PromptMode::Normal, true));
        assert!(!owner.should_generate_automatic_title(PromptMode::Steer, false));
        assert!(!owner.should_generate_automatic_title(PromptMode::FollowUp, false));
        owner.title_generation.in_flight = true;
        assert!(!owner.should_generate_automatic_title(PromptMode::Normal, false));
    }
}

#[test]
fn resumed_prompt_survives_startup_history_without_starting_title_generation() {
    struct Recorder(std::rc::Rc<std::cell::RefCell<Vec<SessionCommand>>>);
    impl crate::agents::SessionTransport for Recorder {
        fn send(&mut self, command: SessionCommand) -> Result<String, String> {
            self.0.borrow_mut().push(command);
            Ok("prompt-1".into())
        }
        fn respond(
            &mut self,
            _: crate::agents::extensions::ExtensionUiResponse,
        ) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<crate::agents::SessionEvent> {
            None
        }
        fn close(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    for (harness, history_first, preserve) in HARNESSES.into_iter().flat_map(|harness| {
        [false, true].into_iter().flat_map(move |history_first| {
            [false, true].map(move |preserve| (harness, history_first, preserve))
        })
    }) {
        let (mut owner, _events) = owner_without_process(std::env::temp_dir());
        owner.harness = Some(harness.parse().expect("fixture backend"));
        let commands = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        owner.process = Some(Box::new(Recorder(commands.clone())));
        owner.active_session = Some(std::env::temp_dir().join("existing-session"));
        let history = vec![serde_json::json!({"role": "user", "content": "Earlier task"})];
        if preserve {
            conversation_mut(&mut owner.snapshot).replace_history(&history);
        }
        owner.pending_prompt_item = Some(
            conversation_mut(&mut owner.snapshot).push_local_user_with_prompt_images(
                "Continue the task".into(),
                &[],
                false,
            ),
        );
        owner.dispatch_prompt(
            PromptMode::Normal,
            "Continue the task".into(),
            None,
            None,
            Vec::new(),
            None,
        );

        assert!(owner.deferred_prompt.is_some());
        assert!(commands.borrow().is_empty());
        let state = crate::agents::SessionResponse::success(
            None,
            crate::agents::SessionResponsePayload::LoadState(
                serde_json::from_value(empty_session_json()).expect("state fixture"),
            ),
        );
        let history = crate::agents::SessionResponse::success(
            None,
            crate::agents::SessionResponsePayload::LoadHistory(if preserve {
                crate::agents::SessionHistory::Preserve
            } else {
                crate::agents::SessionHistory::Replace {
                    messages: history,
                    prompt_deliveries: None,
                }
            }),
        );
        let responses = if history_first {
            [history, state]
        } else {
            [state, history]
        };
        for (index, response) in responses.into_iter().enumerate() {
            owner.apply_response(response);
            if index == 0 {
                assert!(
                    commands.borrow().is_empty(),
                    "must await both startup responses"
                );
            }
        }

        assert!(owner.deferred_prompt.is_none());
        assert!(!owner.title_generation.in_flight, "{harness}");
        assert_eq!(owner.pending_prompt_id.as_deref(), Some("prompt-1"));
        assert!(matches!(
            commands.borrow().as_slice(),
            [SessionCommand::Prompt { .. }]
        ));
        let texts = owner
            .snapshot
            .conversation
            .items
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(texts, ["Earlier task", "Continue the task"]);
        owner.rollback_pending_prompt();
        assert_eq!(owner.snapshot.conversation.items.len(), 1);
        assert_eq!(owner.snapshot.conversation.items[0].text, "Earlier task");
    }
}

#[test]
fn unselected_backend_rejects_prompt_before_enqueuing() {
    let (mut owner, events) = owner_without_process(std::env::temp_dir());
    owner.harness = None;
    owner.send_prompt(
        "draft:unselected".into(),
        PromptMode::Normal,
        "hello".into(),
        vec![],
        false,
    );
    assert!(owner.pending_prompt_target.is_none());
    assert_eq!(
        owner.snapshot.conversation.items[0].text,
        "Choose a backend before sending a message."
    );
    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
            ..
        }
    )));
}
