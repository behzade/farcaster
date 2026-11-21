use crate::{agents::Backend, runtime::RunStatus};
use std::{collections::HashMap, path::PathBuf};

use gpui::{Context, Window};

use super::FarcasterApp;
use crate::{
    app::composer::{
        sessions::{draft_target, session_target},
        submissions::{PendingSubmission, has_pending_submission},
    },
    projects,
    runtime::RuntimeCommand,
    sessions::{
        DraftSession, establish_submission, fill_session_association, normalize_session_path,
        reconciliation_candidates, sync_materialized_draft, update_persisted_submission,
    },
};

#[cfg(test)]
use crate::sessions::{provisional_session_title, submitted_draft_associations};

impl FarcasterApp {
    pub(in crate::app) fn save_session_draft(&mut self, id: &str) -> bool {
        self.save_draft_snapshot(id, false)
    }

    pub(in crate::app) fn save_draft_before_handoff(&mut self, id: &str) -> bool {
        self.save_draft_snapshot(id, true)
    }

    fn save_draft_snapshot(&mut self, id: &str, needs_identity_now: bool) -> bool {
        if let Err(error) = self.persist_session_draft(id, needs_identity_now) {
            self.sessions.error = Some(error);
            return false;
        }
        true
    }

    fn persist_session_draft(&mut self, id: &str, needs_identity_now: bool) -> Result<(), String> {
        if self.sessions.pending_folder_drafts.contains_key(id) {
            return Ok(());
        }
        let Some(draft) = self.sessions.drafts.iter_mut().find(|draft| draft.id == id) else {
            return Ok(());
        };
        self.sessions.pending_draft_saves.insert(id.to_owned());
        let project = draft.project.clone();
        if draft.app_session_id <= 0 {
            let mut snapshot = draft.clone();
            let target = crate::runtime::ArchiveTarget::Draft(id.to_owned());
            if let Some(confirmed) = self.sessions.archive_writes.confirmed(&target) {
                snapshot.archived = confirmed;
            }
            if !needs_identity_now {
                if let Some(last) = self.sessions.allocating_drafts.get_mut(id) {
                    *last = snapshot;
                } else {
                    let receipt = self
                        .sessions
                        .writer
                        .save_draft_with_identity(snapshot.clone())?;
                    self.sessions
                        .draft_receipts
                        .try_send((snapshot.clone(), receipt))
                        .map_err(|_| "Draft save observer stopped".to_owned())?;
                    self.sessions
                        .allocating_drafts
                        .insert(id.to_owned(), snapshot);
                }
                return Ok(());
            }
            let app_session_id = self.sessions.writer.save_draft_before_handoff(snapshot)?;
            self.sessions.allocating_drafts.remove(id);
            self.adopt_draft_identity(id.to_owned(), app_session_id);
            return Ok(());
        }
        self.sessions.writer.save_draft(draft.clone())?;
        self.sessions.allocating_drafts.remove(id);
        self.sessions.pending_draft_saves.remove(id);
        self.remember_rail_projects([project]);
        if self.sessions.selected_draft.as_deref() == Some(id) {
            self.remember_session_selection();
        }
        Ok(())
    }

    pub(in crate::app) fn remove_session_draft(&mut self, id: &str) {
        if let Err(error) = self.sessions.writer.remove_draft(id.to_owned()) {
            self.sessions.error = Some(error);
        }
    }

    fn finish_draft_change(
        &mut self,
        mut previous: DraftSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = previous.id.clone();
        if self.sessions.pending_folder_drafts.contains_key(&id) {
            if let Some(row) = self.sessions.drafts.iter().find(|row| row.id == id)
                && row.project != self.project.path
            {
                self.select_project_for_navigation(row.project.clone(), cx);
            }
            self.notify_session_rail(cx);
            self.notify_composer(cx);
            return;
        }
        if !self.save_draft_before_handoff(&id) {
            if let Some(draft) = self.sessions.drafts.iter_mut().find(|draft| draft.id == id) {
                previous.app_session_id = draft.app_session_id;
                *draft = previous;
            }
            self.notify_session_rail(cx);
            return;
        }
        match futures::executor::block_on(self.sessions.writer.flush()) {
            Ok(()) => {
                let draft = self
                    .sessions
                    .drafts
                    .iter()
                    .find(|row| row.id == id)
                    .expect("changed draft was saved");
                let project = draft.project.clone();
                let harness = draft.harness;
                let profile_id = draft.profile_id.clone();
                previous.app_session_id = draft.app_session_id;
                // A new profile choice supersedes the same draft's pending trust request.
                if self.project.pending_trust_command.take().is_some() {
                    self.dismiss_project_trust(window, cx);
                }
                self.begin_navigation_from_draft(Some(previous));
                if self.project.path != project {
                    self.select_project(project.clone(), cx);
                }
                self.send_project_command(
                    &project,
                    RuntimeCommand::NewSession {
                        request: self.lifecycle.awaiting_switch_route,
                        id,
                        harness,
                        profile_id,
                        project: project.clone(),
                    },
                    window,
                    cx,
                );
            }
            Err(error) => {
                if let Some(draft) = self.sessions.drafts.iter_mut().find(|draft| draft.id == id) {
                    previous.app_session_id = draft.app_session_id;
                    *draft = previous;
                    let _ = self.sessions.writer.save_draft(draft.clone());
                }
                self.sessions.error = Some(error);
            }
        }
        self.notify_session_rail(cx);
        self.notify_composer(cx);
        cx.notify();
    }

    pub(in crate::app) fn available_projects(&self) -> Vec<PathBuf> {
        available_projects(&self.project.registered, &self.project.path)
    }

    pub(in crate::app) fn selected_draft_is_empty_and_unsubmitted(&self) -> bool {
        self.snapshot.conversation.items.is_empty()
            && self.sessions.selected_draft.as_ref().is_some_and(|id| {
                !self.sessions.submitted_drafts.contains_key(id)
                    && !has_pending_submission(
                        &self.composer.pending_submissions,
                        &draft_target(id),
                    )
            })
    }

    pub(in crate::app) fn editable_draft_project(&self) -> Option<PathBuf> {
        let id = self.sessions.selected_draft.as_deref()?;
        let target = draft_target(id);
        if self.composer.sessions.current_target() != target
            || self.sessions.submitted_drafts.contains_key(id)
            || has_pending_submission(&self.composer.pending_submissions, &target)
        {
            return None;
        }
        self.sessions
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .map_or_else(
                || Some(self.project.path.clone()),
                |draft| draft.can_change_project().then(|| draft.project.clone()),
            )
    }

    pub(in crate::app) fn active_harness(&self) -> Option<Backend> {
        if let Some(id) = self.sessions.selected_draft.as_deref()
            && let Some(draft) = self.sessions.drafts.iter().find(|draft| draft.id == id)
        {
            return draft.harness;
        }
        self.snapshot
            .selected_session
            .as_deref()
            .and_then(|path| {
                self.sessions
                    .all
                    .iter()
                    .find(|session| session.path == path)
            })
            .map(|session| session.harness)
            .or(self.snapshot.harness)
    }

    pub(in crate::app) fn active_profile_id(&self) -> Option<String> {
        if let Some(id) = self.sessions.selected_draft.as_deref()
            && let Some(draft) = self.sessions.drafts.iter().find(|draft| draft.id == id)
        {
            return draft.profile_id.clone();
        }
        self.snapshot
            .selected_session
            .as_deref()
            .and_then(|path| {
                self.sessions
                    .all
                    .iter()
                    .find(|session| session.path == path)
            })
            .map(|session| session.profile_id.clone())
            .unwrap_or_else(|| self.snapshot.profile_id.clone())
    }

    pub(in crate::app) fn editable_draft_harness(&self) -> Option<Option<Backend>> {
        let id = self.sessions.selected_draft.as_deref()?;
        let target = draft_target(id);
        if self.composer.sessions.current_target() != target
            || self.sessions.submitted_drafts.contains_key(id)
            || has_pending_submission(&self.composer.pending_submissions, &target)
        {
            return None;
        }
        Some(
            self.sessions
                .drafts
                .iter()
                .find(|draft| draft.id == id)
                .map_or_else(|| self.snapshot.harness, |draft| draft.harness),
        )
    }

    pub(in crate::app) fn change_draft_harness(
        &mut self,
        harness: Backend,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.change_draft_harness_choice(harness, None, window, cx);
    }

    pub(in crate::app) fn change_draft_harness_profile(
        &mut self,
        harness: Backend,
        profile_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = self.settings.harness_profiles.get(&profile_id);
        if !profile.is_ok_and(|profile| profile.backend == harness) {
            self.sessions.error = Some("Harness profile is unavailable".into());
            self.notify_session_rail(cx);
            return;
        }
        self.change_draft_harness_choice(harness, Some(profile_id), window, cx);
    }

    fn change_draft_harness_choice(
        &mut self,
        harness: Backend,
        profile_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.sessions.selected_draft.clone() else {
            return;
        };
        if self.editable_draft_harness().is_none()
            || self.project.pending_trust_command.as_ref().is_some_and(|command| {
                !matches!(command, RuntimeCommand::NewSession { id: pending, .. } if pending == &id)
            })
        {
            return;
        }
        if Self::harness_sign_in_required(harness, profile_id.as_deref()) {
            self.sessions.error =
                Some("Sign in in Settings → Connections before selecting this harness".into());
            self.notify_session_rail(cx);
            cx.notify();
            return;
        }
        if let Err(error) = crate::app::persistence::open().and_then(|store| {
            store.save_preferred_harness(harness)?;
            store.save_preferred_profile_id(profile_id.as_deref())
        }) {
            self.sessions.error = Some(error);
            self.notify_session_rail(cx);
            cx.notify();
            return;
        }
        self.sessions.preferred_harness = Some(harness);
        self.sessions.preferred_profile_id = profile_id.clone();
        let draft = self.materialize_selected_draft(&id);
        let previous = draft.clone();
        let changed = match profile_id {
            Some(profile_id) => draft.change_profile(harness, profile_id),
            None => draft.change_harness(Some(harness)),
        };
        if !changed {
            self.engage_current_draft();
            return;
        }
        self.finish_draft_change(previous, window, cx);
    }

    fn materialize_selected_draft(&mut self, id: &str) -> &mut DraftSession {
        sync_materialized_draft(
            &mut self.sessions.drafts,
            id,
            self.sessions
                .draft_session_ids
                .get(id)
                .copied()
                .unwrap_or_default(),
            &self.project.path,
            self.snapshot.harness,
        );
        self.sessions
            .drafts
            .iter_mut()
            .find(|draft| draft.id == id)
            .expect("selected draft was materialized")
    }

    pub(in crate::app) fn change_draft_project(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.pending_trust_command.is_some() {
            return;
        }
        let Some(id) = self.sessions.selected_draft.clone() else {
            return;
        };
        let target = draft_target(&id);
        if self.sessions.submitted_drafts.contains_key(&id)
            || has_pending_submission(&self.composer.pending_submissions, &target)
        {
            return;
        }
        if self.composer.sessions.current_target() != target {
            return;
        }
        let draft = self.materialize_selected_draft(&id);
        let previous = draft.clone();
        if draft.change_project(project) {
            self.finish_draft_change(previous, window, cx);
        }
    }

    pub(in crate::app) fn engage_current_draft(&mut self) {
        let Some(id) = self.sessions.selected_draft.clone() else {
            return;
        };
        if self.composer.sessions.current_target() != draft_target(&id) {
            return;
        }
        let Some(draft) = self.sessions.drafts.iter().find(|draft| draft.id == id) else {
            return;
        };
        if draft.app_session_id <= 0
            && !self.sessions.submitted_drafts.contains_key(&id)
            && !self.sessions.allocating_drafts.contains_key(&id)
            && !has_pending_submission(
                &self.composer.pending_submissions,
                self.composer.sessions.current_target(),
            )
        {
            self.save_session_draft(&id);
        }
    }

    pub(in crate::app) fn sync_drafts_before_quit(&mut self, target: &str) -> bool {
        if !self.sync_draft_before_leaving(target, true) {
            return false;
        }
        // A parked composer can still be waiting for its first SQLite row.
        for id in self.sessions.pending_draft_saves.clone() {
            if !self.sessions.submitted_drafts.contains_key(&id)
                && !self.save_draft_before_handoff(&id)
            {
                return false;
            }
        }
        true
    }

    pub(in crate::app) fn park_current_draft(&mut self, target: &str) {
        self.sync_draft_before_leaving(target, false);
    }

    fn sync_draft_before_leaving(&mut self, target: &str, needs_identity_now: bool) -> bool {
        let Some(id) = self.sessions.selected_draft.as_deref() else {
            return true;
        };
        let id = id.to_owned();
        let id = id.as_str();
        if self.sessions.pending_folder_drafts.contains_key(id)
            || target != draft_target(id)
            || self.sessions.submitted_drafts.contains_key(id)
            || has_pending_submission(&self.composer.pending_submissions, target)
        {
            return true;
        }
        let has_content =
            !self.composer.sessions.current().text.is_empty() || self.has_composer_attachments();
        if !has_content
            && !self.sessions.pending_draft_saves.contains(id)
            && self
                .sessions
                .drafts
                .iter()
                .any(|draft| draft.id == id && draft.app_session_id <= 0)
        {
            self.sessions.drafts.retain(|draft| draft.id != id);
            self.sessions.draft_session_ids.remove(id);
            self.sessions.selected_draft = None;
            self.composer.sessions.remove(target);
            self.composer.images.remove(target);
            self.composer.pastes.remove(target);
            self.workspace.session_surfaces.remove(target);
            self.workspace.editor.session_tabs.remove(target);
            return true;
        }
        let app_session_id = self
            .sessions
            .draft_session_ids
            .get(id)
            .copied()
            .or_else(|| {
                self.sessions
                    .drafts
                    .iter()
                    .find(|draft| draft.id == id)
                    .map(|draft| draft.app_session_id)
            })
            .unwrap_or_default();
        let changed = sync_materialized_draft(
            &mut self.sessions.drafts,
            id,
            app_session_id,
            &self.project.path,
            self.snapshot.harness,
        );
        if changed || app_session_id <= 0 || self.sessions.pending_draft_saves.contains(id) {
            return self.save_draft_snapshot(id, needs_identity_now);
        }
        true
    }

    fn draft_status_path<'a>(&'a self, draft: &'a DraftSession) -> Option<&'a std::path::Path> {
        self.sessions
            .all
            .by_app_id(draft.app_session_id)
            .map(|row| row.path.as_path())
            .or(draft.session_path.as_deref())
            .or_else(|| {
                self.sessions
                    .submitted_drafts
                    .get(&draft.id)
                    .and_then(Option::as_deref)
            })
    }

    pub(in crate::app) fn draft_rail_status(&self, draft: &DraftSession) -> RunStatus {
        let statuses = &self.activity.run_statuses;
        if let Some(path) = self.draft_status_path(draft)
            && self.sessions.all.has_shared_path(path)
        {
            return statuses
                .get(&draft_target(&draft.id))
                .copied()
                .or_else(|| {
                    self.bound_draft_row(&self.sessions.all, &draft.id, path)
                        .map(|row| self.rail_row_status(row, false))
                })
                .unwrap_or(if draft.submitted {
                    RunStatus::Done
                } else {
                    RunStatus::Draft
                });
        }
        resolved_draft_status(&draft.id, &self.sessions.submitted_drafts, statuses)
    }

    pub(super) fn can_archive_draft(&self, draft: &DraftSession) -> bool {
        draft.submitted || {
            let target = draft_target(&draft.id);
            self.composer
                .pending_submissions
                .values()
                .any(|send| send.submitted_target == target && send.result.is_none())
        }
    }

    pub(in crate::app) fn request_draft_archive(
        &mut self,
        id: String,
        archived: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.sessions.drafts.iter().position(|draft| draft.id == id) else {
            return;
        };
        let draft = &self.sessions.drafts[index];
        let previous = draft.archived;
        if previous == archived || (archived && !self.can_archive_draft(draft)) {
            return;
        }
        if archived {
            let actor = draft_target(&id);
            let path = self.draft_status_path(draft);
            let row =
                path.and_then(|path| self.bound_draft_row(&self.sessions.all, &draft.id, path));
            let active = super::activity::status_has_active_work(&self.draft_rail_status(draft))
                || has_pending_submission(&self.composer.pending_submissions, &actor)
                || (self.sessions.selected_draft.as_deref() == Some(&id)
                    && self.snapshot.selected_session.is_none()
                    && self.snapshot.live_session.is_none()
                    && super::activity::snapshot_has_active_work(&self.snapshot))
                || path
                    .filter(|path| row.is_some() || !self.sessions.all.has_shared_path(path))
                    .is_some_and(|path| {
                        super::activity::session_row_family_has_active_work(
                            &self.sessions.all,
                            row,
                            path,
                            &self.activity.run_statuses,
                            &self.activity.agents,
                            &self.snapshot,
                            &self.composer.pending_submissions,
                        )
                    });
            if active {
                self.confirm_archive(
                    super::archive::ArchiveConfirmationTarget::Draft(draft.clone()),
                    window,
                    cx,
                );
                return;
            }
        }
        let target = crate::sessions::AppSessionId::new(draft.app_session_id)
            .map(crate::runtime::ArchiveTarget::App)
            .unwrap_or_else(|| crate::runtime::ArchiveTarget::Draft(id));
        let mut snapshot = draft.clone();
        if let Some(confirmed) = self.sessions.archive_writes.confirmed(&target) {
            snapshot.archived = confirmed;
        }
        match self.sessions.writer.save_draft_with_identity(snapshot) {
            Ok(receipt) => self.queue_archive(target, previous, archived, Some(receipt), cx),
            Err(error) => {
                self.sessions.error = Some(error);
                self.notify_session_rail(cx);
            }
        }
    }

    pub(in crate::app) fn begin_draft_submission(&mut self, target: &str) {
        let Some(id) = draft_id(target) else {
            return;
        };
        self.materialize_selected_draft(id);
        self.activity
            .run_statuses
            .insert(target.to_owned(), RunStatus::Working);
    }

    pub(in crate::app) fn confirm_draft_submission(&mut self, id: &str, title: Option<&str>) {
        // A delayed receipt can follow promotion or discard.
        if let Some(draft) = self.sessions.drafts.iter_mut().find(|draft| draft.id == id) {
            draft.submitted = true;
            if draft.title.is_none() {
                draft.title = title.map(str::to_owned);
            }
            self.sessions
                .submitted_drafts
                .entry(id.to_owned())
                .or_default();
        }
    }

    pub(in crate::app) fn record_draft_submission(
        &mut self,
        target: &str,
        accepted: bool,
        session: Option<PathBuf>,
    ) {
        // The actor can acknowledge a prompt after its draft has been promoted.
        if !draft_id(target).is_some_and(|id| self.sessions.drafts.iter().any(|row| row.id == id)) {
            return;
        }
        let session = session.map(|path| normalize_session_path(&path));
        let Some(id) = establish_submission(
            &mut self.sessions.submitted_drafts,
            target,
            accepted,
            session,
        ) else {
            return;
        };
        let association = self.sessions.submitted_drafts.get(&id).cloned().flatten();
        if update_persisted_submission(&mut self.sessions.drafts, &id, association.as_deref()) {
            self.save_session_draft(&id);
        }
        if let Some(path) = association {
            self.canonicalize_draft_status(&id, &path);
        }
    }

    pub(in crate::app) fn record_session_status(
        &mut self,
        target: String,
        session: Option<PathBuf>,
        mut status: RunStatus,
        record: Option<&crate::agent_activity::SessionStatusRecord>,
    ) {
        let record_row = record.and_then(|record| record.resolve(&self.sessions.all));
        let record_key = record.map(|record| {
            record_row.map_or_else(|| record.status_key(), super::status::record_status_key)
        });
        if status == RunStatus::Done
            && self
                .activity
                .run_statuses
                .get(record_key.as_ref().unwrap_or(&target))
                .is_some_and(|status| *status == RunStatus::Failed)
        {
            return;
        }
        let session = session.map(|path| normalize_session_path(&path));
        let unique_owner = record_row.is_some_and(|row| {
            session.as_ref() == Some(&row.path) && !self.sessions.all.has_shared_path(&row.path)
        });
        if status == RunStatus::Working
            && session.is_some()
            && has_pending_submission(&self.composer.pending_submissions, &target)
        {
            establish_submission(
                &mut self.sessions.submitted_drafts,
                &target,
                true,
                session.clone(),
            );
        }
        let associated_path = fill_session_association(
            &mut self.sessions.submitted_drafts,
            &target,
            session.as_deref(),
        );
        let status_path = associated_path.as_deref().or(session.as_deref());
        let shared = status_path.is_some_and(|path| self.sessions.all.has_shared_path(path));
        let pending_owner = if shared && crate::sessions::session_path(&target).is_some() {
            record_key.as_ref().unwrap_or(&target)
        } else {
            &target
        };
        if preserve_submission_working_status(
            pending_owner,
            status_path.filter(|_| !shared),
            &status,
            &self.composer.pending_submissions,
            &self.activity.run_statuses,
        ) {
            status = RunStatus::Working;
        }
        if let Some(id) = draft_id(&target)
            && self.sessions.submitted_drafts.contains_key(id)
            && update_persisted_submission(
                &mut self.sessions.drafts,
                id,
                associated_path.as_deref(),
            )
        {
            self.save_session_draft(id);
        }

        if let Some(record_key) = record_key {
            let recent = self.store_run_status(record_key.clone(), status, false);
            let path_alias = session.as_deref().map(session_target);
            if unique_owner && let Some(alias) = &path_alias {
                self.store_run_status(alias.clone(), status, recent);
            } else if let Some(alias) = &path_alias
                && shared
            {
                self.clear_run_status(alias);
            }
            if target != record_key && path_alias.as_ref() != Some(&target) {
                if draft_id(&target).is_some() && unique_owner {
                    self.clear_run_status(&target);
                } else {
                    self.store_run_status(target, status, recent);
                }
            }
            return;
        }

        if let Some(path) = associated_path.or_else(|| {
            draft_id(&target)
                .and(session.as_deref())
                .map(std::path::Path::to_path_buf)
        }) {
            let id = draft_id(&target);
            let record = id
                .and_then(|id| self.bound_draft_row(&self.sessions.all, id, &path))
                .map(super::status::record_status_key);
            let waiting_for_owner = record.is_none()
                && id.is_some_and(|id| self.sessions.drafts.iter().any(|draft| draft.id == id));
            if waiting_for_owner || self.sessions.all.has_shared_path(&path) {
                let recent = self.record_run_status(target, status, false);
                if let Some(record) = record {
                    self.record_run_status(record, status, recent);
                }
                return;
            }
            self.clear_run_status(&target);
            self.record_run_status(session_target(&path), status, false);
            return;
        }

        let path_target = session.as_deref().map(session_target);
        if let Some(path) = session.as_deref()
            && self.sessions.all.has_shared_path(path)
        {
            // A path-only stop from the legacy family command stopped every row at this locator.
            if status == RunStatus::Stopped && path_target.as_ref() == Some(&target) {
                let records: Vec<_> = self
                    .sessions
                    .all
                    .at_path(path)
                    .map(super::status::record_status_key)
                    .collect();
                for key in records {
                    self.record_run_status(key, status, false);
                }
            }
            if let Some(alias) = &path_target {
                self.clear_run_status(alias);
            }
            if path_target.as_ref() != Some(&target) {
                self.record_run_status(target, status, false);
            }
            return;
        }
        let same_owner = session
            .as_deref()
            .and_then(|path| self.sessions.all.by_path(path))
            .is_none_or(|row| {
                crate::sessions::AppSessionId::from_key(&target)
                    .is_none_or(|id| row.app_session_id == id.get())
            });
        let alias = path_target.filter(|alias| alias != &target && same_owner);
        let recent = self.record_run_status(target, status, false);
        if let Some(alias) = alias {
            self.record_run_status(alias, status, recent);
        }
    }

    #[must_use = "draft promotion must invalidate the session rail"]
    pub(in crate::app) fn reconcile_submitted_drafts(&mut self, cx: &mut Context<Self>) -> bool {
        let promotions = reconciliation_candidates(
            &self.sessions.submitted_drafts,
            self.sessions
                .visible
                .iter()
                .map(|session| session.path.as_path()),
        );
        let mut promoted = false;
        for (id, path) in promotions {
            let Some(row) = self.bound_draft_row(&self.sessions.visible, &id, &path) else {
                continue;
            };
            let buffer = crate::sessions::session_buffer(row.harness, &path);
            self.promote_draft(&id, &path, buffer, cx);
            promoted = true;
        }
        promoted
    }

    pub(in crate::app) fn bound_draft_buffer(
        &self,
        id: &str,
        path: &std::path::Path,
    ) -> Option<String> {
        self.bound_draft_row(&self.sessions.all, id, path)
            .map(|row| crate::sessions::session_buffer(row.harness, path))
    }

    fn promote_draft(
        &mut self,
        id: &str,
        path: &std::path::Path,
        session_key: String,
        cx: &mut Context<Self>,
    ) {
        self.capture_composer_session(cx);
        let draft_key = draft_target(id);
        self.composer
            .sessions
            .promote(&draft_key, session_key.clone());
        self.promote_center_surface(
            &draft_key,
            crate::sessions::composer_command_target(&session_key),
        );
        self.promote_composer_images(&draft_key, &session_key);
        self.promote_composer_pastes(&draft_key, &session_key);
        for pending in self.composer.pending_submissions.values_mut() {
            if pending.submitted_target == draft_key {
                pending.submitted_target.clone_from(&session_key);
            }
        }
        self.canonicalize_draft_status(id, path);
        if self.sessions.selected_draft.as_deref() == Some(id) && self.awaiting_selection_snapshot()
        {
            // The composer has a saved row now, even if the actor has yet to publish it.
            self.lifecycle.requested_session_key = self
                .bound_draft_row(&self.sessions.all, id, path)
                .map(crate::sessions::SessionSummary::catalog_key);
            self.lifecycle
                .pending_session_switch
                .get_or_insert_with(|| {
                    (
                        path.to_owned(),
                        crate::app::infrastructure::performance::Timing::new(
                            "switch.session_total",
                        ),
                    )
                });
            self.lifecycle.awaiting_draft_snapshot = None;
        }
        self.sessions.submitted_drafts.remove(id);
        self.sessions.draft_session_ids.remove(id);
        self.sessions.pending_draft_saves.remove(id);
        if self
            .sessions
            .drafts
            .iter()
            .any(|draft| draft.id == id && draft.archived)
        {
            self.notify_archived_session_rail(cx);
        }
        self.sessions.drafts.retain(|draft| draft.id != id);
        clear_promoted_selection(&mut self.sessions.selected_draft, id);
        self.remove_session_draft(id);
    }

    pub(in crate::app) fn promote_composer_images(&mut self, from: &str, to: &str) {
        if let Some(images) = self.composer.images.remove(from) {
            self.composer
                .images
                .entry(to.to_owned())
                .or_default()
                .extend(images);
        }
    }

    fn canonicalize_draft_status(&mut self, id: &str, path: &std::path::Path) {
        let catalog = &self.sessions.all;
        let Some(row) = self.bound_draft_row(catalog, id, path) else {
            return;
        };
        let shared = catalog.has_shared_path(path);
        let record = super::status::record_status_key(row);
        let destination = if shared {
            record.clone()
        } else {
            session_target(path)
        };
        transfer_draft_status(
            &mut self.activity.run_statuses,
            &mut self.activity.recent_completions,
            id,
            destination.clone(),
        );
        if !shared && let Some(status) = self.activity.run_statuses.get(&destination).copied() {
            let recent = self.activity.recent_completions.contains_key(&destination);
            self.record_run_status(record, status, recent);
        }
    }
}

fn available_projects(registered: &[PathBuf], current: &std::path::Path) -> Vec<PathBuf> {
    let mut available = registered.to_vec();
    projects::add_unique(&mut available, current.to_path_buf());
    if let Some(index) = available.iter().position(|project| project == current) {
        available.swap(0, index);
    }
    available
}

fn draft_id(target: &str) -> Option<&str> {
    target.strip_prefix("draft:").filter(|id| !id.is_empty())
}

fn clear_promoted_selection(selected_draft: &mut Option<String>, promoted_id: &str) {
    if selected_draft.as_deref() == Some(promoted_id) {
        *selected_draft = None;
    }
}

fn preserve_submission_working_status(
    target: &str,
    session: Option<&std::path::Path>,
    status: &RunStatus,
    pending: &HashMap<String, PendingSubmission>,
    statuses: &HashMap<String, RunStatus>,
) -> bool {
    if *status != RunStatus::Done {
        return false;
    }
    let session_key = session.map(session_target);
    statuses
        .get(target)
        .or_else(|| session_key.as_ref().and_then(|key| statuses.get(key)))
        .is_some_and(|previous| *previous == RunStatus::Working)
        && pending.values().any(|submission| {
            submission.result.is_none()
                && submission.mode == crate::protocol::PromptMode::Normal
                && (crate::sessions::composer_command_target(&submission.submitted_target)
                    == target
                    || Some(crate::sessions::composer_command_target(
                        &submission.submitted_target,
                    )) == session_key.as_deref())
        })
}

fn transfer_draft_status(
    run_statuses: &mut HashMap<String, RunStatus>,
    recent_completions: &mut HashMap<String, std::time::Instant>,
    id: &str,
    session_key: String,
) {
    let draft_key = draft_target(id);
    let draft_status = run_statuses.remove(&draft_key);
    let draft_completion = recent_completions.remove(&draft_key);
    if !run_statuses.contains_key(&session_key)
        && let Some(status) = draft_status
    {
        run_statuses.insert(session_key.clone(), status);
    }
    if !recent_completions.contains_key(&session_key)
        && let Some(completed) = draft_completion
    {
        recent_completions.insert(session_key, completed);
    }
}

pub(in crate::app) fn resolved_draft_status(
    id: &str,
    submitted_drafts: &HashMap<String, Option<PathBuf>>,
    run_statuses: &HashMap<String, RunStatus>,
) -> RunStatus {
    if let Some(status) = run_statuses.get(&draft_target(id)) {
        return *status;
    }
    if let Some(Some(path)) = submitted_drafts.get(id)
        && let Some(status) = run_statuses.get(&session_target(path))
    {
        return *status;
    }
    if submitted_drafts.contains_key(id) {
        RunStatus::Working
    } else {
        RunStatus::Draft
    }
}

#[cfg(test)]
#[path = "drafts_tests.rs"]
mod tests;
