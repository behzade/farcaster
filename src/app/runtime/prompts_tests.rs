use super::*;
use crate::app::runtime::tests::owner_without_process;

const HARNESSES: [&str; 6] = [
    "codex-cli",
    "pi",
    "opencode2",
    "cursor-cli",
    "claude",
    "antigravity-acp",
];

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
        owner.state = Some(crate::app::persistence::StateStore::open_at(&database)?);
        owner.harness = "claude".into();
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
                item.kind == crate::app::views::transcript::conversation::TranscriptKind::User
                    && item.text == "bad input"
            }),
            "rejected optimistic text must roll back"
        );
        let connection = rusqlite::Connection::open(&database).map_err(|e| e.to_string())?;
        let rejected: (String, String) = connection
            .query_row("SELECT message, state FROM outbox", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|e| e.to_string())?;
        assert_eq!(rejected, ("bad input".into(), "failed".into()));
        let reopened = crate::app::persistence::StateStore::open_at(&database)?;
        assert!(reopened.queued_prompts()?.is_empty());
        let sending: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE state='sending'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(sending, 0, "no Sending blocker remains");
        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::PromptResult {
                accepted: false,
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
            events
                .try_iter()
                .any(|event| matches!(event, RuntimeEvent::PromptResult { accepted: true, .. }))
        );
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
        owner.harness = harness.into();
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
        owner.harness = harness.into();
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
        owner.harness = harness.into();
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
                crate::agents::SessionHistory::Replace(history)
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
    owner.harness.clear();
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
            accepted: false,
            ..
        }
    )));
}
