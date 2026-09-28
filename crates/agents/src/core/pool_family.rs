use super::*;
use crate::core::caller::{CallerSession, SessionBinding};
use std::path::PathBuf;

#[derive(Clone)]
pub(super) struct FamilyFence {
    project: PathBuf,
    keys: BTreeSet<SessionKey>,
    bindings: Vec<SessionBinding>,
    roots: BTreeMap<SessionKey, Vec<SessionBinding>>,
    workers: BTreeSet<String>,
}

impl FamilyFence {
    pub(super) fn new(project: PathBuf, keys: BTreeSet<SessionKey>) -> Self {
        let mut fence = Self {
            project,
            keys,
            bindings: Vec::new(),
            roots: BTreeMap::new(),
            workers: BTreeSet::new(),
        };
        fence.refresh_roots();
        fence
    }

    pub(super) fn refresh_roots(&mut self) {
        for (id, binding) in super::super::CallerRegistry::shared().worker_bindings(&self.project) {
            if self.matches_binding(&binding) {
                self.track_worker(id, binding);
            }
        }
    }

    fn matches_identity(&self, identity: &CallerSession) -> bool {
        let Some(key) = &identity.key else {
            return false;
        };
        (self.keys.contains(key)
            && (matches!(key, SessionKey::App(_)) || identity.native.project == self.project))
            || self.bindings.iter().any(|binding| {
                binding
                    .lock()
                    .ok()
                    .and_then(|binding| binding.clone())
                    .is_some_and(|bound| {
                        bound.key.as_ref() == Some(key)
                            && (matches!(key, SessionKey::App(_))
                                || bound.native.project == identity.native.project)
                    })
            })
    }

    fn matches_binding(&self, binding: &SessionBinding) -> bool {
        let identity = binding.lock().ok().and_then(|binding| binding.clone());
        identity
            .as_ref()
            .is_some_and(|identity| self.matches_identity(identity))
    }

    fn matches_parent(
        &self,
        project: &Path,
        worker: Option<&str>,
        binding: &SessionBinding,
    ) -> bool {
        (project == self.project && worker.is_some_and(|id| self.workers.contains(id)))
            || self.matches_binding(binding)
    }

    pub(super) fn track_worker(&mut self, id: String, binding: SessionBinding) {
        self.workers.insert(id);
        self.track_binding(binding);
    }

    fn track_binding(&mut self, binding: SessionBinding) {
        let key = binding
            .lock()
            .ok()
            .and_then(|bound| bound.as_ref().and_then(|bound| bound.key.clone()));
        if let Some(key) = key.filter(|key| self.keys.contains(key)) {
            let roots = self.roots.entry(key).or_default();
            if !roots.iter().any(|root| Arc::ptr_eq(root, &binding)) {
                roots.push(binding.clone());
            }
        }
        if !self
            .bindings
            .iter()
            .any(|saved| Arc::ptr_eq(saved, &binding))
        {
            self.bindings.push(binding);
        }
    }

    pub(super) fn covered_by(&self, completed: &Self) -> bool {
        self.project == completed.project
            && self.keys.iter().all(|key| {
                completed.keys.contains(key)
                    || self.roots.get(key).is_some_and(|bindings| {
                        bindings
                            .iter()
                            .any(|binding| completed.matches_binding(binding))
                    })
            })
    }

    pub(super) fn refresh_worker(&mut self, id: &str, binding: SessionBinding) {
        if self.workers.contains(id) {
            self.track_worker(id.to_owned(), binding);
        }
    }

    pub(super) fn expand(&mut self, state: &PoolState) -> Vec<String> {
        let mut matched = BTreeSet::new();
        loop {
            let before = matched.len();
            for (id, record) in &state.records {
                if matched.contains(id) {
                    continue;
                }
                let binding = super::super::CallerRegistry::shared()
                    .worker_binding(&record.launch.project, id)
                    .unwrap_or_else(|| record.session_binding.clone());
                let parent_matches = self.matches_parent(
                    &record.launch.project,
                    record.launch.parent_worker_id.as_deref(),
                    &record.parent_binding,
                );
                if self.matches_binding(&binding) || parent_matches {
                    if parent_matches {
                        self.track_binding(record.parent_binding.clone());
                    }
                    matched.insert(id.clone());
                    self.track_worker(id.clone(), binding);
                }
            }
            if matched.len() == before {
                return matched.into_iter().collect();
            }
        }
    }
}

pub(super) fn family_is_stopping(
    state: &PoolState,
    project: &Path,
    worker: Option<&str>,
    binding: &SessionBinding,
) -> bool {
    state
        .stopping_families
        .values()
        .any(|fence| fence.matches_parent(project, worker, binding))
}
