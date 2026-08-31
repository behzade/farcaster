use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

use super::super::contract::{RUNNING_ACTIVITY_TIMEOUT, SessionSummary};

#[derive(Default)]
pub(crate) struct ExternalActivityTracker {
    deadlines: HashMap<PathBuf, Instant>,
}

impl ExternalActivityTracker {
    pub(crate) fn observe_files(
        &mut self,
        catalog: &[SessionSummary],
        owned: &HashSet<PathBuf>,
        paths: &[PathBuf],
        now: Instant,
        normalize: impl Fn(&Path) -> PathBuf,
    ) -> bool {
        let mut refresh = false;
        for candidate in paths {
            if owned.contains(candidate) {
                continue;
            }
            let known = catalog
                .iter()
                .find(|session| session.path == candidate.as_path());
            let (path, is_running) = if let Some(session) = known {
                (session.path.clone(), session.is_running)
            } else {
                let path = if self.deadlines.contains_key(candidate) {
                    candidate.clone()
                } else {
                    normalize(candidate)
                };
                let is_running = catalog
                    .iter()
                    .any(|session| session.path == path && session.is_running);
                (path, is_running)
            };
            if owned.contains(&path) {
                continue;
            }
            let became_active = self
                .deadlines
                .insert(path, now + RUNNING_ACTIVITY_TIMEOUT)
                .is_none();
            refresh |= became_active && !is_running;
        }
        refresh
    }

    pub(crate) fn remove_owned(&mut self, owned: &HashSet<PathBuf>) {
        self.deadlines.retain(|path, _| !owned.contains(path));
    }

    pub(crate) fn sync_catalog(
        &mut self,
        sessions: &[SessionSummary],
        exhaustive: bool,
        owned: &HashSet<PathBuf>,
        now: Instant,
        wall_now: SystemTime,
    ) {
        let mut seen = HashSet::new();
        for session in sessions {
            let path = session.path.clone();
            seen.insert(path.clone());
            if owned.contains(&path) || !session.is_running {
                self.deadlines.remove(&path);
                continue;
            }
            let remaining = wall_now
                .duration_since(session.modified)
                .map_or(RUNNING_ACTIVITY_TIMEOUT, |age| {
                    RUNNING_ACTIVITY_TIMEOUT.saturating_sub(age)
                });
            let due = now + remaining;
            self.deadlines
                .entry(path)
                .and_modify(|current| *current = (*current).max(due))
                .or_insert(due);
        }
        if exhaustive {
            self.deadlines.retain(|path, _| seen.contains(path));
        }
    }

    pub(crate) fn take_expired(&mut self, now: Instant) -> bool {
        let previous_len = self.deadlines.len();
        self.deadlines.retain(|_, due| *due > now);
        self.deadlines.len() != previous_len
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.values().copied().min()
    }
}
