//! Selectable, compact transcript projection.

use std::{
    borrow::Cow,
    hash::{Hash, Hasher},
    rc::Rc,
    sync::Arc,
};

use gpui::{
    AnyElement, Entity, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement as _,
    Overflow, ParentElement as _, Pixels, StatefulInteractiveElement as _, StyleRefinement,
    Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    highlighter::HighlightTheme,
    text::{TextView, TextViewState, TextViewStyle},
};

use crate::{
    app::PiApp,
    conversation::{TranscriptItem, TranscriptKind},
    persistent_vec::{Indexed, PersistentVec},
    primitives::{ButtonTone, button, disclosure_button, disclosure_indicator},
    theme::{MONO_FONT_FAMILY, THEME},
    tool_changes::EmbeddedDiffMode,
    transcript_list::{TranscriptListState, transcript_list_grouped},
    transcript_markdown::{MarkdownStateKey, TranscriptMarkdownCache},
};

const MARKDOWN_CHUNK_TARGET_BYTES: usize = 2 * 1024;
const MARKDOWN_CHUNK_HARD_BYTES: usize = 8 * 1024;
pub(crate) const TRANSCRIPT_ROW_HEIGHT_HINT: Pixels = px(24.0);

pub(crate) fn tail_reserve(viewport_height: Pixels) -> Pixels {
    px((f32::from(viewport_height) * 0.32).clamp(72.0, 280.0))
}

pub(crate) fn estimated_row_height(
    row: TranscriptRow,
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
) -> Pixels {
    let item = |index| items.get(index).expect("transcript row item should exist");
    let text = match row {
        TranscriptRow::MessageChunk {
            index, start, end, ..
        } => Some(&item(index).text[start..end]),
        TranscriptRow::StreamChunk { index, chunk, .. } => Some(
            item(index)
                .stream_chunks
                .get(chunk)
                .map_or(item(index).text.as_str(), |chunk| chunk.as_ref()),
        ),
        TranscriptRow::Item { index, .. }
            if matches!(
                item(index).kind,
                TranscriptKind::User | TranscriptKind::Assistant | TranscriptKind::Custom
            ) && item(index).invocation.is_none() =>
        {
            Some(item(index).text.as_str())
        }
        TranscriptRow::Item { .. } | TranscriptRow::ReadGroup { .. } => None,
    };
    let Some(text) = text else {
        return TRANSCRIPT_ROW_HEIGHT_HINT;
    };

    let visual_lines = text
        .lines()
        .map(|line| line.chars().count().max(1).div_ceil(88))
        .sum::<usize>()
        .max(1);
    px((visual_lines.min(320) as f32).mul_add(20.0, 36.0))
}

#[derive(Clone, Copy)]
pub(crate) struct TranscriptViewport {
    pub(crate) following: bool,
    pub(crate) unseen: usize,
    pub(crate) tail_reserve: Pixels,
    pub(crate) diff_mode: EmbeddedDiffMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MarkdownFence {
    opening_start: usize,
    opening_end: usize,
    marker: char,
    marker_len: usize,
    indent_len: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FenceContinuation {
    fence: MarkdownFence,
    prepend: bool,
    append: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MarkdownChunk {
    start: usize,
    end: usize,
    fence: Option<FenceContinuation>,
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
        fence: Option<FenceContinuation>,
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

pub(crate) fn project_rows(
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
) -> PersistentVec<TranscriptRow> {
    project_rows_from(items, 0)
}

pub(crate) fn update_rows(
    previous_rows: &PersistentVec<TranscriptRow>,
    previous_items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
) -> PersistentVec<TranscriptRow> {
    update_rows_from(previous_rows, previous_items, items, None)
}

pub(crate) fn update_rows_from(
    previous_rows: &PersistentVec<TranscriptRow>,
    previous_items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    changed_from: Option<usize>,
) -> PersistentVec<TranscriptRow> {
    update_rows_incremental(previous_rows, previous_items, items, changed_from)
        .rows
        .unwrap_or_else(|| previous_rows.clone())
}

pub(crate) struct TranscriptRowUpdate {
    rows: Option<PersistentVec<TranscriptRow>>,
    unchanged_prefix_rows: usize,
}

impl TranscriptRowUpdate {
    pub(crate) fn replace(rows: PersistentVec<TranscriptRow>) -> Self {
        Self {
            rows: Some(rows),
            unchanged_prefix_rows: 0,
        }
    }

    pub(crate) fn row_count(&self, current: usize) -> usize {
        self.rows.as_ref().map_or(current, PersistentVec::len)
    }

    pub(crate) fn apply(
        self,
        list: &TranscriptListState,
        current: &mut Arc<PersistentVec<TranscriptRow>>,
        items: &PersistentVec<Arc<TranscriptItem>>,
    ) -> bool {
        let Some(rows) = self.rows else {
            return false;
        };
        sync_transcript_list(list, current, items, rows, self.unchanged_prefix_rows);
        true
    }
}

pub(crate) fn update_rows_incremental(
    previous_rows: &PersistentVec<TranscriptRow>,
    previous_items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    changed_from: Option<usize>,
) -> TranscriptRowUpdate {
    let unchanged_hint = changed_from
        .unwrap_or_default()
        .min(previous_items.len())
        .min(items.len());
    let projected_items = previous_rows
        .last()
        .map_or(0, TranscriptRow::item_end)
        .min(previous_items.len());
    let (matching_items, compared_items) =
        matching_item_prefix_from(previous_items, items, unchanged_hint);
    crate::performance::count_transcript_comparisons(compared_items);
    let unchanged_items = (unchanged_hint + matching_items).min(projected_items);
    if unchanged_items == previous_items.len()
        && unchanged_items == items.len()
        && (items.is_empty() || !previous_rows.is_empty())
    {
        return TranscriptRowUpdate {
            rows: None,
            unchanged_prefix_rows: previous_rows.len(),
        };
    }

    let mut keep_rows = previous_rows.partition_point(|row| row.item_end() <= unchanged_items);
    let mut project_from = previous_rows
        .get(keep_rows)
        .map_or(unchanged_items, TranscriptRow::item_start);
    if project_from == unchanged_items
        && unchanged_items < items.len()
        && items.get(unchanged_items).is_some_and(|item| is_read(item))
        && let Some(TranscriptRow::ReadGroup { start, len, .. }) = keep_rows
            .checked_sub(1)
            .and_then(|index| previous_rows.get(index))
        && start + len == unchanged_items
    {
        keep_rows -= 1;
        project_from = *start;
    }

    let projected = project_rows_from(items, project_from);
    let mut rows = previous_rows.clone();
    rows.splice(keep_rows..previous_rows.len(), projected.iter().copied());
    TranscriptRowUpdate {
        rows: Some(rows),
        unchanged_prefix_rows: keep_rows,
    }
}

fn sync_transcript_list(
    list: &TranscriptListState,
    current: &mut Arc<PersistentVec<TranscriptRow>>,
    items: &PersistentVec<Arc<TranscriptItem>>,
    next: PersistentVec<TranscriptRow>,
    unchanged_prefix_rows: usize,
) {
    let _timing = crate::performance::Timing::new("transcript.sync_rows");
    let unchanged_prefix_rows = unchanged_prefix_rows.min(current.len()).min(next.len());
    let positions_unchanged = current.len() == next.len()
        && (unchanged_prefix_rows..current.len())
            .all(|index| current[index].same_position(&next[index]));
    if positions_unchanged {
        if let Some(first) =
            (unchanged_prefix_rows..current.len()).find(|&index| current[index] != next[index])
        {
            let last = (first..current.len())
                .rev()
                .find(|&index| current[index] != next[index])
                .unwrap_or(first);
            crate::performance::count_remeasured_rows(last + 1 - first);
            list.remeasure_items(first..last + 1);
        }
    } else if let Some((old_range, new_count)) =
        transcript_splice_from(current.as_ref(), &next, unchanged_prefix_rows)
    {
        let anchor = (!list.is_following_tail()).then(|| {
            let offset = list.logical_scroll_top();
            current
                .get(offset.item_ix)
                .copied()
                .map(|row| (row, offset.offset_in_item))
        });
        let new_start = old_range.start;
        list.splice_with_size_hints(
            old_range,
            next.iter()
                .skip(new_start)
                .take(new_count)
                .map(|row| estimated_row_height(*row, items)),
        );
        if let Some(Some((anchored_row, offset_in_item))) = anchor
            && let Some(item_ix) = next.position(|row| row.same_position(&anchored_row))
        {
            list.scroll_to(gpui::ListOffset {
                item_ix,
                offset_in_item,
            });
        }
    }
    *current = Arc::new(next);
}

#[cfg(test)]
pub(crate) fn transcript_splice<T: PartialEq>(
    current: &[T],
    next: &[T],
) -> Option<(std::ops::Range<usize>, usize)> {
    transcript_splice_from(current, next, 0)
}

fn transcript_splice_from<T: PartialEq>(
    current: &(impl Indexed<T> + ?Sized),
    next: &(impl Indexed<T> + ?Sized),
    unchanged_prefix: usize,
) -> Option<(std::ops::Range<usize>, usize)> {
    let mut prefix = unchanged_prefix.min(current.len()).min(next.len());
    while prefix < current.len() && prefix < next.len() && current.get(prefix) == next.get(prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < current.len().saturating_sub(prefix)
        && suffix < next.len().saturating_sub(prefix)
        && current.get(current.len() - 1 - suffix) == next.get(next.len() - 1 - suffix)
    {
        suffix += 1;
    }
    let old_end = current.len().saturating_sub(suffix);
    let new_count = next.len().saturating_sub(prefix + suffix);
    (prefix != old_end || new_count != 0).then_some((prefix..old_end, new_count))
}

#[cfg(test)]
fn matching_item_prefix(
    previous_items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
) -> (usize, usize) {
    matching_item_prefix_from(previous_items, items, 0)
}

fn matching_item_prefix_from(
    previous_items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    start: usize,
) -> (usize, usize) {
    let mut matching = 0;
    let pair_count = previous_items.len().min(items.len()).saturating_sub(start);
    while matching < pair_count {
        let previous = previous_items
            .get(start + matching)
            .expect("matching transcript item should exist");
        let next = items
            .get(start + matching)
            .expect("matching transcript item should exist");
        if !Arc::ptr_eq(previous, next) && previous.as_ref() != next.as_ref() {
            return (matching, matching + 1);
        }
        matching += 1;
    }
    (matching, matching)
}

fn project_rows_from(
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    mut index: usize,
) -> PersistentVec<TranscriptRow> {
    crate::performance::count_transcript_projections(items.len().saturating_sub(index));
    let mut rows = PersistentVec::default();
    while index < items.len() {
        let item = items
            .get(index)
            .expect("projected transcript item should exist");
        if is_read(item) {
            let start = index;
            while index < items.len() && items.get(index).is_some_and(|item| is_read(item)) {
                index += 1;
            }
            rows.push(TranscriptRow::ReadGroup {
                start,
                len: index - start,
                revision: item_revision(items, start..index),
            });
            continue;
        }
        if item.kind == TranscriptKind::Assistant && item.streaming {
            let chunk_count = item.stream_chunks.len() + usize::from(!item.text.is_empty());
            rows.extend((0..chunk_count).map(|chunk| {
                let text = item
                    .stream_chunks
                    .get(chunk)
                    .map_or(item.text.as_str(), |chunk| chunk.as_ref());
                TranscriptRow::StreamChunk {
                    index,
                    chunk,
                    revision: text_revision(text),
                    first: chunk == 0,
                    last: chunk + 1 == chunk_count,
                }
            }));
        } else if matches!(item.kind, TranscriptKind::User | TranscriptKind::Assistant)
            && item.invocation.is_none()
            && (markdown_needs_chunks(&item.text)
                || (item.streaming && item.text.len() > MARKDOWN_CHUNK_TARGET_BYTES))
        {
            let chunks = markdown_chunks(&item.text);
            let last_block = chunks.len().saturating_sub(1);
            rows.extend(chunks.into_iter().enumerate().map(|(block, chunk)| {
                TranscriptRow::MessageChunk {
                    index,
                    start: chunk.start,
                    end: chunk.end,
                    block,
                    revision: text_revision(&markdown_chunk_text(&item.text, chunk)),
                    first: block == 0,
                    last: block == last_block,
                    fence: chunk.fence,
                }
            }));
        } else {
            rows.push(TranscriptRow::Item {
                index,
                revision: item_revision(items, index..index + 1),
            });
        }
        index += 1;
    }
    rows
}

fn item_revision(
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    range: std::ops::Range<usize>,
) -> usize {
    range.fold(0, |revision, index| {
        let item = items
            .get(index)
            .expect("revision transcript item should exist");
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

fn markdown_needs_chunks(text: &str) -> bool {
    text.len() > MARKDOWN_CHUNK_HARD_BYTES || text.lines().take(65).count() > 64
}

fn markdown_chunks(text: &str) -> Vec<MarkdownChunk> {
    let mut chunks = Vec::new();
    let mut outside_start = 0;
    let mut offset = 0;
    let mut fence = None;
    for line in text.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        if let Some(opening) = fence {
            if markdown_fence_closes(line, opening) {
                append_fenced_markdown_chunks(text, opening, line_start, offset, true, &mut chunks);
                fence = None;
                outside_start = offset;
            }
        } else if let Some(opening) = markdown_fence(line, line_start, offset) {
            append_plain_markdown_chunks(text, outside_start, line_start, &mut chunks);
            fence = Some(opening);
        }
    }

    if let Some(opening) = fence {
        append_fenced_markdown_chunks(text, opening, text.len(), text.len(), false, &mut chunks);
    } else {
        append_plain_markdown_chunks(text, outside_start, text.len(), &mut chunks);
    }
    if chunks.is_empty() {
        chunks.push(plain_markdown_chunk(0, text.len()));
    }
    chunks
}

fn plain_markdown_chunk(start: usize, end: usize) -> MarkdownChunk {
    MarkdownChunk {
        start,
        end,
        fence: None,
    }
}

fn append_plain_markdown_chunks(
    text: &str,
    mut start: usize,
    end: usize,
    chunks: &mut Vec<MarkdownChunk>,
) {
    if start >= end {
        return;
    }
    let mut line_end = start;
    for line in text[start..end].split_inclusive('\n') {
        line_end += line.len();
        while line_end - start >= MARKDOWN_CHUNK_HARD_BYTES {
            let split = hard_markdown_break(text, start, start + MARKDOWN_CHUNK_HARD_BYTES);
            chunks.push(plain_markdown_chunk(start, split));
            start = split;
        }
        if line_end - start >= MARKDOWN_CHUNK_TARGET_BYTES && line.trim().is_empty() {
            chunks.push(plain_markdown_chunk(start, line_end));
            start = line_end;
        }
    }
    if start < end {
        chunks.push(plain_markdown_chunk(start, end));
    }
}

fn append_fenced_markdown_chunks(
    text: &str,
    fence: MarkdownFence,
    closing_start: usize,
    fence_end: usize,
    closed: bool,
    chunks: &mut Vec<MarkdownChunk>,
) {
    const FENCED_CHUNK_LINES: usize = 64;
    if fence_end - fence.opening_start <= MARKDOWN_CHUNK_HARD_BYTES
        && text[fence.opening_end..closing_start].lines().count() <= FENCED_CHUNK_LINES
    {
        chunks.push(plain_markdown_chunk(fence.opening_start, fence_end));
        return;
    }

    let mut body = Vec::new();
    let mut start = fence.opening_end;
    let mut end = start;
    let mut lines = 0;
    for line in text[start..closing_start].split_inclusive('\n') {
        end += line.len();
        lines += 1;
        if lines >= FENCED_CHUNK_LINES || end - start >= MARKDOWN_CHUNK_TARGET_BYTES {
            body.push(plain_markdown_chunk(start, end));
            start = end;
            lines = 0;
        }
    }
    if start < closing_start {
        body.push(plain_markdown_chunk(start, closing_start));
    }
    if body.is_empty() {
        chunks.push(plain_markdown_chunk(fence.opening_start, fence_end));
        return;
    }

    let last = body.len() - 1;
    for (index, body_chunk) in body.into_iter().enumerate() {
        let first = index == 0;
        let final_chunk = index == last;
        chunks.push(MarkdownChunk {
            start: if first {
                fence.opening_start
            } else {
                body_chunk.start
            },
            end: if final_chunk && closed {
                fence_end
            } else {
                body_chunk.end
            },
            fence: Some(FenceContinuation {
                fence,
                prepend: !first,
                append: !final_chunk || !closed,
            }),
        });
    }
}

fn markdown_chunk_text(text: &str, chunk: MarkdownChunk) -> Cow<'_, str> {
    let Some(continuation) = chunk.fence else {
        return Cow::Borrowed(&text[chunk.start..chunk.end]);
    };
    let fence = continuation.fence;
    let mut rendered = String::with_capacity(
        chunk.end - chunk.start + fence.opening_end - fence.opening_start + fence.marker_len + 2,
    );
    if continuation.prepend {
        rendered.push_str(&text[fence.opening_start..fence.opening_end]);
    }
    rendered.push_str(&text[chunk.start..chunk.end]);
    if continuation.append {
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        rendered.push_str(&text[fence.opening_start..fence.opening_start + fence.indent_len]);
        rendered.extend(std::iter::repeat_n(fence.marker, fence.marker_len));
        rendered.push('\n');
    }
    Cow::Owned(rendered)
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

fn markdown_fence(line: &str, opening_start: usize, opening_end: usize) -> Option<MarkdownFence> {
    let indent_len = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent_len > 3 {
        return None;
    }
    let trimmed = &line[indent_len..];
    let marker = trimmed.chars().next()?;
    let marker_len = trimmed.chars().take_while(|char| *char == marker).count();
    ((marker == '`' || marker == '~') && marker_len >= 3).then_some(MarkdownFence {
        opening_start,
        opening_end,
        marker,
        marker_len,
        indent_len,
    })
}

fn markdown_fence_closes(line: &str, fence: MarkdownFence) -> bool {
    let indent_len = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent_len > 3 {
        return false;
    }
    let trimmed = line[indent_len..].trim_end();
    let run = trimmed
        .chars()
        .take_while(|character| *character == fence.marker)
        .count();
    run >= fence.marker_len && trimmed.chars().skip(run).all(char::is_whitespace)
}

fn expanded_by_default(
    _row: TranscriptRow,
    _items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
) -> bool {
    false
}

fn resolved_expanded(
    row: TranscriptRow,
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    disclosure_states: &std::collections::HashMap<usize, bool>,
) -> bool {
    disclosure_states
        .get(&row.key())
        .copied()
        .unwrap_or_else(|| expanded_by_default(row, items))
}

fn message_follows_tool(
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

fn copy_transcript_items(
    items: &PersistentVec<Arc<TranscriptItem>>,
    range: std::ops::RangeInclusive<usize>,
) -> String {
    range
        .filter_map(|index| items.get(index))
        .map(|item| {
            let text = item.complete_text();
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
    conversation: Arc<crate::conversation::ConversationState>,
    disclosure_states: std::collections::HashMap<usize, bool>,
    markdown_cache: TranscriptMarkdownCache,
    entity: WeakEntity<PiApp>,
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
            let _timing = crate::performance::OperationTiming::new(
                crate::performance::OperationKind::TranscriptRow,
                1,
            );
            let Some(row) = rows.get(index).copied() else {
                return div().into_any_element();
            };
            let expanded = resolved_expanded(row, &conversation.items, &disclosure_states);
            let reserves_tail = index + 1 == rows.len()
                && latest_allows_tail_reserve(row, &conversation.items, expanded);
            div()
                .w_full()
                .when(reserves_tail, |row| row.pb(viewport.tail_reserve))
                .child(
                    div()
                        .w_full()
                        .when(selection_state.selection_contains(row.key()), |row| {
                            row.bg(THEME.colors.selection)
                        })
                        .child(render_row(
                            row,
                            &conversation.items,
                            expanded,
                            viewport.diff_mode,
                            &markdown_cache,
                            row_entity.clone(),
                            cx,
                        )),
                )
                .into_any_element()
        },
    );

    div()
        .size_full()
        .when(visual_selection_active, |root| {
            root.key_context(crate::transcript_list::TRANSCRIPT_SELECTION_KEY_CONTEXT)
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

fn latest_allows_tail_reserve(
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
                    TranscriptKind::Tool
                        | TranscriptKind::Thinking
                        | TranscriptKind::Error
                        | TranscriptKind::AgentResult
                )
        }
        TranscriptRow::ReadGroup { .. } => !expanded,
    }
}

fn render_row(
    row: TranscriptRow,
    items: &PersistentVec<Arc<TranscriptItem>>,
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
            let resolved = invocation_resolution(&items[index]);
            let markdown = highlighted_invocation_markdown(&items[index].text, resolved);
            render_message(
                key,
                &items[index],
                follows_tool,
                Some(markdown_cache.state(MarkdownStateKey::item(index, revision), &markdown, cx)),
                Some(invocation_transcript_markdown_style(resolved)),
            )
        }
        TranscriptRow::Item { index, .. } if items[index].invocation.is_some() => {
            render_invocation(key, &items[index], expanded, entity)
        }
        TranscriptRow::Item { index, .. } if items[index].kind == TranscriptKind::Tool => {
            render_tool(key, &items[index], expanded, diff_mode, entity)
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
                TranscriptKind::User | TranscriptKind::Assistant
            )
            .then(|| {
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

fn is_mixed_invocation_message(display: &str, resolved: &str) -> bool {
    display.split_whitespace().any(|token| {
        if resolved_contains_skill(resolved) {
            !is_invocation_token(token)
        } else {
            !is_prompt_invocation_token(token)
        }
    })
}

fn is_invocation_token(token: &str) -> bool {
    token.strip_prefix('$').is_some_and(is_invocation_name)
}

fn is_prompt_invocation_token(token: &str) -> bool {
    token.strip_prefix('$').is_some_and(|name| {
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

fn highlighted_invocation_markdown(display: &str, resolved: &str) -> String {
    let skill_names = skill_names(resolved);
    display
        .split_inclusive(char::is_whitespace)
        .map(|chunk| {
            let token = chunk.trim_end_matches(char::is_whitespace);
            let whitespace = &chunk[token.len()..];
            let name = token.strip_prefix('$').unwrap_or_default();
            let bare_name = name.strip_prefix("skill:").unwrap_or(name);
            let recognized = if resolved_contains_skill(resolved) {
                skill_names.contains(&bare_name)
            } else {
                is_prompt_invocation_token(token)
            };
            if recognized {
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
    markdown_style: Option<TextViewStyle>,
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

fn render_agent_result(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    markdown_state: Option<Entity<TextViewState>>,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let summary = item
        .text
        .lines()
        .next()
        .filter(|line| !line.trim().is_empty())
        .unwrap_or("Subagent finished")
        .chars()
        .take(160)
        .collect::<String>();
    div()
        .id(("agent-result-row", key))
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
                    ("agent-result-toggle", key),
                    expanded,
                    format!("subagent result details for {summary}"),
                    key,
                    entity,
                ))
                .child(
                    div()
                        .text_size(THEME.type_scale.body_small)
                        .text_color(THEME.colors.muted)
                        .child(item.label.clone()),
                )
                .child(
                    technical_text(("agent-result-summary", key), summary)
                        .flex_1()
                        .min_w_0()
                        .text_color(THEME.colors.text),
                ),
        )
        .when_some(markdown_state, |row, state| {
            row.child(
                div()
                    .id(("agent-result-detail-scroll", key))
                    .ml(px(22.0))
                    .mt(THEME.space.xs)
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

fn render_error(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let has_details = !item.tool_output.is_empty();
    div()
        .id(("error-row", key))
        .w_full()
        .px(THEME.space.md)
        .py(THEME.space.sm)
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_start()
                .gap(THEME.space.xs)
                .when(has_details, |row| {
                    row.child(transcript_disclosure_button(
                        ("error-toggle", key),
                        expanded,
                        format!("technical details for {}", item.label),
                        key,
                        entity,
                    ))
                })
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
                div().ml(px(22.0)).mt(THEME.space.xs).child(
                    technical_text(("error-details", key), fenced_text(&item.tool_output))
                        .text_color(THEME.colors.muted),
                ),
            )
        })
        .into_any_element()
}

fn message_role_label(kind: TranscriptKind) -> Option<&'static str> {
    match kind {
        TranscriptKind::User => Some("You"),
        TranscriptKind::Assistant => Some("Pi"),
        TranscriptKind::Thinking
        | TranscriptKind::Tool
        | TranscriptKind::Error
        | TranscriptKind::Notice
        | TranscriptKind::Custom
        | TranscriptKind::AgentResult => None,
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
        let _timing = crate::performance::OperationTiming::new(
            crate::performance::OperationKind::ThinkingAssembly,
            item.stream_chunks.len(),
        );
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
    items: &PersistentVec<Arc<TranscriptItem>>,
    start: usize,
    len: usize,
    expanded: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let group_items = || items.iter_range(start..start + len);
    let failed = group_items().filter(|item| item.is_error).count();
    let running = group_items().filter(|item| item.streaming).count();
    let target = (len == 1).then(|| tool_target(&items[start].text));
    let has_target = target.as_ref().is_some_and(|target| !target.is_empty());
    let summary = if len == 1 {
        "Read".to_owned()
    } else {
        format!("Read {len} files")
    };
    let completed = group_items()
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
                    .children(group_items().enumerate().map(|(index, item)| {
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

fn invocation_transcript_markdown_style(resolved: &str) -> TextViewStyle {
    let skill = resolved_contains_skill(resolved);
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
        TranscriptKind::Notice | TranscriptKind::Custom | TranscriptKind::AgentResult => {
            THEME.colors.muted
        }
        TranscriptKind::User | TranscriptKind::Assistant => THEME.colors.text,
        TranscriptKind::Thinking | TranscriptKind::Tool => THEME.colors.subtle,
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
