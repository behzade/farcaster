use std::{
    hash::{Hash as _, Hasher as _},
    path::Path,
    rc::Rc,
};

use crate::app::{FarcasterApp, runtime::RuntimeCommand};
use crate::{
    agents::PeerMessage,
    conversation::{PendingReceiptRef, QueueState},
    protocol::PromptMode,
};

pub(crate) struct QueuePresentation {
    pub groups: Vec<QueueGroup>,
    pub clear: Option<QueueAction>,
}

pub(crate) struct QueueGroup {
    pub id: String,
    pub heading: Option<(&'static str, &'static str)>,
    pub rows: Vec<QueueRow>,
}

pub(crate) struct QueueRow {
    pub id: String,
    pub preview: String,
    pub body: String,
    pub notice: Option<&'static str>,
    pub action: Option<QueueAction>,
    pub details: Option<QueueDetails>,
}

pub(crate) struct QueueDetails {
    pub notice: &'static str,
    pub caption: Option<String>,
    pub actions: Vec<QueueAction>,
}

#[derive(Clone)]
pub(crate) struct QueueAction {
    pub id: String,
    pub label: &'static str,
    pub enabled: bool,
    command: RuntimeCommand,
}

impl QueueAction {
    fn new(id: String, label: &'static str, enabled: bool, command: RuntimeCommand) -> Self {
        Self {
            id,
            label,
            enabled,
            command,
        }
    }

    fn runtime_command(&self) -> Option<RuntimeCommand> {
        self.enabled.then(|| self.command.clone())
    }
}

pub(crate) type QueueActionHandler = Rc<dyn Fn(&QueueAction, &mut gpui::App)>;

impl FarcasterApp {
    fn queue_snapshot_buffer(&self) -> Option<&str> {
        use crate::sessions::{composer_buffer_parts, draft_id, session_path};

        if self.awaiting_selection_snapshot() {
            return None;
        }
        let buffer = self.composer.sessions.current_buffer();
        let selected = self.snapshot.selected_session.as_deref();
        let matches = if let Some((harness, address)) = composer_buffer_parts(buffer) {
            self.snapshot.harness == Some(harness) && selected == session_path(address)
        } else if let Some(path) = session_path(buffer) {
            selected == Some(path) && !self.sessions.all.has_shared_path(path)
        } else {
            let id = draft_id(buffer)?;
            self.sessions.selected_draft.as_deref() == Some(id)
                && self.sidebar_session_path() == selected
                && self.snapshot.project == self.project.path
        };
        matches.then_some(buffer)
    }

    pub(crate) fn composer_queue(
        &self,
        entity: gpui::WeakEntity<FarcasterApp>,
    ) -> Option<(QueuePresentation, QueueActionHandler)> {
        let buffer = self.queue_snapshot_buffer()?;
        let queue = super::submissions::visible_prompt_queue(
            self.snapshot.prompt_queue(),
            &self.composer.pending_submissions,
            buffer,
        );
        let presentation = prepare(
            &queue,
            &self.snapshot.conversation.pending_receipts_ref(),
            self.composer.sessions.current_target(),
            self.snapshot.selected_session.as_deref(),
            crate::agents::supports_individual_queue_cancellation(self.snapshot.harness),
            self.snapshot.history_preview,
        );
        if presentation.groups.is_empty() {
            return None;
        }
        let buffer = buffer.to_owned();
        let generation = self.runtime_generation;
        let on_action: QueueActionHandler = Rc::new(move |action, cx| {
            let _ = entity.update(cx, |app, cx| {
                if app.runtime_generation == generation
                    && app.queue_snapshot_buffer() == Some(buffer.as_str())
                    && let Some(command) = action.runtime_command()
                {
                    app.send(command, cx);
                }
            });
        });
        Some((presentation, on_action))
    }
}

pub(crate) fn prepare(
    queue: &QueueState,
    receipts: &[PendingReceiptRef<'_>],
    target: &str,
    session: Option<&Path>,
    individual: bool,
    history_preview: bool,
) -> QueuePresentation {
    let pending = pending_message_groups(queue, receipts, history_preview);
    let clear = (!individual && !pending.is_empty()).then(|| {
        QueueAction::new(
            "clear-prompt-queue".into(),
            "Clear all queued messages",
            true,
            RuntimeCommand::ClearQueue,
        )
    });
    let mut groups = Vec::new();
    if !queue.saved.is_empty() {
        groups.push(QueueGroup {
            id: "queue-recovered".into(),
            heading: None,
            rows: queue
                .saved
                .iter()
                .map(|saved| saved_row(saved, target))
                .collect(),
        });
    }
    groups.extend(pending.into_iter().map(|(kind, messages)| {
        QueueGroup {
            id: format!("queue-group-{kind:?}"),
            heading: Some((kind.heading(), kind.label())),
            rows: messages
                .into_iter()
                .enumerate()
                .map(|(index, message)| {
                    pending_row(message, kind, index, target, session, individual)
                })
                .collect(),
        }
    }));
    QueuePresentation { groups, clear }
}

fn pending_row(
    message: QueuedMessage<'_>,
    kind: QueuedMessageKind,
    index: usize,
    target: &str,
    session: Option<&Path>,
    individual: bool,
) -> QueueRow {
    let identity = message.id.filter(|id| !id.is_empty()).map_or_else(
        || {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            message.text.hash(&mut hash);
            format!("{index}:{:x}", hash.finish())
        },
        |id| id.to_owned(),
    );
    let action = message
        .id
        .filter(|_| individual && message.cancellable)
        .and_then(|id| {
            Some(if message.dismiss {
                QueueAction::new(
                    format!("dismiss-pending-{id}"),
                    "Dismiss delivery notice",
                    true,
                    RuntimeCommand::DismissReceipt {
                        session: session?.to_owned(),
                        id: id.clone(),
                    },
                )
            } else {
                QueueAction::new(
                    format!("cancel-queue-{id}"),
                    "Cancel queued message",
                    true,
                    RuntimeCommand::CancelQueued {
                        target: target.to_owned(),
                        id: id.clone(),
                    },
                )
            })
        });
    let (preview, body) = display_text(message.text);
    QueueRow {
        id: format!("queue-row:{target}:{kind:?}:{identity}"),
        preview,
        body,
        notice: message.unknown.then_some("Delivery not confirmed"),
        action,
        details: None,
    }
}

fn saved_row(saved: &crate::conversation::SavedPrompt, target: &str) -> QueueRow {
    let (preview, body) = display_text(&saved.text);
    QueueRow {
        id: format!("queue-recovered:{target}:{}", saved.id),
        preview,
        body,
        notice: Some("Delivery not confirmed"),
        action: None,
        details: Some(QueueDetails {
            notice: "Delivery unconfirmed. Sending again may duplicate this message.",
            caption: (saved.image_count > 0)
                .then(|| format!("{} image(s) attached", saved.image_count)),
            actions: vec![
                QueueAction::new(
                    format!("send-saved-{}", saved.id),
                    "Send again",
                    saved.sendable,
                    RuntimeCommand::SendSaved {
                        target: saved.target.clone(),
                        id: saved.id,
                    },
                ),
                QueueAction::new(
                    format!("remove-saved-{}", saved.id),
                    "Remove",
                    true,
                    RuntimeCommand::RemoveSaved {
                        target: saved.target.clone(),
                        id: saved.id,
                    },
                ),
            ],
        }),
    }
}

fn display_text(message: &str) -> (String, String) {
    let (from, message) = PeerMessage::prompt_parts(message)
        .map_or((None, message.trim()), |(from, message)| {
            (Some(from), message.trim_end())
        });
    let decorate = |text: &str| match from {
        Some(from) if text.is_empty() => format!("{from}:"),
        Some(from) => format!("{from}: {text}"),
        None if text.is_empty() => "Message".to_owned(),
        None => text.to_owned(),
    };
    let body = decorate(message);
    let preview = match message.split_once(['\r', '\n']) {
        Some((first, _)) => format!("{}…", decorate(first.trim_end())),
        None => body.clone(),
    };
    (preview, body)
}

struct QueuedMessage<'a> {
    text: &'a String,
    id: Option<&'a String>,
    cancellable: bool,
    dismiss: bool,
    unknown: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueuedMessageKind {
    Steer,
    FollowUp,
}

impl QueuedMessageKind {
    fn heading(self) -> &'static str {
        match self {
            Self::Steer => "Steer",
            Self::FollowUp => "Follow-up",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Steer => "For the current turn",
            Self::FollowUp => "For the next turn",
        }
    }
}

fn pending_message_groups<'a>(
    queue: &'a QueueState,
    receipts: &[PendingReceiptRef<'a>],
    history_preview: bool,
) -> Vec<(QueuedMessageKind, Vec<QueuedMessage<'a>>)> {
    let mut groups: Vec<_> = [
        (
            QueuedMessageKind::Steer,
            &queue.steering,
            &queue.steering_ids,
        ),
        (
            QueuedMessageKind::FollowUp,
            &queue.follow_up,
            &queue.follow_up_ids,
        ),
    ]
    .into_iter()
    .filter(|(_, texts, _)| !texts.is_empty())
    .map(|(kind, texts, ids)| {
        let messages = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let id = ids.get(index);
                QueuedMessage {
                    text,
                    id,
                    cancellable: id.is_some_and(|id| queue.can_cancel(id)),
                    dismiss: false,
                    unknown: receipts
                        .iter()
                        .any(|receipt| id == Some(receipt.id) && receipt.unknown),
                }
            })
            .collect::<Vec<_>>();
        (kind, messages)
    })
    .collect();
    if !history_preview {
        return groups;
    }
    for receipt in receipts {
        if groups.iter().any(|(_, messages)| {
            messages
                .iter()
                .any(|message| message.id == Some(receipt.id))
        }) {
            continue;
        }
        let kind = if receipt.mode == Some(PromptMode::Steer) {
            QueuedMessageKind::Steer
        } else {
            QueuedMessageKind::FollowUp
        };
        let index = groups
            .iter()
            .position(|(candidate, _)| *candidate == kind)
            .unwrap_or_else(|| {
                groups.push((kind, Vec::new()));
                groups.len() - 1
            });
        let messages = &mut groups[index].1;
        messages.push(QueuedMessage {
            text: receipt.text,
            id: Some(receipt.id),
            cancellable: true,
            dismiss: true,
            unknown: receipt.unknown,
        });
    }
    groups
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
