use super::*;
use crate::agents::Backend;

struct IdleTransport;
impl crate::agents::SessionTransport for IdleTransport {
    fn send(&mut self, _: SessionCommand) -> Result<String, String> {
        panic!("unexpected session command")
    }
    fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self) -> Option<crate::agents::SessionEvent> {
        None
    }
    fn close(&mut self) -> Result<(), String> {
        panic!("unexpected session restart")
    }
}

struct AckTransport;
impl crate::agents::SessionTransport for AckTransport {
    fn send(&mut self, _: SessionCommand) -> Result<String, String> {
        Ok("refresh".into())
    }
    fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self) -> Option<crate::agents::SessionEvent> {
        None
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn pending_controls_coalesce_and_apply_model_before_effort() {
    let mut pending = PendingSessionControls::default();
    pending.set(SessionControl::Thinking(Some("low".into())));
    pending.set(SessionControl::ServiceTier("standard".into()));
    pending.set(SessionControl::Model(
        "old-provider".into(),
        "old-model".into(),
    ));
    assert!(
        pending
            .replace_if_pending(SessionControl::Thinking(Some("high".into())))
            .is_none()
    );
    pending.set(SessionControl::ServiceTier("priority".into()));
    assert!(
        pending
            .replace_if_pending(SessionControl::Model(
                "new-provider".into(),
                "new-model".into(),
            ))
            .is_none()
    );

    let requests = pending
        .take()
        .into_iter()
        .map(SessionControl::into_request)
        .collect::<Vec<_>>();

    assert_eq!(
        requests,
        vec![
            SessionCommand::SelectModel {
                provider: "new-provider".into(),
                model_id: "new-model".into(),
            },
            SessionCommand::SelectReasoning {
                level: "high".into(),
            },
            SessionCommand::SelectServiceTier {
                tier: "priority".into()
            },
        ]
    );
}

#[test]
fn launch_only_tier_does_not_queue_a_second_live_change() {
    let mut pending = PendingSessionControls::default();
    pending.set(SessionControl::Model("openai".into(), "gpt-6-sol".into()));
    pending.set(SessionControl::ServiceTier("fast".into()));
    pending.launched_service_tier("fast");
    assert!(!pending.service_tier_pending());
    assert_eq!(
        pending
            .take()
            .into_iter()
            .map(SessionControl::into_request)
            .collect::<Vec<_>>(),
        [SessionCommand::SelectModel {
            provider: "openai".into(),
            model_id: "gpt-6-sol".into(),
        }]
    );
}

#[test]
fn tier_restart_requeues_unconfirmed_effort_instead_of_the_old_effort() {
    let mut pending = PendingSessionControls::default();
    pending.thinking_sent("effort-request".into(), Some("high".into()));
    assert!(pending.thinking_pending());
    pending.set(SessionControl::ServiceTier("fast".into()));
    pending.reset_transport();
    assert_eq!(pending.thinking, Some(Some("high".into())));
    assert!(pending.selection_pending());
}

#[test]
fn acknowledged_effort_is_visible_before_the_followup_state_query() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.process = Some(Box::new(AckTransport));
    owner.snapshot.session = Some(
        serde_json::from_value(serde_json::json!({
            "thinkingLevel":"low","isStreaming":false,"isCompacting":false,
            "sessionId":"session","autoCompactionEnabled":true,
            "messageCount":0,"pendingMessageCount":0
        }))
        .expect("decode state"),
    );
    owner
        .pending_session_controls
        .thinking_sent("effort-request".into(), Some("high".into()));

    owner.apply_response(crate::agents::SessionResponse::success(
        Some("effort-request".into()),
        crate::agents::SessionResponsePayload::SelectReasoning,
    ));

    assert_eq!(owner.snapshot.session_identity().effort, Some("high"));
    assert!(!owner.pending_session_controls.thinking_pending());
}

#[test]
fn tier_restart_reapplies_confirmed_model_and_effort() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::Codex);
    owner.snapshot.harness = Some(Backend::Codex);
    owner.snapshot.session = Some(
        serde_json::from_value(serde_json::json!({
            "model":{"id":"selected","name":"Selected","provider":"openai"},
            "thinkingLevel":"high","isStreaming":false,"isCompacting":false,
            "sessionId":"session","autoCompactionEnabled":true,
            "messageCount":0,"pendingMessageCount":0
        }))
        .expect("decode selected state"),
    );
    owner.process = Some(Box::new(IdleTransport));

    owner.queue_launch_only_service_tier("fast".into());

    assert_eq!(
        owner.pending_session_controls.model.as_ref(),
        Some(&("openai".into(), "selected".into()))
    );
    assert_eq!(
        owner.pending_session_controls.thinking,
        Some(Some("high".into()))
    );
    assert_eq!(owner.snapshot.selected_service_tier(), Some("fast"));
}

#[test]
fn tier_only_resume_replaces_history_preview_after_startup() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.snapshot.history_preview = true;
    owner.snapshot.selected_session = Some("/saved".into());
    owner.active_session = Some("/saved".into());
    owner.parked_snapshot = Some(RuntimeSnapshot {
        selected_session: Some("/saved".into()),
        connected: true,
        ..RuntimeSnapshot::default()
    });
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.pending_session_controls.restore_preview = true;

    owner.maybe_send_pending_session_controls();

    assert!(!owner.snapshot.history_preview);
    assert!(owner.snapshot.connected);
    assert!(owner.parked_snapshot.is_none());
}

#[test]
fn model_reselection_during_history_resume_updates_the_loading_snapshot() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::Codex);
    owner.snapshot.harness = Some(Backend::Codex);
    owner.snapshot.history_preview = true;
    owner.snapshot.selected_session = Some("/saved".into());
    owner.active_session = Some("/saved".into());
    owner.snapshot.models = ["b", "c"]
        .map(|id| {
            serde_json::from_value(serde_json::json!({
                "id":id,"name":id,"provider":"openai",
                "serviceTiers":["standard","fast"]
            }))
            .expect("decode model")
        })
        .to_vec();
    let first = owner.snapshot.models[0].clone();
    let mut later = owner.snapshot.models[1].clone();
    later.service_tiers = vec!["standard".into()];
    owner.snapshot.models[1] = later.clone();
    owner.snapshot.prefill_model = Some(first.clone());
    owner.snapshot.pending_initial_model = true;
    owner.snapshot.prefill_service_tier = Some("fast".into());
    owner.snapshot.pending_initial_service_tier = true;
    owner.parked_snapshot = Some(RuntimeSnapshot {
        selected_session: Some("/saved".into()),
        models: owner.snapshot.models.clone(),
        prefill_model: Some(first.clone()),
        pending_initial_model: true,
        prefill_service_tier: Some("fast".into()),
        pending_initial_service_tier: true,
        ..RuntimeSnapshot::default()
    });
    owner.process = Some(Box::new(AckTransport));
    owner
        .pending_session_controls
        .set(SessionControl::Model(first.provider, first.id));

    owner.set_model(later);
    assert_eq!(owner.snapshot.selected_service_tier(), None);
    assert_eq!(
        owner
            .parked_snapshot
            .as_ref()
            .and_then(RuntimeSnapshot::selected_service_tier),
        None
    );
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.pending_session_controls.restore_preview = true;
    owner.maybe_send_pending_session_controls();

    assert!(!owner.snapshot.history_preview);
    assert_eq!(owner.snapshot.selected_service_tier(), None);
    assert_eq!(
        owner
            .snapshot
            .session_identity()
            .model
            .map(|model| model.id.as_str()),
        Some("c")
    );
    assert_eq!(
        owner
            .pending_session_controls
            .sent_model
            .as_ref()
            .map(|(_, id)| id.as_str()),
        Some("c")
    );
}

#[test]
fn replacing_model_during_startup_updates_the_requested_identity() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::Codex);
    owner.snapshot.harness = Some(Backend::Codex);
    owner.snapshot.models = ["a", "b"]
        .map(|id| {
            serde_json::from_value(serde_json::json!({
                "id":id,"name":id,"provider":"openai","serviceTiers":["standard","fast"]
            }))
            .expect("decode model")
        })
        .to_vec();
    owner.process = Some(Box::new(AckTransport));
    owner.set_model(owner.snapshot.models[0].clone());
    owner.set_model(owner.snapshot.models[1].clone());
    assert_eq!(
        owner
            .snapshot
            .session_identity()
            .model
            .map(|m| m.id.as_str()),
        Some("b")
    );
    assert!(owner.snapshot.pending_initial_model);

    owner.pending_session_controls = Default::default();
    owner.apply_response(crate::agents::SessionResponse::success(
        None,
        crate::agents::SessionResponsePayload::LoadState(Box::new(
            serde_json::from_value(serde_json::json!({
                "model":{"id":"b","name":"b","provider":"openai"},
                "isStreaming":false,"isCompacting":false,"sessionId":"session",
                "autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0
            }))
            .expect("decode state"),
        )),
    ));
    assert!(!owner.snapshot.pending_initial_model);
    assert_eq!(
        owner
            .snapshot
            .session_identity()
            .model
            .map(|m| m.id.as_str()),
        Some("b")
    );
}

#[test]
fn launch_only_tier_change_waits_for_an_active_reply() {
    for backend in [Backend::Codex, Backend::Claude] {
        let (mut owner, _) =
            super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
        owner.harness = Some(backend);
        owner.snapshot.models = vec![
            serde_json::from_value(serde_json::json!({
                "id":"model", "name":"Model", "provider":backend.as_str(),
                "serviceTiers":["standard", "fast"]
            }))
            .expect("decode model"),
        ];
        owner.snapshot.prefill_model = owner.snapshot.models.first().cloned();
        owner.active_session = Some("/session".into());
        owner.process = Some(Box::new(IdleTransport));
        conversation_mut(owner.active_snapshot_mut()).running = true;

        owner.set_service_tier("fast".into());

        assert!(owner.process.is_some());
        assert!(!owner.pending_session_controls.service_tier_pending());
    }
}

#[test]
fn codex_draft_can_choose_tier_before_choosing_a_model() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::Codex);
    owner.snapshot.models = vec![
        serde_json::from_value(serde_json::json!({
            "id":"default", "name":"Default", "provider":"openai",
            "serviceTiers":["standard", "fast"]
        }))
        .expect("decode default model"),
    ];

    owner.set_service_tier("fast".into());

    assert!(owner.process.is_none());
    assert!(owner.snapshot.prefill_model.is_none());
    assert_eq!(owner.snapshot.selected_service_tier(), Some("fast"));
    assert_eq!(
        owner.pending_session_controls.service_tier.as_deref(),
        Some("fast")
    );
}

#[test]
fn pending_reset_survives_coalescing_and_is_not_an_empty_queue() {
    let mut pending = PendingSessionControls::default();
    pending.set(SessionControl::Thinking(Some("high".into())));
    pending.set(SessionControl::Thinking(None));
    pending.restore_selection(None, Some("high"));
    assert!(!pending.is_empty());
    assert_eq!(
        pending
            .take()
            .into_iter()
            .map(SessionControl::into_request)
            .collect::<Vec<_>>(),
        [SessionCommand::ResetReasoning]
    );
    assert!(pending.is_empty());
    pending.set(SessionControl::Thinking(None));
    pending.set(SessionControl::Thinking(Some("low".into())));
    assert_eq!(
        pending
            .take()
            .into_iter()
            .map(SessionControl::into_request)
            .collect::<Vec<_>>(),
        [SessionCommand::SelectReasoning {
            level: "low".into()
        }]
    );
    assert!(SessionControl::Thinking(None).supported_by(Some(Backend::OpenCode)));
    assert!(!SessionControl::Thinking(None).supported_by(Some(Backend::Pi)));
}

#[test]
fn resetting_a_draft_clears_prefill_and_queues_an_explicit_reset() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::OpenCode);
    owner.set_thinking("high".into());
    assert_eq!(owner.snapshot.session_identity().effort, Some("high"));
    owner.reset_thinking();
    assert_eq!(owner.snapshot.session_identity().effort, None);
    assert_eq!(
        owner
            .pending_session_controls
            .take()
            .into_iter()
            .map(SessionControl::into_request)
            .collect::<Vec<_>>(),
        [SessionCommand::ResetReasoning]
    );
}

#[test]
fn cursor_draft_keeps_model_and_tier_through_first_start() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::Cursor);
    let selected: Model = serde_json::from_value(serde_json::json!({
        "id":"chosen", "name":"Chosen", "provider":"cursor",
        "serviceTiers":["standard", "priority"]
    }))
    .expect("decode selected model");
    owner.snapshot.models = vec![selected.clone()];

    owner.set_model(selected.clone());
    owner.set_service_tier("priority".into());
    assert_eq!(owner.snapshot.session_identity().model, Some(&selected));
    assert_eq!(owner.snapshot.selected_service_tier(), Some("priority"));
    assert!(owner.process.is_none());
    assert_eq!(
        owner.pending_session_controls.model.as_ref(),
        Some(&("cursor".into(), "chosen".into()))
    );
    assert_eq!(
        owner.pending_session_controls.service_tier.as_deref(),
        Some("priority")
    );

    // The missing test agent fails after startup resets the snapshot. The
    // draft choice must remain visible and ready for a retry.
    owner.start_process(None);
    assert_eq!(owner.snapshot.session_identity().model, Some(&selected));
    assert_eq!(owner.snapshot.selected_service_tier(), Some("priority"));
    assert!(owner.snapshot.pending_initial_model);
    assert!(owner.snapshot.pending_initial_service_tier);
    assert_eq!(
        owner
            .pending_session_controls
            .take()
            .into_iter()
            .map(SessionControl::into_request)
            .collect::<Vec<_>>(),
        [
            SessionCommand::SelectModel {
                provider: "cursor".into(),
                model_id: "chosen".into(),
            },
            SessionCommand::SelectServiceTier {
                tier: "priority".into(),
            },
        ]
    );
}

#[test]
fn cursor_draft_model_change_clears_an_unsupported_tier() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::Cursor);
    let fast: Model = serde_json::from_value(serde_json::json!({
        "id":"fast", "name":"Fast", "provider":"cursor",
        "serviceTiers":["standard", "priority"]
    }))
    .expect("decode fast model");
    let plain: Model = serde_json::from_value(serde_json::json!({
        "id":"plain", "name":"Plain", "provider":"cursor"
    }))
    .expect("decode plain model");
    owner.snapshot.models = vec![fast.clone(), plain.clone()];
    owner.set_model(fast);
    owner.set_service_tier("priority".into());
    owner.set_model(plain);

    assert!(owner.snapshot.available_service_tiers().is_empty());
    assert_eq!(owner.snapshot.selected_service_tier(), None);
    assert_eq!(
        owner
            .pending_session_controls
            .take()
            .into_iter()
            .map(SessionControl::into_request)
            .collect::<Vec<_>>(),
        [SessionCommand::SelectModel {
            provider: "cursor".into(),
            model_id: "plain".into(),
        }]
    );
}

#[test]
fn live_model_change_clears_an_unsupported_tier_and_queued_control() {
    let (mut owner, events) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::Cursor);
    owner.snapshot.harness = Some(Backend::Cursor);
    owner.active_session = Some("/session".into());
    owner.snapshot.selected_session = owner.active_session.clone();
    owner.process = Some(Box::new(AckTransport));
    let old: Model = serde_json::from_value(serde_json::json!({
        "id":"old", "name":"Old", "provider":"cursor",
        "serviceTiers":["standard", "priority"]
    }))
    .expect("decode old model");
    let plain: Model = serde_json::from_value(serde_json::json!({
        "id":"plain", "name":"Plain", "provider":"cursor",
        "serviceTiers":["standard"]
    }))
    .expect("decode plain model");
    owner.snapshot.models = vec![old, plain.clone()];
    owner.snapshot.session = Some(
        serde_json::from_value(serde_json::json!({
            "model":{"id":"old","name":"Old","provider":"cursor"},
            "serviceTier":"priority","sessionId":"session",
            "isStreaming":false,"isCompacting":false,
            "autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0
        }))
        .expect("decode live state"),
    );
    let previous_state = owner.snapshot.session.clone().expect("live state");
    owner.snapshot.prefill_service_tier = Some("priority".into());
    owner
        .pending_session_controls
        .set(SessionControl::ServiceTier("priority".into()));

    owner.set_model(plain);

    assert_eq!(owner.snapshot.selected_service_tier(), None);
    assert_eq!(owner.snapshot.prefill_service_tier, None);
    assert_eq!(
        owner
            .snapshot
            .session
            .as_ref()
            .and_then(|s| s.service_tier.as_deref()),
        None
    );
    assert!(!owner.pending_session_controls.service_tier_pending());
    assert!(owner.pending_session_controls.model_pending());
    owner.publish_session_metadata();
    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::SessionMetadata(metadata) if metadata.service_tier.is_none()
    )));

    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.maybe_send_pending_session_controls();
    owner.apply_response(crate::agents::SessionResponse::failure(
        Some("refresh".into()),
        SessionOperation::SelectModel,
        "model unavailable".into(),
    ));
    owner.apply_response(crate::agents::SessionResponse::success(
        None,
        crate::agents::SessionResponsePayload::LoadState(Box::new(previous_state)),
    ));
    assert_eq!(owner.snapshot.selected_service_tier(), Some("priority"));
}

#[test]
fn live_model_change_keeps_a_supported_tier_and_rejects_an_invalid_access_choice() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::Claude);
    owner.snapshot.harness = Some(Backend::Claude);
    owner.process = Some(Box::new(AckTransport));
    owner.snapshot.session = Some(
        serde_json::from_value(serde_json::json!({
            "model":{"id":"old","name":"Old","provider":"claude"},
            "serviceTier":"priority","sessionId":"session",
            "isStreaming":false,"isCompacting":false,
            "autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0
        }))
        .expect("decode live state"),
    );
    owner.snapshot.prefill_service_tier = Some("priority".into());
    let supported: Model = serde_json::from_value(serde_json::json!({
        "id":"supported", "name":"Supported", "provider":"claude",
        "serviceTiers":["standard", "priority"],
        "access_modes":["sandboxed", "full"]
    }))
    .expect("decode supported model");
    let limited: Model = serde_json::from_value(serde_json::json!({
        "id":"limited", "name":"Limited", "provider":"claude",
        "serviceTiers":["standard"],
        "access_modes":["full"]
    }))
    .expect("decode limited model");

    owner.set_model(supported);
    assert_eq!(owner.snapshot.selected_service_tier(), Some("priority"));
    owner.set_model_with_access_mode(limited, HarnessAccessMode::Sandboxed);

    assert_eq!(owner.snapshot.selected_service_tier(), Some("priority"));
    assert_eq!(
        owner.snapshot.prefill_service_tier.as_deref(),
        Some("priority")
    );
    assert_eq!(
        owner
            .snapshot
            .session
            .as_ref()
            .and_then(|s| s.service_tier.as_deref()),
        Some("priority")
    );

    owner.snapshot.prefill_service_tier = Some("standard".into());
    owner
        .snapshot
        .session
        .as_mut()
        .expect("live state")
        .service_tier = Some("standard".into());
    let default_tier: Model = serde_json::from_value(serde_json::json!({
        "id":"default", "name":"Default", "provider":"claude"
    }))
    .expect("decode default model");
    owner.set_model(default_tier);
    assert_eq!(owner.snapshot.selected_service_tier(), Some("standard"));
}

#[test]
fn queued_access_mode_model_change_clears_an_unsupported_tier() {
    let (mut owner, _) =
        super::super::tests::owner_without_process(std::path::PathBuf::from("/project"));
    owner.harness = Some(Backend::Claude);
    owner.snapshot.harness = Some(Backend::Claude);
    owner.process_command.access_mode = HarnessAccessMode::Auto;
    owner.process = Some(Box::new(IdleTransport));
    owner.snapshot.session = Some(
        serde_json::from_value(serde_json::json!({
            "model":{"id":"old","name":"Old","provider":"claude"},
            "serviceTier":"priority","sessionId":"session",
            "isStreaming":false,"isCompacting":false,
            "autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0
        }))
        .expect("decode live state"),
    );
    owner.snapshot.prefill_service_tier = Some("priority".into());
    owner
        .pending_session_controls
        .tier_sent("old-tier".into(), "priority".into());
    owner
        .pending_session_controls
        .set(SessionControl::ServiceTier("standard".into()));
    let model: Model = serde_json::from_value(serde_json::json!({
        "id":"limited", "name":"Limited", "provider":"claude",
        "serviceTiers":["standard"],
        "access_modes":["sandboxed", "full"]
    }))
    .expect("decode limited model");

    owner.set_model(model);

    assert_eq!(owner.snapshot.selected_service_tier(), None);
    assert_eq!(owner.pending_session_controls.sent_tier, None);
    owner.pending_session_controls.reset_transport();
    assert_eq!(
        owner.pending_session_controls.service_tier.as_deref(),
        Some("standard")
    );
    assert!(owner.pending_session_controls.model_pending());
    assert_eq!(owner.snapshot.access_mode, HarnessAccessMode::Sandboxed);
}
