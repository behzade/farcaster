use gpui::{
    AnyElement, FontWeight, IntoElement as _, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _,
};

use crate::{
    agents::PeerMessage, app::ui::theme::THEME, app::views::transcript::conversation::QueueState,
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
    match message.split_once(['\r', '\n']) {
        Some((first, _)) => format!("{}…", first.trim_end()),
        None => message,
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
