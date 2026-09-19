use crate::agents::Backend;
use std::{collections::HashMap, path::PathBuf};

use gpui::{Context, Window};

use super::FarcasterApp;
use crate::{
    app::composer::{
        sessions::{draft_target, session_target},
        submissions::{PendingSubmission, has_pending_submission},
    },
    projects::{self, DraftSession},
    runtime::RuntimeCommand,
    sessions::normalize_session_path,
};

impl FarcasterApp {
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
        let Some(id) = self.sessions.selected_draft.clone() else {
            return;
        };
        if self.editable_draft_harness().is_none() {
            return;
        }
        if let Err(error) =
            crate::app::persistence::open().and_then(|store| store.save_preferred_harness(harness))
        {
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
        self.sessions.preferred_harness = Some(harness);
        if !draft.change_harness(Some(harness)) {
            return;
        }
        let project = draft.project.clone();
        self.save_project_registry();
        self.send_project_command(
            &project,
            RuntimeCommand::NewSession {
                id,
                harness: Some(harness),
                project: project.clone(),
            },
            window,
            cx,
        );
        self.notify_session_rail(cx);
        self.notify_composer(cx);
        cx.notify();
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
        let changed =
            if let Some(draft) = self.sessions.drafts.iter_mut().find(|draft| draft.id == id) {
                draft.change_project(project.clone())
            } else {
                self.composer.sessions.current_target() == target && self.project.path != project
            };
        if !changed {
            return;
        }
        self.select_project(project.clone(), cx);
        self.send_project_command(
            &project,
            RuntimeCommand::NewSession {
                id,
                harness: self
                    .sessions
                    .drafts
                    .iter()
                    .find(|draft| {
                        draft.id == self.sessions.selected_draft.as_deref().unwrap_or_default()
                    })
                    .map(|draft| draft.harness)
                    .unwrap_or_else(|| self.active_harness().to_owned()),
                project: project.clone(),
            },
            window,
            cx,
        );
        self.notify_session_rail(cx);
        self.notify_composer(cx);
        cx.notify();
    }

    /// Workspace processes may hold unsaved work even with an empty composer.
    /// Keep this sticky for the draft's lifetime, not just while a surface is visible.
    pub(in crate::app) fn retain_workspace_draft(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_draft.as_deref() else {
            return;
        };
        let target = self.composer.sessions.current_target().to_owned();
        if target != draft_target(id) || !self.composer.sessions.retain_current() {
            return;
        }
        let composer = self.composer.sessions.current();
        self.sync_current_draft(&composer, &target);
        self.notify_session_rail(cx);
    }

    pub(in crate::app) fn sync_current_draft(
        &mut self,
        composer: &crate::app::composer::sessions::ComposerSnapshot,
        target: &str,
    ) -> bool {
        let Some(id) = self.sessions.selected_draft.as_deref() else {
            return false;
        };
        if target != draft_target(id)
            || self.sessions.submitted_drafts.contains_key(id)
            || has_pending_submission(&self.composer.pending_submissions, target)
        {
            return false;
        }
        let has_content = draft_has_content(composer)
            || self
                .composer
                .images
                .get(target)
                .is_some_and(|images| !images.is_empty())
            || self
                .composer
                .pastes
                .get(target)
                .is_some_and(|pastes| !pastes.is_empty());
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
        let retain = has_content || self.composer.sessions.is_retained(target);
        let changed = sync_materialized_draft(
            &mut self.sessions.drafts,
            id,
            app_session_id,
            &self.project.path,
            self.snapshot.harness,
            retain,
        );
        if changed {
            self.save_project_registry();
        }
        if !has_content {
            self.composer.images.remove(target);
            self.composer.pastes.remove(target);
        }
        !retain
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
        self.save_project_registry();
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
            self.save_project_registry();
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
            self.save_project_registry();
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
        self.save_project_registry();
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

pub(in crate::app) fn submitted_draft_associations(
    drafts: &[DraftSession],
) -> HashMap<String, Option<PathBuf>> {
    drafts
        .iter()
        .filter(|draft| draft.submitted)
        .map(|draft| (draft.id.clone(), draft.session_path.clone()))
        .collect()
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

fn establish_submission(
    submitted_drafts: &mut HashMap<String, Option<PathBuf>>,
    target: &str,
    accepted: bool,
    session: Option<PathBuf>,
) -> Option<String> {
    let id = accepted.then(|| draft_id(target)).flatten()?.to_owned();
    let association = submitted_drafts.entry(id.clone()).or_default();
    if association.is_none() {
        *association = session;
    }
    Some(id)
}

fn fill_session_association(
    submitted_drafts: &mut HashMap<String, Option<PathBuf>>,
    target: &str,
    session: Option<&std::path::Path>,
) -> Option<PathBuf> {
    let association = submitted_drafts.get_mut(draft_id(target)?)?;
    if association.is_none() {
        *association = session.map(std::path::Path::to_path_buf);
    }
    association.clone()
}

fn draft_has_content(composer: &crate::app::composer::sessions::ComposerSnapshot) -> bool {
    !composer.text.trim().is_empty()
}

fn sync_materialized_draft(
    drafts: &mut Vec<DraftSession>,
    id: &str,
    app_session_id: i64,
    project: &std::path::Path,
    harness: Option<Backend>,
    retain: bool,
) -> bool {
    let existing = drafts.iter().position(|draft| draft.id == id);
    match (existing, retain) {
        (None, true) => {
            let mut draft =
                DraftSession::with_id(harness.to_owned(), id.to_owned(), project.to_path_buf());
            draft.app_session_id = app_session_id;
            drafts.insert(0, draft);
            true
        }
        (Some(index), false) => {
            drafts.remove(index);
            true
        }
        _ => false,
    }
}

fn update_persisted_submission(
    drafts: &mut [DraftSession],
    id: &str,
    session: Option<&std::path::Path>,
) -> bool {
    let Some(session) = session else {
        return false;
    };
    let Some(draft) = drafts.iter_mut().find(|draft| draft.id == id) else {
        return false;
    };
    let mut changed = false;
    if !draft.submitted {
        draft.submitted = true;
        changed = true;
    }
    if draft.session_path.is_none() {
        draft.session_path = Some(session.to_path_buf());
        changed = true;
    }
    changed
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

fn reconciliation_candidates<'a>(
    submitted_drafts: &HashMap<String, Option<PathBuf>>,
    discovered_paths: impl Iterator<Item = &'a std::path::Path>,
) -> Vec<(String, PathBuf)> {
    let discovered_paths = discovered_paths.collect::<Vec<_>>();
    submitted_drafts
        .iter()
        .filter_map(|(id, path)| {
            let path = path.as_ref()?;
            discovered_paths
                .contains(&path.as_path())
                .then(|| (id.clone(), path.clone()))
        })
        .collect()
}

#[cfg(test)]
#[path = "drafts_tests.rs"]
mod tests;
