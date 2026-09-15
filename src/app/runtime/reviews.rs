use super::*;
use crate::{
    app::reviews::{
        artifact,
        delivery::{self, Submission},
        presentation::TranscriptPresentation,
    },
    conversation::{ToolDetails, ToolExecutionState, TranscriptKind},
};
use std::collections::BTreeMap;

struct Card {
    submission: Submission,
    item: Arc<TranscriptItem>,
    position: Option<usize>,
}

#[derive(Default)]
pub(super) struct ReviewProjection {
    key: Option<(Backend, PathBuf, PathBuf, u64)>,
    source: Option<Arc<ConversationState>>,
    document: Arc<TranscriptPresentation>,
    cards: Vec<Card>,
    native: BTreeMap<usize, String>,
    users: BTreeMap<usize, Arc<TranscriptItem>>,
    user_positions: HashMap<usize, usize>,
    unresolved: bool,
    #[cfg(test)]
    scanned_items: usize,
    #[cfg(test)]
    visited_cards: usize,
}

impl ReviewProjection {
    pub(super) fn apply(&mut self, state: Option<&StateStore>, snapshot: &mut RuntimeSnapshot) {
        let Some(backend) = snapshot.harness else {
            return;
        };
        let Some(path) = snapshot.selected_session.as_ref() else {
            return;
        };
        let key = (
            backend,
            snapshot.project.clone(),
            path.clone(),
            delivery::revision(),
        );
        let reset = self
            .key
            .as_ref()
            .is_none_or(|old| old.0 != key.0 || old.1 != key.1 || old.2 != key.2);
        if reset {
            self.source = None;
            self.document = Arc::default();
            self.native.clear();
            self.users.clear();
            self.user_positions.clear();
        }
        let reloaded = self.key.as_ref() != Some(&key);
        if reloaded {
            let Some(state) = state else { return };
            match state.session_reviews(backend, &snapshot.project, path) {
                Ok(submissions) => {
                    let mut previous = std::mem::take(&mut self.cards)
                        .into_iter()
                        .map(|card| (card.submission.id.clone(), card))
                        .collect::<HashMap<_, _>>();
                    self.cards = submissions
                        .into_iter()
                        .filter_map(|submission| {
                            if !reset && let Some(mut card) = previous.remove(&submission.id) {
                                card.submission = submission;
                                return Some(card);
                            }
                            card(submission)
                        })
                        .collect();
                    self.key = Some(key);
                }
                Err(error) => {
                    zlog::error!("Restore session reviews: {error}");
                    return;
                }
            }
        }
        let source = &snapshot.conversation;
        let same_items = self
            .source
            .as_ref()
            .is_some_and(|previous| previous.items.shares_storage(&source.items));
        let mut dirty = match &self.source {
            Some(_) if same_items => source.items.len(),
            Some(previous) => {
                let start = snapshot
                    .transcript_changed_from
                    .unwrap_or(0)
                    .min(previous.items.len())
                    .min(source.items.len());
                start
                    + previous
                        .items
                        .iter_range(start..previous.items.len())
                        .zip(source.items.iter_range(start..source.items.len()))
                        .take_while(|(left, right)| Arc::ptr_eq(left, right))
                        .count()
            }
            None => 0,
        };
        let removed_native = self.native.split_off(&dirty);
        let removed_users = self.users.split_off(&dirty);
        for item in removed_users.values() {
            self.user_positions.remove(&(Arc::as_ptr(item) as usize));
        }
        let mut structure_changed = !removed_native.is_empty() || !removed_users.is_empty();
        for index in dirty..source.items.len() {
            #[cfg(test)]
            {
                self.scanned_items += 1;
            }
            let item = &source.items[index];
            if item.kind == TranscriptKind::User {
                self.users.insert(index, item.clone());
                self.user_positions
                    .insert(Arc::as_ptr(item) as usize, index);
                structure_changed = true;
            }
            if let Some(id) = artifact::from_item(item).and_then(|artifact| artifact.id) {
                self.native.insert(index, id);
                structure_changed = true;
            }
        }
        let truncated = self
            .source
            .as_ref()
            .is_some_and(|previous| previous.items.len() > source.items.len());
        let bindings_changed = self.source.as_ref().is_none_or(|previous| {
            previous.prompt_binding_revision() != source.prompt_binding_revision()
        });
        let rebind =
            reloaded || structure_changed || truncated || (self.unresolved && bindings_changed);
        if rebind {
            let known = self.native.values().collect::<HashSet<_>>();
            let users = self.users.keys().copied().collect::<Vec<_>>();
            let mut insertions = Vec::new();
            self.unresolved = false;
            for card in &mut self.cards {
                #[cfg(test)]
                {
                    self.visited_cards += 1;
                }
                let exact = card
                    .submission
                    .prompt_id
                    .as_deref()
                    .and_then(|id| source.submitted_prompt_item(id))
                    .and_then(|item| self.user_positions.get(&(Arc::as_ptr(item) as usize)))
                    .copied();
                if let Some(index) = exact
                    && card.submission.user_ordinal.is_none()
                {
                    let ordinal = users.partition_point(|user| *user < index);
                    if let (Some(state), Some(turn)) = (state, card.submission.turn_id.as_deref()) {
                        match state.record_review_position(turn, ordinal) {
                            Ok(()) => card.submission.user_ordinal = Some(ordinal),
                            Err(error) => {
                                zlog::error!("Save review position: {error}");
                            }
                        }
                    }
                }
                let anchor = exact.or_else(|| {
                    card.submission
                        .user_ordinal
                        .and_then(|ordinal| users.get(ordinal).copied())
                });
                // Without receipt evidence leave the card at its arrival point,
                // and retry binding when the delayed receipt becomes visible.
                self.unresolved |= anchor.is_none() && card.submission.prompt_id.is_some();
                if let Some(anchor) = anchor {
                    let end = users
                        .get(users.partition_point(|user| *user <= anchor))
                        .copied()
                        .unwrap_or(source.items.len());
                    if card
                        .position
                        .is_none_or(|position| position <= anchor || position > end)
                        || dirty == 0
                    {
                        card.position = Some(end);
                    }
                }
                let position = *card.position.get_or_insert(source.items.len());
                if !known.contains(&card.submission.id) {
                    insertions.push((position.min(source.items.len()), card.item.clone()));
                }
            }
            if !self.cards.is_empty() {
                zlog::info!(
                    "Review projection: cards={} users={} native={} insertions={} unresolved={}",
                    self.cards.len(),
                    users.len(),
                    self.native.len(),
                    insertions.len(),
                    self.unresolved
                );
            }
            insertions.sort_by_key(|(position, _)| *position);
            let common = self
                .document
                .insertions
                .iter()
                .zip(&insertions)
                .take_while(|((a, left), (b, right))| a == b && Arc::ptr_eq(left, right))
                .count();
            if let Some((position, _)) = self.document.insertions.get(common) {
                dirty = dirty.min(*position);
            }
            if let Some((position, _)) = insertions.get(common) {
                dirty = dirty.min(*position);
            }
            // The old insertion map defines the retained prefix, the new map
            // defines only the rebuilt suffix (including native-result echoes).
            let prefix = dirty
                + self
                    .document
                    .insertions
                    .partition_point(|(position, _)| *position < dirty);
            let document = Arc::make_mut(&mut self.document);
            document
                .items
                .splice(prefix..document.items.len(), std::iter::empty());
            document.insertions = Arc::new(insertions);
            // update_items computes the same prefix because changed insertions
            // start at dirty; the strictly-earlier maps are identical.
            document.update_source(source, dirty);
            snapshot.transcript_changed_from = Some(prefix);
        } else {
            let document = Arc::make_mut(&mut self.document);
            let prefix = if same_items {
                document.items.len()
            } else {
                document.update_items(source, dirty)
            };
            if self.source.as_ref().is_none_or(|previous| {
                previous.active_run_start() != source.active_run_start()
                    || previous.completed_runs.len() != source.completed_runs.len()
            }) {
                document.update_runs(source);
            }
            snapshot.transcript_changed_from = Some(prefix);
        }
        self.source = Some(source.clone());
        snapshot.transcript = Some(self.document.clone());
    }
}

fn card(submission: Submission) -> Option<Card> {
    let details = ToolDetails {
        name: "submit_review".into(),
        arguments: submission.artifact["farcaster_review"]["review"].clone(),
        result: Some(submission.artifact.clone()),
        metadata: Default::default(),
        state: ToolExecutionState::Succeeded,
    };
    let item = Arc::new(TranscriptItem {
        kind: TranscriptKind::Tool,
        label: "submit_review".into(),
        text: String::new(),
        images: Arc::default(),
        files: Arc::default(),
        stream_chunks: Arc::default(),
        streaming: false,
        is_error: false,
        tool_call_id: Some(format!("farcaster-review:{}", submission.id)),
        tool_output: String::new(),
        tool_presentation: None,
        tool_details: Some(Arc::new(details)),
        tool_review: None,
        invocation: None,
    });
    artifact::from_item(&item)?;
    Some(Card {
        submission,
        item,
        position: None,
    })
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
