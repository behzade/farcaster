use std::{
    collections::HashMap,
    hash::{Hash as _, Hasher as _},
    sync::Arc,
};

use super::{HistoryTail, TranscriptView};
use crate::{
    app::{ui::change_tree::ChangeTreeState, views::transcript::TranscriptRow},
    conversation::TranscriptItem,
    utility::persistent_vec::PersistentVec,
};

type Items = PersistentVec<Arc<TranscriptItem>>;

// Indices can change when a backend replaces history. Check the source item
// before restoring an anchor or an expanded detail at that index.
#[derive(Clone)]
struct SourceItem {
    index: usize,
    fingerprint: u64,
    prefix_chars: usize,
}

impl SourceItem {
    fn new(index: usize, items: &Items) -> Option<Self> {
        let item = items.get(index)?;
        let prefix_chars = text(item).take(128).count();
        Some(Self {
            index,
            fingerprint: fingerprint(item, prefix_chars),
            prefix_chars,
        })
    }

    fn matches(&self, items: &Items) -> bool {
        items
            .get(self.index)
            .is_some_and(|item| fingerprint(item, self.prefix_chars) == self.fingerprint)
    }
}

fn text(item: &TranscriptItem) -> impl Iterator<Item = char> + '_ {
    item.stream_chunks
        .iter()
        .flat_map(|chunk| chunk.chars())
        .chain(item.text.chars())
}

fn fingerprint(item: &TranscriptItem, prefix_chars: usize) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    std::mem::discriminant(&item.kind).hash(&mut hash);
    if let Some(id) = &item.tool_call_id {
        id.hash(&mut hash);
    } else {
        item.label.hash(&mut hash);
        // Keep identity stable when a long streaming message grows, without
        // keeping a copy of the message or its attachments.
        for character in text(item).take(prefix_chars) {
            character.hash(&mut hash);
        }
    }
    hash.finish()
}

#[derive(Clone)]
pub(super) struct ReadingState {
    // An index in a saved suffix has no absolute origin. Keep the shared items
    // only while there is a position or disclosure from that suffix to restore.
    suffix: Option<Items>,
    anchor: Option<ReadingAnchor>,
    disclosures: HashMap<usize, (SourceItem, bool)>,
    file_trees: HashMap<usize, (SourceItem, ChangeTreeState)>,
    last_count: usize,
    unseen: usize,
}

#[derive(Clone)]
pub(super) struct ReadingAnchor {
    row: TranscriptRow,
    offset: gpui::Pixels,
    source: SourceItem,
    text_start: usize,
}

fn text_start(row: TranscriptRow, items: &Items) -> usize {
    match row {
        TranscriptRow::MessageChunk { start, .. } => start,
        TranscriptRow::StreamChunk { index, chunk, .. } => items.get(index).map_or(0, |item| {
            item.stream_chunks
                .iter()
                .take(chunk)
                .map(|text| text.len())
                .sum()
        }),
        _ => 0,
    }
}

impl ReadingState {
    pub(super) fn source_offset(
        &self,
        coverage: crate::agents::HistoryCoverage,
        items: &Items,
    ) -> Option<usize> {
        if let Some(suffix) = &self.suffix {
            return unique_page_start(items, suffix);
        }
        (coverage == crate::agents::HistoryCoverage::Complete || self.allows_history_tail())
            .then_some(0)
    }

    pub(super) fn rebase(&mut self, start: usize, rows: &PersistentVec<TranscriptRow>) {
        let Some(suffix) = self.suffix.take() else {
            return;
        };
        // Only rows after the old suffix count as new.
        let end = start + suffix.len();
        self.last_count = rows.iter().filter(|row| row.key() < end).count();
        if start == 0 {
            return;
        }
        if let Some(anchor) = &mut self.anchor {
            anchor.source.index += start;
            anchor.row = anchor.row.at_item(anchor.source.index);
        }
        fn shift_keys<T>(
            map: &mut HashMap<usize, (SourceItem, T)>,
            start: usize,
            rows: &PersistentVec<TranscriptRow>,
        ) {
            *map = std::mem::take(map)
                .into_iter()
                .filter_map(|(key, (mut source, value))| {
                    let group = key != source.index;
                    source.index += start;
                    let key = if group {
                        rows.iter()
                            .find(|row| {
                                matches!(row, TranscriptRow::ActivityGroup { .. })
                                    && row.contains_disclosure_key(source.index)
                            })?
                            .disclosure_key()
                    } else {
                        source.index
                    };
                    Some((key, (source, value)))
                })
                .collect();
        }
        shift_keys(&mut self.disclosures, start, rows);
        shift_keys(&mut self.file_trees, start, rows);
    }

    pub(super) fn apply_to_local(&self, page: &mut HistoryTail) {
        let items = &page.presentation.items;
        page.disclosures = self
            .disclosures
            .iter()
            .filter(|(_, (source, _))| source.matches(items))
            .map(|(key, (_, expanded))| (*key, *expanded))
            .collect();
        page.trees = self
            .file_trees
            .iter()
            .filter(|(_, (source, _))| source.matches(items))
            .map(|(key, (_, tree))| (*key, tree.clone()))
            .collect();
        if let Some(offset) = self
            .anchor
            .as_ref()
            .and_then(|anchor| anchor.resolve(&page.rows, items))
        {
            page.list.restore_scroll(offset);
            page.unseen = self
                .unseen
                .saturating_add(page.rows.len().saturating_sub(self.last_count));
        }
    }

    pub(super) fn allows_history_tail(&self) -> bool {
        self.anchor.is_none() && self.disclosures.is_empty() && self.file_trees.is_empty()
    }

    pub(super) fn capture(view: &mut TranscriptView, items: &Items) -> Self {
        let anchor = view
            .take_pending_reading_anchor()
            .or_else(|| ReadingAnchor::capture(view, items));
        let source = |key| {
            SourceItem::new(
                if key < items.len() {
                    key
                } else {
                    usize::MAX - key
                },
                items,
            )
        };
        let mut saved = Self {
            suffix: None,
            anchor,
            disclosures: std::mem::take(&mut view.disclosure_states)
                .into_iter()
                .filter_map(|(key, value)| Some((key, (source(key)?, value))))
                .collect(),
            file_trees: std::mem::take(&mut view.file_trees)
                .into_iter()
                .filter_map(|(key, value)| Some((key, (source(key)?, value))))
                .collect(),
            last_count: view.last_count,
            unseen: view.unseen,
        };
        if view.coverage == Some(crate::agents::HistoryCoverage::Suffix)
            && !saved.allows_history_tail()
        {
            saved.suffix = Some(items.clone());
        }
        saved
    }

    pub(super) fn disclosure_states(&self) -> HashMap<usize, bool> {
        self.disclosures
            .iter()
            .map(|(key, (_, value))| (*key, *value))
            .collect()
    }

    pub(super) fn file_trees(&self) -> HashMap<usize, ChangeTreeState> {
        self.file_trees
            .iter()
            .map(|(key, (_, value))| (*key, value.clone()))
            .collect()
    }

    pub(super) fn follow_latest(&mut self) {
        self.anchor = None;
        self.unseen = 0;
        if self.allows_history_tail() {
            self.suffix = None;
        }
    }

    pub(super) fn restore(self, view: &mut TranscriptView, items: &Items) -> Option<ReadingAnchor> {
        view.disclosure_states = self
            .disclosures
            .into_iter()
            .filter(|(_, (source, _))| source.matches(items))
            .map(|(key, (_, value))| (key, value))
            .collect();
        view.file_trees = self
            .file_trees
            .into_iter()
            .filter(|(_, (source, _))| source.matches(items))
            .map(|(key, (_, value))| (key, value))
            .collect();
        // Expanded rows need fresh measurements before restoring an offset
        // inside them, even if a loading preview already measured the rows.
        view.list.remeasure_items(0..view.rows.len());
        if self.anchor.is_some() {
            view.unseen = self
                .unseen
                .saturating_add(view.rows.len().saturating_sub(self.last_count));
        }
        self.anchor
    }
}

impl ReadingAnchor {
    pub(super) fn capture(view: &TranscriptView, items: &Items) -> Option<Self> {
        if view.list.is_following_tail() {
            return None;
        }
        let offset = view.list.logical_scroll_top();
        let row = *view.rows.get(offset.item_ix)?;
        Self::at(row, offset.offset_in_item, items)
    }

    pub(super) fn at(row: TranscriptRow, offset: gpui::Pixels, items: &Items) -> Option<Self> {
        Some(Self {
            row,
            offset,
            source: SourceItem::new(row.key(), items)?,
            text_start: text_start(row, items),
        })
    }

    pub(super) fn resolve(
        &self,
        rows: &PersistentVec<TranscriptRow>,
        items: &Items,
    ) -> Option<gpui::ListOffset> {
        if !self.source.matches(items) {
            return None;
        }
        // Prepending earlier tools can move the start of the same activity group.
        let exact = rows.position(|candidate| {
            candidate.same_position(&self.row)
                || (matches!(candidate, TranscriptRow::ActivityGroup { .. })
                    && candidate.contains_disclosure_key(self.source.index))
        });
        // Finishing a response replaces streaming chunks with Markdown rows.
        // Keep the text position until paint, even across several row updates.
        let item_ix = exact.or_else(|| {
            rows.iter()
                .enumerate()
                .filter(|(_, row)| {
                    row.key() == self.source.index && text_start(**row, items) <= self.text_start
                })
                .map(|(index, _)| index)
                .last()
        })?;
        Some(gpui::ListOffset {
            item_ix,
            offset_in_item: if exact.is_some() {
                self.offset
            } else {
                gpui::px(0.0)
            },
        })
    }
}

impl TranscriptView {
    pub(crate) fn prepare_saved_history_handoff(&mut self, previous: &Items, next: &Items) {
        if self.coverage == Some(crate::agents::HistoryCoverage::Suffix)
            && self.list.has_pending_scroll()
            && let Some((_, following)) = self.list.take_history_handoff_position()
        {
            self.following = following;
            if following {
                self.unseen = 0;
            }
        }
        if self.coverage == Some(crate::agents::HistoryCoverage::Suffix)
            && (!self.list.is_following_tail()
                || !self.disclosure_states.is_empty()
                || !self.file_trees.is_empty())
            && let Some(session) = self.session.clone()
        {
            let saved = ReadingState::capture(self, previous);
            self.reading.insert((session, self.record.clone()), saved);
            return;
        }
        let Some(top) = self.list.painted_history_top() else {
            return;
        };
        if self.list.has_pending_scroll() || self.list.has_pending_restore() {
            return;
        }
        let Some(start) = unique_page_before_append(next, previous) else {
            return;
        };
        let Some(row) = self.rows.get(top.item_ix) else {
            return;
        };
        if let Some(anchor) =
            ReadingAnchor::at(row.at_item(start + row.key()), top.offset_in_item, next)
        {
            self.restore_reading_anchor(anchor, top);
        }
    }
}

impl HistoryTail {
    pub(super) fn anchor_before_append(
        &self,
        top: gpui::ListOffset,
        rows: &PersistentVec<TranscriptRow>,
        items: &Items,
    ) -> Option<(ReadingAnchor, gpui::ListOffset, usize)> {
        let page = &self.presentation.items;
        // An earlier review in the same turn can be positioned after the
        // visible reply even when the provider returned identical final items.
        let (start, unseen) = if let Some(start) = unique_page_before_append(items, page) {
            let end = start + page.len();
            (start, rows.iter().filter(|row| row.key() >= end).count())
        } else {
            let start = items.len().checked_sub(page.len())?;
            let last = self.rows.last()?;
            let last = last.at_item(start + last.key());
            if rows.last()?.same_position(&last) || !matches_page_at(items, start, page) {
                return None;
            }
            let position = rows.position(|row| row.same_position(&last))?;
            (start, rows.len() - position - 1)
        };
        let top = self.top_in(top, rows, start)?;
        Some((
            ReadingAnchor::at(rows[top.item_ix], top.offset_in_item, items)?,
            top,
            unseen,
        ))
    }

    pub(super) fn matching_suffix_top(
        &self,
        top: gpui::ListOffset,
        rows: &PersistentVec<TranscriptRow>,
        items: &Items,
    ) -> Option<gpui::ListOffset> {
        let page = &self.presentation.items;
        let start = items.len().checked_sub(page.len())?;
        let last = self.rows.last()?;
        if !rows.last()?.same_position(&last.at_item(start + last.key()))
            || !matches_page_at(items, start, page)
        {
            return None;
        }
        self.top_in(top, rows, start)
    }

    fn top_in(
        &self,
        top: gpui::ListOffset,
        rows: &PersistentVec<TranscriptRow>,
        start: usize,
    ) -> Option<gpui::ListOffset> {
        let row = self.rows.get(top.item_ix)?;
        let expected = row.at_item(start + row.key());
        let item_ix = rows.position(|row| row.same_position(&expected))?;
        let actual = rows[item_ix];
        if let (
            TranscriptRow::ActivityGroup { len: before, .. },
            TranscriptRow::ActivityGroup { len: after, .. },
        ) = (expected, actual)
            && before != after
        {
            return None;
        }
        Some(gpui::ListOffset { item_ix, ..top })
    }
}

fn matches_page_at(items: &Items, start: usize, page: &Items) -> bool {
    items
        .iter_range(start..items.len())
        .zip(page.iter())
        .all(|(full, preview)| crate::conversation::viewport_item_matches(preview, full))
}

fn unique_page_before_append(items: &Items, page: &Items) -> Option<usize> {
    let last_start = items.len().checked_sub(page.len())?;
    if page.is_empty() || last_start == 0 || matches_page_at(items, last_start, page) {
        return None;
    }
    unique_page_start(items, page)
}

fn unique_page_start(items: &Items, page: &Items) -> Option<usize> {
    if page.is_empty() || page.len() > items.len() {
        return None;
    }
    if page.len() == items.len() {
        return (items.shares_storage(page) || matches_page_at(items, 0, page)).then_some(0);
    }
    // Match whole items, including tools and attachments. A prefix table keeps
    // repeated turns linear and lets us detect overlapping copies of the page.
    let mut prefix = vec![0; page.len()];
    let mut matched = 0;
    for i in 1..page.len() {
        while matched > 0 && page[matched] != page[i] {
            matched = prefix[matched - 1];
        }
        matched += usize::from(page[matched] == page[i]);
        prefix[i] = matched;
    }
    matched = 0;
    let mut start = None;
    for (index, item) in items.iter().enumerate() {
        while matched > 0 && !crate::conversation::viewport_item_matches(&page[matched], item) {
            matched = prefix[matched - 1];
        }
        matched += usize::from(crate::conversation::viewport_item_matches(
            &page[matched],
            item,
        ));
        if matched == page.len() {
            if start.replace(index + 1 - matched).is_some() {
                return None;
            }
            matched = prefix[matched - 1];
        }
    }
    start
}

#[cfg(test)]
#[path = "transcript_reading_tests.rs"]
mod tests;
