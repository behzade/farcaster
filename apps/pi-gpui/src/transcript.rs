//! Selectable, compact transcript projection.

use std::sync::Arc;

use gpui::{
    AnyElement, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement as _,
    ListSizingBehavior, ListState, Overflow, ParentElement as _, Pixels, StyleRefinement,
    Styled as _, WeakEntity, div, list, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    highlighter::HighlightTheme,
    text::{TextView, TextViewStyle},
};

use crate::{
    app::PiApp,
    conversation::{TranscriptItem, TranscriptKind},
    primitives::{ButtonTone, button},
    theme::{READING_FONT_FAMILY, THEME},
};

const MARKDOWN_CHUNK_TARGET_BYTES: usize = 2 * 1024;
const MARKDOWN_CHUNK_HARD_BYTES: usize = 8 * 1024;

pub(crate) fn tail_reserve(viewport_height: Pixels) -> Pixels {
    px((f32::from(viewport_height) * 0.32).clamp(72.0, 280.0))
}

#[derive(Clone, Copy)]
pub(crate) struct TranscriptViewport {
    pub(crate) following: bool,
    pub(crate) unseen: usize,
    pub(crate) tail_reserve: Pixels,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptRow {
    Item {
        index: usize,
        revision: usize,
    },
    MessageChunk {
        index: usize,
        start: usize,
        end: usize,
        block: usize,
        revision: usize,
        first: bool,
        last: bool,
    },
    ReadGroup {
        start: usize,
        len: usize,
        revision: usize,
    },
}

impl TranscriptRow {
    pub(crate) fn key(&self) -> usize {
        self.item_start()
    }

    fn item_start(&self) -> usize {
        match self {
            Self::Item { index, .. } | Self::MessageChunk { index, .. } => *index,
            Self::ReadGroup { start, .. } => *start,
        }
    }

    fn item_end(&self) -> usize {
        match self {
            Self::Item { index, .. } | Self::MessageChunk { index, .. } => index + 1,
            Self::ReadGroup { start, len, .. } => start + len,
        }
    }
}

pub(crate) fn project_rows(items: &[Arc<TranscriptItem>]) -> Vec<TranscriptRow> {
    project_rows_from(items, 0)
}

pub(crate) fn update_rows(
    previous_rows: &[TranscriptRow],
    previous_items: &[Arc<TranscriptItem>],
    items: &[Arc<TranscriptItem>],
) -> Vec<TranscriptRow> {
    let unchanged_items = previous_items
        .iter()
        .zip(items)
        .take_while(|(previous, next)| Arc::ptr_eq(previous, next))
        .count();
    if unchanged_items == previous_items.len()
        && unchanged_items == items.len()
        && (items.is_empty() || !previous_rows.is_empty())
    {
        return previous_rows.to_vec();
    }

    let mut keep_rows = previous_rows
        .iter()
        .take_while(|row| row.item_end() <= unchanged_items)
        .count();
    let mut project_from = previous_rows
        .get(keep_rows)
        .map_or(unchanged_items, TranscriptRow::item_start);
    if project_from == unchanged_items
        && unchanged_items < items.len()
        && is_read(&items[unchanged_items])
        && let Some(TranscriptRow::ReadGroup { start, len, .. }) = keep_rows
            .checked_sub(1)
            .and_then(|index| previous_rows.get(index))
        && start + len == unchanged_items
    {
        keep_rows -= 1;
        project_from = *start;
    }

    let mut rows = previous_rows[..keep_rows].to_vec();
    rows.extend(project_rows_from(items, project_from));
    rows
}

fn project_rows_from(items: &[Arc<TranscriptItem>], mut index: usize) -> Vec<TranscriptRow> {
    let mut rows = Vec::new();
    while index < items.len() {
        if is_read(&items[index]) {
            let start = index;
            while index < items.len() && is_read(&items[index]) {
                index += 1;
            }
            rows.push(TranscriptRow::ReadGroup {
                start,
                len: index - start,
                revision: item_revision(&items[start..index]),
            });
            continue;
        }
        if items[index].kind == TranscriptKind::Assistant
            && items[index].text.len() > MARKDOWN_CHUNK_HARD_BYTES
        {
            let chunks = markdown_chunk_ranges(&items[index].text);
            let last_block = chunks.len().saturating_sub(1);
            rows.extend(chunks.into_iter().enumerate().map(|(block, (start, end))| {
                TranscriptRow::MessageChunk {
                    index,
                    start,
                    end,
                    block,
                    revision: item_revision(std::slice::from_ref(&items[index])),
                    first: block == 0,
                    last: block == last_block,
                }
            }));
        } else {
            rows.push(TranscriptRow::Item {
                index,
                revision: item_revision(std::slice::from_ref(&items[index])),
            });
        }
        index += 1;
    }
    rows
}

fn item_revision(items: &[Arc<TranscriptItem>]) -> usize {
    items.iter().fold(0, |revision, item| {
        revision.rotate_left(5) ^ Arc::as_ptr(item) as usize
    })
}

fn is_read(item: &TranscriptItem) -> bool {
    item.kind == TranscriptKind::Tool && item.label == "Read"
}

fn markdown_chunk_ranges(text: &str) -> Vec<(usize, usize)> {
    if text.len() <= MARKDOWN_CHUNK_HARD_BYTES {
        return vec![(0, text.len())];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut end = 0;
    let mut fence = None;
    let mut protected_chunk = false;
    for line in text.split_inclusive('\n') {
        end += line.len();
        let mut closed_fence = false;
        if let Some(marker) = markdown_fence_marker(line) {
            if fence == Some(marker) {
                fence = None;
                closed_fence = true;
            } else if fence.is_none() {
                fence = Some(marker);
                protected_chunk = true;
            }
        }
        if closed_fence {
            chunks.push((start, end));
            start = end;
            protected_chunk = false;
            continue;
        }
        while fence.is_none() && !protected_chunk && end - start >= MARKDOWN_CHUNK_HARD_BYTES {
            let split = hard_markdown_break(text, start, start + MARKDOWN_CHUNK_HARD_BYTES);
            chunks.push((start, split));
            start = split;
        }
        let preferred_break = end - start >= MARKDOWN_CHUNK_TARGET_BYTES && line.trim().is_empty();
        if fence.is_none() && (preferred_break || end - start >= MARKDOWN_CHUNK_HARD_BYTES) {
            chunks.push((start, end));
            start = end;
            protected_chunk = false;
        }
    }
    if start < text.len() {
        chunks.push((start, text.len()));
    }
    if chunks.is_empty() {
        chunks.push((0, text.len()));
    }
    chunks
}

fn hard_markdown_break(text: &str, start: usize, mut limit: usize) -> usize {
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    let minimum = start + MARKDOWN_CHUNK_TARGET_BYTES;
    text[start..limit]
        .char_indices()
        .rev()
        .find(|(offset, char)| start + offset >= minimum && char.is_whitespace())
        .map_or(limit, |(offset, char)| start + offset + char.len_utf8())
}

fn markdown_fence_marker(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    ((marker == '`' || marker == '~')
        && trimmed.chars().take_while(|char| *char == marker).count() >= 3)
        .then_some(marker)
}

fn expanded_by_default(row: TranscriptRow, items: &[Arc<TranscriptItem>]) -> bool {
    matches!(
        row,
        TranscriptRow::Item { index, .. } if items[index].tool_presentation.is_some()
    )
}

pub(crate) fn render(
    list_state: &ListState,
    viewport: TranscriptViewport,
    rows: std::sync::Arc<Vec<TranscriptRow>>,
    snapshot: std::sync::Arc<crate::runtime::RuntimeSnapshot>,
    disclosure_overrides: std::collections::HashSet<usize>,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    if rows.is_empty() {
        return div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(THEME.colors.canvas)
            .child(
                div()
                    .text_size(THEME.type_scale.body)
                    .text_color(THEME.colors.subtle)
                    .child("Ask Pi to start a session"),
            )
            .into_any_element();
    }

    let jump = entity.clone();
    let row_entity = entity;
    let view = list(list_state.clone(), move |index, _, _| {
        let Some(row) = rows.get(index).copied() else {
            return div().into_any_element();
        };
        let expanded = expanded_by_default(row, &snapshot.conversation.items)
            != disclosure_overrides.contains(&row.key());
        let reserves_tail = index + 1 == rows.len()
            && latest_allows_tail_reserve(row, &snapshot.conversation.items, expanded);
        div()
            .w_full()
            .flex()
            .justify_center()
            .when(reserves_tail, |row| row.pb(viewport.tail_reserve))
            .child(
                div()
                    .w_full()
                    .max_w(THEME.layout.transcript_max)
                    .child(render_row(
                        row,
                        &snapshot.conversation.items,
                        expanded,
                        row_entity.clone(),
                    )),
            )
            .into_any_element()
    })
    .with_sizing_behavior(ListSizingBehavior::Auto)
    .w_full()
    .flex_grow_1();

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_hidden()
                .flex()
                .justify_center()
                .bg(THEME.colors.canvas)
                .child(view),
        )
        .when(!viewport.following, |root| {
            root.child(
                div()
                    .flex_none()
                    .flex()
                    .justify_center()
                    .bg(THEME.colors.canvas)
                    .py(THEME.space.xs)
                    .child(button(
                        "jump-to-latest",
                        if viewport.unseen == 0 {
                            "Jump to latest".to_owned()
                        } else {
                            format!("Jump to latest · {} new", viewport.unseen)
                        },
                        ButtonTone::Accent,
                        true,
                        move |_, cx| {
                            let _ = jump.update(cx, |this, cx| this.jump_to_latest(cx));
                        },
                    )),
            )
        })
        .into_any_element()
}

fn latest_allows_tail_reserve(
    row: TranscriptRow,
    items: &[Arc<TranscriptItem>],
    expanded: bool,
) -> bool {
    match row {
        TranscriptRow::MessageChunk { .. } => true,
        TranscriptRow::Item { index, .. } => {
            !expanded
                || !matches!(
                    items[index].kind,
                    TranscriptKind::Tool | TranscriptKind::Thinking
                )
        }
        TranscriptRow::ReadGroup { .. } => !expanded,
    }
}

fn render_row(
    row: TranscriptRow,
    items: &[Arc<TranscriptItem>],
    expanded: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let key = row.key();
    match row {
        TranscriptRow::ReadGroup { start, len, .. } => {
            render_read_group(key, &items[start..start + len], expanded, entity)
        }
        TranscriptRow::MessageChunk {
            index,
            start,
            end,
            block,
            first,
            last,
            ..
        } => render_message_chunk(
            key,
            block,
            &items[index],
            &items[index].text[start..end],
            first,
            last,
        ),
        TranscriptRow::Item { index, .. } if items[index].kind == TranscriptKind::Tool => {
            render_tool(key, &items[index], expanded, entity)
        }
        TranscriptRow::Item { index, .. } if items[index].kind == TranscriptKind::Thinking => {
            render_thinking(key, &items[index], expanded, entity)
        }
        TranscriptRow::Item { index, .. } => render_message(key, &items[index]),
    }
}

fn render_message(key: usize, item: &TranscriptItem) -> AnyElement {
    let separator = item.kind == TranscriptKind::User;
    div()
        .id(("transcript-row", key))
        .w_full()
        .px(THEME.space.md)
        .py(THEME.space.sm)
        .when(separator, |row| {
            row.mt(THEME.space.sm)
                .border_t(THEME.border)
                .border_color(THEME.colors.border)
                .pt(THEME.space.md)
        })
        .child(
            selectable_text(("transcript-text", key), &item.text)
                .text_color(item_color(item))
                .when(item.kind == TranscriptKind::User, |text| {
                    text.font_weight(FontWeight::MEDIUM)
                }),
        )
        .into_any_element()
}

fn render_message_chunk(
    key: usize,
    block: usize,
    item: &TranscriptItem,
    text: &str,
    first: bool,
    last: bool,
) -> AnyElement {
    div()
        .id(format!("transcript-row-{key}-{block}"))
        .w_full()
        .px(THEME.space.md)
        .when(first, |row| row.pt(THEME.space.sm))
        .when(!first, |row| row.pt(THEME.space.xs))
        .when(last, |row| row.pb(THEME.space.sm))
        .child(
            selectable_text(format!("transcript-text-{key}-{block}"), text)
                .text_color(item_color(item)),
        )
        .into_any_element()
}

fn render_thinking(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let body = if expanded {
        item.text.clone()
    } else {
        item.text.lines().next().unwrap_or("Thinking…").to_owned()
    };
    div()
        .id(("thinking-row", key))
        .w_full()
        .flex()
        .items_start()
        .gap(THEME.space.xs)
        .px(THEME.space.md)
        .py(px(2.0))
        .child(disclosure_button(
            ("thinking-toggle", key),
            expanded,
            "Thinking",
            key,
            entity,
        ))
        .child(
            selectable_text(("thinking-text", key), body)
                .flex_1()
                .min_w_0()
                .italic()
                .text_color(THEME.colors.subtle),
        )
        .into_any_element()
}

fn render_read_group(
    key: usize,
    items: &[Arc<TranscriptItem>],
    expanded: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let failed = items.iter().filter(|item| item.is_error).count();
    let running = items.iter().filter(|item| item.streaming).count();
    let summary = if items.len() == 1 {
        format!("Read {}", tool_target(&items[0].text))
    } else {
        format!("Read {} files", items.len())
    };
    let state = tool_state_suffix(running > 0, failed);
    div()
        .id(("read-group", key))
        .w_full()
        .px(THEME.space.md)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .child(disclosure_button(
                    ("read-toggle", key),
                    expanded,
                    "Read details",
                    key,
                    entity.clone(),
                ))
                .child(
                    selectable_text(("read-summary", key), format!("{summary}{state}"))
                        .flex_1()
                        .min_w_0()
                        .font_family(READING_FONT_FAMILY)
                        .text_size(THEME.type_scale.body_small)
                        .text_color(if failed > 0 {
                            THEME.colors.error
                        } else {
                            THEME.colors.muted
                        }),
                ),
        )
        .when(expanded, |group| {
            group.child(
                div()
                    .ml(px(22.0))
                    .mt(THEME.space.xs)
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .children(items.iter().enumerate().map(|(index, item)| {
                        expanded_tool_body(
                            format!("read-detail-{key}-{index}"),
                            item,
                            entity.clone(),
                        )
                    })),
            )
        })
        .into_any_element()
}

fn render_tool(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let summary = format!(
        "{} {}{}",
        item.label,
        tool_target(&item.text),
        tool_state_suffix(item.streaming, usize::from(item.is_error))
    );
    div()
        .id(("tool-row", key))
        .w_full()
        .px(THEME.space.md)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .child(disclosure_button(
                    ("tool-toggle", key),
                    expanded,
                    if item.tool_presentation.is_some() {
                        "File change"
                    } else {
                        "Tool details"
                    },
                    key,
                    entity.clone(),
                ))
                .child(
                    selectable_text(("tool-summary", key), summary)
                        .flex_1()
                        .min_w_0()
                        .font_family(READING_FONT_FAMILY)
                        .text_size(THEME.type_scale.body_small)
                        .text_color(if item.is_error {
                            THEME.colors.error
                        } else if item.streaming {
                            THEME.colors.warning
                        } else {
                            THEME.colors.muted
                        }),
                ),
        )
        .when(expanded, |tool| {
            tool.child(
                div()
                    .ml(px(22.0))
                    .mt(THEME.space.xs)
                    .child(expanded_tool_body(("tool-detail", key), item, entity)),
            )
        })
        .into_any_element()
}

fn expanded_tool_body(
    id: impl Into<gpui::ElementId>,
    item: &TranscriptItem,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    if let Some(presentation) = &item.tool_presentation {
        let output = visible_mutation_output(item).map(|output| {
            selectable_text(id, output)
                .font_family(READING_FONT_FAMILY)
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.error)
        });
        return div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .child(crate::tool_changes::render(
                presentation,
                item.tool_call_id.as_deref(),
                item.tool_call_id.as_ref().map_or(0, |id| stable_key(id)),
                entity,
            ))
            .children(output)
            .into_any_element();
    }
    let mut detail = String::new();
    if !item.text.is_empty() {
        detail.push_str(&item.text);
    }
    if !item.tool_output.is_empty() {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str(&item.tool_output);
    }
    selectable_text(id, fenced_text(&detail))
        .font_family(READING_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .text_color(if item.is_error {
            THEME.colors.error
        } else {
            THEME.colors.muted
        })
        .into_any_element()
}

fn disclosure_button(
    id: impl Into<gpui::ElementId>,
    expanded: bool,
    label: &'static str,
    key: usize,
    entity: WeakEntity<PiApp>,
) -> Button {
    Button::new(id)
        .label(if expanded { "▾" } else { "▸" })
        .tooltip(label)
        .with_size(Size::XSmall)
        .ghost()
        .on_click(move |_, _, cx| {
            let _ = entity.update(cx, |this, cx| this.toggle_transcript_item(key, cx));
        })
}

fn selectable_text(
    id: impl Into<gpui::ElementId>,
    text: impl Into<gpui::SharedString>,
) -> TextView {
    TextView::markdown(id, text)
        .style(transcript_markdown_style())
        .selectable(true)
        .w_full()
        .min_w_0()
        .font_family(READING_FONT_FAMILY)
        .text_size(THEME.type_scale.body)
        .line_height(THEME.type_scale.line_body)
}

fn transcript_markdown_style() -> TextViewStyle {
    let mut code_block = StyleRefinement::default();
    code_block.overflow.x = Some(Overflow::Scroll);
    code_block.restrict_scroll_to_axis = Some(true);
    TextViewStyle {
        paragraph_gap: rems(0.5),
        heading_base_font_size: THEME.type_scale.body,
        highlight_theme: HighlightTheme::default_dark(),
        code_block,
        inline_code: HighlightStyle {
            color: Some(THEME.colors.code.into()),
            background_color: Some(THEME.colors.panel.into()),
            ..HighlightStyle::default()
        },
        is_dark: true,
        ..TextViewStyle::default()
    }
}

fn fenced_text(text: &str) -> String {
    if text.is_empty() {
        return "No output".into();
    }
    format!("```text\n{}\n```", text.replace("```", "``\\`"))
}

fn visible_mutation_output(item: &TranscriptItem) -> Option<&str> {
    (item.is_error && !item.tool_output.is_empty()).then_some(item.tool_output.as_str())
}

fn tool_target(arguments: &str) -> String {
    let first = arguments.lines().next().unwrap_or_default();
    first
        .split_once(':')
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(first)
        .chars()
        .take(96)
        .collect()
}

fn stable_key(value: &str) -> usize {
    value.bytes().fold(0_usize, |hash, byte| {
        hash.wrapping_mul(16777619).wrapping_add(usize::from(byte))
    })
}

fn tool_state_suffix(running: bool, failed: usize) -> String {
    if failed > 0 {
        if failed == 1 {
            " · failed".into()
        } else {
            format!(" · {failed} failed")
        }
    } else if running {
        " · working".into()
    } else {
        String::new()
    }
}

fn item_color(item: &TranscriptItem) -> gpui::Rgba {
    match item.kind {
        TranscriptKind::Error => THEME.colors.error,
        TranscriptKind::Notice | TranscriptKind::Custom => THEME.colors.muted,
        TranscriptKind::User | TranscriptKind::Assistant => THEME.colors.text,
        TranscriptKind::Thinking | TranscriptKind::Tool => THEME.colors.subtle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: TranscriptKind, label: &str, text: &str) -> Arc<TranscriptItem> {
        Arc::new(TranscriptItem {
            kind,
            label: label.into(),
            text: text.into(),
            streaming: false,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
        })
    }

    #[test]
    fn tail_reserve_is_responsive_but_bounded() {
        assert_eq!(tail_reserve(px(100.0)), px(72.0));
        assert_eq!(tail_reserve(px(500.0)), px(160.0));
        assert_eq!(tail_reserve(px(2_000.0)), px(280.0));
    }

    #[test]
    fn markdown_inline_code_uses_the_reading_palette() {
        let style = transcript_markdown_style();

        assert_eq!(style.inline_code.color, Some(THEME.colors.code.into()));
        assert_eq!(
            style.inline_code.background_color,
            Some(THEME.colors.panel.into())
        );
    }

    #[test]
    fn consecutive_reads_collapse_into_one_row() {
        let rows = project_rows(&[
            item(TranscriptKind::User, "", "question"),
            item(TranscriptKind::Tool, "Read", "Path: a"),
            item(TranscriptKind::Tool, "Read", "Path: b"),
            item(TranscriptKind::Tool, "Bash", "Command: true"),
        ]);
        assert_eq!(rows.len(), 3);
        assert!(matches!(&rows[1], TranscriptRow::ReadGroup { len: 2, .. }));
    }

    #[test]
    fn long_assistant_messages_become_independently_virtualized_rows() {
        let text = format!(
            "{}\n\n{}\n\n{}",
            "first ".repeat(600),
            "second ".repeat(600),
            "third ".repeat(600)
        );
        let assistant = item(TranscriptKind::Assistant, "", &text);
        let rows = project_rows(std::slice::from_ref(&assistant));

        assert!(rows.len() >= 3);
        let reconstructed = rows
            .iter()
            .map(|row| match row {
                TranscriptRow::MessageChunk { start, end, .. } => &text[*start..*end],
                _ => panic!("expected only message chunks"),
            })
            .collect::<String>();
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn a_giant_plain_paragraph_is_split_at_word_boundaries() {
        let text = "word ".repeat(5_000);
        let chunks = markdown_chunk_ranges(&text);

        assert!(chunks.len() > 1);
        assert_eq!(
            chunks
                .iter()
                .map(|(start, end)| &text[*start..*end])
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn fenced_code_is_never_split_inside_the_fence() {
        let code = "let value = 1;\n".repeat(1_000);
        let text = format!("before\n\n```rust\n{code}```\n\nafter");
        let closing_end = text.find("```\n\n").expect("closing fence") + 4;
        let chunks = markdown_chunk_ranges(&text);

        assert!(chunks.iter().any(|(_, end)| *end == closing_end));
        assert!(
            !chunks
                .iter()
                .any(|(_, end)| text[..*end].ends_with("let value = 1;\n"))
        );
    }

    #[test]
    fn row_updates_reproject_only_the_changed_shared_item_suffix() {
        let first = item(TranscriptKind::Assistant, "", "unchanged");
        let second = item(TranscriptKind::Assistant, "", "short");
        let previous_items = vec![first.clone(), second];
        let previous_rows = project_rows(&previous_items);
        let long = item(
            TranscriptKind::Assistant,
            "",
            &format!(
                "section\n\n{}\n\n{}",
                "updated ".repeat(700),
                "tail ".repeat(700)
            ),
        );
        let items = vec![first, long];

        let rows = update_rows(&previous_rows, &previous_items, &items);

        assert_eq!(rows[0], previous_rows[0]);
        assert!(matches!(
            rows[1],
            TranscriptRow::MessageChunk { index: 1, .. }
        ));
    }

    #[test]
    fn changed_item_revision_invalidates_an_equal_length_row() {
        let previous_items = vec![item(TranscriptKind::Assistant, "", "old")];
        let previous_rows = project_rows(&previous_items);
        let items = vec![item(TranscriptKind::Assistant, "", "new")];

        let rows = update_rows(&previous_rows, &previous_items, &items);

        assert_ne!(rows, previous_rows);
    }

    #[test]
    fn appended_reads_merge_with_the_existing_read_group() {
        let previous_items = vec![item(TranscriptKind::Tool, "Read", "Path: one")];
        let previous_rows = project_rows(&previous_items);
        let items = vec![
            previous_items[0].clone(),
            item(TranscriptKind::Tool, "Read", "Path: two"),
        ];

        let rows = update_rows(&previous_rows, &previous_items, &items);

        assert!(matches!(
            rows.as_slice(),
            [TranscriptRow::ReadGroup { len: 2, .. }]
        ));
    }

    #[test]
    fn mutation_tools_are_expanded_by_default() {
        let mut edit = item(TranscriptKind::Tool, "Edit", "Path: src/main.rs");
        Arc::make_mut(&mut edit).tool_presentation =
            Some(crate::conversation::ToolPresentation::Edit {
                path: "src/main.rs".into(),
                diff: Some("- old\n+ new".into()),
                format: crate::conversation::EditDiffFormat::Unnumbered,
            });
        let items = vec![edit, item(TranscriptKind::Tool, "Bash", "Command: true")];
        let rows = project_rows(&items);

        assert!(expanded_by_default(rows[0], &items));
        assert!(!expanded_by_default(rows[1], &items));
    }

    #[test]
    fn successful_mutation_output_is_hidden_but_errors_remain_visible() {
        let mut write = item(TranscriptKind::Tool, "Write", "Path: src/main.rs");
        Arc::make_mut(&mut write).tool_output = "Successfully wrote 42 bytes".into();
        assert_eq!(visible_mutation_output(&write), None);

        Arc::make_mut(&mut write).is_error = true;
        Arc::make_mut(&mut write).tool_output = "Permission denied".into();
        assert_eq!(visible_mutation_output(&write), Some("Permission denied"));
    }

    #[test]
    fn targets_use_the_first_readable_argument_value() {
        assert_eq!(tool_target("Path: src/main.rs\nOffset: 2"), "src/main.rs");
        assert_eq!(tool_target(""), "");
    }
}
