use super::*;

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
        TranscriptRow::Item { .. } | TranscriptRow::ActivityGroup { .. } => None,
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
        + if !item(row.item_start()).has_attachments() {
            px(0.0)
        } else {
            ATTACHMENT_ROW_HEIGHT
        }
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
    ActivityGroup {
        start: usize,
        len: usize,
        revision: usize,
    },
}

impl TranscriptRow {
    pub(crate) fn key(&self) -> usize {
        self.item_start()
    }

    /// Groups and their first child must have independent disclosure state.
    pub(crate) fn disclosure_key(&self) -> usize {
        match self {
            Self::ActivityGroup { start, .. } => usize::MAX - start,
            _ => self.key(),
        }
    }

    pub(crate) fn contains_disclosure_key(&self, key: usize) -> bool {
        self.disclosure_key() == key || (self.item_start()..self.item_end()).contains(&key)
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
                Self::ActivityGroup {
                    start: left_start,
                    len: left_len,
                    ..
                },
                Self::ActivityGroup {
                    start: right_start,
                    len: right_len,
                    ..
                },
            ) => left_start == right_start && left_len == right_len,
            _ => false,
        }
    }

    pub(super) fn item_start(&self) -> usize {
        match self {
            Self::Item { index, .. }
            | Self::MessageChunk { index, .. }
            | Self::StreamChunk { index, .. } => *index,
            Self::ActivityGroup { start, .. } => *start,
        }
    }

    pub(super) fn item_end(&self) -> usize {
        match self {
            Self::Item { index, .. }
            | Self::MessageChunk { index, .. }
            | Self::StreamChunk { index, .. } => index + 1,
            Self::ActivityGroup { start, len, .. } => start + len,
        }
    }
}

pub(crate) fn project_rows(
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
) -> PersistentVec<TranscriptRow> {
    project_rows_from(items, 0)
}

#[cfg(test)]
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
    pub(in crate::app::views::transcript) rows: Option<PersistentVec<TranscriptRow>>,
    pub(in crate::app::views::transcript) unchanged_prefix_rows: usize,
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
    crate::app::infrastructure::performance::count_transcript_comparisons(compared_items);
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
    // A newly completed call can join the previous group (or a standalone
    // thinking/tool row). Reproject that boundary, not just the changed item.
    if items
        .get(project_from)
        .is_some_and(|item| is_routine_activity(item))
    {
        while let Some(previous) = keep_rows.checked_sub(1).and_then(|i| previous_rows.get(i)) {
            if previous.item_end() != project_from
                || !items
                    .get(previous.item_start())
                    .is_some_and(|item| is_routine_activity(item))
            {
                break;
            }
            keep_rows -= 1;
            project_from = previous.item_start();
        }
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
    let _timing = crate::app::infrastructure::performance::Timing::new("transcript.sync_rows");
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
            crate::app::infrastructure::performance::count_remeasured_rows(last + 1 - first);
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
pub(in crate::app::views::transcript) fn matching_item_prefix(
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
    crate::app::infrastructure::performance::count_transcript_projections(
        items.len().saturating_sub(index),
    );
    let mut rows = PersistentVec::default();
    while index < items.len() {
        let item = items
            .get(index)
            .expect("projected transcript item should exist");
        if is_routine_activity(item) {
            let start = index;
            let mut end = start;
            let mut has_tool = false;
            while let Some(next) = items.get(end).filter(|next| is_routine_activity(next)) {
                has_tool |= next.kind == TranscriptKind::Tool;
                end += 1;
            }
            if !has_tool && end - start > 1 {
                rows.extend((start..end).map(|index| TranscriptRow::Item {
                    index,
                    revision: item_revision(items, index..index + 1),
                }));
                index = end;
                continue;
            }
            if has_tool && end - start > 1 {
                rows.push(TranscriptRow::ActivityGroup {
                    start,
                    len: end - start,
                    revision: item_revision(items, start..end),
                });
                index = end;
                continue;
            }
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

fn is_routine_activity(item: &TranscriptItem) -> bool {
    use conversation::{ToolExecutionState, ToolReviewState};
    matches!(item.kind, TranscriptKind::Tool | TranscriptKind::Thinking)
        && !item.streaming
        && !item.is_error
        && !item.tool_review.as_ref().is_some_and(|review| {
            matches!(
                review.state,
                ToolReviewState::Reviewing | ToolReviewState::Blocked
            )
        })
        && !item.tool_details.as_ref().is_some_and(|details| {
            matches!(
                details.state,
                ToolExecutionState::Pending
                    | ToolExecutionState::Running
                    | ToolExecutionState::Failed
            )
        })
}
