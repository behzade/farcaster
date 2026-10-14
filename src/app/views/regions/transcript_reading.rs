use std::{
    collections::HashMap,
    hash::{Hash as _, Hasher as _},
    sync::Arc,
};

use super::TranscriptView;
use crate::{
    app::{ui::change_tree::ChangeTreeState, views::transcript::TranscriptRow},
    conversation::TranscriptItem,
    utility::persistent_vec::PersistentVec,
};

type Items = PersistentVec<Arc<TranscriptItem>>;

// Indices can change when a backend replaces history. Check the source item
// before restoring an anchor or an expanded detail at that index.
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

pub(super) struct ReadingState {
    anchor: Option<ReadingAnchor>,
    disclosures: HashMap<usize, (SourceItem, bool)>,
    file_trees: HashMap<usize, (SourceItem, ChangeTreeState)>,
    last_count: usize,
    unseen: usize,
}

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
        Self {
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
        }
    }

    pub(super) fn follow_latest(&mut self) {
        self.anchor = None;
        self.unseen = 0;
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
        Some(Self {
            row,
            offset: offset.offset_in_item,
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
        let exact = rows.position(|candidate| candidate.same_position(&self.row));
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

#[cfg(test)]
#[path = "transcript_reading_tests.rs"]
mod tests;
