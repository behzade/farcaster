use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{Context, IntoElement as _, Render, WeakEntity};

use super::super::{FarcasterApp, transcript};
use crate::{
    agents::{HistoryCoverage, HistoryPage},
    conversation::{ConversationState, TranscriptKind, markdown_structure::MarkdownIndex},
    reviews::presentation::TranscriptPresentation,
    utility::persistent_vec::PersistentVec,
};

#[path = "transcript_local.rs"]
mod local;
#[path = "transcript_reading.rs"]
mod reading;
#[path = "transcript_resident.rs"]
mod resident;
use local::LocalTranscript;
pub(in crate::app) use local::{DisplayQuery, DisplayRead, FitProbe};
use reading::{ReadingAnchor, ReadingState};

#[derive(Clone, Copy, Eq, PartialEq)]
enum HistoryDisplay {
    Waiting,
    Document,
    PositionUnavailable,
}

struct HistoryTail {
    disclosures: HashMap<usize, bool>,
    trees: HashMap<usize, crate::app::ui::change_tree::ChangeTreeState>,
    unseen: usize,
    list: transcript::list::TranscriptListState,
    rows: Arc<PersistentVec<transcript::TranscriptRow>>,
    presentation: Arc<TranscriptPresentation>,
    markdown: transcript::markdown::TranscriptMarkdownCache,
}

pub(in crate::app) struct PreparedPage {
    stamp: Option<crate::storage::TranscriptStamp>,
    full: bool,
    coverage: HistoryCoverage,
    rows: PersistentVec<transcript::TranscriptRow>,
    presentation: Arc<TranscriptPresentation>,
    markdown: Arc<MarkdownIndex>,
    first_safe_row: usize,
}

impl PreparedPage {
    fn from_store(stored: crate::storage::StoredViewport, project: &Path) -> Option<Self> {
        if stored.full_document {
            let mut presentation = TranscriptPresentation::from(&stored.content);
            for (index, item) in stored.content.items.iter().enumerate() {
                let hydrated = crate::reviews::artifact::hydrated_row(item, project);
                if !Arc::ptr_eq(item, &hydrated) {
                    presentation.items.set(index, hydrated);
                }
            }
            return Some(Self {
                stamp: Some(stored.stamp),
                full: true,
                coverage: stored.coverage,
                rows: transcript::project_presentation_rows(&presentation),
                presentation: Arc::new(presentation),
                markdown: Arc::default(),
                first_safe_row: 0,
            });
        }
        let mut prepared = Self::new(
            &HistoryPage {
                content: stored.content,
                coverage: stored.coverage,
            },
            project,
            stored.markdown,
        )?;
        prepared.stamp = Some(stored.stamp);
        Some(prepared)
    }

    fn new(
        page: &HistoryPage<ConversationState>,
        project: &Path,
        markdown: MarkdownIndex,
    ) -> Option<Self> {
        let source = &page.content;
        let presentation = Arc::new(TranscriptPresentation::from(source));
        let projected = transcript::project_indexed_rows(&presentation, Some(&markdown));
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
        Some(Self {
            stamp: None,
            full: false,
            coverage: page.coverage,
            rows: projected,
            presentation,
            markdown: markdown.into(),
            first_safe_row,
        })
    }
}

impl HistoryTail {
    fn new(page: &HistoryPage<ConversationState>, project: &Path) -> Option<Self> {
        PreparedPage::new(page, project, MarkdownIndex::new()).map(Self::from_prepared)
    }

    fn from_prepared(prepared: PreparedPage) -> Self {
        let PreparedPage {
            stamp: _,
            full: _,
            coverage: _,
            rows: projected,
            presentation,
            markdown,
            first_safe_row,
        } = prepared;
        let list = transcript::list::TranscriptListState::new_history_tail(first_safe_row);
        let mut rows = Arc::default();
        transcript::TranscriptRowUpdate::replace(projected).apply(
            &list,
            &mut rows,
            &presentation.items,
        );
        Self {
            disclosures: Default::default(),
            trees: Default::default(),
            unseen: 0,
            list,
            rows,
            presentation,
            markdown: transcript::markdown::TranscriptMarkdownCache::with_index(markdown),
        }
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
    record: Option<crate::sessions::SessionKey>,
    reading: HashMap<(PathBuf, Option<crate::sessions::SessionKey>), ReadingState>,
    coverage: Option<HistoryCoverage>,
    pending_reading_anchor: Option<ReadingAnchor>,
    history_tail: Option<HistoryTail>,
    local: Option<LocalTranscript>,
    resident: resident::ResidentTranscripts,
}

impl TranscriptView {
    fn history_display(&self, snapshot: &crate::runtime::RuntimeSnapshot) -> HistoryDisplay {
        use crate::runtime::SavedHistory;
        match snapshot.history_coverage() {
            None => HistoryDisplay::Waiting,
            // apply_rows consumes the saved reading state when these rows can restore it.
            Some(HistoryCoverage::Suffix)
                if self.session.as_ref().is_some_and(|path| {
                    self.reading
                        .get(&(path.clone(), self.record.clone()))
                        .is_some_and(|saved| !saved.allows_history_tail())
                }) =>
            {
                if snapshot.saved_history == Some(SavedHistory::RefreshingPartial) {
                    HistoryDisplay::Waiting
                } else {
                    HistoryDisplay::PositionUnavailable
                }
            }
            _ => HistoryDisplay::Document,
        }
    }

    pub(in crate::app) fn awaiting_history_rows(
        &self,
        snapshot: &crate::runtime::RuntimeSnapshot,
    ) -> bool {
        self.history_display(snapshot) == HistoryDisplay::Waiting
    }

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
            record: None,
            reading: HashMap::new(),
            coverage: None,
            pending_reading_anchor: None,
            history_tail: None,
            local: None,
            resident: Default::default(),
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
            self.park_resident(&session, items, &saved);
            let entry = self.reading.entry((session, self.record.take()));
            // A partial document may still be waiting for the original reading position.
            if saved.allows_history_tail() {
                entry.or_insert(saved);
            } else {
                entry.insert_entry(saved);
            }
        }
        self.list.reset();
        self.coverage = None;
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
        record: Option<crate::sessions::SessionKey>,
        previous_items: &PersistentVec<Arc<crate::conversation::TranscriptItem>>,
    ) -> bool {
        if self.session.as_deref() == session && (self.record == record || self.record.is_none()) {
            self.record = record;
            return false;
        }
        let reset = self.session.is_some();
        if reset {
            self.reset(previous_items);
        }
        self.session = session.map(Path::to_path_buf);
        self.record = record;
        reset
    }

    pub(crate) fn follow_session(
        &mut self,
        session: &Path,
        record: Option<crate::sessions::SessionKey>,
    ) {
        self.follow_resident_latest(session, record.as_ref());
        if self.session.as_deref() == Some(session) && self.record == record {
            self.jump_to_latest();
        } else if let Some(saved) = self.reading.get_mut(&(session.to_owned(), record)) {
            saved.follow_latest();
        }
    }

    pub(crate) fn jump_to_latest(&mut self) {
        if let Some(session) = &self.session
            && let Some(saved) = self
                .reading
                .get_mut(&(session.clone(), self.record.clone()))
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
        coverage: Option<HistoryCoverage>,
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
        self.coverage = coverage;
        if !self.rows.is_empty()
            && let Some(session) = &self.session
            && let std::collections::hash_map::Entry::Occupied(entry) =
                self.reading.entry((session.clone(), self.record.clone()))
            && let Some(coverage) = coverage
        {
            if let Some(start) = entry.get().source_offset(coverage, items) {
                let mut saved = entry.remove();
                saved.rebase(start, &self.rows);
                anchor = saved.restore(self, items);
            } else if coverage == HistoryCoverage::Complete {
                entry.remove();
                anchor = None;
                self.jump_to_latest();
            }
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
        self.preserve_page_position(items, saved_transcript, false)
    }

    pub(in crate::app) fn finish_local_display(&mut self, app: &FarcasterApp, saved: bool) {
        if self.selected_local_page(app).is_some() {
            self.preserve_page_position(&app.snapshot.transcript_presentation().items, saved, true);
        }
        self.local = None;
        self.resident.authorized = None;
    }

    fn preserve_page_position(
        &mut self,
        items: &PersistentVec<Arc<crate::conversation::TranscriptItem>>,
        saved_transcript: bool,
        local: bool,
    ) {
        let page = if local {
            self.local
                .as_ref()
                .and_then(|display| display.page.as_ref())
        } else {
            self.history_tail.as_ref()
        };
        if self.allows_history_tail()
            && let Some(page) = page
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
                    .get(&(path.clone(), self.record.clone()))
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
        self.retry_short_local_page(&app, cx);
        let app = app.read(cx);
        self.observe_ready_resident(app);

        let viewport = transcript::TranscriptViewport {
            switch_trace: app
                .lifecycle
                .switch_paint
                .as_ref()
                .filter(|trace| trace.active_for(app.composer.sessions.current_buffer()))
                .map(|trace| (trace.clone(), app.lifecycle.awaiting_switch_route.is_some())),
            font_scale: self.font_size / crate::app::ui::theme::theme().type_scale.reading,
            following: self.following,
            unseen: self.unseen,
            tail_reserve: transcript::tail_reserve(window.viewport_size().height),
        };
        let switching = app.lifecycle.pending_session_switch.is_some();
        let display = self.history_display(&app.snapshot);
        if switching || display == HistoryDisplay::Waiting {
            if let Some(view) = self.render_resident(app, viewport.clone()) {
                return view;
            }
            let tail = self.selected_local_page(app).or_else(|| {
                (!switching && self.allows_history_tail())
                    .then_some(self.history_tail.as_ref())
                    .flatten()
            });
            if let Some(tail) = tail {
                let viewport = transcript::TranscriptViewport {
                    following: tail.list.is_following_tail(),
                    unseen: tail.unseen,
                    ..viewport
                };
                let view = transcript::render(
                    &tail.list,
                    viewport,
                    tail.rows.clone(),
                    tail.presentation.clone(),
                    tail.disclosures.clone(),
                    tail.trees.clone(),
                    tail.markdown.clone(),
                    self.app.clone(),
                );
                use gpui::{InteractiveElement as _, ParentElement as _, Styled as _};
                return gpui::div()
                    .relative()
                    .size_full()
                    .child(view)
                    .child(gpui::div().absolute().inset_0().occlude())
                    .into_any_element();
            }
            return transcript::loading_history();
        }
        let document = transcript::render(
            &self.list,
            viewport,
            self.rows.clone(),
            app.snapshot.transcript_presentation(),
            self.disclosure_states.clone(),
            self.file_trees.clone(),
            self.markdown_cache.clone(),
            self.app.clone(),
        );
        if display == HistoryDisplay::PositionUnavailable {
            transcript::with_saved_position_notice(document)
        } else {
            document
        }
    }
}
