use gpui::{
    AnyElement, Entity, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::text::TextViewState;

use crate::app::{
    FarcasterApp,
    ui::theme::THEME,
    views::transcript::conversation::{TranscriptItem, TranscriptKind},
};

use super::{
    TRANSCRIPT_HORIZONTAL_PADDING, disclosure_detail, fenced_text, selectable_text,
    selectable_text_state, technical_text, transcript_title_row,
};

pub(super) fn render_agent_message(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    markdown_state: Option<Entity<TextViewState>>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let (details, fallback) = match item.kind {
        TranscriptKind::PeerMessage => ("worker message", "Message received"),
        _ => ("subagent result", "Subagent finished"),
    };
    let summary = item
        .text
        .lines()
        .next()
        .filter(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .chars()
        .take(160)
        .collect::<String>();
    div()
        .id(("agent-result-row", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            transcript_title_row(
                ("agent-result-title", key),
                expanded,
                true,
                format!("{details} details for {}: {summary}", item.label),
                key,
                entity,
            )
            .child(
                div()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.muted)
                    .child(item.label.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.text)
                    .child(summary),
            ),
        )
        .when_some(markdown_state, |row, state| {
            row.child(
                disclosure_detail()
                    .id(("agent-result-detail-scroll", key))
                    .max_h(THEME.layout.tool_max_height)
                    .overflow_y_scroll()
                    .border_l(THEME.border)
                    .border_color(THEME.colors.accent)
                    .pl(THEME.space.sm)
                    .py(THEME.space.xs)
                    .child(selectable_text_state(&state).text_color(THEME.colors.muted)),
            )
        })
        .into_any_element()
}

pub(super) fn render_error(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let has_details = !item.tool_output.is_empty();
    div()
        .id(("error-row", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(THEME.space.sm)
        .flex()
        .flex_col()
        .child(
            transcript_title_row(
                ("error-title", key),
                expanded,
                has_details,
                format!("technical details for {}", item.label),
                key,
                entity,
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(THEME.colors.error)
                            .child(item.label.clone()),
                    )
                    .child(
                        selectable_text(("error-text", key), &item.text)
                            .text_color(THEME.colors.error),
                    ),
            ),
        )
        .when(expanded && has_details, |error| {
            error.child(
                disclosure_detail().child(
                    technical_text(("error-details", key), fenced_text(&item.tool_output))
                        .text_color(THEME.colors.muted),
                ),
            )
        })
        .into_any_element()
}

fn thinking_source(item: &TranscriptItem) -> &str {
    item.stream_chunks
        .first()
        .map_or(item.text.as_str(), |chunk| chunk.as_ref())
}

pub(in crate::app::views::transcript) fn thinking_preview(item: &TranscriptItem) -> &str {
    thinking_source(item).lines().next().unwrap_or("Thinking…")
}

pub(in crate::app::views::transcript) fn thinking_preview_emphasis(preview: &str) -> (&str, bool) {
    let trimmed = preview.trim();
    trimmed
        .strip_prefix("**")
        .and_then(|text| text.strip_suffix("**"))
        .filter(|text| !text.is_empty())
        .map_or((preview, false), |text| (text, true))
}

fn thinking_has_non_whitespace(text: &str) -> bool {
    text.chars().any(|character| !character.is_whitespace())
}

pub(in crate::app::views::transcript) fn thinking_has_details(item: &TranscriptItem) -> bool {
    if thinking_source(item)
        .split_once('\n')
        .is_some_and(|(_, rest)| thinking_has_non_whitespace(rest))
    {
        return true;
    }
    !item.stream_chunks.is_empty()
        && (item
            .stream_chunks
            .iter()
            .skip(1)
            .any(|chunk| thinking_has_non_whitespace(chunk))
            || thinking_has_non_whitespace(&item.text))
}

pub(super) fn render_thinking(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let has_details = thinking_has_details(item);
    let (preview, emphasized) = thinking_preview_emphasis(thinking_preview(item));
    let preview = preview.to_owned();
    div()
        .id(("thinking-row", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            transcript_title_row(
                ("thinking-title", key),
                expanded,
                has_details,
                "thinking details".into(),
                key,
                entity,
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .italic()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.subtle)
                    .when(emphasized, |preview| {
                        preview.font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(preview),
            ),
        )
        .when(expanded && has_details, |row| {
            let _timing = crate::app::infrastructure::performance::OperationTiming::new(
                crate::app::infrastructure::performance::OperationKind::ThinkingAssembly,
                item.stream_chunks.len(),
            );
            row.child(
                disclosure_detail().child(
                    selectable_text(("thinking-text", key), item.complete_text())
                        .italic()
                        .text_color(THEME.colors.subtle),
                ),
            )
        })
        .into_any_element()
}
