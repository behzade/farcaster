use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::runtime::RunStatus;

use crate::{
    agent_activity::AgentActivity,
    sessions::{SessionCatalog, SessionKey, SessionSummary},
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
    pub(in crate::app) fn rail_row_status(&self, row: &SessionSummary, waiting: bool) -> RunStatus {
        let status = row_status(&self.sessions.all, &self.activity.run_statuses, row);
        let live = self.snapshot.live_session.as_deref().filter(|path| {
            *path == row.path && self.snapshot.matches_catalog_row(&self.sessions.all, row)
        });
        resolved_session_status(row, status, live, self.snapshot.live_status, waiting)
    }

    pub(in crate::app) fn record_run_status(
        &mut self,
        target: String,
        status: RunStatus,
        force_recent: bool,
    ) -> bool {
        // An actor may start with only a locator. Capture the row while that
        // locator is unique so a later import cannot inherit its status.
        if let Some(path) = crate::sessions::session_path(&target)
            && !self.sessions.all.has_shared_path(path)
            && let Some(row) = self.sessions.all.by_path(path)
        {
            self.store_run_status(record_status_key(row), status, force_recent);
        }
        self.store_run_status(target, status, force_recent)
    }

    pub(super) fn store_run_status(
        &mut self,
        target: String,
        status: RunStatus,
        force_recent: bool,
    ) -> bool {
        if let Some(id) = crate::sessions::AppSessionId::from_key(&target)
            && let Some(row) = self.sessions.all.by_app_id(id.get())
            && let native = crate::agent_activity::SessionStatusRecord::Native(row.activity_key())
            && (!self.sessions.all.has_shared_path(&row.path)
                || native.resolve(&self.sessions.all).is_some())
        {
            let old_key = native.status_key();
            if let Some(previous) = self.activity.run_statuses.remove(&old_key) {
                self.activity
                    .run_statuses
                    .entry(target.clone())
                    .or_insert(previous);
            }
            if let Some(previous) = self.activity.recent_completions.remove(&old_key) {
                self.activity
                    .recent_completions
                    .entry(target.clone())
                    .or_insert(previous);
            }
        }
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

    pub(super) fn clear_run_status(&mut self, target: &str) {
        self.activity.run_statuses.remove(target);
        self.activity.recent_completions.remove(target);
        self.activity.recent_completion_expiries.remove(target);
    }
}

pub(super) fn record_status_key(row: &SessionSummary) -> String {
    crate::agent_activity::SessionStatusRecord::for_row(row).status_key()
}

pub(super) fn row_status(
    catalog: &SessionCatalog,
    statuses: &HashMap<String, RunStatus>,
    row: &SessionSummary,
) -> Option<RunStatus> {
    if statuses.is_empty() {
        return None;
    }
    let record = crate::agent_activity::SessionStatusRecord::for_row(row);
    let shared = catalog.has_shared_path(&row.path);
    if shared && row.app_session_id <= 0 && record.resolve(catalog).is_none() {
        return None;
    }
    statuses.get(&record.status_key()).copied().or_else(|| {
        if row.app_session_id > 0 {
            let native = crate::agent_activity::SessionStatusRecord::Native(row.activity_key());
            if let Some(status) = statuses.get(&native.status_key())
                && native.resolve(catalog).is_some()
            {
                return Some(*status);
            }
        }
        if shared {
            return None;
        }
        statuses
            .get(&crate::sessions::session_target(&row.path))
            .copied()
    })
}

pub(in crate::app) fn roots_waiting_for_descendants(
    sessions: &SessionCatalog,
) -> HashSet<SessionKey> {
    waiting_ancestors(sessions, sessions.running())
}

pub(in crate::app) fn roots_waiting_for_active_descendants(
    sessions: &SessionCatalog,
    activities: &HashMap<crate::agent_activity::WorkerActivityKey, AgentActivity>,
) -> HashSet<SessionKey> {
    let active = activities
        .iter()
        .filter(|(_, activity)| activity.lifecycle.has_active_work())
        .filter_map(|(key, _)| sessions.worker_for_activity(key))
        .filter(|row| !row.is_running);
    waiting_ancestors(sessions, sessions.running().chain(active))
}

fn waiting_ancestors<'a>(
    catalog: &SessionCatalog,
    active: impl Iterator<Item = &'a SessionSummary>,
) -> HashSet<SessionKey> {
    active
        .flat_map(|session| catalog.ancestors(session))
        .map(SessionSummary::catalog_key)
        .collect()
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
