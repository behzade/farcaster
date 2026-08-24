//! App-owned repository refresh, backend choice, and stale-result policy.

use std::{collections::BTreeMap, path::PathBuf};

use gpui::{AppContext as _, Context, FocusHandle};

use super::PiApp;
use crate::{
    repository::{BackendPreference, DiffTargetKey, RepositoryBackend, WorkingCopySnapshot},
    state::StateStore,
};

#[derive(Default)]
struct RefreshGate {
    desired: u64,
    in_flight: Option<u64>,
    pending: bool,
}

struct RefreshCompletion {
    publish: bool,
    next: Option<u64>,
}

impl RefreshGate {
    fn request(&mut self) -> Option<u64> {
        self.desired = self.desired.saturating_add(1);
        if self.in_flight.is_some() {
            self.pending = true;
            None
        } else {
            Some(self.start())
        }
    }

    fn invalidate(&mut self) {
        self.desired = self.desired.saturating_add(1);
        self.pending = false;
    }

    fn start(&mut self) -> u64 {
        let generation = self.desired;
        self.in_flight = Some(generation);
        generation
    }

    fn finish(&mut self, generation: u64) -> Option<RefreshCompletion> {
        if self.in_flight != Some(generation) {
            return None;
        }
        self.in_flight = None;
        let publish = generation == self.desired;
        let rerun = std::mem::take(&mut self.pending);
        let next = rerun.then(|| self.start());
        Some(RefreshCompletion { publish, next })
    }
}

pub(super) struct RepositoryState {
    pub(super) project: PathBuf,
    pub(super) execution_allowed: bool,
    pub(super) preference: BackendPreference,
    pub(super) backend: Option<RepositoryBackend>,
    pub(super) snapshot: Option<WorkingCopySnapshot>,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    pub(super) preference_error: Option<String>,
    pub(super) row_focus: std::collections::HashMap<DiffTargetKey, FocusHandle>,
    preferences: BTreeMap<PathBuf, String>,
    refresh: RefreshGate,
    preference_save_in_flight: bool,
    preference_save_pending: bool,
}

impl RepositoryState {
    pub(super) fn load(project: PathBuf, execution_allowed: bool) -> Self {
        let (preferences, preference_error) = StateStore::open()
            .and_then(|store| store.load_repository_backend_preferences())
            .map_or_else(
                |error| (BTreeMap::new(), Some(error)),
                |preferences| (preferences, None),
            );
        let preference = preference_for(&preferences, &project);
        Self {
            project,
            execution_allowed,
            preference,
            backend: None,
            snapshot: None,
            loading: false,
            error: None,
            preference_error,
            row_focus: std::collections::HashMap::new(),
            preferences,
            refresh: RefreshGate::default(),
            preference_save_in_flight: false,
            preference_save_pending: false,
        }
    }

    fn select_project(&mut self, project: PathBuf, execution_allowed: bool) -> bool {
        if self.project == project && self.execution_allowed == execution_allowed {
            return false;
        }
        let project_changed = self.project != project;
        self.project = project;
        self.execution_allowed = execution_allowed;
        if project_changed {
            self.preference = preference_for(&self.preferences, &self.project);
        }
        self.clear_observation();
        true
    }

    fn select_preference(&mut self, preference: BackendPreference) -> bool {
        if self.preference == preference {
            return false;
        }
        self.preference = preference;
        self.preferences
            .insert(self.project.clone(), preference.as_str().to_owned());
        self.clear_observation();
        true
    }

    fn clear_observation(&mut self) {
        self.backend = None;
        self.snapshot = None;
        self.error = None;
        self.row_focus.clear();
    }
}

impl PiApp {
    pub(super) fn select_repository_project(&mut self, project: PathBuf, cx: &mut Context<Self>) {
        let execution_allowed =
            crate::project_trust::repository_execution_allowed(&project).unwrap_or(false);
        self.set_repository_project_execution(project, execution_allowed, cx);
    }

    pub(super) fn set_repository_project_execution(
        &mut self,
        project: PathBuf,
        execution_allowed: bool,
        cx: &mut Context<Self>,
    ) {
        if self.repository.select_project(project, execution_allowed) {
            self.invalidate_repository_diff(cx);
            self.request_repository_refresh(cx);
        }
    }

    pub(super) fn set_repository_backend_preference(
        &mut self,
        preference: BackendPreference,
        cx: &mut Context<Self>,
    ) {
        if !self.repository.select_preference(preference) {
            return;
        }
        self.invalidate_repository_diff(cx);
        self.composer_project_files.clear();
        self.composer_project_files_project = None;
        self.composer_project_files_loading = None;
        self.persist_repository_preferences(cx);
        self.request_repository_refresh(cx);
    }

    pub(crate) fn request_repository_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.repository.execution_allowed {
            self.repository.refresh.invalidate();
            self.repository.loading = false;
            self.repository.clear_observation();
            self.notify_run_panel(cx);
            return;
        }
        self.repository.loading = true;
        self.repository.error = None;
        let generation = self.repository.refresh.request();
        self.notify_run_panel(cx);
        if let Some(generation) = generation {
            self.start_repository_refresh(generation, cx);
        }
    }

    fn start_repository_refresh(&mut self, generation: u64, cx: &mut Context<Self>) {
        let project = self.repository.project.clone();
        let preference = self.repository.preference;
        let task = cx.background_spawn(async move {
            RepositoryBackend::discover(&project, preference).map(|backend| {
                backend.map(|backend| {
                    let snapshot = backend.snapshot();
                    (backend, snapshot)
                })
            })
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ =
                weak.update(cx, |this, cx| {
                    let Some(completion) = this.repository.refresh.finish(generation) else {
                        return;
                    };
                    if completion.publish {
                        match result {
                            Ok(Some((backend, Ok(snapshot)))) => {
                                this.repository.row_focus.retain(|key, _| {
                                    snapshot
                                        .changes
                                        .iter()
                                        .any(|change| &change.target.key == key)
                                });
                                for change in &snapshot.changes {
                                    this.repository
                                        .row_focus
                                        .entry(change.target.key.clone())
                                        .or_insert_with(|| cx.focus_handle());
                                }
                                this.repository.backend = Some(backend);
                                this.repository.snapshot = Some(snapshot);
                                this.repository.error = None;
                                this.invalidate_repository_file_mentions(cx);
                            }
                            Ok(Some((backend, Err(error)))) => {
                                if this.repository.snapshot.as_ref().is_some_and(|snapshot| {
                                    &snapshot.location != backend.location()
                                }) {
                                    this.repository.clear_observation();
                                }
                                this.repository.error = Some(error.to_string());
                            }
                            Ok(None) => {
                                this.repository.backend = None;
                                this.repository.snapshot = None;
                                this.repository.error = None;
                                this.repository.row_focus.clear();
                                this.invalidate_repository_file_mentions(cx);
                            }
                            Err(error) => this.repository.error = Some(error.to_string()),
                        }
                    }
                    if let Some(next) = completion.next {
                        this.start_repository_refresh(next, cx);
                    } else {
                        this.repository.loading = false;
                    }
                    this.notify_run_panel(cx);
                });
        })
        .detach();
    }

    fn invalidate_repository_file_mentions(&mut self, cx: &mut Context<Self>) {
        self.composer_project_files.clear();
        self.composer_project_files_project = None;
        self.composer_project_files_loading = None;
        let input = self.composer.read(cx);
        let has_active_mention =
            super::file_mentions::query_at_cursor(&input.value(), input.cursor()).is_some();
        if has_active_mention {
            self.request_composer_project_files(cx);
        } else {
            self.notify_composer(cx);
        }
    }

    fn persist_repository_preferences(&mut self, cx: &mut Context<Self>) {
        if self.repository.preference_save_in_flight {
            self.repository.preference_save_pending = true;
            return;
        }
        self.repository.preference_save_in_flight = true;
        let preferences = self.repository.preferences.clone();
        let task = cx.background_spawn(async move {
            StateStore::open()?.save_repository_backend_preferences(&preferences)
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.repository.preference_save_in_flight = false;
                this.repository.preference_error = result.err();
                let rerun = std::mem::take(&mut this.repository.preference_save_pending);
                if rerun {
                    this.persist_repository_preferences(cx);
                }
                this.notify_run_panel(cx);
            });
        })
        .detach();
    }
}

fn preference_for(
    preferences: &BTreeMap<PathBuf, String>,
    project: &std::path::Path,
) -> BackendPreference {
    preferences
        .get(project)
        .and_then(|value| value.parse().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_gate_coalesces_and_rejects_superseded_results() {
        let mut gate = RefreshGate::default();
        let first = gate.request().expect("first refresh should start");
        assert!(gate.request().is_none());

        let completion = gate.finish(first).expect("active refresh should finish");
        assert!(!completion.publish);
        let second = completion.next.expect("pending refresh should start");
        assert_ne!(first, second);

        let completion = gate.finish(second).expect("latest refresh should finish");
        assert!(completion.publish);
        assert!(completion.next.is_none());
        assert!(gate.finish(first).is_none());
    }

    #[test]
    fn invalidation_rejects_in_flight_work_without_starting_another_command() {
        let mut gate = RefreshGate::default();
        let generation = gate.request().expect("refresh should start");
        gate.invalidate();
        let completion = gate.finish(generation).expect("refresh should finish");
        assert!(!completion.publish);
        assert!(completion.next.is_none());
    }

    #[test]
    fn repository_preferences_are_project_specific() {
        let first = PathBuf::from("/first");
        let second = PathBuf::from("/second");
        let preferences = BTreeMap::from([
            (first.clone(), "git".to_owned()),
            (second.clone(), "jj".to_owned()),
        ]);

        assert_eq!(preference_for(&preferences, &first), BackendPreference::Git);
        assert_eq!(
            preference_for(&preferences, &second),
            BackendPreference::Jujutsu
        );
        assert_eq!(
            preference_for(&preferences, std::path::Path::new("/other")),
            BackendPreference::Auto
        );
    }
}
