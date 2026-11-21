use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{
    Context, InteractiveElement as _, IntoElement as _, ParentElement as _, Render, Styled as _,
    WeakEntity,
};

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
        if let Some((label, excerpt)) = loading_excerpt(app) {
            let theme = crate::app::ui::theme::theme();
            let mut column = crate::app::ui::layout::chat_column()
                .py(theme.space.md)
                .flex()
                .flex_col()
                .gap(theme.space.sm)
                .child(
                    gpui::div()
                        .text_size(theme.type_scale.body_small)
                        .text_color(theme.colors.muted)
                        .child(label),
                );
            if !excerpt.is_empty() {
                let text = match excerpt.char_indices().nth(1600) {
                    Some((end, _)) => format!("{}…", &excerpt[..end]),
                    None => excerpt.to_owned(),
                };
                column = column.child(
                    gpui::div()
                        .text_size(self.font_size)
                        .text_color(theme.colors.text)
                        .child(text),
                );
            }
            return gpui::div()
                .debug_selector(move || label.into())
                .size_full()
                .overflow_hidden()
                .bg(theme.colors.canvas)
                .child(column)
                .into_any_element();
        }
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

// Keep partial history out of the transcript list. Its item offsets, saved reading
// position, composer recall, and actions all belong to the complete document.
pub(super) fn loading_excerpt(app: &FarcasterApp) -> Option<(&'static str, &str)> {
    if app.sessions.selected_draft.is_some() {
        return None;
    }
    let snapshot = &app.snapshot;
    let row = if app.lifecycle.pending_session_switch.is_some() {
        // Until the runtime acknowledges the request its reply belongs to the previous selection.
        app.sidebar_catalog_row(&app.sessions.all)
    } else {
        if snapshot.document_ready
            || snapshot.status != "Loading history"
            || !snapshot.conversation.items.is_empty()
        {
            return None;
        }
        if let Some(reply) = snapshot.loading_reply.as_deref() {
            return Some(("Latest reply · Loading history…", reply));
        }
        snapshot
            .selected_session
            .as_deref()
            .and_then(|path| snapshot.catalog_row_at(&app.sessions.all, path))
    };
    match row
        .map(|row| row.first_user_message.as_str())
        .filter(|text| !text.is_empty())
    {
        Some(text) => Some(("First message · Loading history…", text)),
        None => Some(("Loading history…", "")),
    }
}
