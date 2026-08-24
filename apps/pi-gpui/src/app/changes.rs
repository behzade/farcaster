//! Application state for file changes retained in Pi session records.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::PathBuf,
};

use gpui::{AppContext as _, Context, FocusHandle};

use super::PiApp;
use crate::{
    agent_activity::FileMutation,
    session_changes::{self, ChangeSet},
    sessions::{descendant_sessions, root_session_for_path},
};

#[derive(Default)]
struct RefreshGate {
    root_identity: Option<String>,
    fingerprint: Option<u64>,
    generation: u64,
    in_flight: bool,
    pending: bool,
}

impl RefreshGate {
    fn select_root(&mut self, next: Option<String>) -> bool {
        if self.root_identity == next {
            return false;
        }
        self.root_identity = next;
        self.fingerprint = None;
        self.generation = self.generation.saturating_add(1);
        self.in_flight = false;
        self.pending = false;
        true
    }

    fn request(&mut self, fingerprint: u64) -> Option<u64> {
        if self.fingerprint == Some(fingerprint) {
            return None;
        }
        self.fingerprint = Some(fingerprint);
        if self.in_flight {
            self.pending = true;
            return None;
        }
        self.in_flight = true;
        self.generation = self.generation.saturating_add(1);
        Some(self.generation)
    }

    fn finish(&mut self, generation: u64) -> Option<bool> {
        if generation != self.generation {
            return None;
        }
        self.in_flight = false;
        let rerun = std::mem::take(&mut self.pending);
        if rerun {
            self.fingerprint = None;
        }
        Some(rerun)
    }
}

pub(crate) struct ChangesState {
    pub set: ChangeSet,
    refresh: RefreshGate,
    pub row_focus: HashMap<PathBuf, FocusHandle>,
}

impl ChangesState {
    pub fn new(_: &mut Context<PiApp>) -> Self {
        Self {
            set: ChangeSet::default(),
            refresh: RefreshGate::default(),
            row_focus: HashMap::new(),
        }
    }
}

impl PiApp {
    pub(crate) fn request_changes_refresh(&mut self, cx: &mut Context<Self>) {
        let root = root_session_for_path(
            &self.all_sessions,
            self.snapshot.selected_session.as_deref(),
        );
        let identity = root.map(|root| root.path.to_string_lossy().into_owned());
        if self.changes.refresh.select_root(identity) {
            self.changes.set = ChangeSet::default();
            self.changes.row_focus.clear();
        }
        let Some(root) = root else {
            return;
        };
        let project = crate::sessions::normalize_lexical(&root.project);
        let descendants = descendant_sessions(&self.all_sessions, &root.id);
        let mut ids = vec![root.id.clone()];
        ids.extend(
            descendants
                .into_iter()
                .map(|(session, _)| session.id.clone()),
        );
        let mut mutations = Vec::<FileMutation>::new();
        let mut incomplete = false;
        for id in ids {
            if let Some(activity) = self.agent_activities.get(&id) {
                incomplete |= activity.limited;
                mutations.extend(activity.file_mutations.iter().cloned());
            }
        }
        mutations.retain(|mutation| is_project_change(&mutation.path, &project));
        mutations.sort_by_key(|mutation| mutation.observed_at);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        root.id.hash(&mut hasher);
        incomplete.hash(&mut hasher);
        for mutation in &mutations {
            mutation.hash(&mut hasher);
        }
        let fingerprint = hasher.finish();
        let Some(generation) = self.changes.refresh.request(fingerprint) else {
            return;
        };
        let task = cx.background_spawn(async move {
            let mut set = session_changes::collect(mutations);
            set.incomplete = incomplete;
            set
        });
        cx.spawn(async move |weak, cx| {
            let set = task.await;
            let _ = weak.update(cx, |this, cx| {
                let Some(rerun) = this.changes.refresh.finish(generation) else {
                    return;
                };
                this.changes
                    .row_focus
                    .retain(|path, _| set.files.iter().any(|file| &file.path == path));
                for file in &set.files {
                    this.changes
                        .row_focus
                        .entry(file.path.clone())
                        .or_insert_with(|| cx.focus_handle());
                }
                this.changes.set = set;
                this.notify_run_panel(cx);
                if rerun {
                    this.request_changes_refresh(cx);
                }
            });
        })
        .detach();
    }
}

fn is_project_change(path: &std::path::Path, project: &std::path::Path) -> bool {
    path.starts_with(project) && path != project
}

#[cfg(test)]
mod tests {
    use super::RefreshGate;

    #[test]
    fn root_identity_invalidates_stale_work_and_revisiting_refreshes() {
        let mut gate = RefreshGate::default();
        assert!(gate.select_root(Some("root-a".into())));
        let stale = gate.request(7).expect("first request");
        assert!(gate.select_root(Some("root-b".into())));
        let current = gate.request(7).expect("same fingerprint on new root");
        assert_ne!(stale, current);
        assert_eq!(gate.finish(stale), None);
        assert_eq!(gate.finish(current), Some(false));

        assert!(gate.select_root(None));
        assert!(gate.select_root(Some("root-a".into())));
        assert!(gate.request(7).is_some());
    }
}
