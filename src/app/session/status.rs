use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, agent_activity_key},
    sessions::{SessionRootIndex, SessionSummary},
};

pub(in crate::app) fn resolved_session_status(
    session: &SessionSummary,
    explicit_status: Option<&str>,
    live_session_path: Option<&Path>,
    live_status: &str,
    waiting_for_descendant: bool,
) -> String {
    let explicit_status = explicit_status.and_then(normalized_session_status);
    let live_status = (live_session_path == Some(session.path.as_path()))
        .then(|| normalized_session_status(live_status))
        .flatten();
    explicit_status
        .filter(|status| status != "Done" || !waiting_for_descendant)
        .or_else(|| live_status.filter(|status| status != "Done" || !waiting_for_descendant))
        .or_else(|| waiting_for_descendant.then(|| "Waiting".into()))
        .or_else(|| session.is_running.then(|| "Working".into()))
        .unwrap_or_else(|| "Done".into())
}

fn normalized_session_status(status: &str) -> Option<String> {
    match status {
        "" | "Idle" => None,
        "Ready" => Some("Done".into()),
        status => Some(status.into()),
    }
}

pub(in crate::app) fn roots_waiting_for_descendants(
    sessions: &[SessionSummary],
) -> HashSet<PathBuf> {
    roots_waiting_for_descendants_where(sessions, |session| session.is_running)
}

pub(in crate::app) fn roots_waiting_for_active_descendants(
    sessions: &[SessionSummary],
    activities: &HashMap<String, AgentActivity>,
) -> HashSet<PathBuf> {
    let active_paths = active_activity_session_paths(sessions, activities);
    roots_waiting_for_descendants_where(sessions, |session| {
        session.is_running || active_paths.contains(session.path.as_path())
    })
}

fn active_activity_session_paths<'a>(
    sessions: &'a [SessionSummary],
    activities: &HashMap<String, AgentActivity>,
) -> HashSet<&'a Path> {
    let sessions_by_path = sessions
        .iter()
        .map(|session| (session.path.as_path(), session))
        .collect::<HashMap<_, _>>();
    activities
        .iter()
        .filter(|(_, activity)| agent_activity_keeps_parent_waiting(activity))
        .filter_map(|(key, activity)| {
            sessions_by_path
                .get(activity.session_path.as_path())
                .copied()
                .or_else(|| {
                    sessions
                        .iter()
                        .find(|session| agent_activity_key(&session.path) == *key)
                })
                .map(|session| session.path.as_path())
        })
        .collect()
}

fn agent_activity_keeps_parent_waiting(activity: &AgentActivity) -> bool {
    matches!(
        activity.lifecycle,
        AgentLifecycle::NeedsInput | AgentLifecycle::Working
    )
}

fn roots_waiting_for_descendants_where(
    sessions: &[SessionSummary],
    active: impl Fn(&SessionSummary) -> bool,
) -> HashSet<PathBuf> {
    let index = SessionRootIndex::new(sessions);
    let mut waiting = HashSet::new();
    for session in sessions.iter().filter(|session| active(session)) {
        waiting.extend(
            index
                .ancestors(session)
                .into_iter()
                .map(|parent| parent.path.clone()),
        );
    }
    waiting
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
