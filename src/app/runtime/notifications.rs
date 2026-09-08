use super::*;
use crate::app::views::transcript::conversation::TranscriptKind;

impl RuntimeOwner {
    pub(super) fn notify_attention(&self, title: &str, detail: Option<&str>) {
        let snapshot = self.active_snapshot();
        let _ = self.event_tx.send(RuntimeEvent::SystemNotification {
            title: format!("Farcaster: {title}"),
            body: detail
                .map(str::to_owned)
                .or_else(|| completion_text(&snapshot.conversation))
                .or_else(|| {
                    snapshot
                        .session
                        .as_ref()
                        .and_then(|session| session.session_name.clone())
                })
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

fn completion_text(conversation: &ConversationState) -> Option<String> {
    conversation
        .items
        .iter_rev()
        .take_while(|item| item.kind != TranscriptKind::User)
        .filter(|item| matches!(item.kind, TranscriptKind::Assistant | TranscriptKind::Error))
        .find_map(|item| {
            let text = item.complete_text();
            (!text.trim().is_empty()).then(|| text.trim().to_owned())
        })
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
        ExtensionUiRequest::Select { title, options, .. } => (
            "Farcaster: Input needed",
            request_text(title, &options.join("\n")),
        ),
        ExtensionUiRequest::Confirm { title, message, .. } => {
            ("Farcaster: Input needed", request_text(title, message))
        }
        ExtensionUiRequest::Input {
            title,
            placeholder: detail,
            ..
        }
        | ExtensionUiRequest::Editor {
            title,
            prefill: detail,
            ..
        } => (
            "Farcaster: Input needed",
            request_text(title, detail.as_deref().unwrap_or_default()),
        ),
        ExtensionUiRequest::Notify { message, .. } => {
            let (title, body) = request
                .gpui_system_notification()
                .unwrap_or(("Farcaster", message));
            (title, body.to_owned())
        }
        _ => return None,
    };
    Some(RuntimeEvent::SystemNotification {
        title: title.into(),
        body,
        target,
    })
}

fn request_text(title: &str, detail: &str) -> String {
    if detail.trim().is_empty() || detail.trim() == title.trim() {
        title.to_owned()
    } else {
        format!("{title}\n{detail}")
    }
}

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
