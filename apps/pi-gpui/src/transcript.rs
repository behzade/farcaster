//! Selectable, compact transcript projection.

use std::{
    hash::{Hash, Hasher},
    rc::Rc,
    sync::Arc,
};

use gpui::{
    AnyElement, Entity, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement as _,
    ListSizingBehavior, ListState, Overflow, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, StyleRefinement, Styled as _, WeakEntity, div, list,
    prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    highlighter::HighlightTheme,
    text::{TextView, TextViewState, TextViewStyle},
};

use crate::{
    app::PiApp,
    conversation::{TranscriptItem, TranscriptKind},
    primitives::{ButtonTone, button, disclosure_button, disclosure_indicator},
    theme::{MONO_FONT_FAMILY, THEME},
    tool_changes::EmbeddedDiffMode,
    transcript_markdown::{MarkdownStateKey, TranscriptMarkdownCache},
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
    pub(crate) diff_mode: EmbeddedDiffMode,
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
    StreamChunk {
        index: usize,
        chunk: usize,
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

    pub(crate) fn same_position(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Item { index: left, .. }, Self::Item { index: right, .. }) => left == right,
            (
                Self::MessageChunk {
                    index: left_index,
                    start: left_start,
                    block: left_block,
                    first: left_first,
                    ..
                },
                Self::MessageChunk {
                    index: right_index,
                    start: right_start,
                    block: right_block,
                    first: right_first,
                    ..
                },
            ) => {
                left_index == right_index
                    && left_start == right_start
                    && left_block == right_block
                    && left_first == right_first
            }
            (
                Self::StreamChunk {
                    index: left_index,
                    chunk: left_chunk,
                    first: left_first,
                    ..
                },
                Self::StreamChunk {
                    index: right_index,
                    chunk: right_chunk,
                    first: right_first,
                    ..
                },
            ) => {
                left_index == right_index && left_chunk == right_chunk && left_first == right_first
            }
            (
                Self::ReadGroup {
                    start: left_start,
                    len: left_len,
                    ..
                },
                Self::ReadGroup {
                    start: right_start,
                    len: right_len,
                    ..
                },
            ) => left_start == right_start && left_len == right_len,
            _ => false,
        }
    }

    fn item_start(&self) -> usize {
        match self {
            Self::Item { index, .. }
            | Self::MessageChunk { index, .. }
            | Self::StreamChunk { index, .. } => *index,
            Self::ReadGroup { start, .. } => *start,
        }
    }

    fn item_end(&self) -> usize {
        match self {
            Self::Item { index, .. }
            | Self::MessageChunk { index, .. }
            | Self::StreamChunk { index, .. } => index + 1,
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
    update_rows_from(previous_rows, previous_items, items, None)
}

pub(crate) fn update_rows_from(
    previous_rows: &[TranscriptRow],
    previous_items: &[Arc<TranscriptItem>],
    items: &[Arc<TranscriptItem>],
    changed_from: Option<usize>,
) -> Vec<TranscriptRow> {
    let unchanged_hint = changed_from
        .unwrap_or_default()
        .min(previous_items.len())
        .min(items.len());
    let projected_items = previous_rows
        .last()
        .map_or(0, TranscriptRow::item_end)
        .min(previous_items.len());
    let unchanged_items = (unchanged_hint
        + previous_items[unchanged_hint..]
            .iter()
            .zip(&items[unchanged_hint..])
            .take_while(|(previous, next)| {
                Arc::ptr_eq(previous, next) || previous.as_ref() == next.as_ref()
            })
            .count())
    .min(projected_items);
    crate::performance::count_transcript_items(
        unchanged_items.saturating_add(usize::from(unchanged_items < items.len())),
    );
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
    crate::performance::count_transcript_items(items.len().saturating_sub(index));
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
        if items[index].kind == TranscriptKind::Assistant && items[index].streaming {
            let chunk_count =
                items[index].stream_chunks.len() + usize::from(!items[index].text.is_empty());
            rows.extend((0..chunk_count).map(|chunk| {
                let text = items[index]
                    .stream_chunks
                    .get(chunk)
                    .map_or(items[index].text.as_str(), |chunk| chunk.as_ref());
                TranscriptRow::StreamChunk {
                    index,
                    chunk,
                    revision: text_revision(text),
                    first: chunk == 0,
                    last: chunk + 1 == chunk_count,
                }
            }));
        } else if matches!(
            items[index].kind,
            TranscriptKind::User | TranscriptKind::Assistant
        ) && items[index].invocation.is_none()
            && (items[index].text.len() > MARKDOWN_CHUNK_HARD_BYTES
                || (items[index].streaming
                    && items[index].text.len() > MARKDOWN_CHUNK_TARGET_BYTES))
        {
            let chunks = markdown_chunk_ranges(&items[index].text);
            let last_block = chunks.len().saturating_sub(1);
            rows.extend(chunks.into_iter().enumerate().map(|(block, (start, end))| {
                TranscriptRow::MessageChunk {
                    index,
                    start,
                    end,
                    block,
                    revision: text_revision(&items[index].text[start..end]),
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

fn text_revision(text: &str) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish() as usize
}

fn is_read(item: &TranscriptItem) -> bool {
    item.kind == TranscriptKind::Tool && item.label == "Read"
}

fn markdown_chunk_ranges(text: &str) -> Vec<(usize, usize)> {
    if text.len() <= MARKDOWN_CHUNK_TARGET_BYTES {
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

fn expanded_by_default(_row: TranscriptRow, _items: &[Arc<TranscriptItem>]) -> bool {
    false
}

fn resolved_expanded(
    row: TranscriptRow,
    items: &[Arc<TranscriptItem>],
    disclosure_states: &std::collections::HashMap<usize, bool>,
) -> bool {
    disclosure_states
        .get(&row.key())
        .copied()
        .unwrap_or_else(|| expanded_by_default(row, items))
}

fn message_follows_tool(row: TranscriptRow, items: &[Arc<TranscriptItem>]) -> bool {
    let is_first_assistant_row = match row {
        TranscriptRow::Item { index, .. } => items[index].kind == TranscriptKind::Assistant,
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

pub(crate) fn render(
    list_state: &ListState,
    viewport: TranscriptViewport,
    rows: std::sync::Arc<Vec<TranscriptRow>>,
    snapshot: std::sync::Arc<crate::runtime::RuntimeSnapshot>,
    disclosure_states: std::collections::HashMap<usize, bool>,
    markdown_cache: TranscriptMarkdownCache,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    if rows.is_empty() {
        return div().size_full().bg(THEME.colors.canvas).into_any_element();
    }

    let jump = entity.clone();
    let row_entity = entity;
    let view = list(list_state.clone(), move |index, _, cx| {
        let Some(row) = rows.get(index).copied() else {
            return div().into_any_element();
        };
        let expanded = resolved_expanded(row, &snapshot.conversation.items, &disclosure_states);
        let reserves_tail = index + 1 == rows.len()
            && latest_allows_tail_reserve(row, &snapshot.conversation.items, expanded);
        div()
            .w_full()
            .when(reserves_tail, |row| row.pb(viewport.tail_reserve))
            .child(render_row(
                row,
                &snapshot.conversation.items,
                expanded,
                viewport.diff_mode,
                &markdown_cache,
                row_entity.clone(),
                cx,
            ))
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
        TranscriptRow::MessageChunk { .. } | TranscriptRow::StreamChunk { .. } => true,
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
    diff_mode: EmbeddedDiffMode,
    markdown_cache: &TranscriptMarkdownCache,
    entity: WeakEntity<PiApp>,
    cx: &mut gpui::App,
) -> AnyElement {
    let key = row.key();
    let follows_tool = message_follows_tool(row, items);
    match row {
        TranscriptRow::ReadGroup { start, len, .. } => {
            render_read_group(key, &items[start..start + len], expanded, entity)
        }
        TranscriptRow::MessageChunk {
            index,
            start,
            end,
            block,
            revision,
            first,
            last,
        } => render_message_chunk(
            key,
            block,
            &items[index],
            first,
            last,
            follows_tool,
            markdown_cache.state(
                MarkdownStateKey::message_chunk(index, block, revision),
                &items[index].text[start..end],
                cx,
            ),
        ),
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
            )
        }
        TranscriptRow::Item { index, .. }
            if is_mixed_skill_message(&items[index].text, invocation_resolution(&items[index])) =>
        {
            render_message(
                key,
                &items[index],
                follows_tool,
                None,
                Some(invocation_resolution(&items[index])),
            )
        }
        TranscriptRow::Item { index, .. } if items[index].invocation.is_some() => {
            render_invocation(key, &items[index], expanded, entity)
        }
        TranscriptRow::Item { index, .. } if items[index].kind == TranscriptKind::Tool => {
            render_tool(key, &items[index], expanded, diff_mode, entity)
        }
        TranscriptRow::Item { index, .. } if items[index].kind == TranscriptKind::Thinking => {
            render_thinking(key, &items[index], expanded, entity)
        }
        TranscriptRow::Item { index, revision } => {
            let markdown_state = (items[index].kind == TranscriptKind::Assistant).then(|| {
                markdown_cache.state(
                    MarkdownStateKey::item(index, revision),
                    &items[index].text,
                    cx,
                )
            });
            render_message(key, &items[index], follows_tool, markdown_state, None)
        }
    }
}

fn render_invocation(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let resolved = invocation_resolution(item);
    let kind = invocation_kind(&item.text, resolved);
    let resolved_ready = !resolved.is_empty();
    let skill = kind == "Skill";
    div()
        .id(("invocation-row", key))
        .w_full()
        .px(THEME.space.md)
        .py(THEME.space.sm)
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .when(resolved_ready, |row| {
                    row.child(transcript_disclosure_button(
                        ("invocation-toggle", key),
                        expanded,
                        format!("resolved {kind}"),
                        key,
                        entity,
                    ))
                })
                .when(!resolved_ready, |row| {
                    row.child(
                        div()
                            .size(THEME.controls.icon_button)
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(THEME.colors.subtle)
                            .child(disclosure_indicator(false)),
                    )
                })
                .child(
                    technical_text(("invocation-name", key), item.text.clone())
                        .flex_1()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if skill {
                            THEME.colors.skill
                        } else {
                            THEME.colors.accent
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .px(THEME.space.xs)
                        .py(px(1.0))
                        .rounded(THEME.radius)
                        .border(THEME.border)
                        .border_color(THEME.colors.border)
                        .bg(if skill {
                            THEME.colors.skill_surface
                        } else {
                            THEME.colors.panel
                        })
                        .text_size(THEME.type_scale.caption)
                        .text_color(if skill {
                            THEME.colors.skill
                        } else {
                            THEME.colors.muted
                        })
                        .child(if resolved_ready { kind } else { "Resolving" }),
                ),
        )
        .when(expanded && resolved_ready, |row| {
            row.child(
                div()
                    .id(("invocation-detail-scroll", key))
                    .ml(px(22.0))
                    .mt(THEME.space.xs)
                    .max_h(THEME.layout.tool_max_height)
                    .overflow_y_scroll()
                    .border_l(THEME.border)
                    .border_color(if skill {
                        THEME.colors.skill
                    } else {
                        THEME.colors.accent
                    })
                    .pl(THEME.space.sm)
                    .py(THEME.space.xs)
                    .child(
                        selectable_text(("invocation-detail", key), fenced_text(resolved))
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.body_small)
                            .text_color(THEME.colors.muted),
                    ),
            )
        })
        .into_any_element()
}

fn invocation_kind(display: &str, resolved: &str) -> &'static str {
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

fn invocation_resolution(item: &TranscriptItem) -> &str {
    item.invocation.as_deref().unwrap_or_default()
}

fn resolved_contains_skill(resolved: &str) -> bool {
    resolved.contains("<skill name=")
}

fn is_mixed_skill_message(display: &str, resolved: &str) -> bool {
    resolved_contains_skill(resolved)
        && display
            .split_whitespace()
            .any(|token| !is_invocation_token(token))
}

fn is_invocation_token(token: &str) -> bool {
    token.strip_prefix('$').is_some_and(|name| {
        !name.is_empty()
            && name.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, ':' | '_' | '-')
            })
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

fn highlighted_skill_markdown(display: &str, resolved: &str) -> String {
    let skill_names = skill_names(resolved);
    display
        .split_inclusive(char::is_whitespace)
        .map(|chunk| {
            let token = chunk.trim_end_matches(char::is_whitespace);
            let whitespace = &chunk[token.len()..];
            let name = token.strip_prefix('$').unwrap_or_default();
            let bare_name = name.strip_prefix("skill:").unwrap_or(name);
            if skill_names.contains(&bare_name) {
                format!("`{token}`{whitespace}")
            } else {
                chunk.to_owned()
            }
        })
        .collect()
}

fn render_message(
    key: usize,
    item: &TranscriptItem,
    follows_tool: bool,
    markdown_state: Option<Entity<TextViewState>>,
    skill_resolution: Option<&str>,
) -> AnyElement {
    let user = item.kind == TranscriptKind::User;
    let role = message_role_label(item.kind);
    div()
        .id(("transcript-row", key))
        .w_full()
        .px(THEME.space.md)
        .py(THEME.space.sm)
        .when(user, |row| {
            row.mt(THEME.space.sm)
                .py(THEME.space.md)
                .bg(THEME.colors.selection)
        })
        .when(follows_tool, |row| {
            row.mt(THEME.space.md).pt(THEME.space.sm)
        })
        .children(role.map(|role| message_role(role, user)))
        .child(
            skill_resolution
                .map(|resolved| {
                    styled_selectable_text(TextView::markdown(
                        ("transcript-text", key),
                        highlighted_skill_markdown(&item.text, resolved),
                    ))
                    .style(skill_transcript_markdown_style())
                })
                .unwrap_or_else(|| {
                    markdown_state.map_or_else(
                        || selectable_text(("transcript-text", key), &item.text),
                        |state| selectable_text_state(&state),
                    )
                })
                .text_color(item_color(item))
                .when(user, |text| text.font_weight(FontWeight::MEDIUM)),
        )
        .into_any_element()
}

fn render_message_chunk(
    key: usize,
    block: usize,
    item: &TranscriptItem,
    first: bool,
    last: bool,
    follows_tool: bool,
    markdown_state: Entity<TextViewState>,
) -> AnyElement {
    let user = item.kind == TranscriptKind::User;
    div()
        .id(format!("transcript-row-{key}-{block}"))
        .w_full()
        .px(THEME.space.md)
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
            row.children(message_role_label(item.kind).map(|role| message_role(role, user)))
        })
        .child(
            selectable_text_state(&markdown_state)
                .text_color(item_color(item))
                .when(user, |text| text.font_weight(FontWeight::MEDIUM)),
        )
        .into_any_element()
}

fn message_role_label(kind: TranscriptKind) -> Option<&'static str> {
    match kind {
        TranscriptKind::User => Some("You"),
        TranscriptKind::Assistant => Some("Pi"),
        _ => None,
    }
}

fn message_role(label: &'static str, user: bool) -> impl gpui::IntoElement {
    div()
        .mb(THEME.space.xs)
        .text_size(THEME.type_scale.caption)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if user {
            THEME.colors.accent
        } else {
            THEME.colors.muted
        })
        .child(label)
}

fn render_thinking(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let body = if expanded {
        item.complete_text()
    } else {
        item.stream_chunks
            .first()
            .map_or(item.text.as_str(), |chunk| chunk.as_ref())
            .lines()
            .next()
            .unwrap_or("Thinking…")
            .to_owned()
    };
    div()
        .id(("thinking-row", key))
        .w_full()
        .flex()
        .items_start()
        .gap(THEME.space.xs)
        .px(THEME.space.md)
        .py(px(2.0))
        .child(transcript_disclosure_button(
            ("thinking-toggle", key),
            expanded,
            "thinking details".into(),
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
    let target = (items.len() == 1).then(|| tool_target(&items[0].text));
    let has_target = target.as_ref().is_some_and(|target| !target.is_empty());
    let summary = if items.len() == 1 {
        "Read".to_owned()
    } else {
        format!("Read {} files", items.len())
    };
    let completed = items
        .iter()
        .all(|item| !item.streaming && (item.is_error || !item.tool_output.is_empty()));
    let state = tool_state(running > 0, failed, completed);
    let disclosure_label = format!(
        "read call details for {summary}. {}",
        state.map_or("No result", |state| state.label)
    );
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
                .child(transcript_disclosure_button(
                    ("read-toggle", key),
                    expanded,
                    disclosure_label,
                    key,
                    entity.clone(),
                ))
                .child(
                    div()
                        .text_size(THEME.type_scale.body_small)
                        .text_color(THEME.colors.muted)
                        .when(!has_target, |label| label.flex_1())
                        .child(summary),
                )
                .children(target.filter(|target| !target.is_empty()).map(|target| {
                    technical_text(("read-target", key), target)
                        .flex_1()
                        .min_w_0()
                        .text_color(THEME.colors.text)
                }))
                .children(state.map(|state| {
                    div()
                        .flex_none()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child(state.glyph)
                })),
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
    diff_mode: EmbeddedDiffMode,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let target = tool_target(&item.text);
    let state = tool_state(
        item.streaming,
        usize::from(item.is_error),
        !item.streaming && (item.is_error || !item.tool_output.is_empty()),
    );
    let presentation = item.tool_presentation.as_ref();
    let has_target = !target.is_empty();
    let detail_label = if has_target {
        format!("{} tool call details for {target}", item.label)
    } else {
        format!("{} tool call details", item.label)
    };
    let disclosure_label = format!(
        "{detail_label}. {}",
        state.map_or("No result", |state| state.label)
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
                .when(presentation.is_none(), |row| {
                    row.child(transcript_disclosure_button(
                        ("tool-toggle", key),
                        expanded,
                        disclosure_label,
                        key,
                        entity.clone(),
                    ))
                })
                .child(
                    div()
                        .text_size(THEME.type_scale.body_small)
                        .text_color(THEME.colors.muted)
                        .when(!has_target, |label| label.flex_1())
                        .child(item.label.clone()),
                )
                .when(has_target, |row| {
                    row.child(
                        technical_text(("tool-target", key), target)
                            .flex_1()
                            .min_w_0()
                            .text_color(THEME.colors.text),
                    )
                })
                .children(state.map(|state| {
                    div()
                        .flex_none()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child(state.glyph)
                })),
        )
        .when(expanded && presentation.is_none(), |tool| {
            tool.child(
                div()
                    .ml(px(22.0))
                    .mt(THEME.space.xs)
                    .child(expanded_tool_body(("tool-detail", key), item)),
            )
        })
        .when_some(presentation, |tool, presentation| {
            let source = presentation.clone();
            let expand_entity = entity.clone();
            let on_expand = Rc::new(move |window: &mut gpui::Window, cx: &mut gpui::App| {
                let opener = window.focused(cx);
                let _ = expand_entity.update(cx, |this, cx| {
                    this.open_tool_diff(source.clone(), opener, window, cx)
                });
            });
            tool.child(div().mt(THEME.space.xs).child(crate::tool_changes::render(
                presentation,
                item.tool_call_id.as_ref().map_or(0, |id| stable_key(id)),
                diff_mode,
                Some(on_expand),
            )))
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
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .text_color(if item.is_error {
            THEME.colors.error
        } else {
            THEME.colors.muted
        })
        .into_any_element()
}

fn transcript_disclosure_button(
    id: impl Into<gpui::ElementId>,
    expanded: bool,
    label: String,
    key: usize,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    disclosure_button(id, expanded, label, move |_, cx| {
        let _ = entity.update(cx, |this, cx| {
            this.set_transcript_item_expanded(key, !expanded, cx)
        });
    })
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

fn transcript_markdown_style() -> TextViewStyle {
    transcript_markdown_style_with_inline_code(HighlightStyle {
        color: Some(THEME.colors.code.into()),
        background_color: Some(THEME.colors.panel.into()),
        ..HighlightStyle::default()
    })
}

fn skill_transcript_markdown_style() -> TextViewStyle {
    transcript_markdown_style_with_inline_code(HighlightStyle {
        color: Some(THEME.colors.skill.into()),
        background_color: Some(THEME.colors.skill_surface.into()),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ToolState {
    glyph: &'static str,
    label: &'static str,
}

fn tool_state(running: bool, failed: usize, completed: bool) -> Option<ToolState> {
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
        TranscriptKind::Notice | TranscriptKind::Custom => THEME.colors.muted,
        TranscriptKind::User | TranscriptKind::Assistant => THEME.colors.text,
        TranscriptKind::Thinking | TranscriptKind::Tool => THEME.colors.subtle,
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
