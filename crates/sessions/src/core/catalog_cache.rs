use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

use super::catalog::SessionRootIndex;
use crate::SessionSummary;

/// A catalog whose relationships are indexed once per revision. Mutable access
/// drops the index before any row, path, or parent can change.
#[derive(Default)]
pub struct SessionCatalog {
    sessions: Vec<SessionSummary>,
    relationships: OnceCell<Relationships>,
}

struct Relationships {
    by_path: HashMap<PathBuf, usize>,
    parents: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
}

impl From<Vec<SessionSummary>> for SessionCatalog {
    fn from(sessions: Vec<SessionSummary>) -> Self {
        Self {
            sessions,
            relationships: OnceCell::new(),
        }
    }
}

impl Deref for SessionCatalog {
    type Target = Vec<SessionSummary>;

    fn deref(&self) -> &Self::Target {
        &self.sessions
    }
}

impl DerefMut for SessionCatalog {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.relationships.take();
        &mut self.sessions
    }
}

impl SessionCatalog {
    fn relationships(&self) -> &Relationships {
        self.relationships.get_or_init(|| {
            let index = SessionRootIndex::new(&self.sessions);
            let by_path: HashMap<_, _> = self
                .sessions
                .iter()
                .enumerate()
                .map(|(i, session)| (session.path.clone(), i))
                .collect();
            let parents: Vec<_> = self
                .sessions
                .iter()
                .map(|session| {
                    index
                        .parent(session)
                        .and_then(|parent| by_path.get(&parent.path).copied())
                })
                .collect();
            let mut children = vec![Vec::new(); self.sessions.len()];
            for (child, parent) in parents.iter().enumerate() {
                if let Some(parent) = parent {
                    children[*parent].push(child);
                }
            }
            Relationships {
                by_path,
                parents,
                children,
            }
        })
    }

    pub fn root_for_path(&self, selected: Option<&Path>) -> Option<&SessionSummary> {
        let selected = selected?;
        let relationships = self.relationships();
        let mut current = *relationships.by_path.get(selected)?;
        for _ in 0..relationships.by_path.len() {
            let Some(parent) = relationships.parents[current] else {
                break;
            };
            current = parent;
        }
        Some(&self.sessions[current])
    }

    pub fn roots(&self) -> impl Iterator<Item = &SessionSummary> {
        self.sessions
            .iter()
            .zip(&self.relationships().parents)
            .filter_map(|(session, parent)| parent.is_none().then_some(session))
    }

    pub fn descendants(&self, root: &SessionSummary) -> Vec<(&SessionSummary, usize)> {
        let relationships = self.relationships();
        let Some(&root) = relationships.by_path.get(&root.path) else {
            return Vec::new();
        };
        let mut stack: Vec<_> = relationships.children[root]
            .iter()
            .rev()
            .map(|&child| (child, 1_usize))
            .collect();
        let mut seen = HashSet::from([root]);
        let mut descendants = Vec::new();
        while let Some((child, depth)) = stack.pop() {
            if !seen.insert(child) {
                continue;
            }
            descendants.push((&self.sessions[child], depth));
            stack.extend(
                relationships.children[child]
                    .iter()
                    .rev()
                    .map(|&child| (child, depth.saturating_add(1))),
            );
        }
        descendants
    }
}

#[cfg(test)]
#[path = "catalog_cache_tests.rs"]
mod tests;
