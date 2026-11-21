use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{Context, IntoElement as _, Render, WeakEntity};

use super::super::{FarcasterApp, transcript};
use crate::{
    agents::{HistoryCoverage, HistoryPage},
    conversation::{ConversationState, TranscriptKind},
    reviews::presentation::TranscriptPresentation,
    utility::persistent_vec::PersistentVec,
};

#[path = "transcript_reading.rs"]
mod reading;
use reading::{ReadingAnchor, ReadingState};

struct HistoryTail {
    list: transcript::list::TranscriptListState,
    rows: Arc<PersistentVec<transcript::TranscriptRow>>,
    presentation: Arc<TranscriptPresentation>,
    markdown: transcript::markdown::TranscriptMarkdownCache,
}

impl HistoryTail {
    fn new(page: &HistoryPage<ConversationState>, project: &Path) -> Option<Self> {
        let source = &page.content;
        let presentation = Arc::new(TranscriptPresentation::from(source));
        let projected = transcript::project_presentation_rows(&presentation);
        if projected.is_empty()
            || projected
                .iter()
                .any(|row| matches!(row, transcript::TranscriptRow::Review { .. }))
            || source
                .items
                .iter()
                .any(|item| crate::reviews::artifact::hydration_result(item, project).is_some())
        {
            return None;
        }
        let first_safe_row = match page.coverage {
            HistoryCoverage::Complete => 0,
            HistoryCoverage::Suffix => {
                // The first user separates these rows from older activity and reviews.
                let first_user = source
                    .items
                    .position(|item| item.kind == TranscriptKind::User)?;
                projected.position(|row| row.key() == first_user)? + 1
            }
        };
        let list = transcript::list::TranscriptListState::new_history_tail(first_safe_row);
        let mut rows = Arc::default();
        transcript::TranscriptRowUpdate::replace(projected).apply(
            &list,
            &mut rows,
            &presentation.items,
        );
        Some(Self {
            list,
            rows,
            presentation,
            markdown: Default::default(),
        })
    }
}

pub(crate) struct TranscriptView {
    app: WeakEntity<FarcasterApp>,
    markdown_cache: transcript::markdown::TranscriptMarkdownCache,
    pub(crate) font_size: gpui::Pixels,
    pub(crate) list: transcript::list::TranscriptListState,
    pub(crate) rows: Arc<PersistentVec<transcript::TranscriptRow>>,
    pub(crate) following: bool,
    pub(crate) unseen: usize,
    pub(crate) disclosure_states: HashMap<usize, bool>,
    pub(crate) file_trees: HashMap<usize, crate::app::ui::change_tree::ChangeTreeState>,
    pub(crate) last_count: usize,
    session: Option<PathBuf>,
    reading: HashMap<PathBuf, ReadingState>,
    pending_reading_anchor: Option<ReadingAnchor>,
    history_tail: Option<HistoryTail>,
}

impl TranscriptView {
    pub(crate) fn new(
        app: WeakEntity<FarcasterApp>,
        list: transcript::list::TranscriptListState,
    ) -> Self {
        Self {
            app,
            font_size: crate::app::ui::theme::theme().type_scale.reading,
            markdown_cache: transcript::markdown::TranscriptMarkdownCache::default(),
            list,
            rows: Arc::new(PersistentVec::default()),
            following: true,
            unseen: 0,
            disclosure_states: HashMap::new(),
            file_trees: HashMap::new(),
            last_count: 0,
            session: None,
            reading: HashMap::new(),
            pending_reading_anchor: None,
            history_tail: None,
        }
    }

    pub(crate) fn reset(
        &mut self,
        items: &PersistentVec<Arc<crate::conversation::TranscriptItem>>,
    ) {
        if let Some(session) = self.session.take()
            && !self.rows.is_empty()
        {
            let saved = ReadingState::capture(self, items);
            self.reading.insert(session, saved);
        }
        self.list.reset();
        self.history_tail = None;
        self.pending_reading_anchor = None;
        self.list.scroll_to_end();
        self.rows = Arc::new(PersistentVec::default());
        self.disclosure_states.clear();
        self.file_trees.clear();
        self.following = true;
        self.unseen = 0;
        self.last_count = 0;
    }

    pub(crate) fn select_session(
        &mut self,
        session: Option<&Path>,
        previous_items: &PersistentVec<Arc<crate::conversation::TranscriptItem>>,
    ) -> bool {
        if self.session.as_deref() == session {
            return false;
        }
        let reset = self.session.is_some();
        if reset {
            self.reset(previous_items);
        }
        self.session = session.map(Path::to_path_buf);
        reset
    }

    pub(crate) fn follow_session(&mut self, session: &Path) {
        if self.session.as_deref() == Some(session) {
            self.jump_to_latest();
        } else if let Some(saved) = self.reading.get_mut(session) {
            saved.follow_latest();
        }
    }

    pub(crate) fn jump_to_latest(&mut self) {
        if let Some(session) = &self.session
            && let Some(saved) = self.reading.get_mut(session)
        {
            saved.follow_latest();
        }
        self.following = true;
        self.unseen = 0;
        self.list.scroll_to_end();
        self.pending_reading_anchor = None;
    }

    pub(crate) fn apply_rows(
        &mut self,
        update: transcript::TranscriptRowUpdate,
        previous_items: &PersistentVec<Arc<crate::conversation::TranscriptItem>>,
        items: &PersistentVec<Arc<crate::conversation::TranscriptItem>>,
    ) -> bool {
        let mut anchor = self.take_pending_reading_anchor();
        // Final Markdown rows have different boundaries from streaming chunks.
        // Capture the current text position before replacing those rows, too.
        if anchor.is_none()
            && !self.list.has_pending_scroll()
            && let Some(transcript::TranscriptRow::StreamChunk { index, .. }) =
                self.rows.get(self.list.logical_scroll_top().item_ix)
            && previous_items
                .get(*index)
                .is_some_and(|item| item.streaming)
            && items.get(*index).is_some_and(|item| !item.streaming)
        {
            anchor = ReadingAnchor::capture(self, previous_items);
        }
        let changed = update.apply(&self.list, &mut self.rows, items);
        if !self.rows.is_empty()
            && let Some(session) = &self.session
            && let Some(saved) = self.reading.remove(session)
        {
            anchor = saved.restore(self, items);
        }
        if let Some(anchor) = anchor {
            if let Some(offset) = anchor.resolve(&self.rows, items) {
                self.restore_reading_anchor(anchor, offset);
            } else {
                self.jump_to_latest();
            }
        }
        changed
    }

    fn restore_reading_anchor(&mut self, anchor: ReadingAnchor, offset: gpui::ListOffset) {
        self.list.restore_scroll(offset);
        self.following = false;
        self.pending_reading_anchor = Some(anchor);
    }

    pub(crate) fn preserve_painted_history(
        &mut self,
        items: &PersistentVec<Arc<crate::conversation::TranscriptItem>>,
        saved_transcript: bool,
    ) {
        if self.allows_history_tail()
            && let Some(page) = &self.history_tail
            && page.list.painted_history_top().is_some()
        {
            if let Some((anchor, offset, unseen)) = page.anchor_before_append(&self.rows, items) {
                self.restore_reading_anchor(anchor, offset);
                self.unseen = unseen;
            } else if saved_transcript
                && let Some(offset) = page.matching_suffix_top(&self.rows, items)
            {
                // The backend can append before the saved document gets a draw.
                self.list.inherit_painted_top(offset);
            }
        }
    }

    fn take_pending_reading_anchor(&mut self) -> Option<ReadingAnchor> {
        self.pending_reading_anchor
            .take()
            .filter(|_| self.list.has_pending_restore())
    }

    pub(crate) fn update_count(&mut self, count: usize) {
        if count > self.last_count && !self.following {
            self.unseen = self.unseen.saturating_add(count - self.last_count);
        }
        self.last_count = count;
    }

    pub(crate) fn set_history_tail(
        &mut self,
        source: Option<&HistoryPage<ConversationState>>,
        project: &Path,
    ) {
        self.history_tail = source.and_then(|source| HistoryTail::new(source, project));
    }

    #[cfg(test)]
    pub(crate) fn history_tail_row_count(&self) -> usize {
        self.history_tail.as_ref().map_or(0, |tail| tail.rows.len())
    }

    fn allows_history_tail(&self) -> bool {
        self.following
            && self.list.is_following_tail()
            && !self.list.has_pending_scroll()
            && !self.list.has_pending_restore()
            && self.pending_reading_anchor.is_none()
            && self.disclosure_states.is_empty()
            && self.file_trees.is_empty()
            && self.session.as_ref().is_some_and(|path| {
                self.reading
                    .get(path)
                    .is_none_or(ReadingState::allows_history_tail)
            })
    }
}

impl Render for TranscriptView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let _timing = crate::app::infrastructure::performance::Timing::new("render.transcript");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let app = app.read(cx);
        if app.lifecycle.pending_session_switch.is_some() {
            return transcript::loading_history();
        }
        let viewport = transcript::TranscriptViewport {
            font_scale: self.font_size / crate::app::ui::theme::theme().type_scale.reading,
            following: self.following,
            unseen: self.unseen,
            tail_reserve: transcript::tail_reserve(window.viewport_size().height),
        };
        if app.awaiting_history_rows() {
            if let Some(tail) = &self.history_tail
                && self.allows_history_tail()
            {
                return transcript::render(
                    &tail.list,
                    viewport,
                    tail.rows.clone(),
                    tail.presentation.clone(),
                    HashMap::new(),
                    HashMap::new(),
                    tail.markdown.clone(),
                    self.app.clone(),
                );
            }
            return transcript::loading_history();
        }
        transcript::render(
            &self.list,
            viewport,
            self.rows.clone(),
            app.snapshot.transcript_presentation(),
            self.disclosure_states.clone(),
            self.file_trees.clone(),
            self.markdown_cache.clone(),
            self.app.clone(),
        )
    }
}
