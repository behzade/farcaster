use std::{cell::RefCell, collections::HashMap, hash::Hash, rc::Rc};

use gpui::{AppContext as _, Entity};
use gpui_component::text::TextViewState;

const MAX_CACHED_MARKDOWN_ROWS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MarkdownRowKind {
    Item,
    MessageChunk,
    StreamChunk,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MarkdownStateKey {
    item: usize,
    block: usize,
    revision: usize,
    kind: MarkdownRowKind,
}

impl MarkdownStateKey {
    pub(crate) fn item(item: usize, revision: usize) -> Self {
        Self {
            item,
            block: 0,
            revision,
            kind: MarkdownRowKind::Item,
        }
    }

    pub(crate) fn message_chunk(item: usize, block: usize, revision: usize) -> Self {
        Self {
            item,
            block,
            revision,
            kind: MarkdownRowKind::MessageChunk,
        }
    }

    pub(crate) fn stream_chunk(item: usize, block: usize, revision: usize) -> Self {
        Self {
            item,
            block,
            revision,
            kind: MarkdownRowKind::StreamChunk,
        }
    }
}

struct RecentCache<K, V> {
    entries: HashMap<K, (V, u64)>,
    clock: u64,
    capacity: usize,
}

impl<K: Copy + Eq + Hash, V: Clone> RecentCache<K, V> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            capacity,
        }
    }

    fn get_or_insert_with(&mut self, key: K, create: impl FnOnce() -> V) -> (V, bool) {
        self.clock = self.clock.wrapping_add(1);
        if let Some((value, last_used)) = self.entries.get_mut(&key) {
            *last_used = self.clock;
            return (value.clone(), true);
        }

        let value = create();
        if self.entries.len() >= self.capacity
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, last_used))| last_used)
                .map(|(key, _)| *key)
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(key, (value.clone(), self.clock));
        (value, false)
    }
}

#[derive(Clone)]
pub(crate) struct TranscriptMarkdownCache {
    states: Rc<RefCell<RecentCache<MarkdownStateKey, Entity<TextViewState>>>>,
}

impl Default for TranscriptMarkdownCache {
    fn default() -> Self {
        Self {
            states: Rc::new(RefCell::new(RecentCache::new(MAX_CACHED_MARKDOWN_ROWS))),
        }
    }
}

impl TranscriptMarkdownCache {
    pub(crate) fn state(
        &self,
        key: MarkdownStateKey,
        text: &str,
        cx: &mut gpui::App,
    ) -> Entity<TextViewState> {
        let (state, hit) = self.states.borrow_mut().get_or_insert_with(key, || {
            let _timing = crate::app::infrastructure::performance::OperationTiming::new(
                crate::app::infrastructure::performance::OperationKind::MarkdownParse,
                text.len(),
            );
            cx.new(|cx| TextViewState::markdown(text, cx))
        });
        if hit {
            crate::app::infrastructure::performance::count_markdown_cache_hit();
        }
        state
    }
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
