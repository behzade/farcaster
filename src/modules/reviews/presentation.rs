use crate::{
    conversation::{ConversationState, TranscriptItem},
    utility::persistent_vec::PersistentVec,
};
use std::{ops::Range, sync::Arc};

/// Rendering data only. It contains no protocol reducer, tool offsets, or live
/// message bookkeeping, so app-owned rows cannot corrupt protocol execution.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TranscriptPresentation {
    pub(crate) items: PersistentVec<Arc<TranscriptItem>>,
    pub(crate) active_start: Option<usize>,
    pub(crate) completed_runs: Arc<Vec<Range<usize>>>,
    pub(crate) insertions: Arc<Vec<(usize, Arc<TranscriptItem>)>>,
}

impl From<&ConversationState> for TranscriptPresentation {
    fn from(source: &ConversationState) -> Self {
        Self {
            items: source.items.clone(),
            active_start: source.active_run_start(),
            completed_runs: Arc::new(source.completed_runs.clone()),
            insertions: Arc::default(),
        }
    }
}

impl TranscriptPresentation {
    /// Apply a UI-owned optimistic edit without removing durable cards or
    /// writing presentation indices back into the protocol state.
    pub(crate) fn update_source(&mut self, source: &ConversationState, dirty: usize) -> usize {
        let prefix = self.update_items(source, dirty);
        self.update_runs(source);
        prefix
    }

    pub(crate) fn update_items(&mut self, source: &ConversationState, dirty: usize) -> usize {
        let dirty = dirty.min(source.items.len());
        let prefix = dirty
            + self
                .insertions
                .partition_point(|(position, _)| *position < dirty);
        if self
            .insertions
            .last()
            .is_some_and(|(position, _)| *position > source.items.len())
        {
            for (position, _) in Arc::make_mut(&mut self.insertions) {
                *position = (*position).min(source.items.len());
            }
        }
        let mut suffix = Vec::new();
        let mut insertions = self
            .insertions
            .iter()
            .skip(
                self.insertions
                    .partition_point(|(position, _)| *position < dirty),
            )
            .peekable();
        for index in dirty..=source.items.len() {
            while insertions
                .peek()
                .is_some_and(|(position, _)| *position == index)
            {
                if let Some((_, item)) = insertions.next() {
                    suffix.push(item.clone());
                }
            }
            if let Some(item) = source.items.get(index) {
                suffix.push(item.clone());
            }
        }
        self.items.splice(prefix..self.items.len(), suffix);
        prefix
    }

    pub(crate) fn update_runs(&mut self, source: &ConversationState) {
        let shift = |index: usize, inclusive: bool| {
            index
                + self.insertions.partition_point(|(position, _)| {
                    if inclusive {
                        *position <= index
                    } else {
                        *position < index
                    }
                })
        };
        self.active_start = source.active_run_start().map(|start| shift(start, false));
        self.completed_runs = Arc::new(
            source
                .completed_runs
                .iter()
                .map(|run| shift(run.start, true)..shift(run.end, true))
                .collect(),
        );
    }
}
