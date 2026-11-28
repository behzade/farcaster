use std::{collections::HashMap, path::Path};

use crate::{
    app::composer::{sessions::session_target, submissions::PendingSubmission},
    protocol::{BackgroundJob, BackgroundJobState},
    runtime::{RunStatus, RuntimeSnapshot},
    sessions::{SessionCatalog, SessionSummary},
};

pub(in crate::app) fn status_has_active_work(status: &RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Working | RunStatus::Compacting | RunStatus::Retrying | RunStatus::NeedsInput
    )
}

pub(in crate::app) fn snapshot_has_active_work(snapshot: &RuntimeSnapshot) -> bool {
    !snapshot.history_preview
        && (snapshot.conversation.running
            || snapshot.conversation.compacting
            || snapshot.conversation.retrying
            || snapshot.pending_question.is_some())
}

pub(in crate::app) fn application_has_active_work(
    statuses: &HashMap<String, RunStatus>,
    snapshot: &RuntimeSnapshot,
    submissions: &HashMap<String, PendingSubmission>,
    sessions: &[SessionSummary],
    jobs: &[BackgroundJob],
) -> bool {
    statuses
        .values()
        .any(|status| status_has_active_work(status))
        || snapshot_has_active_work(snapshot)
        || !submissions.is_empty()
        || sessions.iter().any(|session| session.is_running)
        || jobs.iter().any(|job| {
            matches!(
                job.state,
                BackgroundJobState::Starting | BackgroundJobState::Running
            )
        })
}

pub(in crate::app) fn session_has_live_work(
    path: &Path,
    statuses: &HashMap<String, RunStatus>,
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
            &self.sessions.all,
            path,
            &self.activity.run_statuses,
            &self.activity.agents,
            &self.snapshot,
            &self.composer.pending_submissions,
        )
    }
}

pub(in crate::app) fn session_family_has_active_work(
    sessions: &SessionCatalog,
    path: &Path,
    statuses: &HashMap<String, RunStatus>,
    activities: &HashMap<
        crate::agent_activity::WorkerActivityKey,
        crate::agent_activity::AgentActivity,
    >,
    snapshot: &RuntimeSnapshot,
    pending_submissions: &HashMap<String, PendingSubmission>,
) -> bool {
    session_row_family_has_active_work(
        sessions,
        sessions.by_path(path),
        path,
        statuses,
        activities,
        snapshot,
        pending_submissions,
    )
}

pub(in crate::app) fn session_row_family_has_active_work(
    sessions: &SessionCatalog,
    row: Option<&SessionSummary>,
    path: &Path,
    statuses: &HashMap<String, RunStatus>,
    activities: &HashMap<
        crate::agent_activity::WorkerActivityKey,
        crate::agent_activity::AgentActivity,
    >,
    snapshot: &RuntimeSnapshot,
    pending_submissions: &HashMap<String, PendingSubmission>,
) -> bool {
    let row_has_work = |row: &SessionSummary| {
        let path = &row.path;
        let shared = sessions.has_shared_path(path);
        let status = super::status::row_status(sessions, statuses, row);
        let pending = crate::app::composer::submissions::has_pending_submission(
            pending_submissions,
            &crate::sessions::session_buffer(row.harness, path),
        ) || crate::app::composer::submissions::has_pending_submission(
            pending_submissions,
            &super::status::record_status_key(row),
        ) || (!shared
            && crate::app::composer::submissions::has_pending_submission(
                pending_submissions,
                &session_target(path),
            ));
        let live = snapshot
            .live_session
            .as_ref()
            .or(snapshot.selected_session.as_ref())
            == Some(path)
            && snapshot.matches_catalog_row(sessions, row)
            && snapshot_has_active_work(snapshot);
        row.is_running
            || pending
            || status.as_ref().is_some_and(status_has_active_work)
            || live
            || activities
                .get(&row.activity_key())
                .is_some_and(|activity| activity.lifecycle.has_active_work())
    };
    let Some(root) = row.and_then(|row| sessions.root_for_row(row)) else {
        if sessions.has_shared_path(path) {
            return row.is_some_and(row_has_work);
        }
        return session_has_live_work(path, statuses, snapshot)
            || activities
                .iter()
                .any(|(key, activity)| key.path == path && activity.lifecycle.has_active_work())
            || crate::app::composer::submissions::has_pending_submission(
                pending_submissions,
                &session_target(path),
            );
    };
    row_has_work(root)
        || sessions
            .descendants(root)
            .into_iter()
            .any(|(session, _)| row_has_work(session))
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
