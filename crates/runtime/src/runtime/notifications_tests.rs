use super::*;

#[test]
fn ordinary_extension_notifications_do_not_request_desktop_attention() {
    for tone in [
        crate::protocol::NotifyTone::Info,
        crate::protocol::NotifyTone::Warning,
        crate::protocol::NotifyTone::Error,
    ] {
        let request = ExtensionUiRequest::Notify {
            id: "notice".into(),
            message: "MCP: 1 servers connected (5 tools)".into(),
            tone,
        };
        assert!(interaction_notification(&request, &[], None).is_none());
    }
}

#[test]
fn explicit_desktop_notification_keeps_its_title_body_and_target() {
    let request = ExtensionUiRequest::Notify {
        id: "attention".into(),
        message: "\u{1f}farcaster-notification\u{1f}Finished\u{1f}Ready for review".into(),
        tone: crate::protocol::NotifyTone::Info,
    };
    let target = Some((PathBuf::from("/session"), PathBuf::from("/project")));
    assert!(matches!(
        interaction_notification(&request, &[], target.clone()),
        Some(RuntimeEvent::SystemNotification { title, body, target: actual })
            if title == "Finished" && body == "Ready for review" && actual == target
    ));
    let (mut owner, events) = super::super::tests::owner_without_process(PathBuf::from("/project"));
    owner.apply_process_item(SessionEvent::Interaction(request.clone()));
    assert!(
        matches!(events.try_recv(), Ok(RuntimeEvent::ExtensionUi { request: actual, .. })
        if actual == request)
    );
    assert!(owner.snapshot.conversation.items.is_empty());
}

#[test]
fn informational_extension_events_update_transcript_without_a_toast() {
    let (mut owner, events) = super::super::tests::owner_without_process(PathBuf::from("/project"));
    let message = "MCP: 1 servers connected (5 tools)";
    let change = owner.apply_process_item(SessionEvent::Interaction(ExtensionUiRequest::Notify {
        id: "notice".into(),
        message: message.into(),
        tone: crate::protocol::NotifyTone::Info,
    }));
    assert!(matches!(change, SnapshotChange::Immediate));
    assert_eq!(owner.transcript_changed_from, Some(0));
    assert_eq!(owner.snapshot.conversation.items.len(), 1);
    assert_eq!(
        owner.snapshot.conversation.items[0].kind,
        TranscriptKind::Notice
    );
    assert_eq!(
        owner.snapshot.conversation.items[0].complete_text(),
        message
    );
    assert!(events.try_iter().next().is_none());
}

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
