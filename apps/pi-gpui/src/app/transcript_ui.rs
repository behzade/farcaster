//! Transcript projection and long-session UI synchronization.

use std::sync::Arc;

use gpui::{Context, FollowMode};

use super::{PiApp, transcript_splice};
use crate::{conversation::TranscriptKind, transcript::TranscriptRow};

impl PiApp {
    pub(super) fn project_transcript_rows(
        &self,
        snapshot: &crate::runtime::RuntimeSnapshot,
    ) -> Vec<TranscriptRow> {
        let _timing = crate::performance::Timing::new("transcript.project_rows");
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
        let _timing = crate::performance::Timing::new("transcript.sync_rows");
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
