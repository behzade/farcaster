use super::*;
use crate::agents::Backend;

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
fn pending_reset_survives_coalescing_and_is_not_an_empty_queue() {
    let mut pending = PendingSessionControls::default();
    pending.set(SessionControl::Thinking(Some("high".into())));
    pending.set(SessionControl::Thinking(None));
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
