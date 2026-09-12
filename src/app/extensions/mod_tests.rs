use super::*;

fn input(id: &str) -> ExtensionUiRequest {
    ExtensionUiRequest::Input {
        id: id.into(),
        title: "Value".into(),
        placeholder: None,
        timeout: None,
    }
}

#[test]
fn expired_dialog_dismissal_preserves_other_questions_and_advances_fifo() {
    let mut state = ExtensionUiState::default();
    for id in ["active", "expired-queued", "next"] {
        state.apply(input(id));
    }
    assert_eq!(
        state.dismiss_dialog("expired-queued"),
        DialogDismissal::Queued
    );
    assert_eq!(
        state
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("active")
    );
    assert_eq!(state.dismiss_dialog("unrelated"), DialogDismissal::NotFound);
    assert_eq!(
        state.dismiss_dialog("active"),
        DialogDismissal::ActiveWithNext
    );
    assert_eq!(
        state
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("next")
    );
    assert_eq!(state.dismiss_dialog("next"), DialogDismissal::ActiveFinal);
    assert!(state.dialog.is_none());
    assert_eq!(state.dismiss_dialog("next"), DialogDismissal::NotFound);
}

#[test]
fn selected_dialogs_can_stay_visible_while_the_rest_are_parked() {
    let mut state = ExtensionUiState::default();
    for id in ["approval", "farcaster-recovery-7", "follow-up"] {
        state.apply(input(id));
    }
    let visible = state.take_dialogs_matching(|request| {
        request
            .dialog_id()
            .is_some_and(|id| id.starts_with("farcaster-recovery-"))
    });
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].dialog_id(), Some("farcaster-recovery-7"));
    assert_eq!(
        state
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("approval")
    );

    state.prepend_dialogs(visible);
    assert_eq!(
        state
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("farcaster-recovery-7")
    );
    assert_eq!(
        state.dismiss_dialog("farcaster-recovery-7"),
        DialogDismissal::ActiveWithNext
    );
    assert_eq!(
        state
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("approval")
    );
}

#[test]
fn rpc_capability_notices_are_not_user_facing_errors() {
    let mut state = ExtensionUiState::default();
    let effect = state.apply(ExtensionUiRequest::Notify {
        id: "notice".into(),
        message: "Theme switching not supported in RPC mode".into(),
        tone: NotifyTone::Error,
    });

    assert_eq!(effect, ExtensionEffect::None);
    assert!(state.notifications.is_empty());
}

#[test]
fn workspace_errors_are_transient_notifications() {
    let mut state = ExtensionUiState::default();
    state.push_notification(
        "workspace:Neovim".into(),
        "Neovim: request timed out".into(),
        NotifyTone::Error,
    );
    assert!(state.dialog.is_none());
    assert_eq!(state.notifications.len(), 1);
    let notice = state.notifications[0].clone();
    assert_eq!(notice.tone, NotifyTone::Error);
    assert!(state.remove_notification(&notice.id, notice.expires_at));
    assert_eq!(state, ExtensionUiState::default());
}

#[test]
fn notification_expiry_removes_only_the_matching_instance() {
    let mut state = ExtensionUiState::default();
    state.apply(ExtensionUiRequest::Notify {
        id: "notice".into(),
        message: "first".into(),
        tone: NotifyTone::Info,
    });
    let first_expiry = state.notifications[0].expires_at;
    state.apply(ExtensionUiRequest::Notify {
        id: "notice".into(),
        message: "replacement".into(),
        tone: NotifyTone::Info,
    });

    assert!(state.remove_notification("notice", first_expiry));
    assert_eq!(state.notifications.len(), 1);
    assert_eq!(state.notifications[0].message, "replacement");
    assert!(!state.remove_notification("notice", first_expiry));
}

#[test]
fn keyed_status_and_widget_set_clear_and_bound_content() {
    let mut state = ExtensionUiState::default();
    state.apply(ExtensionUiRequest::SetStatus {
        id: "1".into(),
        key: "x".into(),
        text: Some("busy".into()),
    });
    state.apply(ExtensionUiRequest::SetWidget {
        id: "2".into(),
        key: "x".into(),
        lines: Some(vec![
            "x".repeat(MAX_WIDGET_LINE_CHARS + 10);
            MAX_WIDGET_LINES + 10
        ]),
        placement: WidgetPlacement::AboveEditor,
    });
    assert_eq!(state.statuses.get("x").map(String::as_str), Some("busy"));
    let lines = &state.above_widgets["x"];
    assert_eq!(lines.len(), MAX_WIDGET_LINES);
    assert_eq!(lines[0].chars().count(), MAX_WIDGET_LINE_CHARS);
    state.apply(ExtensionUiRequest::SetStatus {
        id: "3".into(),
        key: "x".into(),
        text: None,
    });
    state.apply(ExtensionUiRequest::SetWidget {
        id: "4".into(),
        key: "x".into(),
        lines: None,
        placement: WidgetPlacement::AboveEditor,
    });
    assert!(state.statuses.is_empty());
    assert!(state.above_widgets.is_empty());
}

#[test]
fn dialog_requests_are_fifo_and_late_responses_are_ignored() {
    let mut state = ExtensionUiState::default();
    assert_eq!(state.apply(input("first")), ExtensionEffect::DialogOpened);
    assert_eq!(state.apply(input("second")), ExtensionEffect::None);
    assert!(state.respond_value("expired", "x".into()).is_none());
    assert!(state.respond_value("first", "x".into()).is_some());
    assert_eq!(
        state
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("second")
    );
    assert!(state.respond_value("first", "late".into()).is_none());
    assert!(state.cancel("second").is_some());
    assert!(state.dialog.is_none());
}

#[test]
fn reset_clears_every_session_owned_surface() {
    let mut state = ExtensionUiState::default();
    state.apply(input("dialog"));
    state.apply(ExtensionUiRequest::SetStatus {
        id: "s".into(),
        key: "k".into(),
        text: Some("busy".into()),
    });
    state.apply(ExtensionUiRequest::Notify {
        id: "n".into(),
        message: "bad".into(),
        tone: NotifyTone::Error,
    });
    state.reset();
    assert_eq!(state, ExtensionUiState::default());
}
