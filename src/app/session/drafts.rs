use std::{collections::HashMap, path::PathBuf};

use gpui::{Context, Window};

use super::FarcasterApp;
use crate::{
    app::composer::sessions::{draft_target, session_target},
    projects::{self, DraftSession},
    runtime::RuntimeCommand,
    sessions::normalize_session_path,
};

impl FarcasterApp {
    pub(in crate::app) fn available_projects(&self) -> Vec<PathBuf> {
        available_projects(&self.projects, &self.project)
    }

    pub(in crate::app) fn selected_draft_is_empty_and_unsubmitted(&self) -> bool {
        self.snapshot.conversation.items.is_empty()
            && self
                .selected_draft
                .as_ref()
                .is_some_and(|id| !self.submitted_drafts.contains_key(id))
    }

    pub(in crate::app) fn editable_draft_project(&self) -> Option<PathBuf> {
        let id = self.selected_draft.as_deref()?;
        let target = draft_target(id);
        if self.composer_sessions.current_target() != target
            || self.submitted_drafts.contains_key(id)
            || self.pending_submissions.contains_key(&target)
        {
            return None;
        }
        self.drafts.iter().find(|draft| draft.id == id).map_or_else(
            || Some(self.project.clone()),
            |draft| draft.can_change_project().then(|| draft.project.clone()),
        )
    }

    pub(in crate::app) fn active_harness(&self) -> &str {
        if let Some(id) = self.selected_draft.as_deref()
            && let Some(draft) = self.drafts.iter().find(|draft| draft.id == id)
        {
            return &draft.harness;
        }
        self.snapshot
            .selected_session
            .as_deref()
            .and_then(|path| {
                self.all_sessions
                    .iter()
                    .find(|session| session.path == path)
            })
            .map(|session| session.harness.as_str())
            .unwrap_or(&self.snapshot.harness)
    }

    pub(in crate::app) fn editable_draft_harness(&self) -> Option<String> {
        let id = self.selected_draft.as_deref()?;
        let target = draft_target(id);
        if self.composer_sessions.current_target() != target
            || self.submitted_drafts.contains_key(id)
            || self.pending_submissions.contains_key(&target)
        {
            return None;
        }
        Some(self.drafts.iter().find(|draft| draft.id == id).map_or_else(
            || self.snapshot.harness.clone(),
            |draft| draft.harness.clone(),
        ))
    }

    pub(in crate::app) fn change_draft_harness(
        &mut self,
        harness: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.selected_draft.clone() else {
            return;
        };
        if self.editable_draft_harness().is_none() {
            return;
        }
        if let Err(error) = crate::app::infrastructure::persistence::StateStore::open()
            .and_then(|store| store.save_preferred_harness(&harness))
        {
            self.sessions_error = Some(error);
            self.notify_session_rail(cx);
            cx.notify();
            return;
        }
        if !self.drafts.iter().any(|draft| draft.id == id) {
            let mut draft = DraftSession::with_id(
                self.snapshot.harness.clone(),
                id.clone(),
                self.project.clone(),
            );
            draft.app_session_id = self.draft_session_ids.get(&id).copied().unwrap_or_default();
            self.drafts.insert(0, draft);
        }
        let draft = self
            .drafts
            .iter_mut()
            .find(|draft| draft.id == id)
            .expect("selected draft was materialized");
        self.preferred_harness.clone_from(&harness);
        if !draft.change_harness(harness.clone()) {
            return;
        }
        let project = draft.project.clone();
        self.save_project_registry();
        self.send_project_command(
            &project,
            RuntimeCommand::NewSession {
                id,
                harness,
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
        if self.pending_project_trust_command.is_some() {
            return;
        }
        let Some(id) = self.selected_draft.clone() else {
            return;
        };
        let target = draft_target(&id);
        if self.submitted_drafts.contains_key(&id) || self.pending_submissions.contains_key(&target)
        {
            return;
        }
        let changed = if let Some(draft) = self.drafts.iter_mut().find(|draft| draft.id == id) {
            draft.change_project(project.clone())
        } else {
            self.composer_sessions.current_target() == target && self.project != project
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
                    .drafts
                    .iter()
                    .find(|draft| draft.id == self.selected_draft.as_deref().unwrap_or_default())
                    .map(|draft| draft.harness.clone())
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

    pub(in crate::app) fn sync_current_draft(
        &mut self,
        composer: &crate::app::composer::sessions::ComposerSnapshot,
        target: &str,
    ) -> bool {
        let Some(id) = self.selected_draft.as_deref() else {
            return false;
        };
        if target != draft_target(id)
            || self.submitted_drafts.contains_key(id)
            || self.pending_submissions.contains_key(target)
        {
            return false;
        }
        let has_content = draft_has_content(composer)
            || self
                .composer_images
                .get(target)
                .is_some_and(|images| !images.is_empty())
            || self
                .composer_pastes
                .get(target)
                .is_some_and(|pastes| !pastes.is_empty());
        let app_session_id = self
            .draft_session_ids
            .get(id)
            .copied()
            .or_else(|| {
                self.drafts
                    .iter()
                    .find(|draft| draft.id == id)
                    .map(|draft| draft.app_session_id)
            })
            .unwrap_or_default();
        let changed = sync_materialized_draft(
            &mut self.drafts,
            id,
            app_session_id,
            &self.project,
            &self.snapshot.harness,
            has_content,
        );
        if changed {
            self.save_project_registry();
        }
        if !has_content {
            self.composer_images.remove(target);
            self.composer_pastes.remove(target);
        }
        !has_content
    }

    pub(in crate::app) fn begin_draft_submission(&mut self, target: &str, prompt: &str) {
        let Some(id) = draft_id(target) else {
            return;
        };
        self.submitted_drafts.entry(id.to_owned()).or_default();
        self.run_statuses
            .insert(target.to_owned(), "Working".into());
        if !self.drafts.iter().any(|draft| draft.id == id) {
            let app_session_id = self.draft_session_ids.get(id).copied().unwrap_or_default();
            let mut draft = DraftSession::with_id(
                self.snapshot.harness.clone(),
                id.to_owned(),
                self.project.clone(),
            );
            draft.app_session_id = app_session_id;
            self.drafts.insert(0, draft);
        }
        let draft = self
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
        let Some(id) = establish_submission(&mut self.submitted_drafts, target, accepted, session)
        else {
            return;
        };
        let association = self.submitted_drafts.get(&id).cloned().flatten();
        if update_persisted_submission(&mut self.drafts, &id, association.as_deref()) {
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
        status: String,
    ) {
        if status == "Done"
            && self
                .run_statuses
                .get(&target)
                .is_some_and(|status| status == "Failed")
        {
            return;
        }
        let session = session.map(|path| normalize_session_path(&path));
        if status == "Working" && self.pending_submissions.contains_key(&target) {
            establish_submission(&mut self.submitted_drafts, &target, true, session.clone());
        }
        let associated_path =
            fill_session_association(&mut self.submitted_drafts, &target, session.as_deref());
        if let Some(id) = draft_id(&target)
            && self.submitted_drafts.contains_key(id)
            && update_persisted_submission(&mut self.drafts, id, associated_path.as_deref())
        {
            self.save_project_registry();
        }

        if let Some(path) = associated_path.or_else(|| {
            draft_id(&target)
                .and(session.as_deref())
                .map(std::path::Path::to_path_buf)
        }) {
            self.run_statuses.remove(&target);
            self.recent_completions.remove(&target);
            self.recent_completion_expiries.remove(&target);
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
            &self.submitted_drafts,
            self.sessions.iter().map(|session| session.path.as_path()),
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
        self.composer_sessions
            .promote(&draft_key, session_key.clone());
        self.promote_center_surface(&draft_key, &session_key);
        self.promote_composer_images(&draft_key, &session_key);
        self.promote_composer_pastes(&draft_key, &session_key);
        if let Some(pending) = self.pending_submissions.remove(&draft_key) {
            self.pending_submissions
                .insert(session_key.clone(), pending);
        }
        self.canonicalize_draft_status(id, path);
        self.submitted_drafts.remove(id);
        self.draft_session_ids.remove(id);
        self.drafts.retain(|draft| draft.id != id);
        clear_promoted_selection(&mut self.selected_draft, id);
        self.save_project_registry();
    }

    pub(in crate::app) fn promote_composer_images(&mut self, from: &str, to: &str) {
        if let Some(images) = self.composer_images.remove(from) {
            self.composer_images
                .entry(to.to_owned())
                .or_default()
                .extend(images);
        }
    }

    fn canonicalize_draft_status(&mut self, id: &str, path: &std::path::Path) {
        transfer_draft_status(
            &mut self.run_statuses,
            &mut self.recent_completions,
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
    harness: &str,
    has_content: bool,
) -> bool {
    let existing = drafts.iter().position(|draft| draft.id == id);
    match (existing, has_content) {
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
