use super::*;
use crate::agents::{
    SessionResponse,
    extensions::{AgentMode, SlashCommand, SlashCommandSource},
};
use crate::app::runtime::tests::owner_without_process;

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
