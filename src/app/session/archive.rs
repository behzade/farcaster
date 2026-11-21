use std::path::{Path, PathBuf};

use gpui::{Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{
    runtime::{ArchiveRequest, ArchiveTarget, RuntimeCommand},
    sessions::{AppSessionId, DraftSession, SessionCatalog, SessionSummary},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) enum ChatArchiveTarget {
    Draft(String),
    Session(PathBuf),
}

pub(in crate::app) fn chat_archive_target(
    drafts: &[DraftSession],
    sessions: &[SessionSummary],
    app_session_id: i64,
) -> Option<ChatArchiveTarget> {
    if let Some(draft) = drafts
        .iter()
        .find(|draft| draft.app_session_id == app_session_id)
    {
        return Some(ChatArchiveTarget::Draft(draft.id.clone()));
    }
    sessions
        .iter()
        .find(|session| session.app_session_id == app_session_id)
        .map(|session| ChatArchiveTarget::Session(session.path.clone()))
}

pub(in crate::app) enum RailSessionIdentity {
    App(AppSessionId),
    Draft(String),
    Session(PathBuf, crate::sessions::NativeSessionIdentity),
}

pub(in crate::app) enum ArchiveConfirmationTarget {
    Session {
        path: PathBuf,
        key: Option<crate::sessions::SessionKey>,
    },
    Draft(DraftSession),
}

pub(in crate::app) struct PendingArchive {
    pub(in crate::app) focus: FocusHandle,
    target: ArchiveConfirmationTarget,
    return_focus: Option<FocusHandle>,
    pub(in crate::app) replacement: Option<RailSessionIdentity>,
}

#[derive(Default)]
pub(in crate::app) struct ArchiveWrites {
    next_sequence: u64,
    pending: std::collections::HashMap<ArchiveTarget, PendingWrite>,
    error: Option<(ArchiveTarget, String)>,
}

struct PendingWrite {
    confirmed: bool,
    intents: Vec<(u64, bool)>,
}

fn archive_target(row: &SessionSummary) -> ArchiveTarget {
    AppSessionId::new(row.app_session_id)
        .map(ArchiveTarget::App)
        .unwrap_or_else(|| ArchiveTarget::Native(row.target()))
}

impl ArchiveWrites {
    pub(in crate::app) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(in crate::app) fn needs_overlay(&self) -> bool {
        self.has_pending()
            || matches!(
                &self.error,
                Some((ArchiveTarget::Native(_) | ArchiveTarget::Path(_), _))
            )
    }
    pub(in crate::app) fn confirmed(&self, target: &ArchiveTarget) -> Option<bool> {
        self.pending.get(target).map(|write| write.confirmed)
    }

    fn begin(&mut self, target: ArchiveTarget, confirmed: bool, archived: bool) -> ArchiveRequest {
        self.next_sequence += 1;
        self.pending
            .entry(target.clone())
            .or_insert_with(|| PendingWrite {
                confirmed,
                intents: Vec::new(),
            })
            .intents
            .push((self.next_sequence, archived));
        ArchiveRequest {
            target,
            archived,
            sequence: Some(self.next_sequence),
        }
    }

    fn finish(&mut self, request: &ArchiveRequest, success: bool) -> Option<bool> {
        let Some(pending) = self.pending.get_mut(&request.target) else {
            return success.then_some(request.archived);
        };
        if success {
            pending.confirmed = request.archived;
        }
        pending
            .intents
            .retain(|(seq, _)| Some(*seq) != request.sequence);
        if let Some((_, archived)) = pending.intents.last() {
            return Some(*archived);
        }
        let confirmed = pending.confirmed;
        self.pending.remove(&request.target);
        Some(confirmed)
    }

    pub(in crate::app) fn rebind(&mut self, old: &ArchiveTarget, new: ArchiveTarget) {
        if let Some((target, _)) = self.error.as_mut()
            && target == old
        {
            *target = new.clone();
        }
        let Some(write) = self.pending.remove(old) else {
            return;
        };
        match self.pending.entry(new) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(write);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let pending = entry.get_mut();
                pending.intents.extend(write.intents);
                pending.intents.sort_by_key(|(seq, _)| *seq);
            }
        }
    }

    pub(in crate::app) fn overlay(&mut self, row: &mut SessionSummary, catalog: &SessionCatalog) {
        if !self.needs_overlay() {
            return;
        }
        let key = archive_target(row);
        if matches!(key, ArchiveTarget::App(_)) {
            // Metadata can publish the durable ID before the archive receipt. The old
            // record may still use its locator as its backend ID.
            if let Some(previous) = catalog
                .matching_record(row)
                .filter(|old| old.app_session_id <= 0)
            {
                self.rebind(&archive_target(previous), key.clone());
                self.rebind(&ArchiveTarget::Path(previous.path.clone()), key.clone());
            }
            self.rebind(&ArchiveTarget::Native(row.target()), key.clone());
            self.rebind(&ArchiveTarget::Path(row.path.clone()), key.clone());
        }
        if let Some(write) = self.pending.get_mut(&key) {
            write.confirmed = row.archived;
            row.archived = write.intents.last().expect("pending archive intent").1;
        }
    }
}

impl FarcasterApp {
    pub(in crate::app) fn record_archive_draft_identity(&mut self, draft: &str, id: AppSessionId) {
        self.sessions.archive_writes.rebind(
            &ArchiveTarget::Draft(draft.to_owned()),
            ArchiveTarget::App(id),
        );
        if let Some(pending) = &mut self.sessions.pending_archive
            && let ArchiveConfirmationTarget::Draft(original) = &mut pending.target
            && original.id == draft
        {
            original.app_session_id = id.get();
        }
    }

    pub(in crate::app) fn adopt_draft_identity(&mut self, draft: String, id: AppSessionId) -> bool {
        self.record_archive_draft_identity(&draft, id);
        if let Some(row) = self.sessions.drafts.iter_mut().find(|row| row.id == draft) {
            if row.app_session_id > 0 {
                return false;
            }
            row.app_session_id = id.get();
            let project = row.project.clone();
            self.sessions
                .draft_session_ids
                .insert(draft.clone(), id.get());
            self.sessions.pending_draft_saves.remove(&draft);
            self.composer
                .sessions
                .resave_buffer(&crate::sessions::draft_target(&draft));
            self.remember_rail_projects([project]);
            if self.sessions.selected_draft.as_deref() == Some(&draft) {
                self.remember_session_selection();
            }
            return true;
        }
        false
    }

    pub(in crate::app) fn queue_archive(
        &mut self,
        target: ArchiveTarget,
        previous: bool,
        archived: bool,
        draft_saved: Option<async_channel::Receiver<Result<AppSessionId, String>>>,
        cx: &mut Context<Self>,
    ) {
        if let ArchiveTarget::Native(native) = &target {
            self.sessions.all.remember_archive_path(&native.path);
            self.sessions.visible.remember_archive_path(&native.path);
        }
        let request = self
            .sessions
            .archive_writes
            .begin(target, previous, archived);
        if let Err(message) = self.runtime.send(RuntimeCommand::SetSessionArchived {
            request: request.clone(),
            draft_saved,
        }) {
            self.finish_archive(request, Err(message), cx);
        } else {
            self.show_archive_state(&request.target, archived, cx);
        }
    }

    fn adopt_archived_catalog_row(
        &mut self,
        stored: &SessionSummary,
        requested_path: Option<&Path>,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = AppSessionId::new(stored.app_session_id) else {
            return;
        };
        let all = self
            .sessions
            .all
            .adopt_archive_identity(stored, requested_path);
        let visible = self
            .sessions
            .visible
            .adopt_archive_identity(stored, requested_path);
        if all.is_none() && visible.is_none() {
            return;
        }
        if all.is_some() {
            let catalog = &self.sessions.all;
            let family = catalog
                .by_app_id(id.get())
                .and_then(|row| catalog.root_for_row(row))
                .map(SessionSummary::catalog_key);
            let panel = self.views.run_panel.read(cx);
            if let Some(family) = family {
                // Once its missing parent gains an ID, the cached orphan or its
                // former locator can identify a different root, even off screen.
                if panel.prepared_worker_root().is_some_and(|key| {
                    catalog
                        .by_key(&key)
                        .and_then(|row| catalog.root_for_row(row))
                        .is_none_or(|row| row.catalog_key() == family)
                }) {
                    panel.invalidate_workers();
                }
                if self
                    .sidebar_worker_root()
                    .is_some_and(|root| root.catalog_key() == family)
                {
                    self.notify_run_panel(cx);
                }
            }
        }
        for native in all.into_iter().chain(visible) {
            self.sessions
                .archive_writes
                .rebind(&ArchiveTarget::Native(native), ArchiveTarget::App(id));
        }
        self.notify_session_rail(cx);
    }

    pub(in crate::app) fn finish_archive(
        &mut self,
        request: ArchiveRequest,
        rows: Result<Vec<SessionSummary>, String>,
        cx: &mut Context<Self>,
    ) {
        let mut request = request;
        if let Ok(rows) = &rows {
            let requested_path = match &request.target {
                ArchiveTarget::Native(native) => Some(native.path.as_path()),
                ArchiveTarget::Path(path) => Some(path.as_path()),
                _ => None,
            };
            for row in rows {
                self.adopt_archived_catalog_row(row, requested_path, cx);
            }
            if let [row] = rows.as_slice()
                && let Some(id) = AppSessionId::new(row.app_session_id)
                && matches!(
                    request.target,
                    ArchiveTarget::Native(_) | ArchiveTarget::Path(_)
                )
            {
                let target = ArchiveTarget::App(id);
                self.sessions
                    .archive_writes
                    .rebind(&request.target, target.clone());
                request.target = target;
            }
        }
        if let Some(sequence) = request.sequence
            && !self
                .sessions
                .archive_writes
                .pending
                .contains_key(&request.target)
            && let Some((target, _)) = self
                .sessions
                .archive_writes
                .pending
                .iter()
                .find(|(_, write)| write.intents.iter().any(|&(id, _)| id == sequence))
        {
            request.target = target.clone();
        }
        let success = rows.is_ok();
        let value = self.sessions.archive_writes.finish(&request, success);
        match rows {
            Ok(rows) => {
                if let Some((target, previous)) = &self.sessions.archive_writes.error
                    && target == &request.target
                {
                    if self.sessions.error.as_ref() == Some(previous) {
                        self.sessions.error = None;
                        self.notify_session_rail_shell(cx);
                    }
                    self.sessions.archive_writes.error = None;
                }
                for row in rows {
                    let target = archive_target(&row);
                    if target != request.target {
                        let mut row = row;
                        self.sessions
                            .archive_writes
                            .overlay(&mut row, &self.sessions.all);
                        self.show_archive_state(&target, row.archived, cx);
                    }
                }
            }
            Err(message) => {
                let message = if request.sequence.is_none() {
                    format!("Session family stopped, but could not be archived: {message}")
                } else {
                    message
                };
                self.sessions.archive_writes.error =
                    Some((request.target.clone(), message.clone()));
                self.sessions.error = Some(message);
                self.notify_session_rail_shell(cx);
            }
        }
        if let Some(archived) = value {
            self.show_archive_state(&request.target, archived, cx);
        }
        if success
            && !request.archived
            && !self
                .sessions
                .archive_writes
                .pending
                .contains_key(&request.target)
            && value == Some(false)
            && self.sessions.archived_expanded
            && !self.sessions.drafts.iter().any(|draft| draft.archived)
            && !self.sessions.visible.roots().any(|row| row.archived)
        {
            self.sessions.archived_expanded = false;
            self.notify_session_rail_shell(cx);
        }
    }

    fn show_archive_state(
        &mut self,
        target: &ArchiveTarget,
        archived: bool,
        cx: &mut Context<Self>,
    ) {
        let row = match target {
            ArchiveTarget::Draft(_) => None,
            ArchiveTarget::Native(t) => self
                .sessions
                .all
                .at_path(&t.path)
                .chain(self.sessions.visible.at_path(&t.path))
                .find(|row| row.harness == t.harness && row.id == t.id),
            ArchiveTarget::App(id) => self
                .sessions
                .all
                .by_app_id(id.get())
                .or_else(|| self.sessions.visible.by_app_id(id.get())),
            ArchiveTarget::Path(path) => self
                .sessions
                .all
                .by_path(path)
                .or_else(|| self.sessions.visible.by_path(path)),
        };
        let row = row.map(|row| (row.target(), row.app_session_id, row.catalog_key()));
        let mut changed = false;
        if let Some((native, id, key)) = row {
            changed |= self.sessions.all.set_row_archived(&native, id, archived);
            changed |= self
                .sessions
                .visible
                .set_row_archived(&native, id, archived);
            if changed
                && let Some(root) = self
                    .sessions
                    .all
                    .by_key(&key)
                    .and_then(|row| self.sessions.all.root_for_row(row))
            {
                let visible_family = self
                    .sidebar_worker_root()
                    .is_some_and(|row| row.same_record(root));
                let panel = self.views.run_panel.read(cx);
                if panel.has_prepared_workers_for(&root.catalog_key()) || visible_family {
                    panel.invalidate_workers();
                }
                if visible_family {
                    self.notify_run_panel(cx);
                }
            }
        }
        if let Some(draft) = self.sessions.drafts.iter_mut().find(|row| match target {
            ArchiveTarget::App(id) => row.app_session_id == id.get(),
            ArchiveTarget::Draft(key) => row.id == *key,
            ArchiveTarget::Native(_) | ArchiveTarget::Path(_) => false,
        }) {
            changed |= draft.archived != archived;
            draft.archived = archived;
        }
        if changed {
            self.notify_session_rail(cx);
        }
    }

    pub(in crate::app) fn request_session_archive(
        &mut self,
        path: PathBuf,
        archive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = self
            .sessions
            .all
            .by_path(&path)
            .or_else(|| self.sessions.visible.by_path(&path))
            .map(SessionSummary::catalog_key);
        self.request_archive(path, key, archive, window, cx);
    }

    pub(in crate::app) fn request_rail_archive(
        &mut self,
        key: crate::sessions::SessionKey,
        archive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self
            .sessions
            .all
            .by_key(&key)
            .or_else(|| self.sessions.visible.by_key(&key))
        else {
            return;
        };
        self.request_archive(row.path.clone(), Some(key), archive, window, cx);
    }

    fn request_archive(
        &mut self,
        path: PathBuf,
        key: Option<crate::sessions::SessionKey>,
        archive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalog = &self.sessions.all;
        let row = key.as_ref().and_then(|key| catalog.by_key(key));
        if !archive
            || !super::activity::session_row_family_has_active_work(
                catalog,
                row,
                &path,
                &self.activity.run_statuses,
                &self.activity.agents,
                &self.snapshot,
                &self.composer.pending_submissions,
            )
        {
            let row = row.or_else(|| {
                key.as_ref()
                    .and_then(|key| self.sessions.visible.by_key(key))
            });
            let (target, previous) = row
                .map(|row| (archive_target(row), row.archived))
                .unwrap_or_else(|| (ArchiveTarget::Path(path), !archive));
            self.queue_archive(target, previous, archive, None, cx);
            return;
        }

        self.confirm_archive(ArchiveConfirmationTarget::Session { path, key }, window, cx);
    }

    pub(in crate::app) fn confirm_archive(
        &mut self,
        target: ArchiveConfirmationTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cover_native_workspace_surface(cx);
        let pending = PendingArchive {
            focus: cx.focus_handle(),
            target,
            return_focus: window.focused(cx),
            replacement: None,
        };
        self.focus_overlay(&pending.focus, window, cx);
        self.sessions.pending_archive = Some(pending);
        cx.notify();
    }

    pub(in crate::app) fn request_chat_archive(
        &mut self,
        app_session_id: i64,
        archive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match chat_archive_target(
            &self.sessions.drafts,
            &self.sessions.visible,
            app_session_id,
        ) {
            Some(ChatArchiveTarget::Draft(id)) => {
                self.request_draft_archive(id, archive, window, cx);
            }
            Some(ChatArchiveTarget::Session(path)) => {
                if let Some(id) = AppSessionId::new(app_session_id) {
                    self.request_rail_archive(
                        crate::sessions::SessionKey::App(id),
                        archive,
                        window,
                        cx,
                    );
                } else {
                    self.request_session_archive(path, archive, window, cx);
                }
            }
            None => {}
        }
    }

    pub(in crate::app) fn stop_and_archive_pending_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.close_archive_confirmation(window, cx) else {
            return;
        };
        match pending.target {
            ArchiveConfirmationTarget::Session { path, key } => {
                self.send(RuntimeCommand::StopSessionFamily { path, key }, cx)
            }
            ArchiveConfirmationTarget::Draft(original) => {
                let identity = if let Some(draft) = self
                    .sessions
                    .drafts
                    .iter()
                    .find(|row| row.id == original.id)
                {
                    // A first send can be rejected while the confirmation is open.
                    if !self.can_archive_draft(draft) {
                        return;
                    }
                    let mut draft = draft.clone();
                    let target = AppSessionId::new(draft.app_session_id)
                        .map(ArchiveTarget::App)
                        .unwrap_or_else(|| ArchiveTarget::Draft(draft.id.clone()));
                    if let Some(confirmed) = self.sessions.archive_writes.confirmed(&target) {
                        draft.archived = confirmed;
                    }
                    match self.sessions.writer.save_draft_with_identity(draft) {
                        Ok(saved) => crate::runtime::DraftArchiveIdentity::Pending(saved),
                        Err(error) => {
                            self.sessions.error = Some(error);
                            self.notify_session_rail_shell(cx);
                            return;
                        }
                    }
                } else if let Some(id) = AppSessionId::new(original.app_session_id) {
                    crate::runtime::DraftArchiveIdentity::Saved(id)
                } else {
                    self.sessions.error = Some("The chat is no longer available to archive".into());
                    self.notify_session_rail_shell(cx);
                    return;
                };
                self.send(
                    RuntimeCommand::StopAndArchiveDraft {
                        draft: original.id,
                        identity,
                    },
                    cx,
                );
            }
        }
        if let Some(replacement) = pending.replacement {
            self.select_visible_replacement(replacement, window, cx);
        }
    }

    pub(in crate::app) fn close_archive_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingArchive> {
        let pending = self.sessions.pending_archive.take()?;
        self.restore_overlay_focus(pending.return_focus.clone(), &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some(pending)
    }
}

pub(in crate::app) fn session_event_affects_archived_rail(
    sessions: &SessionCatalog,
    target: &str,
    session_path: Option<&Path>,
) -> bool {
    session_path
        .and_then(|path| sessions.root_for_path(Some(path)))
        .or_else(|| sessions.root_for_path(crate::sessions::session_path(target)))
        .is_some_and(|root| root.archived)
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;
