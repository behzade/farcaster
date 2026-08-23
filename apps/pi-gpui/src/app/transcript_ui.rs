//! Transcript projection and long-session UI synchronization.

use gpui::Context;

use super::PiApp;
use crate::conversation::TranscriptKind;

impl PiApp {
    pub(super) fn project_transcript_rows(
        &self,
        snapshot: &crate::runtime::RuntimeSnapshot,
    ) -> crate::transcript::TranscriptRowUpdate {
        let _timing = crate::performance::Timing::new("transcript.project_rows");
        crate::transcript::update_rows_incremental(
            &self.transcript_rows,
            &self.snapshot.conversation.items,
            &snapshot.conversation.items,
            snapshot.transcript_changed_from,
        )
    }

    pub(crate) fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.transcript_following = true;
        self.transcript_unseen = 0;
        self.transcript_list.scroll_to_end();
        self.notify_transcript(cx);
    }

    pub(crate) fn set_transcript_item_expanded(
        &mut self,
        key: usize,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        if expanded {
            // Keep the disclosure header under the pointer while its detail grows below it.
            self.transcript_list.pause_following_tail();
        }
        self.transcript_disclosure_states.insert(key, expanded);
        if let Some(index) = self.transcript_rows.iter().position(|row| row.key() == key) {
            self.transcript_list
                .remeasure_items(index..index.saturating_add(1));
        }
        self.notify_transcript(cx);
    }

    pub(super) fn sync_composer_history(&mut self) {
        let _timing = crate::performance::OperationTiming::new(
            crate::performance::OperationKind::ComposerHistory,
            self.snapshot.conversation.items.len(),
        );
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

    pub(super) fn apply_transcript_rows(
        &mut self,
        update: crate::transcript::TranscriptRowUpdate,
    ) -> bool {
        update.apply(
            &self.transcript_list,
            &mut self.transcript_rows,
            &self.snapshot.conversation.items,
        )
    }

    pub(super) fn mark_transcript_changed(&mut self, index: usize, _was_empty: bool) {
        let rows = crate::transcript::update_rows_from(
            &self.transcript_rows,
            &self.snapshot.conversation.items,
            &self.snapshot.conversation.items,
            Some(index),
        );
        let _changed =
            self.apply_transcript_rows(crate::transcript::TranscriptRowUpdate::replace(rows));
    }
}
