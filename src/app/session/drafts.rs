use crate::agents::Backend;
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
use crate::sessions::submitted_draft_associations;

impl FarcasterApp {
    pub(in crate::app) fn save_session_draft(&mut self, id: &str) -> bool {
        let Some(draft) = self.sessions.drafts.iter_mut().find(|draft| draft.id == id) else {
            return true;
        };
        let project = draft.project.clone();
        if draft.app_session_id <= 0 {
            match super::draft_store::save(draft) {
                Ok(app_session_id) => {
                    draft.app_session_id = app_session_id;
                    self.sessions
                        .draft_session_ids
                        .insert(id.to_owned(), app_session_id);
                }
                Err(error) => {
                    self.sessions.error = Some(error);
                    return false;
                }
            }
        }
        if let Err(error) = self.sessions.writer.save_draft(draft.clone()) {
            self.sessions.error = Some(error);
            return false;
        }
        self.remember_rail_projects([project]);
        true
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
        action: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>),
    ) {
        let id = previous.id.clone();
        if !self.save_session_draft(&id) {
            if let Some(draft) = self.sessions.drafts.iter_mut().find(|draft| draft.id == id) {
                previous.app_session_id = draft.app_session_id;
                *draft = previous;
            }
            self.notify_session_rail(cx);
            return;
        }
        // These explicit handoffs already require durable state on main: the
        // runtime reads the draft profile from storage when NewSession arrives.
        match futures::executor::block_on(self.sessions.writer.flush()) {
            Ok(()) => action(self, window, cx),
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
            && self
                .sessions
                .selected_draft
                .as_ref()
                .is_some_and(|id| !self.sessions.submitted_drafts.contains_key(id))
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
            .and_then(crate::agents::profile_id_from_locator)
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
        if self.editable_draft_harness().is_none() {
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
        if !self.sessions.drafts.iter().any(|draft| draft.id == id) {
            let mut draft =
                DraftSession::with_id(self.snapshot.harness, id.clone(), self.project.path.clone());
            draft.app_session_id = self
                .sessions
                .draft_session_ids
                .get(&id)
                .copied()
                .unwrap_or_default();
            self.sessions.drafts.insert(0, draft);
        }
        let draft = self
            .sessions
            .drafts
            .iter_mut()
            .find(|draft| draft.id == id)
            .expect("selected draft was materialized");
        let previous = draft.clone();
        self.sessions.preferred_harness = Some(harness);
        self.sessions.preferred_profile_id = profile_id.clone();
        let changed = match profile_id {
            Some(profile_id) => draft.change_profile(harness, profile_id),
            None => draft.change_harness(Some(harness)),
        };
        if !changed {
            return;
        }
        let project = draft.project.clone();
        self.finish_draft_change(previous, window, cx, move |app, window, cx| {
            if app.sessions.selected_draft.as_deref() == Some(id.as_str()) {
                app.send_project_command(
                    &project,
                    RuntimeCommand::NewSession {
                        id,
                        harness: Some(harness),
                        project: project.clone(),
                    },
                    window,
                    cx,
                );
            }
        });
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
        let previous = self
            .sessions
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .cloned();
        let changed =
            if let Some(draft) = self.sessions.drafts.iter_mut().find(|draft| draft.id == id) {
                draft.change_project(project.clone())
            } else {
                self.composer.sessions.current_target() == target && self.project.path != project
            };
        if !changed {
            return;
        }
        let action = move |app: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            if app.sessions.selected_draft.as_deref() != Some(id.as_str()) {
                return;
            }
            app.select_project(project.clone(), cx);
            app.send_project_command(
                &project,
                RuntimeCommand::NewSession {
                    harness: app.active_harness(),
                    id,
                    project: project.clone(),
                },
                window,
                cx,
            );
        };
        if let Some(previous) = previous {
            self.finish_draft_change(previous, window, cx, action);
        } else {
            action(self, window, cx);
            self.notify_session_rail(cx);
            self.notify_composer(cx);
            cx.notify();
        }
    }

    pub(in crate::app) fn sync_current_draft(&mut self, target: &str) -> bool {
        let Some(id) = self.sessions.selected_draft.as_deref() else {
            return true;
        };
        let id = id.to_owned();
        let id = id.as_str();
        if target != draft_target(id)
            || self.sessions.submitted_drafts.contains_key(id)
            || has_pending_submission(&self.composer.pending_submissions, target)
        {
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
        if changed || app_session_id <= 0 {
            return self.save_session_draft(id);
        }
        true
    }

    /// Archive submitted chats and keep their session record in sync.
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
        let previous = self.sessions.drafts[index].clone();
        if !self.sessions.drafts[index].set_archived(archived) {
            return;
        }
        let session = self.sessions.drafts[index].session_path.clone();
        self.finish_draft_change(previous, window, cx, move |app, _, cx| {
            if let Some(path) = session {
                app.set_session_archived(path, archived, cx);
            }
        });
    }

    pub(in crate::app) fn begin_draft_submission(&mut self, target: &str, prompt: &str) {
        let Some(id) = draft_id(target) else {
            return;
        };
        self.sessions
            .submitted_drafts
            .entry(id.to_owned())
            .or_default();
        self.activity
            .run_statuses
            .insert(target.to_owned(), "Working".into());
        if !self.sessions.drafts.iter().any(|draft| draft.id == id) {
            let app_session_id = self
                .sessions
                .draft_session_ids
                .get(id)
                .copied()
                .unwrap_or_default();
            let mut draft = DraftSession::with_id(
                self.snapshot.harness,
                id.to_owned(),
                self.project.path.clone(),
            );
            draft.app_session_id = app_session_id;
            self.sessions.drafts.insert(0, draft);
        }
        let draft = self
            .sessions
            .drafts
            .iter_mut()
            .find(|draft| draft.id == id)
            .expect("submitted draft was materialized");
        draft.submitted = true;
        if draft.title.is_none() {
            draft.title = provisional_session_title(prompt);
        }
        self.save_session_draft(id);
    }

    pub(in crate::app) fn record_draft_submission(
        &mut self,
        target: &str,
        accepted: bool,
        session: Option<PathBuf>,
    ) {
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
        mut status: String,
    ) {
        if status == "Done"
            && self
                .activity
                .run_statuses
                .get(&target)
                .is_some_and(|status| status == "Failed")
        {
            return;
        }
        let session = session.map(|path| normalize_session_path(&path));
        if status == "Working"
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
        if preserve_submission_working_status(
            &target,
            associated_path.as_deref().or(session.as_deref()),
            &status,
            &self.composer.pending_submissions,
            &self.activity.run_statuses,
        ) {
            status = "Working".into();
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

        if let Some(path) = associated_path.or_else(|| {
            draft_id(&target)
                .and(session.as_deref())
                .map(std::path::Path::to_path_buf)
        }) {
            self.activity.run_statuses.remove(&target);
            self.activity.recent_completions.remove(&target);
            self.activity.recent_completion_expiries.remove(&target);
            self.record_run_status(session_target(&path), status, false);
            return;
        }

        let recent = self.record_run_status(target, status.clone(), false);
        if let Some(path) = session {
            self.record_run_status(session_target(&path), status, recent);
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
        let promoted = !promotions.is_empty();
        for (id, path) in promotions {
            self.promote_draft(&id, &path, cx);
        }
        promoted
    }

    fn promote_draft(&mut self, id: &str, path: &std::path::Path, cx: &mut Context<Self>) {
        self.capture_composer_session(cx);
        let draft_key = draft_target(id);
        let session_key = session_target(path);
        self.composer
            .sessions
            .promote(&draft_key, session_key.clone());
        self.promote_center_surface(&draft_key, &session_key);
        self.promote_composer_images(&draft_key, &session_key);
        self.promote_composer_pastes(&draft_key, &session_key);
        for pending in self.composer.pending_submissions.values_mut() {
            if pending.submitted_target == draft_key {
                pending.submitted_target.clone_from(&session_key);
            }
        }
        self.canonicalize_draft_status(id, path);
        self.sessions.submitted_drafts.remove(id);
        self.sessions.draft_session_ids.remove(id);
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
        transfer_draft_status(
            &mut self.activity.run_statuses,
            &mut self.activity.recent_completions,
            id,
            path,
        );
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

fn provisional_session_title(prompt: &str) -> Option<String> {
    const MAX_WORDS: usize = 12;
    const MAX_CHARS: usize = 80;

    let line = prompt.lines().find(|line| !line.trim().is_empty())?.trim();
    let words = line
        .trim_matches(|character| matches!(character, '"' | '`'))
        .split_whitespace()
        .take(MAX_WORDS)
        .collect::<Vec<_>>()
        .join(" ");
    let title = words.chars().take(MAX_CHARS).collect::<String>();
    let title = title.trim_end_matches(['.', ':', ';']).trim();
    (!title.is_empty()).then(|| title.to_owned())
}

fn clear_promoted_selection(selected_draft: &mut Option<String>, promoted_id: &str) {
    if selected_draft.as_deref() == Some(promoted_id) {
        *selected_draft = None;
    }
}

// Idle startup snapshots must not overwrite an unresolved draft send's
// optimistic Working badge, including after its session identity is promoted.
fn preserve_submission_working_status(
    target: &str,
    session: Option<&std::path::Path>,
    status: &str,
    pending: &HashMap<String, PendingSubmission>,
    statuses: &HashMap<String, String>,
) -> bool {
    if status != "Done" || draft_id(target).is_none() {
        return false;
    }
    let session_key = session.map(session_target);
    statuses
        .get(target)
        .or_else(|| session_key.as_ref().and_then(|key| statuses.get(key)))
        .is_some_and(|previous| previous == "Working")
        && pending.values().any(|submission| {
            submission.result.is_none()
                && submission.mode == crate::protocol::PromptMode::Normal
                && (submission.submitted_target == target
                    || Some(&submission.submitted_target) == session_key.as_ref())
        })
}

fn transfer_draft_status(
    run_statuses: &mut HashMap<String, String>,
    recent_completions: &mut HashMap<String, std::time::Instant>,
    id: &str,
    path: &std::path::Path,
) {
    let draft_key = draft_target(id);
    let session_key = session_target(path);
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
    run_statuses: &HashMap<String, String>,
) -> String {
    if let Some(status) = run_statuses.get(&draft_target(id)) {
        return status.clone();
    }
    if let Some(Some(path)) = submitted_drafts.get(id)
        && let Some(status) = run_statuses.get(&session_target(path))
    {
        return status.clone();
    }
    if submitted_drafts.contains_key(id) {
        "Working".into()
    } else {
        "Draft".into()
    }
}

#[cfg(test)]
#[path = "drafts_tests.rs"]
mod tests;
