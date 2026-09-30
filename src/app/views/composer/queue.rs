use std::hash::{Hash as _, Hasher as _};

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};

use crate::app::{
    FarcasterApp,
    runtime::RuntimeCommand,
    ui::{
        assets::AppIcon,
        primitives::{
            AppIconSize, AppTooltip as _, ButtonTone, activates_button, app_icon, button,
            icon_button, icon_control, preserve_pointer_focus,
        },
    },
};
use crate::{
    agents::PeerMessage,
    app::ui::theme::theme,
    conversation::{PendingReceiptRef, QueueState},
    protocol::PromptMode,
};
use gpui::WeakEntity;

#[derive(Clone)]
pub(super) struct QueuedMessage<'a> {
    pub text: &'a String,
    pub id: Option<&'a String>,
    pub cancellable: bool,
    pub dismiss: bool,
    pub unknown: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QueuedMessageKind {
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

pub(super) fn queued_message_groups(
    queue: &QueueState,
) -> Vec<(QueuedMessageKind, Vec<QueuedMessage<'_>>)> {
    [
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
                    unknown: false,
                }
            })
            .collect();
        (kind, messages)
    })
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

#[derive(IntoElement)]
struct QueueMessageRow {
    id: SharedString,
    text: String,
    unknown: bool,
    action: Option<AnyElement>,
    expanded_content: Option<AnyElement>,
}

impl RenderOnce for QueueMessageRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let expanded = *state.read(cx);
        let click = state.clone();
        let preview = queued_message_preview(&self.text);
        let label = format!("Inspect message: {preview}");
        let selector = self.id.clone();
        div()
            .id(self.id.clone())
            .debug_selector(move || selector.to_string())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(theme().type_scale.caption)
            .text_color(theme().colors.muted)
            .child(
                div()
                    .min_h(theme().layout.status_row_height)
                    .flex()
                    .items_center()
                    .gap(theme().space.xs)
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-preview", self.id)))
                            .debug_selector({
                                let id = self.id.clone();
                                move || format!("{id}-preview")
                            })
                            .role(Role::Button)
                            .aria_label(label)
                            .aria_expanded(expanded)
                            .tab_index(0)
                            .flex_1()
                            .min_w_0()
                            .min_h(theme().layout.status_row_height)
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .hover(|row| row.text_color(theme().colors.text))
                            .focus_visible(|row| row.bg(theme().colors.highlight))
                            .on_mouse_down(MouseButton::Left, preserve_pointer_focus)
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                click.update(cx, |expanded, cx| {
                                    *expanded = !*expanded;
                                    cx.notify();
                                });
                            })
                            .on_key_down(move |event, _, cx| {
                                if activates_button(event) {
                                    cx.stop_propagation();
                                    state.update(cx, |expanded, cx| {
                                        *expanded = !*expanded;
                                        cx.notify();
                                    });
                                }
                            })
                            .child(div().flex_1().min_w_0().text_ellipsis().child(preview)),
                    )
                    .when(self.unknown, |row| {
                        row.child(
                            div()
                                .id(SharedString::from(format!("{}-warning", self.id)))
                                .debug_selector(|| "queue-delivery-warning".into())
                                .flex_none()
                                .text_color(theme().colors.warning)
                                .app_tooltip("Delivery not confirmed")
                                .child(app_icon(AppIcon::WarningCircle, AppIconSize::Inline)),
                        )
                    })
                    .children(self.action),
            )
            .when(expanded, |row| {
                row.child(self.expanded_content.unwrap_or_else(|| {
                    div()
                        .when(self.unknown, |body| {
                            body.child(
                                div()
                                    .text_color(theme().colors.warning)
                                    .child("Delivery not confirmed"),
                            )
                        })
                        .child(
                            div()
                                .id(SharedString::from(format!("{}-body", self.id)))
                                .debug_selector(|| "queue-message-body".into())
                                .max_h(theme().layout.tool_max_height)
                                .overflow_scroll()
                                .text_color(theme().colors.text)
                                .child(
                                    PeerMessage::from_prompt(&self.text)
                                        .map_or(self.text, |peer| peer.message),
                                ),
                        )
                        .into_any_element()
                }))
            })
    }
}

fn queued_message_row(
    message: &QueuedMessage<'_>,
    kind: QueuedMessageKind,
    index: usize,
    target: &str,
    session: Option<&std::path::Path>,
    entity: WeakEntity<FarcasterApp>,
    individual: bool,
) -> AnyElement {
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
        .filter(|_| individual && message.cancellable && (!message.dismiss || session.is_some()))
        .map(|id| {
            let id = id.clone();
            let target = target.to_owned();
            let dismiss = message.dismiss;
            let session = session.map(std::path::Path::to_path_buf);
            let action_id = SharedString::from(format!(
                "{}-{id}",
                if dismiss {
                    "dismiss-pending"
                } else {
                    "cancel-queue"
                }
            ));
            let remove = std::rc::Rc::new(move |cx: &mut App| {
                let _ = entity.update(cx, |app, cx| {
                    let command = if dismiss {
                        session
                            .clone()
                            .map(|session| RuntimeCommand::DismissReceipt {
                                session,
                                id: id.clone(),
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
            });
            let click = remove.clone();
            icon_control(
                action_id.clone(),
                if dismiss {
                    "Dismiss delivery notice"
                } else {
                    "Cancel queued message"
                },
            )
            .h(theme().layout.status_row_height)
            .debug_selector(move || action_id.to_string())
            .child(app_icon(AppIcon::X, AppIconSize::Inline))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                click(cx);
            })
            .on_key_down(move |event, _, cx| {
                if activates_button(event) {
                    cx.stop_propagation();
                    remove(cx);
                }
            })
            .into_any_element()
        });
    QueueMessageRow {
        id: format!("queue-row:{target}:{kind:?}:{identity}").into(),
        text: message.text.clone(),
        unknown: message.unknown,
        action,
        expanded_content: None,
    }
    .into_any_element()
}

pub(super) fn render(
    queue: &QueueState,
    receipts: &[PendingReceiptRef<'_>],
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
    let mut clear = (!individual && !groups.is_empty()).then(|| {
        let entity = entity.clone();
        icon_button(
            "clear-prompt-queue",
            AppIcon::Trash,
            "Clear all queued messages",
            ButtonTone::Quiet,
            move |_, cx| {
                let _ = entity.update(cx, |app, cx| app.send(RuntimeCommand::ClearQueue, cx));
            },
        )
        .into_any_element()
    });
    let recovered = queue
        .saved
        .iter()
        .map(|saved| {
            let send_entity = entity.clone();
            let remove_entity = entity.clone();
            let send_target = saved.target.clone();
            let remove_target = saved.target.clone();
            let id = saved.id;
            let content = div()
                .py(theme().space.xs)
                .child(
                    div()
                        .text_color(theme().colors.warning)
                        .child("Delivery unconfirmed. Sending again may duplicate this message."),
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
                .into_any_element();
            QueueMessageRow {
                id: format!("queue-recovered:{target}:{}", saved.id).into(),
                text: saved.text.clone(),
                unknown: true,
                action: None,
                expanded_content: Some(content),
            }
            .into_any_element()
        })
        .collect::<Vec<_>>();
    Some(
        div()
            .id("composer-queue-tray")
            .debug_selector(|| "composer-queue-tray".into())
            .w_full()
            .min_w_0()
            .mb(theme().space.xs)
            .px(theme().space.sm)
            .max_h(theme().layout.tool_max_height)
            .overflow_y_scroll()
            .children(recovered)
            .children(groups.into_iter().map(|(kind, messages)| {
                div()
                    .child(
                        div()
                            .id(format!("queue-group-{kind:?}"))
                            .debug_selector(move || format!("queue-group-{kind:?}"))
                            .flex()
                            .items_center()
                            .justify_between()
                            .py(theme().space.xs)
                            .text_size(theme().type_scale.caption)
                            .text_color(theme().colors.subtle)
                            .app_tooltip(kind.label())
                            .child(kind.heading())
                            .children(clear.take()),
                    )
                    .children(messages.iter().enumerate().map(|(index, message)| {
                        queued_message_row(
                            message,
                            kind,
                            index,
                            target,
                            session,
                            entity.clone(),
                            individual,
                        )
                    }))
            }))
            .into_any_element(),
    )
}

pub(super) fn pending_message_groups<'a>(
    queue: &'a QueueState,
    receipts: &[PendingReceiptRef<'a>],
    history_preview: bool,
) -> Vec<(QueuedMessageKind, Vec<QueuedMessage<'a>>)> {
    let mut groups = queued_message_groups(queue);
    for (_, messages) in &mut groups {
        for message in messages {
            message.unknown = receipts
                .iter()
                .any(|receipt| message.id == Some(receipt.id) && receipt.unknown);
        }
    }
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
