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

fn empty_session() -> crate::protocol::SessionState {
    serde_json::from_value(serde_json::json!({
        "sessionId": "test-session",
        "isStreaming": false,
        "isCompacting": false,
        "autoCompactionEnabled": true,
        "messageCount": 0,
        "pendingMessageCount": 0
    }))
    .expect("test operation should succeed")
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
        let history = serde_json::json!([{"role": "user", "content": "Earlier task"}]);
        if preserve {
            conversation_mut(&mut owner.snapshot)
                .replace_history(history.as_array().expect("history"));
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
        let state = crate::agents::SessionResponse {
            id: None,
            operation: crate::agents::SessionOperation::LoadState,
            success: true,
            data: serde_json::to_value(empty_session()).expect("session state"),
            error: None,
        };
        let history = crate::agents::SessionResponse {
            id: None,
            operation: crate::agents::SessionOperation::LoadHistory,
            success: true,
            data: serde_json::json!({"messages": history, "preserve": preserve}),
            error: None,
        };
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
