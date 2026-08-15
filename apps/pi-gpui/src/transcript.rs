//! Selectable, compact transcript projection.

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement as _, ListSizingBehavior,
    ListState, ParentElement as _, Styled as _, WeakEntity, div, list, prelude::FluentBuilder as _,
    px,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    text::TextView,
};

use crate::{
    app::PiApp,
    conversation::{TranscriptItem, TranscriptKind},
    primitives::{ButtonTone, button},
    theme::THEME,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptRow {
    Item {
        index: usize,
        item: TranscriptItem,
    },
    ReadGroup {
        start: usize,
        items: Vec<TranscriptItem>,
    },
}

impl TranscriptRow {
    pub(crate) fn key(&self) -> usize {
        match self {
            Self::Item { index, .. } => *index,
            Self::ReadGroup { start, .. } => *start,
        }
    }
}

pub(crate) fn project_rows(items: &[TranscriptItem]) -> Vec<TranscriptRow> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < items.len() {
        if items[index].kind == TranscriptKind::Tool && items[index].label == "Read" {
            let start = index;
            let mut reads = Vec::new();
            while index < items.len()
                && items[index].kind == TranscriptKind::Tool
                && items[index].label == "Read"
            {
                reads.push(items[index].clone());
                index += 1;
            }
            rows.push(TranscriptRow::ReadGroup {
                start,
                items: reads,
            });
            continue;
        }
        rows.push(TranscriptRow::Item {
            index,
            item: items[index].clone(),
        });
        index += 1;
    }
    rows
}

pub(crate) fn render(
    list_state: &ListState,
    following: bool,
    unseen: usize,
    rows: Vec<TranscriptRow>,
    expanded: std::collections::HashSet<usize>,
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
    let rows = std::sync::Arc::new(rows);
    let view = list(list_state.clone(), move |index, _, _| {
        let Some(row) = rows.get(index).cloned() else {
            return div().into_any_element();
        };
        render_row(
            row,
            expanded.contains(&rows[index].key()),
            row_entity.clone(),
        )
    })
    .with_sizing_behavior(ListSizingBehavior::Auto)
    .w_full()
    .max_w(THEME.layout.transcript_max)
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
        .when(!following, |root| {
            root.child(
                div()
                    .flex_none()
                    .flex()
                    .justify_center()
                    .bg(THEME.colors.canvas)
                    .py(THEME.space.xs)
                    .child(button(
                        "jump-to-latest",
                        if unseen == 0 {
                            "Jump to latest".to_owned()
                        } else {
                            format!("Jump to latest · {unseen} new")
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

fn render_row(row: TranscriptRow, expanded: bool, entity: WeakEntity<PiApp>) -> AnyElement {
    let key = row.key();
    match row {
        TranscriptRow::ReadGroup { items, .. } => render_read_group(key, &items, expanded, entity),
        TranscriptRow::Item { item, .. } if item.kind == TranscriptKind::Tool => {
            render_tool(key, &item, expanded, entity)
        }
        TranscriptRow::Item { item, .. } if item.kind == TranscriptKind::Thinking => {
            render_thinking(key, &item, expanded, entity)
        }
        TranscriptRow::Item { item, .. } => render_message(key, &item),
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
    items: &[TranscriptItem],
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
                    entity,
                ))
                .child(
                    selectable_text(("read-summary", key), format!("{summary}{state}"))
                        .flex_1()
                        .min_w_0()
                        .font_family("monospace")
                        .text_size(THEME.type_scale.caption)
                        .text_color(if failed > 0 {
                            THEME.colors.error
                        } else {
                            THEME.colors.subtle
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
                        expanded_tool_body(format!("read-detail-{key}-{index}"), item)
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
                    "Tool details",
                    key,
                    entity,
                ))
                .child(
                    selectable_text(("tool-summary", key), summary)
                        .flex_1()
                        .min_w_0()
                        .font_family("monospace")
                        .text_size(THEME.type_scale.caption)
                        .text_color(if item.is_error {
                            THEME.colors.error
                        } else if item.streaming {
                            THEME.colors.warning
                        } else {
                            THEME.colors.subtle
                        }),
                ),
        )
        .when(expanded, |tool| {
            tool.child(
                div()
                    .ml(px(22.0))
                    .mt(THEME.space.xs)
                    .child(expanded_tool_body(("tool-detail", key), item)),
            )
        })
        .into_any_element()
}

fn expanded_tool_body(id: impl Into<gpui::ElementId>, item: &TranscriptItem) -> AnyElement {
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
        .font_family("monospace")
        .text_size(THEME.type_scale.caption)
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
        .selectable(true)
        .w_full()
        .text_size(THEME.type_scale.body_small)
        .line_height(THEME.type_scale.line_body)
}

fn fenced_text(text: &str) -> String {
    if text.is_empty() {
        return "No output".into();
    }
    format!("```text\n{}\n```", text.replace("```", "``\\`"))
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

    fn item(kind: TranscriptKind, label: &str, text: &str) -> TranscriptItem {
        TranscriptItem {
            kind,
            label: label.into(),
            text: text.into(),
            streaming: false,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
        }
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
        assert!(matches!(
            &rows[1],
            TranscriptRow::ReadGroup { items, .. } if items.len() == 2
        ));
    }

    #[test]
    fn targets_use_the_first_readable_argument_value() {
        assert_eq!(tool_target("Path: src/main.rs\nOffset: 2"), "src/main.rs");
        assert_eq!(tool_target(""), "");
    }
}
