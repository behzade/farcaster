use super::*;

#[test]
fn pending_controls_coalesce_and_apply_model_before_effort() {
    let mut pending = PendingSessionControls::default();
    pending.set(SessionControl::Thinking("low".into()));
    pending.set(SessionControl::Model(
        "old-provider".into(),
        "old-model".into(),
    ));
    pending.set(SessionControl::Thinking("high".into()));
    pending.set(SessionControl::Model(
        "new-provider".into(),
        "new-model".into(),
    ));

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
        ]
    );
}
