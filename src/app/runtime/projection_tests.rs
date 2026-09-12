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
