use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::runtime::RunStatus;

use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, agent_activity_key},
    sessions::{SessionRootIndex, SessionSummary},
};

pub(in crate::app) fn resolved_session_status(
    session: &SessionSummary,
    explicit_status: Option<RunStatus>,
    live_session_path: Option<&Path>,
    live_status: RunStatus,
    waiting_for_descendant: bool,
) -> RunStatus {
    explicit_status
        .into_iter()
        .chain((live_session_path == Some(session.path.as_path())).then_some(live_status))
        .find(|status| {
            *status != RunStatus::Invalid && (*status != RunStatus::Done || !waiting_for_descendant)
        })
        .unwrap_or_else(|| {
            if waiting_for_descendant {
                RunStatus::Waiting
            } else if session.is_running {
                RunStatus::Working
            } else {
                RunStatus::Done
            }
        })
}

pub(in crate::app) fn run_status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Invalid => "",
        RunStatus::Draft => "Draft",
        RunStatus::Done => "Done",
        RunStatus::Working => "Working",
        RunStatus::Compacting => "Compacting",
        RunStatus::Retrying => "Retrying",
        RunStatus::NeedsInput => "Needs input",
        RunStatus::Waiting => "Waiting",
        RunStatus::Stopped => "Stopped",
        RunStatus::Failed => "Failed",
    }
}

impl crate::app::FarcasterApp {
    pub(in crate::app) fn record_run_status(
        &mut self,
        target: String,
        status: RunStatus,
        force_recent: bool,
    ) -> bool {
        let recent = match status {
            RunStatus::Done => {
                if crate::app::starts_recent_completion(
                    self.activity.run_statuses.get(&target),
                    &status,
                    force_recent,
                ) {
                    self.activity
                        .recent_completions
                        .insert(target.clone(), std::time::Instant::now());
                }
                self.activity.recent_completions.contains_key(&target)
            }
            _ => {
                self.activity.recent_completions.remove(&target);
                false
            }
        };
        if status == RunStatus::Done && !recent {
            self.activity.run_statuses.remove(&target);
        } else {
            self.activity.run_statuses.insert(target, status);
        }
        recent
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
