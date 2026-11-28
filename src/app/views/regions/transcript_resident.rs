use super::*;
use crate::app::ui::change_tree::ChangeTreeState;
use crate::sessions::{SessionKey, SessionSummary};
use crate::storage::TranscriptStamp;
use gpui::{AnyElement, InteractiveElement as _, ParentElement as _, Styled as _};

const CAPACITY: usize = 4;

struct ResidentDocument {
    key: crate::sessions::SessionKey,
    native_id: String,
    path: PathBuf,
    project: PathBuf,
    harness: crate::agents::Backend,
    profile: Option<String>,
    modified: std::time::SystemTime,
    message_count: usize,
    presentation: Arc<TranscriptPresentation>,
    stamp: Option<TranscriptStamp>,
    font_size: gpui::Pixels,
}

impl ResidentDocument {
    fn matches(&self, row: &SessionSummary, font_size: gpui::Pixels) -> bool {
        !row.is_running
            && self.font_size == font_size
            && self.path == row.path
            && self.harness == row.harness
            && self.project == row.project
            && self.profile == row.profile_id
            && self.native_id == row.id
            && self.modified == row.modified
            && self.message_count == row.message_count
            && self.key == row.catalog_key()
    }
}

struct ResidentLayout {
    document: ResidentDocument,
    rows: Arc<PersistentVec<transcript::TranscriptRow>>,
    list: transcript::list::TranscriptListState,
    markdown: transcript::markdown::TranscriptMarkdownCache,
    disclosures: HashMap<usize, bool>,
    trees: HashMap<usize, ChangeTreeState>,
    unseen: usize,
}

#[derive(Default)]
pub(super) struct ResidentTranscripts {
    active: Option<ResidentDocument>,
    pub(super) authorized: Option<(u64, SessionKey, Option<TranscriptStamp>)>,
    parked: std::collections::VecDeque<ResidentLayout>,
}

impl TranscriptView {
    pub(super) fn observe_ready_resident(&mut self, app: &FarcasterApp) {
        let snapshot = &app.snapshot;
        if !snapshot.document_ready
            || snapshot.status != "Ready"
            || snapshot.conversation.running
            || snapshot.selected_session != self.session
            || self.rows.is_empty()
        {
            self.resident.active = None;
            return;
        }
        let Some(row) = self
            .session
            .as_ref()
            .and_then(|path| snapshot.catalog_row_at(&app.sessions.all, path))
        else {
            self.resident.active = None;
            return;
        };
        if row.is_running
            || snapshot.harness != Some(row.harness)
            || snapshot.project != row.project
            || snapshot.profile_id != row.profile_id
        {
            self.resident.active = None;
            return;
        }
        let presentation = snapshot.transcript_presentation();
        if let Some(active) = &mut self.resident.active
            && active.matches(row, self.font_size)
        {
            active.presentation = presentation;
            active.stamp = snapshot.transcript_stamp;
            return;
        }
        self.resident.active = Some(ResidentDocument {
            key: row.catalog_key(),
            native_id: row.id.clone(),
            path: row.path.clone(),
            project: row.project.clone(),
            profile: row.profile_id.clone(),
            harness: row.harness,
            modified: row.modified,
            message_count: row.message_count,
            presentation,
            stamp: snapshot.transcript_stamp,
            font_size: self.font_size,
        });
    }

    pub(super) fn park_resident(
        &mut self,
        session: &Path,
        items: &PersistentVec<Arc<crate::conversation::TranscriptItem>>,
        reading: &ReadingState,
    ) {
        let Some(document) = self.resident.active.take() else {
            return;
        };
        if document.path != session || !document.presentation.items.shares_storage(items) {
            return;
        }
        self.resident
            .parked
            .retain(|old| old.document.key != document.key);
        if self.resident.parked.len() == CAPACITY {
            self.resident.parked.pop_front();
        }
        let list = transcript::list::TranscriptListState::new();
        self.list.move_layout_to(&list);
        self.resident.parked.push_back(ResidentLayout {
            document,
            rows: self.rows.clone(),
            list,
            markdown: std::mem::take(&mut self.markdown_cache),
            disclosures: reading.disclosure_states(),
            trees: reading.file_trees(),
            unseen: self.unseen,
        });
    }

    pub(super) fn follow_resident_latest(&mut self, path: &Path, record: Option<&SessionKey>) {
        if let Some(entry) = self
            .resident
            .parked
            .iter_mut()
            .find(|entry| entry.document.path == path && Some(&entry.document.key) == record)
        {
            entry.list.scroll_to_end();
            entry.unseen = 0;
        }
    }

    fn resident_document(&self, row: &SessionSummary) -> Option<&ResidentDocument> {
        self.resident
            .parked
            .iter()
            .map(|entry| &entry.document)
            .chain(self.resident.active.iter())
            .find(|doc| doc.matches(row, self.font_size))
    }

    pub(in crate::app) fn has_resident_for(&self, row: &SessionSummary) -> bool {
        self.resident_document(row).is_some()
    }

    pub(in crate::app) fn resident_stamp_for(
        &self,
        row: &SessionSummary,
    ) -> Option<TranscriptStamp> {
        self.resident_document(row)?.stamp
    }

    pub(in crate::app) fn authorize_resident(
        &mut self,
        row: &SessionSummary,
        observed: Option<TranscriptStamp>,
        request: u64,
    ) -> bool {
        let valid = self
            .resident_document(row)
            .is_some_and(|doc| doc.stamp == observed);
        self.resident.authorized = valid.then(|| (request, row.catalog_key(), observed));
        valid
    }

    pub(super) fn authorized_resident<'a>(
        &self,
        app: &'a FarcasterApp,
    ) -> Option<&'a SessionSummary> {
        let row = app.sidebar_catalog_row(&app.sessions.all)?;
        let (request, key, stamp) = self.resident.authorized.as_ref()?;
        (*request == app.lifecycle.switch_sequence
            && key == &row.catalog_key()
            && self
                .resident_document(row)
                .is_some_and(|doc| doc.stamp == *stamp))
        .then_some(row)
    }

    pub(super) fn render_resident(
        &self,
        app: &FarcasterApp,
        viewport: transcript::TranscriptViewport,
    ) -> Option<AnyElement> {
        let row = self.authorized_resident(app)?;
        let (list, rows, presentation, disclosures, trees, markdown, unseen) = if let Some(entry) =
            self.resident
                .parked
                .iter()
                .find(|entry| entry.document.matches(row, self.font_size))
        {
            (
                &entry.list,
                &entry.rows,
                &entry.document.presentation,
                &entry.disclosures,
                &entry.trees,
                &entry.markdown,
                entry.unseen,
            )
        } else {
            let doc = self
                .resident
                .active
                .as_ref()
                .filter(|doc| doc.matches(row, self.font_size))?;
            (
                &self.list,
                &self.rows,
                &doc.presentation,
                &self.disclosure_states,
                &self.file_trees,
                &self.markdown_cache,
                self.unseen,
            )
        };
        let rendered = transcript::render(
            list,
            transcript::TranscriptViewport {
                following: list.is_following_tail(),
                unseen,
                ..viewport
            },
            rows.clone(),
            presentation.clone(),
            disclosures.clone(),
            trees.clone(),
            markdown.clone(),
            self.app.clone(),
        );
        Some(
            gpui::div()
                .relative()
                .size_full()
                .child(rendered)
                .child(gpui::div().absolute().inset_0().occlude())
                .into_any_element(),
        )
    }

    pub(in crate::app) fn restore_resident(
        &mut self,
        row: &SessionSummary,
        snapshot: &crate::runtime::RuntimeSnapshot,
    ) -> bool {
        if !snapshot.document_ready || snapshot.status != "Ready" || snapshot.conversation.running {
            return false;
        }
        let Some(index) = self.resident.parked.iter().position(|entry| {
            entry.document.matches(row, self.font_size)
                && entry.document.stamp == snapshot.transcript_stamp
        }) else {
            return false;
        };
        let delivered = snapshot.transcript_presentation();
        let source = &self.resident.parked[index].document.presentation;
        if !source.items.shares_storage(&delivered.items)
            || source.active_start != delivered.active_start
            || (!Arc::ptr_eq(&source.completed_runs, &delivered.completed_runs)
                && source.completed_runs != delivered.completed_runs)
        {
            return false;
        }
        let cached = self.resident.parked.remove(index).expect("matched entry");
        cached.list.move_layout_to(&self.list);
        self.rows = cached.rows;
        self.following = self.list.is_following_tail();
        self.unseen = cached.unseen;
        self.disclosure_states = cached.disclosures;
        self.file_trees = cached.trees;
        self.markdown_cache = cached.markdown;
        self.last_count = self.rows.len();
        self.reading
            .remove(&(row.path.clone(), Some(row.catalog_key())));
        self.pending_reading_anchor = None;
        self.resident.active = Some(cached.document);
        true
    }
}
