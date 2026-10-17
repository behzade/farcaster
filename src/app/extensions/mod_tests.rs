use super::*;

fn questions(id: &str, blocking: bool) -> ExtensionUiRequest {
    ExtensionUiRequest::Questions(crate::agents::QuestionRequest {
        id: id.into(),
        blocking,
        questions: ["first", "second"]
            .into_iter()
            .map(|id| crate::agents::Question {
                id: id.into(),
                header: id.into(),
                prompt: "Which?".into(),
                options: vec![],
                allow_free_text: true,
                secret: false,
            })
            .collect(),
    })
}

#[test]
fn question_batches_keep_answers_across_blocking_prompt_and_submit_once() {
    let mut state = ExtensionUiState::default();
    state.apply(questions("async", false));
    assert!(!state.dialog.as_ref().unwrap().is_blocking());
    assert!(
        state
            .answer_question("async", "first", "one".into())
            .unwrap()
            .is_none()
    );
    state.apply(input("approval"));
    state.apply(input("second-approval"));
    assert_eq!(state.dialog.as_ref().unwrap().dialog_id(), Some("approval"));
    state.cancel("approval");
    assert_eq!(
        state.dialog.as_ref().unwrap().dialog_id(),
        Some("second-approval")
    );
    state.cancel("second-approval");
    assert_eq!(state.current_question().unwrap().id, "second");
    let Some(ExtensionUiResponse::Questions(response)) = state
        .answer_question("async", "second", "two".into())
        .unwrap()
    else {
        panic!("structured response");
    };
    assert_eq!(
        response.answers,
        BTreeMap::from([
            ("first".into(), vec!["one".into()]),
            ("second".into(), vec!["two".into()])
        ])
    );
    assert!(!response.cancelled);
    assert!(state.dialog.is_none());
    assert!(
        state
            .question_progress
            .values()
            .all(|progress| progress.answers.is_empty())
    );
}

#[test]
fn question_cancellation_and_resolution_discard_drafts_without_answering() {
    let mut state = ExtensionUiState::default();
    state.apply(questions("one", false));
    state.apply(questions("two", false));
    state.apply(questions("two", false));
    state
        .answer_question("one", "first", "draft".into())
        .unwrap();
    assert!(
        state
            .answer_question("one", "first", "stale double click".into())
            .is_err()
    );
    assert_eq!(state.current_question().unwrap().id, "second");
    assert_eq!(state.dismiss_dialog("one"), DialogDismissal::ActiveWithNext);
    assert!(
        state
            .question_progress
            .values()
            .all(|progress| progress.answers.is_empty())
    );
    let Some(ExtensionUiResponse::Questions(response)) = state.cancel("two") else {
        panic!("question cancellation");
    };
    assert!(response.cancelled);
    assert!(response.answers.is_empty());
    assert!(state.dialog.is_none(), "duplicate batch must not reappear");
}

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
    assert!(state.notifications.is_empty());
}

#[test]
fn expired_notifications_stay_in_the_history_until_they_are_seen() {
    let mut state = ExtensionUiState::default();
    state.push_notification(
        "workspace:Neovim".into(),
        "Neovim: request timed out".into(),
        NotifyTone::Error,
    );

    assert_eq!(state.unseen_notifications(), 1);
    assert_eq!(state.notification_history.len(), 1);
    assert_eq!(
        state.notification_history[0].message,
        "Neovim: request timed out"
    );
    assert_eq!(state.notification_history[0].tone, NotifyTone::Error);

    let notice = state.notifications[0].clone();
    assert!(state.remove_notification(&notice.id, notice.expires_at));

    assert!(state.notifications.is_empty());
    assert_eq!(state.notification_history.len(), 1);
    assert_eq!(state.unseen_notifications(), 1);

    state.mark_notifications_seen();
    assert_eq!(state.unseen_notifications(), 0);
    assert_eq!(state.notification_history.len(), 1);
}

#[test]
fn notification_history_is_bounded() {
    let mut state = ExtensionUiState::default();
    for index in 0..MAX_NOTIFICATION_HISTORY + 5 {
        state.push_notification(
            format!("notice-{index}"),
            format!("message {index}"),
            NotifyTone::Info,
        );
    }

    assert_eq!(state.notification_history.len(), MAX_NOTIFICATION_HISTORY);
    assert_eq!(state.notification_history[0].message, "message 5");
    assert_eq!(state.unseen_notifications(), MAX_NOTIFICATION_HISTORY + 5);
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

#[test]
fn question_drafts_stay_with_their_request_and_clear_on_answer_or_resolution() {
    let mut state = ExtensionUiState::default();
    state.apply(questions("one", false));
    state.apply(questions("two", false));
    let one = ("one".into(), "first".into());
    let two = ("two".into(), "first".into());
    state.remember_question_draft(&one, "First draft".into());
    state.remember_question_draft(&two, "Second draft".into());
    state.apply(input("approval"));
    state.cancel("approval");
    assert_eq!(state.question_draft(&one), "First draft");
    state
        .answer_question("one", "first", "Submitted".into())
        .unwrap();
    state.remember_question_draft(&one, "Stale input".into());
    assert!(state.question_draft(&one).is_empty());
    assert_eq!(state.question_draft(&two), "Second draft");
    state.dismiss_dialog("two");
    state.remember_question_draft(&two, "Stale dismissed input".into());
    assert!(
        state
            .question_progress
            .values()
            .all(|progress| progress.drafts.is_empty())
    );
}
