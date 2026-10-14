use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{Context, IntoElement as _, Render, WeakEntity};

use super::super::{FarcasterApp, transcript};
use crate::utility::persistent_vec::PersistentVec;

#[path = "transcript_reading.rs"]
mod reading;
use reading::{ReadingAnchor, ReadingState};

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
        items: &PersistentVec<Arc<crate::conversation::TranscriptItem>>,
    ) -> bool {
        let mut anchor = self.take_pending_reading_anchor();
        let changed = update.apply(&self.list, &mut self.rows, items);
        if !self.rows.is_empty()
            && let Some(session) = &self.session
            && let Some(saved) = self.reading.remove(session)
        {
            anchor = saved.restore(self, items);
        }
        if let Some(anchor) = anchor {
            if let Some(offset) = anchor.resolve(&self.rows, items) {
                self.list.restore_scroll(offset);
                self.following = false;
                self.pending_reading_anchor = Some(anchor);
            } else {
                self.jump_to_latest();
            }
        }
        changed
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
        let viewport = window.viewport_size();
        transcript::render(
            &self.list,
            transcript::TranscriptViewport {
                font_scale: self.font_size / crate::app::ui::theme::theme().type_scale.reading,
                following: self.following,
                unseen: self.unseen,
                tail_reserve: transcript::tail_reserve(viewport.height),
            },
            self.rows.clone(),
            app.snapshot.transcript_presentation(),
            self.disclosure_states.clone(),
            self.file_trees.clone(),
            self.markdown_cache.clone(),
            self.app.clone(),
        )
    }
}
