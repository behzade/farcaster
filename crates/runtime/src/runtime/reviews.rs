use super::*;
use crate::reviews::{artifact, presentation::TranscriptPresentation};
use std::{collections::HashMap, path::Path, sync::Arc};

#[derive(Default)]
pub(super) struct ReviewProjection {
    synced: HashMap<usize, Arc<TranscriptItem>>,
    document: Arc<TranscriptPresentation>,
}

impl ReviewProjection {
    pub(super) fn apply(&mut self, snapshot: &mut RuntimeSnapshot) {
        let source = snapshot.conversation.clone();
        let document = Arc::make_mut(&mut self.document);
        let length = source.items.len();
        if document.items.len() > length {
            document.items.splice(length..document.items.len(), []);
            self.synced.retain(|index, _| *index < length);
        }
        let mut changed = None;
        if document.items.len() < length {
            let append_from = document.items.len();
            let mut suffix = Vec::with_capacity(length - append_from);
            for index in append_from..length {
                let source_item = source
                    .items
                    .get(index)
                    .expect("conversation item should exist");
                self.synced.insert(index, source_item.clone());
                suffix.push(hydrated_row(source_item, &snapshot.project));
            }
            document.items.splice(append_from..append_from, suffix);
            changed = Some(changed.unwrap_or(append_from).min(append_from));
        }
        for index in 0..length.min(document.items.len()) {
            let source_item = source
                .items
                .get(index)
                .expect("conversation item should exist");
            if self
                .synced
                .get(&index)
                .is_some_and(|synced| Arc::ptr_eq(synced, source_item))
            {
                continue;
            }
            self.synced.insert(index, source_item.clone());
            document
                .items
                .set(index, hydrated_row(source_item, &snapshot.project));
            changed = Some(changed.unwrap_or(index).min(index));
        }
        if let Some(changed) = changed {
            let from = changed.min(snapshot.transcript_changed_from.unwrap_or(changed));
            snapshot.transcript_changed_from = Some(from);
        }
        document.update_runs(&source);
        snapshot.transcript = Some(self.document.clone());
    }
}

fn hydrated_row(source: &Arc<TranscriptItem>, project: &Path) -> Arc<TranscriptItem> {
    let Some(result) = artifact::hydration_result(source, project) else {
        return source.clone();
    };
    let mut item = source.as_ref().clone();
    let details = Arc::make_mut(item.tool_details.as_mut().expect("review row has details"));
    details.result = Some(result);
    Arc::new(item)
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
