use super::*;

impl RuntimeOwner {
    pub(super) fn notify_attention(&self, title: &str) {
        let snapshot = self.active_snapshot();
        let _ = self.event_tx.send(RuntimeEvent::SystemNotification {
            title: format!("Farcaster: {title}"),
            body: snapshot
                .session
                .as_ref()
                .and_then(|session| session.session_name.clone())
                .unwrap_or_else(|| snapshot.project.display().to_string()),
            target: self.attention_target(),
        });
    }

    pub(super) fn attention_target(&self) -> Option<(PathBuf, PathBuf)> {
        self.active_session
            .clone()
            .map(|path| (path, self.active_snapshot().project.clone()))
            .or_else(|| notification_target(self.active_snapshot()))
    }
}

pub(super) fn interaction_notification(
    request: &ExtensionUiRequest,
    active_dialogs: &[ExtensionUiRequest],
    target: Option<(PathBuf, PathBuf)>,
) -> Option<RuntimeEvent> {
    if request.dialog_id().is_some_and(|id| {
        active_dialogs
            .iter()
            .any(|dialog| dialog.dialog_id() == Some(id))
    }) {
        return None;
    }
    let (title, body) = match request {
        ExtensionUiRequest::Select { title, .. }
        | ExtensionUiRequest::Confirm { title, .. }
        | ExtensionUiRequest::Input { title, .. }
        | ExtensionUiRequest::Editor { title, .. } => ("Farcaster: Input needed", title.as_str()),
        ExtensionUiRequest::Notify { message, .. } => request
            .gpui_system_notification()
            .unwrap_or(("Farcaster", message)),
        _ => return None,
    };
    Some(RuntimeEvent::SystemNotification {
        title: title.into(),
        body: body.into(),
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Some(RuntimeEvent::SystemNotification { target: actual, .. }) if actual == target)
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
}
