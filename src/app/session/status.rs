use std::collections::{HashMap, HashSet};

use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, agent_activity_key},
    sessions::SessionSummary,
};

pub(in crate::app) fn resolved_session_status(
    session: &SessionSummary,
    explicit_status: Option<&str>,
    live_session_id: Option<&str>,
    live_status: &str,
    waiting_for_descendant: bool,
) -> String {
    let explicit_status = explicit_status.and_then(normalized_session_status);
    let live_status = (live_session_id == Some(session.id.as_str()))
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
) -> HashSet<String> {
    roots_waiting_for_descendants_where(sessions, |session| session.is_running)
}

pub(in crate::app) fn roots_waiting_for_active_descendants(
    sessions: &[SessionSummary],
    activities: &HashMap<String, AgentActivity>,
) -> HashSet<String> {
    roots_waiting_for_descendants_where(sessions, |session| {
        session.is_running
            || activities
                .get(&agent_activity_key(&session.path))
                .is_some_and(|activity| {
                    matches!(
                        activity.lifecycle,
                        AgentLifecycle::NeedsInput | AgentLifecycle::Working
                    )
                })
    })
}

fn roots_waiting_for_descendants_where(
    sessions: &[SessionSummary],
    active: impl Fn(&SessionSummary) -> bool,
) -> HashSet<String> {
    let parent_by_id = sessions
        .iter()
        .filter_map(|session| {
            session
                .parent_session
                .as_ref()
                .map(|parent| (session.id.as_str(), parent.as_str()))
        })
        .collect::<HashMap<_, _>>();
    let mut waiting = HashSet::new();
    for session in sessions.iter().filter(|session| active(session)) {
        let mut current = session.id.as_str();
        let mut seen = HashSet::new();
        while seen.insert(current) {
            let Some(parent) = parent_by_id.get(current).copied() else {
                break;
            };
            waiting.insert(parent.to_owned());
            current = parent;
        }
    }
    waiting
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
