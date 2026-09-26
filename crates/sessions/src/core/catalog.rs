use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use super::super::SessionSummary;

pub fn document_is_live(
    session: &SessionSummary,
    interacted: bool,
    transport_attached: bool,
) -> bool {
    transport_attached || session.is_running || (interacted && !session.archived)
}

pub fn filter_session_tree(mut sessions: Vec<SessionSummary>, query: &str) -> Vec<SessionSummary> {
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return sessions;
    }
    let index = SessionRootIndex::new(&sessions);
    let mut included = sessions
        .iter()
        .filter(|session| session.search_text().contains(&needle))
        .map(|session| session.path.clone())
        .collect::<HashSet<_>>();
    for path in included.clone() {
        let mut current = index.by_path.get(path.as_path()).copied();
        let mut seen = HashSet::new();
        while let Some(session) = current {
            if !seen.insert(session.path.as_path()) {
                break;
            }
            included.insert(session.path.clone());
            current = index.parent(session);
        }
    }
    let by_parent = index.children(&sessions);
    let mut stack = included.iter().cloned().collect::<Vec<_>>();
    while let Some(parent) = stack.pop() {
        if let Some(children) = by_parent.get(parent.as_path()) {
            for child in children {
                if included.insert(child.path.clone()) {
                    stack.push(child.path.clone());
                }
            }
        }
    }
    sessions.retain(|session| included.contains(&session.path));
    sessions
}

pub fn root_sessions(sessions: &[SessionSummary]) -> Vec<&SessionSummary> {
    let index = SessionRootIndex::new(sessions);
    sessions
        .iter()
        .filter(|session| index.parent(session).is_none())
        .collect()
}

pub struct SessionRootIndex<'a> {
    by_id: HashMap<
        (
            &'a Path,
            farcaster_contracts::Backend,
            Option<&'a str>,
            &'a str,
        ),
        &'a SessionSummary,
    >,
    by_path: HashMap<&'a Path, &'a SessionSummary>,
    by_app_id: HashMap<i64, &'a SessionSummary>,
}

impl<'a> SessionRootIndex<'a> {
    pub fn new(sessions: &'a [SessionSummary]) -> Self {
        Self {
            by_app_id: sessions
                .iter()
                .filter(|session| session.app_session_id > 0)
                .map(|session| (session.app_session_id, session))
                .collect(),
            by_id: sessions
                .iter()
                .map(|session| {
                    (
                        (
                            session.project.as_path(),
                            session.harness,
                            profile_id(&session.path),
                            session.id.as_str(),
                        ),
                        session,
                    )
                })
                .collect(),
            by_path: sessions
                .iter()
                .map(|session| (session.path.as_path(), session))
                .collect(),
        }
    }

    pub(super) fn parent(&self, session: &SessionSummary) -> Option<&'a SessionSummary> {
        if let Some(id) = session.parent_app_session_id {
            // An unresolved cached parent must not bind to a native-ID homonym.
            return self.by_app_id.get(&id).copied();
        }
        let parent = session.parent_session.as_deref()?;
        let harness = session.parent_harness.unwrap_or(session.harness);
        self.by_id
            .get(&(
                session.project.as_path(),
                harness,
                profile_id(&session.path),
                parent,
            ))
            .copied()
    }

    fn children(
        &self,
        sessions: &'a [SessionSummary],
    ) -> HashMap<&'a Path, Vec<&'a SessionSummary>> {
        let mut children: HashMap<&Path, Vec<&SessionSummary>> = HashMap::new();
        for session in sessions {
            if let Some(parent) = self.parent(session) {
                children
                    .entry(parent.path.as_path())
                    .or_default()
                    .push(session);
            }
        }
        children
    }

    pub fn root_for_path(&self, selected: Option<&Path>) -> Option<&'a SessionSummary> {
        let mut current = *self.by_path.get(selected?)?;
        for _ in 0..self.by_path.len() {
            let Some(parent) = self.parent(current) else {
                break;
            };
            current = parent;
        }
        Some(current)
    }
}

fn profile_id(path: &Path) -> Option<&str> {
    let backend = path.parent()?;
    let profile = backend.parent()?;
    (profile.parent()?.file_name()? == "profiles")
        .then(|| profile.file_name()?.to_str())
        .flatten()
}

pub fn root_session_for_path<'a>(
    sessions: &'a [SessionSummary],
    selected: Option<&Path>,
) -> Option<&'a SessionSummary> {
    SessionRootIndex::new(sessions).root_for_path(selected)
}

pub fn is_subagent_path(sessions: &[SessionSummary], path: &Path) -> bool {
    sessions
        .iter()
        .any(|session| session.path == path && session.parent_session.is_some())
}

#[cfg(test)]
pub fn descendant_sessions<'a>(
    sessions: &'a [SessionSummary],
    root_id: &str,
) -> Vec<(&'a SessionSummary, usize)> {
    sessions
        .iter()
        .find(|session| session.id == root_id)
        .map_or_else(Vec::new, |root| {
            descendant_sessions_for_root(sessions, root)
        })
}

pub fn descendant_sessions_for_root<'a>(
    sessions: &'a [SessionSummary],
    root: &SessionSummary,
) -> Vec<(&'a SessionSummary, usize)> {
    let index = SessionRootIndex::new(sessions);
    let by_parent = index.children(sessions);
    let mut stack = by_parent
        .get(root.path.as_path())
        .into_iter()
        .flatten()
        .rev()
        .map(|session| (*session, 1_usize))
        .collect::<Vec<_>>();
    let mut descendants = Vec::new();
    let mut seen = HashSet::from([root.path.as_path()]);
    while let Some((session, depth)) = stack.pop() {
        if !seen.insert(session.path.as_path()) {
            continue;
        }
        descendants.push((session, depth));
        if let Some(children) = by_parent.get(session.path.as_path()) {
            stack.extend(
                children
                    .iter()
                    .rev()
                    .map(|child| (*child, depth.saturating_add(1))),
            );
        }
    }
    descendants
}

pub fn session_family_for_path<'a>(
    sessions: &'a [SessionSummary],
    path: &Path,
) -> Option<Vec<&'a SessionSummary>> {
    let root = root_session_for_path(sessions, Some(path))?;
    let mut family = vec![root];
    family.extend(
        descendant_sessions_for_root(sessions, root)
            .into_iter()
            .map(|(session, _)| session),
    );
    Some(family)
}

pub fn archived_root_family_for_path<'a>(
    sessions: &'a [SessionSummary],
    path: &Path,
) -> Option<Vec<&'a SessionSummary>> {
    let requested = sessions.iter().find(|session| session.path == path)?;
    if requested.parent_session.is_some() || !requested.archived {
        return None;
    }
    session_family_for_path(sessions, path)
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
