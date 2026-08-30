#[path = "repository_watching.rs"]
mod watching;

use std::{collections::BTreeMap, path::PathBuf};

use gpui::{AppContext as _, Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{
    repository::{
        BackendPreference, DiffTargetKey, RepositoryBackend, RepositoryLocation,
        RepositorySyncAction, RepositoryWatcher, WorkingCopySnapshot,
    },
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

pub(super) struct PendingJjInit {
    pub(super) repository: PathBuf,
    project: PathBuf,
    return_focus: Option<FocusHandle>,
}

#[derive(Default)]
pub(super) struct RepositorySyncState {
    pub(super) action: Option<RepositorySyncAction>,
    pub(super) error: Option<String>,
    generation: u64,
}

impl RepositorySyncState {
    fn clear(&mut self) {
        self.action = None;
        self.error = None;
        self.generation = self.generation.saturating_add(1);
    }
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
    pub(super) initialized: bool,
    pub(super) error: Option<String>,
    pub(super) preference_error: Option<String>,
    pub(super) watcher_error: Option<String>,
    pub(super) pending_jj_init: Option<PendingJjInit>,
    jj_init_in_flight: bool,
    pub(super) sync: RepositorySyncState,
    pub(super) additions: Option<u64>,
    pub(super) deletions: Option<u64>,
    pub(super) visible_changes: usize,
    pub(super) row_focus: std::collections::HashMap<DiffTargetKey, FocusHandle>,
    preferences: BTreeMap<PathBuf, String>,
    refresh: RefreshGate,
    preference_save_in_flight: bool,
    preference_save_pending: bool,
    watcher: Option<RepositoryWatcher>,
    watcher_binding: Option<watching::WatchBinding>,
    watcher_generation: u64,
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
            initialized: false,
            error: None,
            preference_error,
            watcher_error: None,
            pending_jj_init: None,
            jj_init_in_flight: false,
            sync: RepositorySyncState::default(),
            additions: None,
            deletions: None,
            visible_changes: 5,
            row_focus: std::collections::HashMap::new(),
            preferences,
            refresh: RefreshGate::default(),
            preference_save_in_flight: false,
            preference_save_pending: false,
            watcher: None,
            watcher_binding: None,
            watcher_generation: 0,
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
            self.pending_jj_init = None;
            self.jj_init_in_flight = false;
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
        self.loading = false;
        self.initialized = false;
        self.error = None;
        self.watcher_error = None;
        self.sync.clear();
        self.additions = None;
        self.deletions = None;
        self.visible_changes = 5;
        self.row_focus.clear();
        self.watcher = None;
        self.watcher_binding = None;
        self.watcher_generation = self.watcher_generation.saturating_add(1);
    }
}

impl FarcasterApp {
    pub(super) fn expand_repository_changes(&mut self, cx: &mut Context<Self>) {
        self.repository.visible_changes = self.repository.visible_changes.saturating_add(20);
        self.notify_run_panel(cx);
    }

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
            self.request_repository_refresh(cx);
        }
    }

    pub(super) fn set_repository_backend_preference(
        &mut self,
        preference: BackendPreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if preference == BackendPreference::Jujutsu {
            if self.repository.jj_init_in_flight {
                return;
            }
            let location = self
                .repository
                .snapshot
                .as_ref()
                .map(|snapshot| Ok(Some(snapshot.location.clone())))
                .unwrap_or_else(|| {
                    RepositoryBackend::discover(&self.repository.project, BackendPreference::Auto)
                        .map(|backend| backend.map(|backend| backend.location().clone()))
                });
            match location.and_then(|location| {
                location
                    .map(|location| {
                        RepositoryBackend::jj_init_required(&location)
                            .map(|required| (location, required))
                    })
                    .transpose()
            }) {
                Ok(Some((location, true))) => {
                    self.repository.pending_jj_init = Some(PendingJjInit {
                        repository: location.workspace_root.clone(),
                        project: self.repository.project.clone(),
                        return_focus: window.focused(cx),
                    });
                    self.cover_native_workspace_surface(cx);
                    self.sheet_focus.focus(window, cx);
                    cx.notify();
                    return;
                }
                Ok(Some((_, false)) | None) => {}
                Err(error) => {
                    self.repository.error = Some(error.to_string());
                    self.notify_run_panel(cx);
                    return;
                }
            }
        }
        self.apply_repository_backend_preference(preference, false, cx);
    }

    fn apply_repository_backend_preference(
        &mut self,
        preference: BackendPreference,
        refresh_unchanged: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.repository.select_preference(preference) {
            if refresh_unchanged {
                self.repository.clear_observation();
                self.request_repository_refresh(cx);
            }
            return;
        }
        self.composer_project_files.clear();
        self.composer_project_files_project = None;
        self.composer_project_files_loading = None;
        self.persist_repository_preferences(cx);
        self.request_repository_refresh(cx);
    }

    pub(super) fn close_jj_init_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingJjInit> {
        let pending = self.repository.pending_jj_init.take()?;
        pending
            .return_focus
            .clone()
            .unwrap_or_else(|| self.composer_focus.clone())
            .focus(window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some(pending)
    }

    pub(super) fn confirm_jj_init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.close_jj_init_confirmation(window, cx) else {
            return;
        };
        let repository = pending.repository;
        let project = pending.project;
        self.repository.jj_init_in_flight = true;
        let task =
            cx.background_spawn(async move { RepositoryBackend::init_jj_colocated(&repository) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.repository.project != project {
                    return;
                }
                this.repository.jj_init_in_flight = false;
                match result {
                    Ok(()) => this.apply_repository_backend_preference(
                        BackendPreference::Jujutsu,
                        true,
                        cx,
                    ),
                    Err(error) => {
                        this.repository.error =
                            Some(format!("Jujutsu initialization failed: {error}"));
                        this.notify_run_panel(cx);
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn request_repository_sync(
        &mut self,
        action: RepositorySyncAction,
        cx: &mut Context<Self>,
    ) {
        if !self.repository.execution_allowed || self.repository.sync.action.is_some() {
            return;
        }
        let (Some(backend), Some(snapshot)) = (
            self.repository.backend.clone(),
            self.repository.snapshot.clone(),
        ) else {
            return;
        };
        self.repository.sync.action = Some(action);
        self.repository.sync.error = None;
        let generation = self.repository.sync.generation;
        self.notify_run_panel(cx);
        let task = cx.background_spawn(async move { backend.sync(&snapshot, action) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.repository.sync.generation != generation
                    || this.repository.sync.action != Some(action)
                {
                    return;
                }
                this.repository.sync.action = None;
                this.repository.sync.error = result.err().map(|error| error.to_string());
                this.notify_run_panel(cx);
                if this.repository.sync.error.is_none() {
                    this.request_repository_refresh(cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn request_repository_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.repository.execution_allowed {
            self.repository.refresh.invalidate();
            self.repository.loading = false;
            self.repository.clear_observation();
            self.notify_run_panel(cx);
            return;
        }
        let notify = !self.repository.initialized && !self.repository.loading;
        self.repository.loading = true;
        if !self.repository.initialized {
            self.repository.error = None;
        }
        let generation = self.repository.refresh.request();
        if notify {
            self.notify_run_panel(cx);
        }
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
                    let snapshot = backend.snapshot().map(|snapshot| {
                        let (additions, deletions) = backend
                            .working_copy_totals(&snapshot)
                            .unwrap_or((None, None));
                        (snapshot, additions, deletions)
                    });
                    (backend, snapshot)
                })
            })
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                let Some(completion) = this.repository.refresh.finish(generation) else {
                    return;
                };
                let mut display_changed = completion.publish && !this.repository.initialized;
                if completion.publish {
                    this.repository.initialized = true;
                    match result {
                        Ok(Some((backend, Ok((snapshot, additions, deletions))))) => {
                            let observation_changed =
                                this.repository.snapshot.as_ref().is_none_or(|current| {
                                    !displayed_snapshot_eq(current, &snapshot)
                                }) || this.repository.additions != additions
                                    || this.repository.deletions != deletions;
                            if observation_changed {
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
                            }
                            let location = snapshot.location.clone();
                            this.repository.backend = Some(backend);
                            this.repository.snapshot = Some(snapshot);
                            this.repository.additions = additions;
                            this.repository.deletions = deletions;
                            display_changed |= this.repository.error.take().is_some();
                            display_changed |= this.install_repository_watcher(location, cx);
                            if observation_changed {
                                display_changed = true;
                                this.invalidate_repository_file_mentions(cx);
                            }
                        }
                        Ok(Some((backend, Err(error)))) => {
                            let location = backend.location().clone();
                            if this
                                .repository
                                .snapshot
                                .as_ref()
                                .is_some_and(|snapshot| snapshot.location != location)
                            {
                                this.repository.backend = None;
                                this.repository.snapshot = None;
                                this.repository.additions = None;
                                this.repository.deletions = None;
                                this.repository.row_focus.clear();
                                display_changed = true;
                            }
                            this.repository.backend = Some(backend);
                            display_changed |= this.install_repository_watcher(location, cx);
                            let error = error.to_string();
                            display_changed |=
                                this.repository.error.as_deref() != Some(error.as_str());
                            this.repository.error = Some(error);
                        }
                        Ok(None) => {
                            let had_observation = this.repository.backend.is_some()
                                || this.repository.snapshot.is_some();
                            this.repository.backend = None;
                            this.repository.snapshot = None;
                            this.repository.additions = None;
                            this.repository.deletions = None;
                            this.repository.error = None;
                            this.repository.row_focus.clear();
                            display_changed |= this.install_repository_discovery_watcher(cx);
                            if had_observation {
                                display_changed = true;
                                this.invalidate_repository_file_mentions(cx);
                            }
                        }
                        Err(error) => {
                            if this.repository.snapshot.is_none() {
                                display_changed |= this.install_repository_discovery_watcher(cx);
                            }
                            let error = error.to_string();
                            display_changed |=
                                this.repository.error.as_deref() != Some(error.as_str());
                            this.repository.error = Some(error);
                        }
                    }
                }
                if let Some(next) = completion.next {
                    this.start_repository_refresh(next, cx);
                } else {
                    this.repository.loading = false;
                }
                if display_changed {
                    this.notify_run_panel(cx);
                }
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

fn displayed_snapshot_eq(left: &WorkingCopySnapshot, right: &WorkingCopySnapshot) -> bool {
    left.location == right.location
        && displayed_identity_eq(&left.identity, &right.identity)
        && left.changes.len() == right.changes.len()
        && left
            .changes
            .iter()
            .zip(&right.changes)
            .all(|(left, right)| {
                left.relative_path == right.relative_path
                    && left.original_relative_path == right.original_relative_path
                    && left.layer == right.layer
                    && left.kind == right.kind
                    && left.target.exists == right.target.exists
            })
}

fn displayed_identity_eq(
    left: &crate::repository::SnapshotIdentity,
    right: &crate::repository::SnapshotIdentity,
) -> bool {
    match (left, right) {
        (
            crate::repository::SnapshotIdentity::Git(left),
            crate::repository::SnapshotIdentity::Git(right),
        ) => left == right,
        (
            crate::repository::SnapshotIdentity::Jujutsu(left),
            crate::repository::SnapshotIdentity::Jujutsu(right),
        ) => {
            left.change_id == right.change_id
                && left.description == right.description
                && left.bookmarks == right.bookmarks
                && left.closest_bookmarks == right.closest_bookmarks
                && left.ahead == right.ahead
                && left.conflicted == right.conflicted
        }
        _ => false,
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
    fn display_equality_ignores_snapshot_capture_time() {
        let snapshot = WorkingCopySnapshot {
            location: RepositoryLocation {
                kind: crate::repository::RepositoryKind::Git,
                workspace_root: PathBuf::from("/workspace"),
                project_root: PathBuf::from("/workspace/project"),
            },
            identity: crate::repository::SnapshotIdentity::Git(Default::default()),
            changes: Vec::new(),
            captured_at: std::time::SystemTime::UNIX_EPOCH,
        };
        let mut later = snapshot.clone();
        later.captured_at = std::time::SystemTime::now();

        assert!(displayed_snapshot_eq(&snapshot, &later));
    }

    #[test]
    fn display_equality_ignores_unrendered_jujutsu_operation_and_commit_ids() {
        let identity = crate::repository::JujutsuIdentity {
            operation_id: "operation-a".into(),
            commit_id: "commit-a".into(),
            change_id: "change".into(),
            description: "description".into(),
            bookmarks: vec!["main".into()],
            closest_bookmarks: vec!["main".into()],
            ahead: 0,
            conflicted_paths: Vec::new(),
            conflicted: false,
            empty: false,
        };
        let mut later = identity.clone();
        later.operation_id = "operation-b".into();
        later.commit_id = "commit-b".into();

        assert!(displayed_identity_eq(
            &crate::repository::SnapshotIdentity::Jujutsu(identity),
            &crate::repository::SnapshotIdentity::Jujutsu(later),
        ));
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
