use super::*;
use crate::agents::{
    SessionResponse,
    extensions::{AgentMode, SlashCommand, SlashCommandSource},
};
use crate::runtime::tests::owner_without_process;

#[test]
fn account_usage_event_updates_the_runtime_snapshot() {
    let mut usage = crate::agents::AccountUsage::default();
    let event = json!({
        "type": "account_usage_changed",
        "usage": {
            "weekly": {
                "remainingPercent": 68.0,
                "resetsAt": 1_788_766_092_i64
            }
        }
    });

    assert!(update_account_usage_from_event(
        &mut usage,
        &SessionActivityKind::AccountUsageChanged,
        &event,
    ));
    assert_eq!(usage.weekly.unwrap().remaining_percent, 68.0);
}

#[test]
fn failed_catalog_responses_preserve_last_valid_values_and_report_error() {
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    let models = vec![Model {
        id: "model".into(),
        name: "Model".into(),
        provider: "provider".into(),
        context_window: 1000,
        reasoning: true,
        efforts: Some(vec!["high".into()]),
        resolved_model: None,
        service_tiers: Vec::new(),
        access_modes: None,
    }];
    let modes = vec![AgentMode {
        id: "plan".into(),
        name: "Plan".into(),
        description: None,
    }];
    let commands = vec![SlashCommand {
        name: "review".into(),
        description: None,
        source: SlashCommandSource::Extension,
    }];
    for payload in [
        Payload::ListModels(models.clone()),
        Payload::ListReasoningLevels(vec!["high".into()]),
        Payload::ListModes {
            modes: modes.clone(),
            selected: Some("plan".into()),
        },
        Payload::ListCommands(commands.clone()),
    ] {
        let operation = payload.operation();
        owner.apply_response(SessionResponse::success(None, payload));
        owner.apply_response(SessionResponse::failure(
            Some("refresh".into()),
            operation,
            "invalid catalog payload".into(),
        ));
        assert_eq!(owner.active_snapshot().status, "Command failed");
    }
    let snapshot = owner.active_snapshot();
    assert_eq!(snapshot.models, models);
    assert_eq!(snapshot.thinking_levels, ["high"]);
    assert_eq!(snapshot.modes, modes);
    assert_eq!(snapshot.selected_mode.as_deref(), Some("plan"));
    assert_eq!(snapshot.commands, commands);
    assert!(
        snapshot
            .conversation
            .items
            .iter()
            .any(|item| item.text.contains("invalid catalog payload"))
    );

    // A valid empty catalog is different from a failed refresh.
    for payload in [
        Payload::ListModels(Vec::new()),
        Payload::ListReasoningLevels(Vec::new()),
        Payload::ListModes {
            modes: Vec::new(),
            selected: None,
        },
        Payload::ListCommands(Vec::new()),
    ] {
        owner.apply_response(SessionResponse::success(None, payload));
    }
    let snapshot = owner.active_snapshot();
    assert!(snapshot.models.is_empty());
    assert!(snapshot.thinking_levels.is_empty());
    assert!(snapshot.modes.is_empty());
    assert!(snapshot.selected_mode.is_none());
    assert!(snapshot.commands.is_empty());
}

#[test]
fn failed_startup_payload_does_not_mark_state_or_history_loaded() {
    for operation in [SessionOperation::LoadState, SessionOperation::LoadHistory] {
        let (mut owner, _events) = owner_without_process(std::env::temp_dir());
        owner.apply_response(SessionResponse::failure(
            Some("startup".into()),
            operation,
            "malformed startup payload".into(),
        ));
        assert!(!owner.startup_state_loaded);
        assert!(!owner.startup_history_loaded);
        assert_eq!(owner.active_snapshot().status, "Command failed");
    }
}

#[test]
fn startup_state_keeps_requested_cursor_controls_until_state_confirms_them() {
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    owner.harness = Some(crate::agents::Backend::Cursor);
    let chosen: Model = serde_json::from_value(json!({
        "id":"chosen", "name":"Chosen", "provider":"cursor",
        "serviceTiers":["standard", "priority"]
    }))
    .expect("decode chosen model");
    owner.snapshot.models = vec![chosen.clone()];
    owner.set_model(chosen.clone());
    owner.set_service_tier("priority".into());
    let state = |model: &str, tier: &str| {
        serde_json::from_value(json!({
            "model":{"id":model,"name":model,"provider":"cursor"},
            "serviceTier":tier,"serviceTiers":["standard", "priority"],
            "isStreaming":false,"isCompacting":false,
            "sessionId":"new-session","autoCompactionEnabled":true,
            "messageCount":0,"pendingMessageCount":0
        }))
        .expect("decode Cursor state")
    };

    owner.apply_response(SessionResponse::success(
        None,
        Payload::LoadState(Box::new(state("saved", "standard"))),
    ));
    assert_eq!(owner.snapshot.session_identity().model, Some(&chosen));
    assert_eq!(owner.snapshot.selected_service_tier(), Some("priority"));
    assert!(owner.snapshot.pending_initial_model);
    assert!(owner.snapshot.pending_initial_service_tier);

    owner.pending_session_controls = Default::default();
    owner.apply_response(SessionResponse::success(
        None,
        Payload::LoadState(Box::new(state("chosen", "priority"))),
    ));
    assert!(!owner.snapshot.pending_initial_model);
    assert!(!owner.snapshot.pending_initial_service_tier);
    assert_eq!(
        owner
            .snapshot
            .session_identity()
            .model
            .map(|model| model.id.as_str()),
        Some("chosen")
    );
    assert_eq!(owner.snapshot.selected_service_tier(), Some("priority"));
}

#[test]
fn first_prompt_waits_for_cursor_service_tier_acknowledgement() {
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.deferred_prompt = Some(super::prompts::DeferredPrompt {
        mode: crate::protocol::PromptMode::Normal,
        message: "first message".into(),
        display_message: None,
        invocation: None,
        images: Vec::new(),
        outbox_id: None,
    });
    owner
        .pending_session_controls
        .tier_sent("tier-request".into(), "priority".into());

    owner.maybe_send_deferred_prompt();

    assert!(owner.deferred_prompt.is_some());
    assert!(owner.pending_prompt_id.is_none());
}

#[test]
fn cancelled_background_refreshes_preserve_transcript_and_status() {
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    let status = owner.active_snapshot().status.clone();
    let item_count = owner.active_snapshot().conversation.items.len();
    for operation in [
        SessionOperation::LoadState,
        SessionOperation::LoadHistory,
        SessionOperation::LoadUsage,
    ] {
        owner.apply_response(SessionResponse::cancelled(
            "refresh".into(),
            operation,
            "transport restarted".into(),
        ));
        assert_eq!(owner.active_snapshot().status, status);
        assert_eq!(owner.active_snapshot().conversation.items.len(), item_count);
    }
}

#[test]
fn cancelled_undelivered_prompt_returns_ownership_without_command_error() {
    let (mut owner, events) = owner_without_process(std::env::temp_dir());
    owner.pending_prompt_id = Some("pending-steer".into());
    owner.pending_submission_id = Some("composer-steer".into());
    owner.pending_prompt_target = Some("session:one".into());
    owner.active_snapshot_mut().status = "Stopping".into();

    owner.apply_response(SessionResponse::cancelled(
        "pending-steer".into(),
        SessionOperation::Prompt(PromptMode::Steer),
        "Prompt cancelled before delivery".into(),
    ));

    assert!(owner.pending_prompt_id.is_none());
    assert!(owner.pending_prompt_target.is_none());
    assert_eq!(owner.active_snapshot().status, "Stopping");
    assert!(owner.active_snapshot().conversation.items.is_empty());
    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            submission_id: Some(id),
            outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
            ..
        } if id == "composer-steer"
    )));
}

#[test]
fn cancelled_startup_query_resolves_deferred_prompt_failure() {
    for operation in [SessionOperation::LoadState, SessionOperation::LoadHistory] {
        let (mut owner, _events) = owner_without_process(std::env::temp_dir());
        owner.deferred_prompt = Some(super::prompts::DeferredPrompt {
            mode: PromptMode::Normal,
            message: "waiting for startup".into(),
            display_message: None,
            invocation: None,
            images: Vec::new(),
            outbox_id: None,
        });
        owner.apply_response(SessionResponse::cancelled(
            "startup".into(),
            operation,
            "transport restarted".into(),
        ));
        assert!(owner.deferred_prompt.is_none());
        assert_eq!(owner.active_snapshot().status, "Command failed");
    }
}

#[test]
fn real_refresh_failure_and_cancelled_user_command_remain_visible() {
    for response in [
        SessionResponse::failure(
            Some("refresh".into()),
            SessionOperation::LoadUsage,
            "backend error".into(),
        ),
        SessionResponse::cancelled(
            "command".into(),
            SessionOperation::Compact,
            "transport restarted".into(),
        ),
    ] {
        let (mut owner, _events) = owner_without_process(std::env::temp_dir());
        owner.apply_response(response);
        assert_eq!(owner.active_snapshot().status, "Command failed");
        assert!(!owner.active_snapshot().conversation.items.is_empty());
    }
}

#[test]
fn cancelled_startup_query_resolves_pending_control_failure() {
    for operation in [SessionOperation::LoadState, SessionOperation::LoadHistory] {
        let (mut owner, _events) = owner_without_process(std::env::temp_dir());
        owner.set_thinking("high".into());
        assert!(!owner.pending_session_controls.is_empty());
        owner.apply_response(SessionResponse::cancelled(
            "startup".into(),
            operation,
            "transport restarted".into(),
        ));
        assert!(owner.pending_session_controls.is_empty());
        assert_eq!(owner.active_snapshot().status, "Command not sent");
        assert!(!owner.active_snapshot().conversation.items.is_empty());
    }
}

#[test]
fn failed_effort_change_rejects_the_waiting_prompt() {
    let (mut owner, events) = owner_without_process(std::env::temp_dir());
    owner.deferred_prompt = Some(super::prompts::DeferredPrompt {
        mode: PromptMode::Normal,
        message: "first message".into(),
        display_message: None,
        invocation: None,
        images: Vec::new(),
        outbox_id: None,
    });
    owner.pending_prompt_target = Some("session:one".into());
    owner.pending_submission_id = Some("composer:one".into());
    owner
        .pending_session_controls
        .thinking_sent("effort-request".into(), Some("high".into()));

    owner.apply_response(SessionResponse::failure(
        Some("effort-request".into()),
        SessionOperation::SelectReasoning,
        "effort unavailable".into(),
    ));

    assert!(owner.deferred_prompt.is_none());
    assert!(owner.pending_prompt_target.is_none());
    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            submission_id: Some(id),
            outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
            ..
        } if id == "composer:one"
    )));
}

#[test]
fn stale_model_failure_does_not_reject_a_newer_startup_choice() {
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    owner.snapshot.pending_initial_model = true;
    owner.deferred_prompt = Some(super::prompts::DeferredPrompt {
        mode: PromptMode::Normal,
        message: "first message".into(),
        display_message: None,
        invocation: None,
        images: Vec::new(),
        outbox_id: None,
    });
    owner
        .pending_session_controls
        .model_sent("older".into(), ("openai".into(), "old-model".into()));
    owner
        .pending_session_controls
        .model_sent("newer".into(), ("openai".into(), "new-model".into()));

    owner.apply_response(SessionResponse::failure(
        Some("older".into()),
        SessionOperation::SelectModel,
        "old model unavailable".into(),
    ));

    assert!(owner.deferred_prompt.is_some());
    assert!(owner.snapshot.pending_initial_model);
    assert!(owner.pending_session_controls.model_pending());
}
