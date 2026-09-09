use std::{collections::HashMap, path::Path};

use crate::{
    app::composer::{sessions::session_target, submissions::PendingSubmission},
    runtime::RuntimeSnapshot,
    sessions::{SessionSummary, session_family_for_path},
};

pub(in crate::app) fn status_has_active_work(status: &str) -> bool {
    matches!(
        status,
        "Working" | "Compacting" | "Retrying" | "Needs input"
    )
}

pub(in crate::app) fn snapshot_has_active_work(snapshot: &RuntimeSnapshot) -> bool {
    !snapshot.history_preview
        && (snapshot.conversation.running
            || snapshot.conversation.compacting
            || snapshot.conversation.retrying
            || snapshot.pending_question.is_some())
}

pub(in crate::app) fn session_has_live_work(
    path: &Path,
    statuses: &HashMap<String, String>,
    snapshot: &RuntimeSnapshot,
) -> bool {
    statuses
        .get(&session_target(path))
        .is_some_and(|status| status_has_active_work(status))
        || (snapshot
            .live_session
            .as_deref()
            .or(snapshot.selected_session.as_deref())
            == Some(path)
            && snapshot_has_active_work(snapshot))
}

impl super::FarcasterApp {
    pub(super) fn session_family_has_active_work(&self, path: &Path) -> bool {
        session_family_has_active_work(
            &self.all_sessions,
            path,
            &self.run_statuses,
            &self.snapshot,
            &self.pending_submissions,
        )
    }
}

pub(super) fn session_family_has_active_work(
    sessions: &[SessionSummary],
    path: &Path,
    statuses: &HashMap<String, String>,
    snapshot: &RuntimeSnapshot,
    pending_submissions: &HashMap<String, PendingSubmission>,
) -> bool {
    let has_live_work = |path: &Path| {
        session_has_live_work(path, statuses, snapshot)
            || pending_submissions.contains_key(&session_target(path))
    };
    has_live_work(path)
        || session_family_for_path(sessions, path).is_some_and(|family| {
            family
                .into_iter()
                .any(|session| session.is_running || has_live_work(&session.path))
        })
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
