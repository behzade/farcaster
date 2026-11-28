use super::*;
use crate::app::infrastructure::performance::{SwitchPaintStage, SwitchPaintTrace};
use crate::sessions::{SessionKey, SessionSummary};
use crate::storage::{SessionRepository as _, TranscriptSession, TranscriptStamp};
use gpui::AppContext as _;

pub(in crate::app) enum DisplayRead {
    Resident(Option<TranscriptStamp>),
    Page(PreparedPage),
    Missing,
}

impl DisplayRead {
    pub(in crate::app) fn stamp(&self) -> Option<TranscriptStamp> {
        match self {
            Self::Resident(stamp) => *stamp,
            Self::Page(page) => page.stamp,
            Self::Missing => None,
        }
    }
}

#[derive(Clone)]
pub(in crate::app) struct DisplayQuery {
    pub trace: Option<Arc<SwitchPaintTrace>>,
    pub path: PathBuf,
    pub project: PathBuf,
    pub harness: crate::agents::Backend,
    pub profile: Option<String>,
}

impl DisplayQuery {
    pub(in crate::app) fn matches(&self, row: &SessionSummary) -> bool {
        row.path == self.path
            && row.project == self.project
            && row.harness == self.harness
            && row
                .profile_id
                .clone()
                .or_else(|| crate::agents::profile_id_from_locator(&row.path))
                == self.profile
    }

    pub(in crate::app) fn read(
        self,
        resident: Option<Option<TranscriptStamp>>,
        full: bool,
        cx: &mut Context<FarcasterApp>,
    ) -> gpui::Task<DisplayRead> {
        cx.background_spawn(async move {
            let mark = |stage| {
                if let Some(trace) = &self.trace {
                    trace.mark(stage);
                }
            };
            mark(SwitchPaintStage::ReaderStart);
            let session = TranscriptSession {
                path: &self.path,
                project: &self.project,
                harness: self.harness,
                profile_id: self.profile.as_deref(),
            };
            let result = crate::app::persistence::shared().and_then(|repository| {
                mark(SwitchPaintStage::RepositoryReady);
                if let Some(cached) = resident {
                    let current = repository.transcript_stamp(session)?;
                    if cached == current {
                        mark(SwitchPaintStage::ReadDone);
                        return Ok(DisplayRead::Resident(current));
                    }
                }
                let page = repository.load_transcript_display(session, full)?;
                mark(SwitchPaintStage::ReadDone);
                let compact = page.as_ref().is_some_and(|s| !s.full_document);
                let mut prepared =
                    page.and_then(|page| PreparedPage::from_store(page, &self.project));
                if compact && prepared.is_none() {
                    prepared = repository
                        .load_transcript_display(session, true)?
                        .and_then(|page| PreparedPage::from_store(page, &self.project));
                }
                Ok(prepared.map_or(DisplayRead::Missing, DisplayRead::Page))
            });
            mark(SwitchPaintStage::PagePrepared);
            result.unwrap_or_else(|error| {
                zlog::warn!("Read local transcript viewport: {error}");
                DisplayRead::Missing
            })
        })
    }
}

pub(super) struct LocalTranscript {
    request: u64,
    buffer: String,
    stamp: Option<TranscriptStamp>,
    pub(super) page: Option<HistoryTail>,
    _read: Option<gpui::Task<()>>,
}

impl LocalTranscript {
    fn matches(&self, app: &FarcasterApp) -> bool {
        self.request == app.lifecycle.switch_sequence
            && self.buffer == app.composer.sessions.current_buffer()
    }
}

impl FarcasterApp {
    pub(in crate::app) fn present_or_read_transcript(
        &mut self,
        query: DisplayQuery,
        key: Option<SessionKey>,
        prepared: Option<DisplayRead>,
        cx: &mut Context<Self>,
    ) {
        let request = self.lifecycle.switch_sequence;
        let buffer = self.composer.sessions.current_buffer().to_owned();
        let view = &self.views.transcript;
        let full = view.read(cx).needs_full_display(&query.path, key.clone());
        if let Some(read) = prepared {
            let installed = view.update(cx, |view, _| {
                view.local = None;
                match read {
                    DisplayRead::Page(prepared) => {
                        let stamp = prepared.stamp;
                        let page = view.local_layout(prepared, &query.path, key.clone());
                        view.local = Some(LocalTranscript {
                            request,
                            buffer: buffer.clone(),
                            stamp,
                            page,
                            _read: None,
                        });
                        true
                    }
                    DisplayRead::Resident(stamp) => self
                        .sidebar_catalog_row(&self.sessions.all)
                        .is_some_and(|row| view.authorize_resident(row, stamp, request)),
                    DisplayRead::Missing => true,
                }
            });
            if installed {
                return;
            }
        }
        let path = query.path.clone();
        let read = query.read(None, full, cx);
        let apply = cx.spawn(async move |weak, cx| {
            let DisplayRead::Page(prepared) = read.await else {
                return;
            };
            let _ = weak.update(cx, |app, cx| {
                if app.lifecycle.pending_session_switch.is_none() && !app.awaiting_history_rows(cx)
                {
                    return;
                }
                app.views.transcript.update(cx, |view, cx| {
                    if view
                        .local
                        .as_ref()
                        .is_some_and(|local| local.request == request && local.matches(app))
                    {
                        let stamp = prepared.stamp;
                        let page = view.local_layout(prepared, &path, key);
                        let local = view.local.as_mut().unwrap();
                        local.stamp = stamp;
                        local.page = page;
                        cx.notify();
                    }
                });
            });
        });
        self.views.transcript.update(cx, |view, _| {
            view.local = Some(LocalTranscript {
                request,
                buffer,
                stamp: None,
                page: None,
                _read: Some(apply),
            });
        });
    }
}

impl TranscriptView {
    pub(in crate::app) fn has_verified_display(&self, app: &FarcasterApp) -> bool {
        self.local
            .as_ref()
            .is_some_and(|local| local.matches(app) && local.stamp.is_some())
            || self.authorized_resident(app).is_some()
    }

    pub(in crate::app) fn needs_full_display(
        &self,
        path: &Path,
        record: Option<SessionKey>,
    ) -> bool {
        self.reading
            .get(&(path.to_owned(), record))
            .is_some_and(|state| !state.allows_history_tail())
    }

    pub(super) fn local_layout(
        &self,
        prepared: PreparedPage,
        path: &Path,
        record: Option<SessionKey>,
    ) -> Option<HistoryTail> {
        let full = prepared.full;
        let coverage = prepared.coverage;
        let mut page = HistoryTail::from_prepared(prepared);
        if full && let Some(saved) = self.reading.get(&(path.to_owned(), record)) {
            if let Some(start) = saved.source_offset(coverage, &page.presentation.items) {
                let mut saved = saved.clone();
                saved.rebase(start, &page.rows);
                saved.apply_to_local(&mut page);
            } else if coverage == crate::agents::HistoryCoverage::Suffix {
                return None;
            }
        }
        Some(page)
    }

    pub(super) fn selected_local_page(&self, app: &FarcasterApp) -> Option<&HistoryTail> {
        let local = self.local.as_ref().filter(|display| display.matches(app))?;
        local.page.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn local_display_items(
        &self,
        app: &FarcasterApp,
    ) -> Option<&PersistentVec<Arc<crate::conversation::TranscriptItem>>> {
        Some(&self.selected_local_page(app)?.presentation.items)
    }

    #[cfg(test)]
    pub(crate) fn local_display_row_count(&self, app: &FarcasterApp) -> usize {
        self.selected_local_page(app)
            .map_or(0, |page| page.rows.len())
    }
}
