use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};

use crate::app::{
    FarcasterApp,
    runtime::RuntimeCommand,
    ui::primitives::{ButtonTone, button, icon_button},
};
use crate::{
    agents::PeerMessage,
    app::ui::theme::theme,
    conversation::{PendingReceipt, QueueState},
    protocol::PromptMode,
};
use gpui::WeakEntity;
use gpui_component::IconName;

#[derive(Clone)]
pub(super) struct QueuedMessage<'a> {
    pub text: &'a String,
    pub id: Option<&'a String>,
    pub cancellable: bool,
    pub dismiss: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QueuedMessageKind {
    Peer,
    Steer,
    FollowUp,
}

impl QueuedMessageKind {
    fn border_color(self) -> gpui::Rgba {
        match self {
            Self::Peer => theme().colors.border,
            Self::Steer => theme().colors.accent.opacity(0.45),
            Self::FollowUp => theme().colors.subtle.opacity(0.45),
        }
    }
}

pub(super) fn queued_message_groups(
    queue: &QueueState,
) -> Vec<(QueuedMessageKind, Vec<QueuedMessage<'_>>)> {
    let mut peers = Vec::new();
    let mut steering = Vec::new();
    let mut follow_up = Vec::new();
    for (index, text) in queue.steering.iter().enumerate() {
        let message = QueuedMessage {
            text,
            id: queue.steering_ids.get(index),
            cancellable: queue
                .steering_ids
                .get(index)
                .is_some_and(|id| queue.can_cancel(id)),
            dismiss: false,
        };
        if PeerMessage::from_prompt(text).is_some() {
            peers.push(message);
        } else {
            steering.push(message);
        }
    }
    for (index, text) in queue.follow_up.iter().enumerate() {
        let message = QueuedMessage {
            text,
            id: queue.follow_up_ids.get(index),
            cancellable: queue
                .follow_up_ids
                .get(index)
                .is_some_and(|id| queue.can_cancel(id)),
            dismiss: false,
        };
        if PeerMessage::from_prompt(text).is_some() {
            peers.push(message);
        } else {
            follow_up.push(message);
        }
    }
    [
        (QueuedMessageKind::Peer, peers),
        (QueuedMessageKind::Steer, steering),
        (QueuedMessageKind::FollowUp, follow_up),
    ]
    .into_iter()
    .filter(|(_, messages)| !messages.is_empty())
    .collect()
}

pub(super) fn queued_message_preview(message: &str) -> String {
    let message = PeerMessage::from_prompt(message).map_or_else(
        || message.to_owned(),
        |peer| format!("{}: {}", peer.from, peer.message),
    );
    let message = message.trim();
    if message.is_empty() {
        return "Message".to_owned();
    }
    match message.split_once(['\r', '\n']) {
        Some((first, _)) => format!("{}…", first.trim_end()),
        None => message.to_owned(),
    }
}

pub(super) fn saved_prompt_body(id: i64, text: &str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(format!("saved-prompt-body-{id}"))
        .w_full()
        .min_w_0()
        .max_h(theme().layout.tool_max_height)
        .overflow_scroll()
        .text_color(theme().colors.text)
        .child(text.to_owned())
}

fn queued_message_group(
    kind: QueuedMessageKind,
    messages: &[QueuedMessage<'_>],
    separated: bool,
    target: &str,
    session: Option<&std::path::Path>,
    entity: WeakEntity<FarcasterApp>,
    individual: bool,
) -> AnyElement {
    div()
        .when(separated, |group| {
            group
                .border_t(theme().border)
                .border_color(theme().colors.border)
        })
        .children(messages.iter().map(|message| {
            div()
                .flex()
                .items_center()
                .gap(theme().space.xs)
                .border_t(theme().border)
                .border_color(kind.border_color())
                .border_l(px(2.0))
                .px(theme().space.sm)
                .py(theme().space.xs)
                .text_size(theme().type_scale.body)
                .text_color(theme().colors.text)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .line_clamp(1)
                        .child(queued_message_preview(message.text)),
                )
                .when_some(
                    message.id.filter(|_| {
                        individual && message.cancellable && (!message.dismiss || session.is_some())
                    }),
                    |row, id| {
                        let id = id.clone();
                        let target = target.to_owned();
                        let entity = entity.clone();
                        let dismiss = message.dismiss;
                        let session = session.map(std::path::Path::to_path_buf);
                        row.child(icon_button(
                            gpui::SharedString::from(format!(
                                "{}-{id}",
                                if dismiss {
                                    "dismiss-pending"
                                } else {
                                    "cancel-queue"
                                }
                            )),
                            IconName::Close,
                            "Remove pending message",
                            ButtonTone::Quiet,
                            move |_, cx| {
                                let _ = entity.update(cx, |app, cx| {
                                    let command = if dismiss {
                                        session.clone().map(|session| {
                                            RuntimeCommand::DismissReceipt {
                                                session,
                                                id: id.clone(),
                                            }
                                        })
                                    } else {
                                        Some(RuntimeCommand::CancelQueued {
                                            target: target.clone(),
                                            id: id.clone(),
                                        })
                                    };
                                    if let Some(command) = command {
                                        app.send(command, cx);
                                    }
                                });
                            },
                        ))
                    },
                )
        }))
        .into_any_element()
}

pub(super) fn render(
    queue: &QueueState,
    receipts: &[PendingReceipt],
    target: &str,
    session: Option<&std::path::Path>,
    entity: WeakEntity<FarcasterApp>,
    individual: bool,
    history_preview: bool,
) -> Option<AnyElement> {
    let groups = pending_message_groups(queue, receipts, history_preview);
    if groups.is_empty() && queue.saved.is_empty() {
        return None;
    }
    Some(
        div()
            .mb(theme().space.sm)
            .border(theme().border)
            .border_color(theme().colors.border)
            .rounded(theme().radius)
            .overflow_hidden()
            .bg(theme().colors.surface)
            .when(!individual && !groups.is_empty(), |queue| {
                let entity = entity.clone();
                queue.child(div().flex().justify_end().child(button(
                    "clear-prompt-queue",
                    "Clear pending",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        let _ =
                            entity.update(cx, |app, cx| app.send(RuntimeCommand::ClearQueue, cx));
                    },
                )))
            })
            .children(queue.saved.iter().map(|saved| {
                let send_entity = entity.clone();
                let remove_entity = entity.clone();
                let send_target = saved.target.clone();
                let remove_target = saved.target.clone();
                let id = saved.id;
                div()
                    .border_t(theme().border)
                    .border_color(theme().colors.border)
                    .px(theme().space.sm)
                    .py(theme().space.xs)
                    .child(
                        div().text_color(theme().colors.subtle).child(
                            "Delivery unconfirmed. Sending again may duplicate this message.",
                        ),
                    )
                    .child(saved_prompt_body(saved.id, &saved.text))
                    .when(saved.image_count > 0, |row| {
                        row.child(format!("{} image(s) attached", saved.image_count))
                    })
                    .child(
                        div()
                            .flex()
                            .gap(theme().space.xs)
                            .child(button(
                                format!("send-saved-{}", saved.id),
                                "Send again",
                                ButtonTone::Quiet,
                                saved.sendable,
                                move |_, cx| {
                                    let _ = send_entity.update(cx, |app, cx| {
                                        app.send(
                                            RuntimeCommand::SendSaved {
                                                target: send_target.clone(),
                                                id,
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ))
                            .child(button(
                                format!("remove-saved-{}", saved.id),
                                "Remove",
                                ButtonTone::Quiet,
                                true,
                                move |_, cx| {
                                    let _ = remove_entity.update(cx, |app, cx| {
                                        app.send(
                                            RuntimeCommand::RemoveSaved {
                                                target: remove_target.clone(),
                                                id,
                                            },
                                            cx,
                                        );
                                    });
                                },
                            )),
                    )
            }))
            .children(
                groups
                    .into_iter()
                    .enumerate()
                    .map(|(index, (kind, messages))| {
                        queued_message_group(
                            kind,
                            &messages,
                            index > 0,
                            target,
                            session,
                            entity.clone(),
                            individual,
                        )
                    }),
            )
            .into_any_element(),
    )
}

pub(super) fn pending_message_groups<'a>(
    queue: &'a QueueState,
    receipts: &'a [PendingReceipt],
    history_preview: bool,
) -> Vec<(QueuedMessageKind, Vec<QueuedMessage<'a>>)> {
    let mut groups = queued_message_groups(queue);
    // Live submissions already come from the queue/composer projection. Receipt
    // history restores rows only when viewing a saved session, not a second copy
    // of each live submission.
    if !history_preview {
        return groups;
    }
    for receipt in receipts {
        if groups.iter().any(|(_, messages)| {
            messages
                .iter()
                .any(|message| message.id == Some(&receipt.id))
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
            text: &receipt.text,
            id: Some(&receipt.id),
            cancellable: true,
            dismiss: true,
        });
    }
    groups
}
