use std::path::{Path, PathBuf};

use super::super::contract::SessionSummary;

pub trait SessionStore {
    fn cached(&self, query: &str) -> Result<Vec<SessionSummary>, String>;
    fn index(&mut self, sessions: &[SessionSummary], prune_missing: bool) -> Result<(), String>;
    fn relocate(
        &mut self,
        paths: &[(PathBuf, PathBuf)],
        target_project: &Path,
    ) -> Result<(), String>;
    fn delete(&mut self, paths: &[PathBuf]) -> Result<(), String>;
    fn set_archived(&self, path: &Path, archived: bool) -> Result<(), String>;
}

pub fn cached(store: &impl SessionStore, query: &str) -> Result<Vec<SessionSummary>, String> {
    store.cached(query)
}

pub fn index(
    store: &mut impl SessionStore,
    sessions: &[SessionSummary],
    prune_missing: bool,
) -> Result<(), String> {
    store.index(sessions, prune_missing)
}

pub fn relocate(
    store: &mut impl SessionStore,
    paths: &[(PathBuf, PathBuf)],
    target_project: &Path,
) -> Result<(), String> {
    store.relocate(paths, target_project)
}

pub fn delete(store: &mut impl SessionStore, paths: &[PathBuf]) -> Result<(), String> {
    store.delete(paths)
}

pub fn set_archived(store: &impl SessionStore, path: &Path, archived: bool) -> Result<(), String> {
    store.set_archived(path, archived)
}
