//! Cached transcript projection and long-session UI synchronization.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use gpui::{Context, FollowMode, ListOffset};

use super::{PiApp, transcript_splice};
const MAX_TRANSCRIPT_CACHE_ENTRIES: usize = 16;

use crate::{
    conversation::{ConversationState, TranscriptKind},
    transcript::TranscriptRow,
};

#[derive(Clone)]
pub(super) struct TranscriptUiCache {
    rows: Arc<Vec<TranscriptRow>>,
    conversation: Arc<ConversationState>,
    scroll: ListOffset,
    following: bool,
    unseen: usize,
    disclosures: HashMap<usize, bool>,
}

impl PiApp {
    pub(super) fn cache_current_transcript(&mut self) {
        let Some(path) = transcript_session_path(&self.snapshot) else {
            return;
        };
        if !self.transcript_cache.contains_key(&path)
            && self.transcript_cache.len() >= MAX_TRANSCRIPT_CACHE_ENTRIES
            && let Some(oldest) = self.transcript_cache.keys().next().cloned()
        {
            self.transcript_cache.remove(&oldest);
        }
        self.transcript_cache.insert(
            path,
            TranscriptUiCache {
                rows: self.transcript_rows.clone(),
                conversation: self.snapshot.conversation.clone(),
                scroll: self.transcript_list.logical_scroll_top(),
                following: self.transcript_list.is_following_tail(),
                unseen: self.transcript_unseen,
                disclosures: self.transcript_disclosure_states.clone(),
            },
        );
    }

    pub(super) fn restore_cached_transcript(
        &mut self,
        snapshot: &crate::runtime::RuntimeSnapshot,
    ) -> bool {
        let Some(path) = transcript_session_path(snapshot) else {
            return false;
        };
        let Some(cached) = self.transcript_cache.get(&path).cloned() else {
            return false;
        };
        if !cache_matches_snapshot(&cached, snapshot) {
            return false;
        }
        self.transcript_list.reset(cached.rows.len());
        self.transcript_rows = cached.rows;
        self.transcript_disclosure_states = cached.disclosures;
        self.transcript_unseen = cached.unseen;
        self.last_transcript_count = self.transcript_rows.len();
        self.transcript_following = cached.following;
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        if cached.following {
            self.transcript_list.scroll_to_end();
        } else {
            self.transcript_list.scroll_to(cached.scroll);
        }
        true
    }

    pub(super) fn preview_cached_session(
        &mut self,
        path: &std::path::Path,
        project: &std::path::Path,
    ) -> bool {
        let Some(cached) = self.transcript_cache.get(path).cloned() else {
            return false;
        };
        self.transcript_list.reset(cached.rows.len());
        self.transcript_rows = cached.rows;
        self.transcript_disclosure_states = cached.disclosures;
        self.transcript_unseen = cached.unseen;
        self.last_transcript_count = self.transcript_rows.len();
        self.transcript_following = cached.following;
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        if cached.following {
            self.transcript_list.scroll_to_end();
        } else {
            self.transcript_list.scroll_to(cached.scroll);
        }
        let snapshot = Arc::make_mut(&mut self.snapshot);
        snapshot.selected_session = Some(path.to_path_buf());
        snapshot.project = project.to_path_buf();
        snapshot.conversation = cached.conversation;
        snapshot.history_preview = true;
        true
    }

    pub(super) fn project_transcript_rows(
        &self,
        snapshot: &crate::runtime::RuntimeSnapshot,
    ) -> Vec<TranscriptRow> {
        crate::transcript::update_rows_from(
            &self.transcript_rows,
            &self.snapshot.conversation.items,
            &snapshot.conversation.items,
            snapshot.transcript_changed_from,
        )
    }

    pub(crate) fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.transcript_following = true;
        self.transcript_unseen = 0;
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        self.transcript_list.scroll_to_end();
        self.notify_transcript(cx);
    }

    pub(crate) fn set_transcript_item_expanded(
        &mut self,
        key: usize,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        self.transcript_disclosure_states.insert(key, expanded);
        if let Some(index) = self.transcript_rows.iter().position(|row| row.key() == key) {
            self.transcript_list
                .remeasure_items(index..index.saturating_add(1));
        }
        self.notify_transcript(cx);
    }

    pub(super) fn sync_composer_history(&mut self) {
        let target = self.composer_sessions.current_target().to_owned();
        let mut user_count = 0;
        let mut last_user = "";
        for item in &self.snapshot.conversation.items {
            if item.kind == TranscriptKind::User && !item.is_error {
                user_count += 1;
                last_user = &item.text;
            }
        }
        if self.composer_history_marker.as_ref().is_some_and(
            |(saved_target, saved_count, saved_last)| {
                saved_target == &target && *saved_count == user_count && saved_last == last_user
            },
        ) {
            return;
        }
        let history = self
            .snapshot
            .conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::User && !item.is_error)
            .map(|item| item.text.clone())
            .collect::<Vec<_>>();
        self.composer_sessions.sync_history(&target, &history);
        self.composer_history_marker = Some((target, user_count, last_user.to_owned()));
    }

    pub(super) fn sync_transcript_rows(&mut self, next: Vec<TranscriptRow>) {
        let positions_unchanged = self.transcript_rows.len() == next.len()
            && self
                .transcript_rows
                .iter()
                .zip(&next)
                .all(|(current, next)| current.same_position(next));
        if positions_unchanged {
            if let Some(first) = self
                .transcript_rows
                .iter()
                .zip(&next)
                .position(|(current, next)| current != next)
            {
                let last = self
                    .transcript_rows
                    .iter()
                    .zip(&next)
                    .rposition(|(current, next)| current != next)
                    .unwrap_or(first);
                crate::performance::count_remeasured_rows(last + 1 - first);
                self.transcript_list.remeasure_items(first..last + 1);
            }
        } else if let Some((old_range, new_count)) = transcript_splice(&self.transcript_rows, &next)
        {
            let anchor = (!self.transcript_list.is_following_tail()).then(|| {
                let offset = self.transcript_list.logical_scroll_top();
                self.transcript_rows
                    .get(offset.item_ix)
                    .copied()
                    .map(|row| (row, offset.offset_in_item))
            });
            self.transcript_list.splice(old_range, new_count);
            if let Some(Some((anchored_row, offset_in_item))) = anchor
                && let Some(item_ix) = next.iter().position(|row| row.same_position(&anchored_row))
            {
                self.transcript_list.scroll_to(gpui::ListOffset {
                    item_ix,
                    offset_in_item,
                });
            }
        }
        self.transcript_rows = Arc::new(next);
    }

    pub(super) fn mark_transcript_changed(&mut self, _index: usize, _was_empty: bool) {
        let rows = crate::transcript::project_rows(&self.snapshot.conversation.items);
        self.transcript_list.reset(rows.len());
        self.transcript_rows = Arc::new(rows);
    }
}

fn cache_matches_snapshot(
    cached: &TranscriptUiCache,
    snapshot: &crate::runtime::RuntimeSnapshot,
) -> bool {
    Arc::ptr_eq(&cached.conversation, &snapshot.conversation)
}

fn transcript_session_path(snapshot: &crate::runtime::RuntimeSnapshot) -> Option<PathBuf> {
    snapshot
        .selected_session
        .clone()
        .or_else(|| snapshot.live_session.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_rows_are_reused_only_for_the_same_conversation_revision() {
        let snapshot = crate::runtime::RuntimeSnapshot::default();
        let cached = TranscriptUiCache {
            rows: Arc::default(),
            conversation: snapshot.conversation.clone(),
            scroll: ListOffset::default(),
            following: true,
            unseen: 0,
            disclosures: HashMap::new(),
        };
        assert!(cache_matches_snapshot(&cached, &snapshot));

        let mut changed = snapshot.clone();
        Arc::make_mut(&mut changed.conversation).push_local_user("changed".into(), 0);
        assert!(!cache_matches_snapshot(&cached, &changed));
    }
}
