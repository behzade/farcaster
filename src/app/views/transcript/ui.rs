use gpui::Context;

use super::{TranscriptRowUpdate, conversation::TranscriptKind, update_conversation_rows};
use crate::app::FarcasterApp;

impl FarcasterApp {
    pub(in crate::app) fn adjust_transcript_font_size(
        &mut self,
        delta: f32,
        cx: &mut Context<Self>,
    ) {
        let size = f32::from(self.transcript_view.read(cx).font_size) + delta;
        self.set_transcript_font_size(size, cx);
    }

    pub(in crate::app) fn set_transcript_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        use crate::app::infrastructure::persistence::StateStore;
        use crate::app::ui::theme::TRANSCRIPT_FONT_SIZE_RANGE;

        let size = size.clamp(
            *TRANSCRIPT_FONT_SIZE_RANGE.start(),
            *TRANSCRIPT_FONT_SIZE_RANGE.end(),
        );
        if gpui::px(size) == self.transcript_view.read(cx).font_size {
            return;
        }
        match StateStore::open().and_then(|store| store.save_transcript_font_size(size)) {
            Ok(()) => {
                self.transcript_view.update(cx, |transcript, cx| {
                    transcript.font_size = gpui::px(size);
                    transcript.list.remeasure_items(0..transcript.rows.len());
                    cx.notify();
                });
                self.settings_transcript_error = None;
            }
            Err(error) => {
                zlog::error!("{error}");
                self.settings_transcript_error = Some(error);
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn toggle_transcript_folder(
        &mut self,
        key: usize,
        project: &std::path::Path,
        path: &std::path::Path,
        cx: &mut Context<Self>,
    ) {
        let expanded = self.expand_transcript_folders;
        self.transcript_view.update(cx, |transcript, cx| {
            transcript.list.pause_following_tail();
            transcript
                .file_trees
                .entry(key)
                .or_insert_with(|| {
                    crate::app::ui::change_tree::ChangeTreeState::with_default(project, expanded)
                })
                .toggle(project, path);
            if let Some(index) = transcript
                .rows
                .iter()
                .position(|row| row.disclosure_key() == key)
            {
                transcript.list.remeasure_items(index..index + 1);
            }
            cx.notify();
        });
    }

    pub(in crate::app) fn transcript_selected_text(
        &self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        self.transcript_view
            .read(cx)
            .list
            .clone()
            .copy_selection_text(window, cx)
    }

    pub(in crate::app) fn project_transcript_rows(
        &self,
        snapshot: &crate::runtime::RuntimeSnapshot,
        cx: &Context<Self>,
    ) -> TranscriptRowUpdate {
        let _timing =
            crate::app::infrastructure::performance::Timing::new("transcript.project_rows");
        let transcript = self.transcript_view.read(cx);
        update_conversation_rows(
            &transcript.rows,
            &self.snapshot.conversation,
            &snapshot.conversation,
            snapshot.transcript_changed_from,
        )
    }

    pub(in crate::app) fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.transcript_view.update(cx, |transcript, cx| {
            transcript.following = true;
            transcript.unseen = 0;
            transcript.list.scroll_to_end();
            cx.notify();
        });
    }

    pub(in crate::app) fn set_transcript_item_expanded(
        &mut self,
        key: usize,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        self.transcript_view.update(cx, |transcript, cx| {
            if expanded {
                transcript.list.pause_following_tail();
            }
            transcript.disclosure_states.insert(key, expanded);
            if let Some(index) = transcript
                .rows
                .iter()
                .position(|row| row.contains_disclosure_key(key))
            {
                transcript
                    .list
                    .remeasure_items(index..index.saturating_add(1));
            }
            cx.notify();
        });
    }

    pub(in crate::app) fn sync_composer_history(&mut self) {
        let _timing = crate::app::infrastructure::performance::OperationTiming::new(
            crate::app::infrastructure::performance::OperationKind::ComposerHistory,
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

    pub(in crate::app) fn apply_transcript_rows(
        &mut self,
        update: TranscriptRowUpdate,
        cx: &mut Context<Self>,
    ) -> bool {
        let items = self.snapshot.conversation.items.clone();
        self.transcript_view
            .update(cx, |transcript, _| transcript.apply_rows(update, &items))
    }

    pub(in crate::app) fn mark_transcript_changed(
        &mut self,
        index: usize,
        _was_empty: bool,
        cx: &mut Context<Self>,
    ) {
        let conversation = self.snapshot.conversation.clone();
        self.transcript_view.update(cx, |transcript, cx| {
            let rows = update_conversation_rows(
                &transcript.rows,
                &conversation,
                &conversation,
                Some(index),
            );
            let _changed = transcript.apply_rows(rows, &conversation.items);
            cx.notify();
        });
    }
}
