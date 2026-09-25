use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{AppContext as _, Context};

use super::{FarcasterApp, RepositoryObservation, observations::ScanResult, preference_for};
use crate::app::project::trust::repository_execution_allowed;
use crate::repository::{BackendPreference, RepositoryBackend};

// Trust is checked inside the background job, before any repository command.
fn observe_if_allowed(
    allowed: impl FnOnce() -> Result<bool, String>,
    scan: impl FnOnce() -> Option<ScanResult>,
) -> Option<ScanResult> {
    allowed().unwrap_or(false).then(scan).flatten()
}

fn observe_background(project: &Path, preference: BackendPreference) -> Option<ScanResult> {
    observe_if_allowed(
        || repository_execution_allowed(project),
        || {
            let backend = match RepositoryBackend::discover(project, preference) {
                Ok(Some(backend)) => backend,
                result => return Some(result.map(|_| None)),
            };
            let scanned = backend.try_snapshot_with_totals(|| {
                repository_execution_allowed(project).unwrap_or(false)
            });
            match scanned {
                Ok(Some(snapshot)) => Some(Ok(Some((backend, Ok(snapshot))))),
                Ok(None) => None,
                Err(error) => Some(Ok(Some((backend, Err(error))))),
            }
        },
    )
}

impl FarcasterApp {
    fn known_repository_projects(&self) -> Vec<PathBuf> {
        let mut projects = self.project.registered.clone();
        projects.extend(
            self.sessions
                .visible
                .iter()
                .map(|session| session.project.clone()),
        );
        projects.sort();
        projects.dedup();
        projects.retain(|project| !self.project.excluded.contains(project));
        projects
    }

    pub(in crate::app) fn forget_repository_project(&mut self, project: &Path) {
        self.project.repository.observations.forget(project);
        self.project.repository.warmed.remove(project);
    }

    pub(in crate::app) fn warm_repository_observations(&mut self, cx: &mut Context<Self>) {
        let projects = self.known_repository_projects();
        self.project.repository.observations.retain(&projects);
        self.project
            .repository
            .warmed
            .retain(|project| projects.contains(project));
        self.start_offscreen_observation_pass(cx);
    }

    fn next_offscreen_project(&mut self) -> Option<PathBuf> {
        let repository = &self.project.repository;
        // Do not queue work behind an existing background or foreground scan.
        if repository.observations.busy() || repository.loading {
            return None;
        }
        let mut projects = self.known_repository_projects();
        self.project.repository.observations.retain(&projects);
        projects.retain(|project| project != &self.project.repository.project);
        if projects.is_empty() {
            return None;
        }
        let repository = &mut self.project.repository;
        repository
            .warmed
            .retain(|project| projects.contains(project));
        if let Some(project) = projects
            .iter()
            .find(|project| !repository.warmed.contains(*project))
        {
            return Some(project.clone());
        }
        let cursor = repository.pass_cursor % projects.len();
        repository.pass_cursor = cursor.wrapping_add(1);
        Some(projects[cursor].clone())
    }

    // Startup warming and periodic refresh share one scheduler and one busy
    // slot. Slow scans cannot accumulate detached tasks every timer tick.
    pub(in crate::app) fn start_offscreen_observation_pass(&mut self, cx: &mut Context<Self>) {
        if self.project.repository.pass_task.is_some() {
            return;
        }
        self.project.repository.pass_task = Some(cx.spawn(async move |weak, cx| {
            loop {
                let Ok(delay) = weak.update(cx, |this, _| {
                    let warming = this.known_repository_projects().iter().any(|project| {
                        project != &this.project.repository.project
                            && !this.project.repository.warmed.contains(project)
                    });
                    if warming {
                        Duration::from_millis(500)
                    } else {
                        Duration::from_secs(5)
                    }
                }) else {
                    break;
                };
                cx.background_executor().timer(delay).await;
                if weak
                    .update(cx, |this, cx| {
                        if let Some(project) = this.next_offscreen_project() {
                            this.prefetch_repository_observation(project, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    pub(in crate::app) fn prefetch_repository_observation(
        &mut self,
        project: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if project == self.project.repository.project
            || self.project.repository.loading
            || self.project.repository.observations.busy()
        {
            return;
        }
        if !self.known_repository_projects().contains(&project) {
            self.forget_repository_project(&project);
            return;
        }
        self.project.repository.warmed.insert(project.clone());
        if !repository_execution_allowed(&project).unwrap_or(false) {
            self.project.repository.observations.forget(&project);
            return;
        }
        let preference = preference_for(&self.project.repository.preferences, &project);
        let Some(ticket) = self
            .project
            .repository
            .observations
            .begin(project.clone(), preference)
        else {
            return;
        };
        cx.spawn(async move |weak, cx| {
            let target = project.clone();
            let scanned = cx
                .background_spawn(async move { observe_background(&target, preference) })
                .await;
            let _ = weak.update(cx, |this, _| {
                if !this.project.repository.observations.finish(&ticket) {
                    return;
                }
                let eligible = project != this.project.repository.project
                    && this.known_repository_projects().contains(&project)
                    && preference_for(&this.project.repository.preferences, &project) == preference
                    && repository_execution_allowed(&project).unwrap_or(false);
                if !eligible {
                    this.project.repository.observations.forget(&project);
                    return;
                }
                // A busy operation lock is not evidence that the old cache is
                // wrong. Leave it provisional and retry on the next pass.
                let Some(scanned) = scanned else { return };
                if let Some(observation) = RepositoryObservation::from_scan(preference, scanned) {
                    this.project
                        .repository
                        .observations
                        .remember(project, observation);
                } else {
                    // Negative/error results never leave a known-stale positive
                    // entry available for the next project switch.
                    this.project.repository.observations.forget(&project);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
#[path = "prefetch_tests.rs"]
mod tests;
