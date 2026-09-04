use std::{
    borrow::Cow,
    hash::{Hash, Hasher},
    sync::Arc,
};

use gpui::{
    AnyElement, ClipboardItem, Div, Entity, FontWeight, HighlightStyle, InteractiveElement as _,
    IntoElement as _, MouseButton, Overflow, ParentElement as _, Pixels, Stateful, StyleRefinement,
    Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    highlighter::HighlightTheme,
    menu::{DropdownMenu as _, PopupMenuItem},
    text::{TextView, TextViewState, TextViewStyle},
};

use crate::{
    app::ui::persistent_vec::{Indexed, PersistentVec},
    app::ui::primitives::{
        ButtonTone, ContextMenuTrigger, button, disclosure_detail, disclosure_title_row,
    },
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    app::{
        FarcasterApp,
        views::transcript::{
            attachments::ATTACHMENT_ROW_HEIGHT,
            conversation::{self, TranscriptItem, TranscriptKind},
            list::{self, TranscriptListState, transcript_list_grouped},
            markdown::{MarkdownStateKey, TranscriptMarkdownCache},
        },
    },
};

mod chunking;
mod detail_rows;
mod message_rows;
mod rows;
mod tool_rows;

use chunking::*;
#[cfg(test)]
pub(super) use chunking::{
    MARKDOWN_CHUNK_HARD_BYTES, markdown_chunk_text, markdown_chunks, markdown_fence,
    markdown_fence_closes,
};
use detail_rows::{render_agent_result, render_error, render_thinking};
#[allow(unused_imports)]
pub(super) use detail_rows::{thinking_has_details, thinking_preview, thinking_preview_emphasis};
#[allow(unused_imports)]
pub(super) use message_rows::{
    highlighted_invocation_markdown, invocation_kind, is_mixed_invocation_message,
    message_role_label,
};
use message_rows::{render_invocation, render_message, render_message_chunk};
#[cfg(test)]
pub(super) use rows::matching_item_prefix;
pub(crate) use rows::*;
use tool_rows::{render_read_group, render_tool};

const TRANSCRIPT_HORIZONTAL_PADDING: Pixels = px(18.0);
pub(crate) const TRANSCRIPT_ROW_HEIGHT_HINT: Pixels = px(24.0);

#[derive(Clone, Copy)]
pub(crate) struct TranscriptViewport {
    pub(crate) following: bool,
    pub(crate) unseen: usize,
    pub(crate) tail_reserve: Pixels,
}

pub(super) fn expanded_by_default(
    _row: TranscriptRow,
    _items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
) -> bool {
    false
}

pub(super) fn resolved_expanded(
    row: TranscriptRow,
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    disclosure_states: &std::collections::HashMap<usize, bool>,
) -> bool {
    disclosure_states
        .get(&row.key())
        .copied()
        .unwrap_or_else(|| expanded_by_default(row, items))
}

pub(super) fn message_follows_tool(
    row: TranscriptRow,
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
) -> bool {
    let is_first_assistant_row = match row {
        TranscriptRow::Item { index, .. } => items
            .get(index)
            .is_some_and(|item| item.kind == TranscriptKind::Assistant),
        TranscriptRow::MessageChunk { first, .. } | TranscriptRow::StreamChunk { first, .. } => {
            first
        }
        TranscriptRow::ReadGroup { .. } => false,
    };
    is_first_assistant_row
        && row
            .item_start()
            .checked_sub(1)
            .and_then(|index| items.get(index))
            .is_some_and(|item| item.kind == TranscriptKind::Tool)
}

pub(super) fn copy_transcript_items(
    items: &PersistentVec<Arc<TranscriptItem>>,
    range: std::ops::RangeInclusive<usize>,
) -> String {
    range
        .filter_map(|index| items.get(index))
        .map(|item| {
            let mut text = item.complete_text();
            if !item.images.is_empty() {
                let label = if item.images.len() == 1 {
                    "[Image attachment]".to_owned()
                } else {
                    format!("[{} image attachments]", item.images.len())
                };
                if text.trim().is_empty() {
                    text = label;
                } else {
                    text.push_str("\n\n");
                    text.push_str(&label);
                }
            }
            if !text.trim().is_empty() {
                text
            } else if !item.tool_output.trim().is_empty() {
                item.tool_output.clone()
            } else {
                item.label.clone()
            }
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn render(
    list_state: &TranscriptListState,
    viewport: TranscriptViewport,
    rows: std::sync::Arc<PersistentVec<TranscriptRow>>,
    conversation: Arc<conversation::ConversationState>,
    disclosure_states: std::collections::HashMap<usize, bool>,
    markdown_cache: TranscriptMarkdownCache,
    assistant_label: Arc<str>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    if rows.is_empty() {
        return div().size_full().bg(THEME.colors.canvas).into_any_element();
    }

    let visual_selection_active = list_state.selected_text().is_some();
    let jump = entity.clone();
    let row_entity = entity;
    let selection_rows = rows.clone();
    let selection_items = conversation.items.clone();
    let selection_state = list_state.clone();
    let view = transcript_list_grouped(
        list_state.clone(),
        move |index| selection_rows.get(index).map_or(index, TranscriptRow::key),
        move |range| copy_transcript_items(&selection_items, range),
        move |index, _, cx| {
            let _timing = crate::app::infrastructure::performance::OperationTiming::new(
                crate::app::infrastructure::performance::OperationKind::TranscriptRow,
                1,
            );
            let Some(row) = rows.get(index).copied() else {
                return div().into_any_element();
            };
            let expanded = resolved_expanded(row, &conversation.items, &disclosure_states);
            let reserves_tail = index + 1 == rows.len()
                && latest_allows_tail_reserve(row, &conversation.items, expanded);
            let content = div()
                .w_full()
                .when(reserves_tail, |row| row.pb(viewport.tail_reserve))
                .child(
                    div()
                        .w_full()
                        .when(selection_state.selection_contains(row.key()), |row| {
                            row.bg(THEME.colors.selection)
                        })
                        .child(div().w_full().child(render_row(
                            row,
                            &conversation.items,
                            expanded,
                            &markdown_cache,
                            &assistant_label,
                            row_entity.clone(),
                            cx,
                        ))),
                )
                .into_any_element();
            transcript_context_menu(
                index,
                row,
                conversation.items.clone(),
                selection_state.clone(),
                content,
            )
        },
    );

    div()
        .size_full()
        .when(visual_selection_active, |root| {
            root.key_context(list::TRANSCRIPT_SELECTION_KEY_CONTEXT)
        })
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_hidden()
                .flex()
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

fn transcript_context_menu(
    row_index: usize,
    row: TranscriptRow,
    items: PersistentVec<Arc<TranscriptItem>>,
    selection_state: TranscriptListState,
    content: AnyElement,
) -> AnyElement {
    ContextMenuTrigger::new(format!("transcript-context-trigger-{row_index}"), content)
        .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
            let selected_text = selection_state
                .selected_text()
                .filter(|text| !text.trim().is_empty());
            let mut menu = menu.min_w(px(190.0));
            if let Some(text) = selected_text {
                menu = menu
                    .item(
                        PopupMenuItem::new("Copy selection").on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        }),
                    )
                    .separator();
            }

            let row_text = copy_transcript_items(&items, row.item_start()..=row.item_end() - 1);
            let all_text =
                (!items.is_empty()).then(|| copy_transcript_items(&items, 0..=items.len() - 1));
            menu.item(
                PopupMenuItem::new(if matches!(row, TranscriptRow::ReadGroup { .. }) {
                    "Copy tool group"
                } else {
                    "Copy message"
                })
                .disabled(row_text.trim().is_empty())
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(row_text.clone()));
                }),
            )
            .item(
                PopupMenuItem::new("Copy transcript")
                    .disabled(all_text.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(text) = &all_text {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        }
                    }),
            )
        })
        .mouse_button(MouseButton::Right)
        .anchor_to_cursor()
        .into_any_element()
}

pub(super) fn latest_allows_tail_reserve(
    row: TranscriptRow,
    items: &PersistentVec<Arc<TranscriptItem>>,
    expanded: bool,
) -> bool {
    match row {
        TranscriptRow::MessageChunk { .. } | TranscriptRow::StreamChunk { .. } => true,
        TranscriptRow::Item { index, .. } => {
            !expanded
                || !matches!(
                    items[index].kind,
                    TranscriptKind::Thinking | TranscriptKind::Error | TranscriptKind::AgentResult
                )
        }
        TranscriptRow::ReadGroup { .. } => true,
    }
}

fn render_row(
    row: TranscriptRow,
    items: &PersistentVec<Arc<TranscriptItem>>,
    expanded: bool,
    markdown_cache: &TranscriptMarkdownCache,
    assistant_label: &str,
    entity: WeakEntity<FarcasterApp>,
    cx: &mut gpui::App,
) -> AnyElement {
    let key = row.key();
    let follows_tool = message_follows_tool(row, items);
    match row {
        TranscriptRow::ReadGroup { start, len, .. } => {
            render_read_group(key, items, start, len, expanded, entity)
        }
        TranscriptRow::MessageChunk {
            index,
            start,
            end,
            block,
            revision,
            first,
            last,
            fence,
        } => {
            let markdown =
                markdown_chunk_text(&items[index].text, MarkdownChunk { start, end, fence });
            render_message_chunk(
                key,
                block,
                &items[index],
                first,
                last,
                follows_tool,
                markdown_cache.state(
                    MarkdownStateKey::message_chunk(index, block, revision),
                    &markdown,
                    cx,
                ),
                assistant_label,
                entity.clone(),
            )
        }
        TranscriptRow::StreamChunk {
            index,
            chunk,
            revision,
            first,
            last,
        } => {
            let text = items[index]
                .stream_chunks
                .get(chunk)
                .map_or(items[index].text.as_str(), |chunk| chunk.as_ref());
            render_message_chunk(
                key,
                chunk,
                &items[index],
                first,
                last,
                follows_tool,
                markdown_cache.state(
                    MarkdownStateKey::stream_chunk(index, chunk, revision),
                    text,
                    cx,
                ),
                assistant_label,
                entity.clone(),
            )
        }
        TranscriptRow::Item { index, .. } if items[index].kind == TranscriptKind::Error => {
            render_error(key, &items[index], expanded, entity)
        }
        TranscriptRow::Item { index, revision }
            if items[index].invocation.as_ref().is_some_and(|resolved| {
                is_mixed_invocation_message(&items[index].text, resolved)
            }) =>
        {
            let resolved = message_rows::invocation_resolution(&items[index]);
            let markdown = highlighted_invocation_markdown(&items[index].text, resolved);
            render_message(
                key,
                &items[index],
                follows_tool,
                Some(markdown_cache.state(MarkdownStateKey::item(index, revision), &markdown, cx)),
                Some(invocation_transcript_markdown_style(resolved)),
                assistant_label,
                entity.clone(),
            )
        }
        TranscriptRow::Item { index, .. } if items[index].invocation.is_some() => {
            render_invocation(key, &items[index], expanded, entity)
        }
        TranscriptRow::Item { index, .. } if items[index].kind == TranscriptKind::Tool => {
            render_tool(key, &items[index], expanded, entity)
        }
        TranscriptRow::Item { index, revision }
            if items[index].kind == TranscriptKind::AgentResult =>
        {
            let markdown_state = expanded.then(|| {
                markdown_cache.state(
                    MarkdownStateKey::item(index, revision),
                    &items[index].text,
                    cx,
                )
            });
            render_agent_result(key, &items[index], expanded, markdown_state, entity)
        }
        TranscriptRow::Item { index, .. } if items[index].kind == TranscriptKind::Thinking => {
            render_thinking(key, &items[index], expanded, entity)
        }
        TranscriptRow::Item { index, revision } => {
            let markdown_state = matches!(
                items[index].kind,
                TranscriptKind::User | TranscriptKind::Assistant | TranscriptKind::PeerMessage
            )
            .then(|| {
                markdown_cache.state(
                    MarkdownStateKey::item(index, revision),
                    &items[index].text,
                    cx,
                )
            });
            render_message(
                key,
                &items[index],
                follows_tool,
                markdown_state,
                None,
                assistant_label,
                entity,
            )
        }
    }
}

fn transcript_title_row(
    id: impl Into<gpui::ElementId>,
    expanded: bool,
    expandable: bool,
    label: String,
    key: usize,
    entity: WeakEntity<FarcasterApp>,
) -> Stateful<Div> {
    disclosure_title_row(
        id,
        key,
        expanded,
        expandable,
        label,
        toggle_transcript_item(entity, key, expanded),
    )
}

fn toggle_transcript_item(
    entity: WeakEntity<FarcasterApp>,
    key: usize,
    expanded: bool,
) -> impl Fn(&mut gpui::Window, &mut gpui::App) + 'static {
    move |_, cx| {
        let _ = entity.update(cx, |this, cx| {
            this.set_transcript_item_expanded(key, !expanded, cx)
        });
    }
}

fn selectable_text(
    id: impl Into<gpui::ElementId>,
    text: impl Into<gpui::SharedString>,
) -> TextView {
    styled_selectable_text(TextView::markdown(id, text))
}

fn selectable_text_state(state: &Entity<TextViewState>) -> TextView {
    styled_selectable_text(TextView::new(state))
}

fn styled_selectable_text(text: TextView) -> TextView {
    text.style(transcript_markdown_style())
        .selectable(true)
        .w_full()
        .min_w_0()
        .text_size(THEME.type_scale.body)
        .line_height(THEME.type_scale.line_body)
}

fn technical_text(id: impl Into<gpui::ElementId>, text: impl Into<gpui::SharedString>) -> TextView {
    selectable_text(id, text)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
}

pub(super) fn transcript_markdown_style() -> TextViewStyle {
    transcript_markdown_style_with_inline_code(HighlightStyle {
        color: Some(THEME.colors.code.into()),
        background_color: Some(THEME.colors.panel.into()),
        ..HighlightStyle::default()
    })
}

pub(super) fn invocation_transcript_markdown_style(resolved: &str) -> TextViewStyle {
    let skill = message_rows::resolved_contains_skill(resolved);
    transcript_markdown_style_with_inline_code(HighlightStyle {
        color: Some(
            if skill {
                THEME.colors.skill
            } else {
                THEME.colors.accent
            }
            .into(),
        ),
        background_color: if skill {
            None
        } else {
            Some(THEME.colors.panel.into())
        },
        font_weight: Some(FontWeight::SEMIBOLD),
        ..HighlightStyle::default()
    })
}

fn transcript_markdown_style_with_inline_code(inline_code: HighlightStyle) -> TextViewStyle {
    let mut code_block = StyleRefinement::default();
    code_block.overflow.x = Some(Overflow::Scroll);
    code_block.restrict_scroll_to_axis = Some(true);
    TextViewStyle {
        paragraph_gap: rems(0.5),
        heading_base_font_size: THEME.type_scale.body,
        highlight_theme: HighlightTheme::default_dark(),
        code_block,
        inline_code,
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

pub(super) fn tool_target(arguments: &str) -> String {
    let first = if let Some((_, command)) = conversation::split_command_block(arguments) {
        command.lines().next().unwrap_or_default().trim()
    } else {
        let first = arguments.lines().next().unwrap_or_default();
        first
            .split_once(':')
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or(first)
    };
    first.chars().take(96).collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ToolState {
    pub(super) glyph: &'static str,
    pub(super) label: &'static str,
}

pub(super) fn tool_state(running: bool, failed: usize, completed: bool) -> Option<ToolState> {
    if failed > 0 {
        Some(ToolState {
            glyph: "×",
            label: "Failed",
        })
    } else if running {
        Some(ToolState {
            glyph: "…",
            label: "Working",
        })
    } else if completed {
        Some(ToolState {
            glyph: "✓",
            label: "Done",
        })
    } else {
        None
    }
}

fn item_color(item: &TranscriptItem) -> gpui::Rgba {
    match item.kind {
        TranscriptKind::Error => THEME.colors.error,
        TranscriptKind::Notice | TranscriptKind::Custom | TranscriptKind::AgentResult => {
            THEME.colors.muted
        }
        TranscriptKind::User | TranscriptKind::Assistant | TranscriptKind::PeerMessage => {
            THEME.colors.text
        }
        TranscriptKind::Thinking | TranscriptKind::Tool => THEME.colors.subtle,
    }
}
