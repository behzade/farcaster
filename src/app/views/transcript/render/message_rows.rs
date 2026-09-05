use gpui::{
    AnyElement, Entity, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    text::{TextViewState, TextViewStyle},
    tooltip::Tooltip,
};

use crate::app::{
    FarcasterApp,
    composer::prompt_fragments::invocation_token,
    ui::theme::THEME,
    views::transcript::{
        attachments::render_attachments,
        conversation::{TranscriptItem, TranscriptKind},
    },
};

use super::{
    TRANSCRIPT_HORIZONTAL_PADDING, item_color, selectable_text, selectable_text_state,
    technical_text,
};

pub(super) fn render_invocation(
    key: usize,
    item: &TranscriptItem,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let skill = invocation_kind(&item.text, invocation_resolution(item)) == "Skill";
    let tooltip = invocation_tooltip_text(item);
    div()
        .id(("invocation-row", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(THEME.space.sm)
        .flex()
        .flex_col()
        .when(item.has_attachments(), |row| {
            row.child(render_attachments(key, item, entity))
        })
        .child(
            technical_text(("invocation-name", key), item.text.clone())
                .min_w_0()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if skill {
                    THEME.colors.skill
                } else {
                    THEME.colors.accent
                }),
        )
        .when_some(tooltip, |row, tooltip| {
            row.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        })
        .into_any_element()
}

pub(in crate::app::views::transcript) fn invocation_kind(
    display: &str,
    resolved: &str,
) -> &'static str {
    let count = display
        .split_whitespace()
        .filter(|token| {
            token
                .strip_prefix('$')
                .is_some_and(|name| name.chars().any(|character| character.is_ascii_lowercase()))
        })
        .count();
    if count > 1 {
        "Stack"
    } else if resolved.contains("<skill name=") {
        "Skill"
    } else if resolved.is_empty() {
        "Invocation"
    } else {
        "Prompt"
    }
}

pub(super) fn invocation_resolution(item: &TranscriptItem) -> &str {
    item.invocation.as_deref().unwrap_or_default()
}

fn invocation_tooltip_text(item: &TranscriptItem) -> Option<String> {
    const MAX_PREVIEW_CHARS: usize = 320;

    let resolved = item.invocation.as_deref()?.trim();
    if resolved.is_empty() {
        return None;
    }
    let compact = resolved.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut characters = compact.chars();
    let mut preview = characters
        .by_ref()
        .take(MAX_PREVIEW_CHARS)
        .collect::<String>();
    if characters.next().is_some() {
        preview.push('…');
    }
    Some(format!(
        "{} expansion: {preview}",
        invocation_kind(&item.text, resolved)
    ))
}

pub(super) fn resolved_contains_skill(resolved: &str) -> bool {
    resolved.contains("<skill name=")
}

pub(in crate::app::views::transcript) fn is_mixed_invocation_message(
    display: &str,
    resolved: &str,
) -> bool {
    display.split_whitespace().any(|token| {
        if resolved_contains_skill(resolved) {
            !is_invocation_token(token)
        } else {
            !is_prompt_invocation_token(token)
        }
    })
}

fn is_invocation_token(token: &str) -> bool {
    invocation_token(token)
        .strip_prefix('$')
        .is_some_and(is_invocation_name)
}

fn is_prompt_invocation_token(token: &str) -> bool {
    invocation_token(token)
        .strip_prefix('$')
        .is_some_and(|name| {
            is_invocation_name(name) && name.chars().any(|character| character.is_ascii_lowercase())
        })
}

fn is_invocation_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, ':' | '_' | '-')
        })
}

fn skill_names(resolved: &str) -> Vec<&str> {
    resolved
        .match_indices("<skill name=\"")
        .filter_map(|(start, marker)| {
            let name = &resolved[start + marker.len()..];
            name.split_once('"').map(|(name, _)| name)
        })
        .collect()
}

pub(in crate::app::views::transcript) fn highlighted_invocation_markdown(
    display: &str,
    resolved: &str,
) -> String {
    let skill_names = skill_names(resolved);
    display
        .split_inclusive(char::is_whitespace)
        .map(|chunk| {
            let token = chunk.trim_end_matches(char::is_whitespace);
            let invocation = invocation_token(token);
            let suffix = &chunk[invocation.len()..];
            let name = invocation.strip_prefix('$').unwrap_or_default();
            let bare_name = name.strip_prefix("skill:").unwrap_or(name);
            let recognized = if resolved_contains_skill(resolved) {
                skill_names.contains(&bare_name)
            } else {
                is_prompt_invocation_token(token)
            };
            if recognized {
                format!("`{invocation}`{suffix}")
            } else {
                chunk.to_owned()
            }
        })
        .collect()
}

pub(super) fn render_message(
    key: usize,
    item: &TranscriptItem,
    follows_tool: bool,
    markdown_state: Option<Entity<TextViewState>>,
    markdown_style: Option<TextViewStyle>,
    assistant_label: &str,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let user = item.kind == TranscriptKind::User;
    let role = item_role_label(item, assistant_label);
    let tooltip = invocation_tooltip_text(item);
    div()
        .id(("transcript-row", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(THEME.space.sm)
        .when(user, |row| {
            row.mt(THEME.space.sm)
                .py(THEME.space.md)
                .bg(THEME.colors.selection)
        })
        .when(follows_tool, |row| {
            row.mt(THEME.space.md).pt(THEME.space.sm)
        })
        .when_some(tooltip, |row, tooltip| {
            row.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        })
        .children(role.map(|role| message_role(role, user)))
        .when(user && item.has_attachments(), |row| {
            row.child(render_attachments(key, item, entity))
        })
        .child({
            let text = markdown_state.map_or_else(
                || selectable_text(("transcript-text", key), &item.text),
                |state| selectable_text_state(&state),
            );
            let text = match markdown_style {
                Some(style) => text.style(style),
                None => text,
            };
            text.text_color(item_color(item))
                .when(user, |text| text.font_weight(FontWeight::MEDIUM))
        })
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_message_chunk(
    key: usize,
    block: usize,
    item: &TranscriptItem,
    first: bool,
    last: bool,
    follows_tool: bool,
    markdown_state: Entity<TextViewState>,
    assistant_label: &str,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let user = item.kind == TranscriptKind::User;
    div()
        .id(format!("transcript-row-{key}-{block}"))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .when(user, |row| row.bg(THEME.colors.selection))
        .when(first, |row| row.pt(THEME.space.sm))
        .when(first && user, |row| {
            row.mt(THEME.space.sm).pt(THEME.space.md)
        })
        .when(first && follows_tool, |row| {
            row.mt(THEME.space.md).pt(THEME.space.sm)
        })
        .when(!first, |row| row.pt(THEME.space.xs))
        .when(last, |row| row.pb(THEME.space.md))
        .when(first, |row| {
            row.children(
                item_role_label(item, assistant_label).map(|role| message_role(role, user)),
            )
        })
        .when(first && user && item.has_attachments(), |row| {
            row.child(render_attachments(key, item, entity))
        })
        .child(
            selectable_text_state(&markdown_state)
                .text_color(item_color(item))
                .when(user, |text| text.font_weight(FontWeight::MEDIUM)),
        )
        .into_any_element()
}

pub(in crate::app::views::transcript) fn message_role_label(
    kind: TranscriptKind,
    assistant_label: &str,
) -> Option<&str> {
    match kind {
        TranscriptKind::User => Some("You"),
        TranscriptKind::Assistant => Some(assistant_label),
        TranscriptKind::Thinking
        | TranscriptKind::Tool
        | TranscriptKind::Error
        | TranscriptKind::Notice
        | TranscriptKind::Custom
        | TranscriptKind::AgentResult
        | TranscriptKind::PeerMessage => None,
    }
}

fn item_role_label<'a>(item: &'a TranscriptItem, assistant_label: &'a str) -> Option<&'a str> {
    if item.kind == TranscriptKind::PeerMessage {
        Some(item.label.as_str())
    } else {
        message_role_label(item.kind, assistant_label)
    }
}

fn message_role(label: &str, user: bool) -> impl gpui::IntoElement {
    div()
        .mb(px(7.0))
        .text_size(THEME.type_scale.caption)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if user {
            THEME.colors.accent
        } else {
            THEME.colors.muted
        })
        .child(label.to_owned())
}
