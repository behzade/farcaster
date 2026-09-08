use super::*;

#[test]
fn completion_uses_full_reply_from_current_turn() {
    let mut conversation = ConversationState::default();
    let user = conversation.push_local_user("Fix it".into(), 0, false);
    let mut reply = (*user).clone();
    reply.kind = TranscriptKind::Assistant;
    reply.stream_chunks = Arc::new(vec![Arc::from("Fixed the bug. ")]);
    reply.text = "Checks passed.".into();
    conversation.items.push(Arc::new(reply));
    assert_eq!(
        completion_text(&conversation).as_deref(),
        Some("Fixed the bug. Checks passed.")
    );
    conversation.push_local_user("Next task".into(), 0, false);
    assert_eq!(completion_text(&conversation), None);
}

#[test]
fn pending_dialogs_notify_once_without_suppressing_other_requests() {
    let request = ExtensionUiRequest::Confirm {
        id: "permission".into(),
        title: "Allow command?".into(),
        message: "Command".into(),
        timeout: None,
    };
    let target = Some((PathBuf::from("/session"), PathBuf::from("/project")));
    assert!(
        matches!(interaction_notification(&request, &[], target.clone()),
            Some(RuntimeEvent::SystemNotification { target: actual, body, .. })
                if actual == target && body == "Allow command?\nCommand")
    );
    assert!(interaction_notification(&request, std::slice::from_ref(&request), None).is_none());
    let other = ExtensionUiRequest::Input {
        id: "question".into(),
        title: "Which branch?".into(),
        placeholder: None,
        timeout: None,
    };
    assert!(interaction_notification(&other, &[request], None).is_some());
}
