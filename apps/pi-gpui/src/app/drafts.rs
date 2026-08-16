//! Submitted-draft identity and draft-to-session reconciliation.

use std::{collections::HashMap, path::PathBuf};

use gpui::Context;

use super::PiApp;
use crate::{
    composer_sessions::{draft_target, session_target},
    projects::DraftSession,
    sessions::normalize_session_path,
};

impl PiApp {
    pub(super) fn record_draft_submission(
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

    pub(super) fn record_session_status(
        &mut self,
        target: String,
        session: Option<PathBuf>,
        status: String,
    ) {
        let session = session.map(|path| normalize_session_path(&path));
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
            self.record_run_status(session_target(&path), status, false);
            return;
        }

        let recent = self.record_run_status(target, status.clone(), false);
        if let Some(path) = session {
            self.record_run_status(session_target(&path), status, recent);
        }
    }

    pub(super) fn reconcile_submitted_drafts(&mut self, cx: &mut Context<Self>) {
        let promotions = reconciliation_candidates(
            self.drafts.iter().map(|draft| draft.id.as_str()),
            &self.submitted_drafts,
            self.sessions.iter().map(|session| session.path.as_path()),
        );
        for (id, path) in promotions {
            self.promote_draft(&id, &path, cx);
        }
    }

    fn promote_draft(&mut self, id: &str, path: &std::path::Path, cx: &mut Context<Self>) {
        self.capture_composer_session(cx);
        let draft_key = draft_target(id);
        let session_key = session_target(path);
        self.composer_sessions
            .promote(&draft_key, session_key.clone());
        if let Some(images) = self.composer_images.remove(&draft_key) {
            self.composer_images
                .entry(session_key.clone())
                .or_default()
                .extend(images);
        }
        if let Some(pending) = self.pending_submission.as_mut()
            && pending.target == draft_key
        {
            pending.target = session_key.clone();
        }
        if let Some((target, _)) = self.pending_submission_result.as_mut()
            && *target == draft_key
        {
            *target = session_key.clone();
        }
        self.canonicalize_draft_status(id, path);
        self.submitted_drafts.remove(id);
        self.drafts.retain(|draft| draft.id != id);
        clear_promoted_selection(&mut self.selected_draft, id);
        self.save_project_registry();
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

fn draft_id(target: &str) -> Option<&str> {
    target.strip_prefix("draft:").filter(|id| !id.is_empty())
}

pub(super) fn submitted_draft_associations(
    drafts: &[DraftSession],
) -> HashMap<String, Option<PathBuf>> {
    drafts
        .iter()
        .filter_map(|draft| {
            draft
                .submitted
                .then(|| draft.session_path.clone())
                .flatten()
                .map(|path| (draft.id.clone(), Some(path)))
        })
        .collect()
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

pub(super) fn resolved_draft_status(
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
    draft_ids: impl Iterator<Item = &'a str>,
    submitted_drafts: &HashMap<String, Option<PathBuf>>,
    discovered_paths: impl Iterator<Item = &'a std::path::Path>,
) -> Vec<(String, PathBuf)> {
    let discovered_paths = discovered_paths.collect::<Vec<_>>();
    draft_ids
        .filter_map(|id| {
            let path = submitted_drafts.get(id)?.as_ref()?;
            discovered_paths
                .contains(&path.as_path())
                .then(|| (id.to_owned(), path.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submitted_a_and_selected_empty_b_keep_distinct_identity() {
        let path = PathBuf::from("/sessions/a.jsonl");
        let mut submitted = HashMap::new();

        assert_eq!(
            establish_submission(&mut submitted, "draft:a", true, Some(path.clone()),),
            Some("a".into())
        );
        // Selecting B has no submitted-draft state transition.
        let selected_draft = "b";

        assert_eq!(selected_draft, "b");
        assert_eq!(submitted.get("a"), Some(&Some(path)));
        assert_eq!(
            resolved_draft_status("a", &submitted, &HashMap::new()),
            "Working"
        );
        assert_eq!(
            resolved_draft_status("b", &submitted, &HashMap::new()),
            "Draft"
        );
    }

    #[test]
    fn later_draft_status_fills_only_an_established_submission() {
        let path = PathBuf::from("/sessions/a.jsonl");
        let mut submitted = HashMap::new();
        establish_submission(&mut submitted, "draft:a", true, None);

        assert_eq!(
            fill_session_association(&mut submitted, "draft:a", Some(&path)),
            Some(path.clone())
        );
        assert_eq!(
            fill_session_association(&mut submitted, "draft:b", Some(&path)),
            None
        );
        assert!(!submitted.contains_key("b"));
    }

    #[test]
    fn submitted_draft_status_prefers_draft_then_associated_session_then_fallback() {
        let path = PathBuf::from("/sessions/a.jsonl");
        let submitted = HashMap::from([("a".into(), Some(path.clone()))]);
        let session_key = session_target(&path);
        let mut statuses = HashMap::from([(session_key, "Needs input".into())]);

        assert_eq!(
            resolved_draft_status("a", &submitted, &statuses),
            "Needs input"
        );
        statuses.insert(draft_target("a"), "Failed".into());
        assert_eq!(resolved_draft_status("a", &submitted, &statuses), "Failed");
        statuses.remove(&draft_target("a"));
        statuses.insert(session_target(&path), "Done".into());
        assert_eq!(resolved_draft_status("a", &submitted, &statuses), "Done");
        statuses.insert(session_target(&path), "Working".into());
        assert_eq!(resolved_draft_status("a", &submitted, &statuses), "Working");
    }

    #[test]
    fn accepted_draft_with_exact_path_reconciles_after_store_reopen()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::{fs, time::SystemTime};

        use tempfile::tempdir;

        use crate::{
            projects::Registry,
            sessions::{SessionSummary, UsageSummary},
            state::StateStore,
        };

        let temp = tempdir()?;
        let project = temp.path().join("project");
        fs::create_dir(&project)?;
        let session = temp.path().join("a.jsonl");
        fs::write(&session, "{}")?;
        let project = project.canonicalize()?;
        let session = session.canonicalize()?;
        let mut drafts = vec![DraftSession {
            id: "a".into(),
            project: project.clone(),
            created_ms: 1,
            submitted: false,
            session_path: None,
        }];
        let mut submitted = HashMap::new();

        establish_submission(&mut submitted, "draft:a", true, Some(session.clone()));
        assert!(update_persisted_submission(
            &mut drafts,
            "a",
            Some(&session)
        ));
        let database = temp.path().join("gui-state.sqlite3");
        {
            let mut store = StateStore::open_at(&database)?;
            store.save_registry(&Registry {
                projects: vec![project.clone()],
                drafts,
            })?;
            store.replace_sessions(&[SessionSummary::from_cached(
                "session-a".into(),
                session.clone(),
                project,
                "Session A".into(),
                "hello".into(),
                "2026-08-15T00:00:00Z".into(),
                None,
                SystemTime::now(),
                1,
                UsageSummary::default(),
                false,
                false,
                "session a hello".into(),
            )])?;
        }

        let store = StateStore::open_at(&database)?;
        let restarted = store.load_registry()?;
        let restarted_submitted = submitted_draft_associations(&restarted.drafts);
        let catalog = store.cached_sessions("")?;

        assert_eq!(
            reconciliation_candidates(
                restarted.drafts.iter().map(|draft| draft.id.as_str()),
                &restarted_submitted,
                catalog.iter().map(|summary| summary.path.as_path()),
            ),
            vec![("a".into(), session)]
        );
        Ok(())
    }

    #[test]
    fn accepted_draft_without_a_path_is_never_durable() {
        let mut drafts = vec![DraftSession {
            id: "a".into(),
            project: PathBuf::from("/project"),
            created_ms: 1,
            submitted: false,
            session_path: None,
        }];
        let mut submitted = HashMap::new();

        establish_submission(&mut submitted, "draft:a", true, None);

        assert_eq!(submitted.get("a"), Some(&None));
        assert!(!update_persisted_submission(&mut drafts, "a", None));
        assert!(!drafts[0].submitted);
        assert_eq!(drafts[0].session_path, None);
        assert!(submitted_draft_associations(&drafts).is_empty());
    }

    #[test]
    fn background_submitted_draft_reconciles_while_b_stays_selected() {
        let path = PathBuf::from("/sessions/a.jsonl");
        let submitted = HashMap::from([("a".into(), Some(path.clone()))]);
        let mut selected_draft = Some("b".to_owned());

        assert_eq!(
            reconciliation_candidates(
                ["a", "b"].into_iter(),
                &submitted,
                [path.as_path()].into_iter(),
            ),
            vec![("a".into(), path)]
        );
        clear_promoted_selection(&mut selected_draft, "a");
        assert_eq!(selected_draft.as_deref(), Some("b"));
    }

    #[test]
    fn promotion_transfers_working_status_to_one_canonical_session_key() {
        let path = PathBuf::from("/sessions/a.jsonl");
        let draft_key = draft_target("a");
        let session_key = session_target(&path);
        let mut statuses = HashMap::from([
            (draft_key.clone(), "Working".into()),
            (session_key.clone(), "Working".into()),
        ]);
        let mut completions = HashMap::new();

        transfer_draft_status(&mut statuses, &mut completions, "a", &path);

        assert_eq!(
            statuses.get(&session_key).map(String::as_str),
            Some("Working")
        );
        assert!(!statuses.contains_key(&draft_key));
        assert_eq!(statuses.len(), 1);
    }

    #[test]
    fn reconciliation_requires_exact_draft_id_and_discovered_path() {
        let path = PathBuf::from("/sessions/a.jsonl");
        let submitted = HashMap::from([
            ("a".into(), Some(path)),
            ("b".into(), Some(PathBuf::from("/sessions/b.jsonl"))),
        ]);

        assert!(
            reconciliation_candidates(
                ["other"].into_iter(),
                &submitted,
                [std::path::Path::new("/sessions/a.jsonl")].into_iter(),
            )
            .is_empty()
        );
        assert!(
            reconciliation_candidates(
                ["a", "b"].into_iter(),
                &submitted,
                [std::path::Path::new("/sessions/other.jsonl")].into_iter(),
            )
            .is_empty()
        );
    }
}
