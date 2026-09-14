use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, prelude::FluentBuilder as _,
};

use crate::{
    agents::PeerMessage,
    app::ui::theme::THEME,
    conversation::{PendingReceipt, QueueState},
    protocol::PromptMode,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QueuedMessageKind {
    Peer,
    Steer,
    FollowUp,
}

impl QueuedMessageKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Peer => "Worker messages",
            Self::Steer => "Steer next",
            Self::FollowUp => "Follow-ups",
        }
    }
}

pub(super) fn queued_message_groups(queue: &QueueState) -> Vec<(QueuedMessageKind, Vec<&String>)> {
    let mut peers = Vec::new();
    let mut steering = Vec::new();
    let mut follow_up = Vec::new();
    for message in &queue.steering {
        if PeerMessage::from_prompt(message).is_some() {
            peers.push(message);
        } else {
            steering.push(message);
        }
    }
    for message in &queue.follow_up {
        if PeerMessage::from_prompt(message).is_some() {
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
        return "Queued message".to_owned();
    }
    match message.split_once(['\r', '\n']) {
        Some((first, _)) => format!("{}…", first.trim_end()),
        None => message.to_owned(),
    }
}

fn queued_message_group(
    kind: QueuedMessageKind,
    messages: &[&String],
    separated: bool,
) -> AnyElement {
    div()
        .when(separated, |group| {
            group
                .border_t(THEME.border)
                .border_color(THEME.colors.border)
        })
        .child(
            div()
                .px(THEME.space.sm)
                .py(THEME.space.xs)
                .bg(match kind {
                    QueuedMessageKind::Peer | QueuedMessageKind::Steer => THEME.colors.selection,
                    QueuedMessageKind::FollowUp => THEME.colors.hover,
                })
                .text_size(THEME.type_scale.caption)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(match kind {
                    QueuedMessageKind::Peer | QueuedMessageKind::Steer => THEME.colors.accent,
                    QueuedMessageKind::FollowUp => THEME.colors.subtle,
                })
                .child(kind.label()),
        )
        .children(messages.iter().map(|message| {
            div()
                .line_clamp(1)
                .border_t(THEME.border)
                .border_color(THEME.colors.border)
                .px(THEME.space.sm)
                .py(THEME.space.xs)
                .text_size(THEME.type_scale.body)
                .text_color(THEME.colors.text)
                .child(queued_message_preview(message))
        }))
        .into_any_element()
}

pub(super) fn render(queue: &QueueState) -> Option<AnyElement> {
    let groups = queued_message_groups(queue);
    if groups.is_empty() {
        return None;
    }
    Some(
        div()
            .mb(THEME.space.sm)
            .border(THEME.border)
            .border_color(THEME.colors.border)
            .rounded(THEME.radius)
            .overflow_hidden()
            .bg(THEME.colors.surface)
            .children(
                groups
                    .into_iter()
                    .enumerate()
                    .map(|(index, (kind, messages))| {
                        queued_message_group(kind, &messages, index > 0)
                    }),
            )
            .into_any_element(),
    )
}

pub(super) fn pending_receipt_label(receipt: &PendingReceipt) -> String {
    let mode = match receipt.mode {
        Some(PromptMode::Steer) => "Steer",
        Some(PromptMode::FollowUp) => "Follow-up",
        Some(PromptMode::Normal) | None => "Message",
    };
    let status = if receipt.unknown {
        "Delivery unknown"
    } else {
        "Awaiting delivery"
    };
    match receipt.images.len() {
        0 => format!("{mode} · {status}"),
        1 => format!("{mode} · {status} · 1 image"),
        count => format!("{mode} · {status} · {count} images"),
    }
}

pub(super) fn render_pending_receipts(receipts: &[PendingReceipt]) -> Option<AnyElement> {
    if receipts.is_empty() {
        return None;
    }
    Some(
        div()
            .mb(THEME.space.sm)
            .border(THEME.border)
            .border_color(THEME.colors.border)
            .rounded(THEME.radius)
            .overflow_hidden()
            .bg(THEME.colors.surface)
            .child(
                div()
                    .px(THEME.space.sm)
                    .py(THEME.space.xs)
                    .bg(THEME.colors.hover)
                    .text_size(THEME.type_scale.caption)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(THEME.colors.subtle)
                    .child("Saved pending messages"),
            )
            .children(receipts.iter().map(|receipt| {
                div()
                    .id(gpui::SharedString::from(format!(
                        "pending-receipt-{}",
                        receipt.id
                    )))
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
                    .px(THEME.space.sm)
                    .py(THEME.space.xs)
                    .child(
                        div()
                            .line_clamp(1)
                            .text_size(THEME.type_scale.body)
                            .text_color(THEME.colors.text)
                            .child(queued_message_preview(&receipt.text)),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(pending_receipt_label(receipt)),
                    )
            }))
            .into_any_element(),
    )
}
