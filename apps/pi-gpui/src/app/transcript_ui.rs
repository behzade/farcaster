//! Cached transcript projection and long-session UI synchronization.

use std::sync::Arc;

use gpui::{Context, FollowMode};

use super::{PiApp, transcript_splice};
use crate::{conversation::TranscriptKind, transcript::TranscriptRow};

impl PiApp {
    pub(crate) fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.transcript_following = true;
        self.transcript_unseen = 0;
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        self.transcript_list.scroll_to_end();
        cx.notify();
    }

    pub(crate) fn toggle_transcript_item(&mut self, key: usize, cx: &mut Context<Self>) {
        if !self.transcript_disclosure_overrides.remove(&key) {
            self.transcript_disclosure_overrides.insert(key);
        }
        if let Some(index) = self.transcript_rows.iter().position(|row| row.key() == key) {
            self.transcript_list
                .remeasure_items(index..index.saturating_add(1));
        }
        cx.notify();
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
        if let Some((old_range, new_count)) = transcript_splice(&self.transcript_rows, &next) {
            self.transcript_list.splice(old_range, new_count);
        }
        self.transcript_rows = Arc::new(next);
    }

    pub(super) fn mark_transcript_changed(&mut self, _index: usize, _was_empty: bool) {
        let rows = crate::transcript::project_rows(&self.snapshot.conversation.items);
        self.transcript_list.reset(rows.len());
        self.transcript_rows = Arc::new(rows);
    }
}
