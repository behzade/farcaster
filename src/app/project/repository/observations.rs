use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use crate::repository::{
    BackendPreference, RepositoryBackend, RepositoryError, WorkingCopySnapshot,
};

const CACHE_CAPACITY: usize = 24;

/// The last working copy observed for one project.
pub(super) struct RepositoryObservation {
    pub(super) preference: BackendPreference,
    pub(super) backend: Option<RepositoryBackend>,
    pub(super) snapshot: Option<WorkingCopySnapshot>,
    pub(super) additions: Option<u64>,
    pub(super) deletions: Option<u64>,
}

impl RepositoryObservation {
    fn reusable_for(&self, preference: BackendPreference) -> bool {
        self.preference == preference
    }

    pub(super) fn from_scan(preference: BackendPreference, scanned: ScanResult) -> Option<Self> {
        match scanned {
            Ok(Some((backend, Ok((snapshot, additions, deletions))))) => Some(Self {
                preference,
                backend: Some(backend),
                snapshot: Some(snapshot),
                additions,
                deletions,
            }),
            _ => None,
        }
    }
}

pub(super) type ScanResult = Result<
    Option<(
        RepositoryBackend,
        Result<(WorkingCopySnapshot, Option<u64>, Option<u64>), RepositoryError>,
    )>,
    RepositoryError,
>;

/// Reads a project's working copy without publishing it anywhere.
pub(super) fn observe_project(project: &Path, preference: BackendPreference) -> ScanResult {
    RepositoryBackend::discover(project, preference).map(|backend| {
        backend.map(|backend| {
            let snapshot = backend.snapshot().map(|mut snapshot| {
                let (additions, deletions) = backend
                    .working_copy_totals(&mut snapshot)
                    .unwrap_or((None, None));
                (snapshot, additions, deletions)
            });
            (backend, snapshot)
        })
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ObservationTicket {
    pub(super) project: PathBuf,
    pub(super) preference: BackendPreference,
    generation: u64,
}

/// Inactive projects, oldest first. Reuse moves an observation into the active
/// project; remembering it again makes it the most recently used entry.
#[derive(Default)]
pub(super) struct ObservationCache {
    projects: VecDeque<(PathBuf, RepositoryObservation)>,
    pending: Option<ObservationTicket>,
    generation: u64,
}

impl ObservationCache {
    pub(super) fn remember(&mut self, project: PathBuf, observation: RepositoryObservation) {
        self.forget(&project);
        self.projects.push_back((project, observation));
        if self.projects.len() > CACHE_CAPACITY {
            self.projects.pop_front();
        }
    }

    pub(super) fn reuse(
        &mut self,
        project: &Path,
        preference: BackendPreference,
    ) -> Option<RepositoryObservation> {
        self.invalidate(project);
        let index = self.projects.iter().position(|(path, _)| path == project)?;
        let (_, observation) = self.projects.remove(index)?;
        observation.reusable_for(preference).then_some(observation)
    }

    pub(super) fn begin(
        &mut self,
        project: PathBuf,
        preference: BackendPreference,
    ) -> Option<ObservationTicket> {
        if self.busy() {
            return None;
        }
        self.advance_generation();
        let ticket = ObservationTicket {
            project,
            preference,
            generation: self.generation,
        };
        self.pending = Some(ticket.clone());
        Some(ticket)
    }

    /// Invalid work still occupies the scan slot until it finishes.
    pub(super) fn invalidate(&mut self, project: &Path) {
        if self
            .pending
            .as_ref()
            .is_some_and(|ticket| ticket.project == project)
        {
            self.advance_generation();
        }
    }

    pub(super) fn finish(&mut self, ticket: &ObservationTicket) -> bool {
        if self.pending.as_ref() != Some(ticket) {
            return false;
        }
        self.pending = None;
        ticket.generation == self.generation
    }

    pub(super) fn forget(&mut self, project: &Path) {
        self.projects.retain(|(path, _)| path != project);
        self.invalidate(project);
    }

    pub(super) fn retain(&mut self, projects: &[PathBuf]) {
        self.projects.retain(|(path, _)| projects.contains(path));
        if let Some(ticket) = &self.pending
            && !projects.contains(&ticket.project)
        {
            self.advance_generation();
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    fn advance_generation(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("repository observation generation exhausted");
    }
}

#[cfg(test)]
#[path = "observations_tests.rs"]
mod tests;
