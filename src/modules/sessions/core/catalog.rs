use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use super::super::SessionSummary;

pub(crate) fn document_is_live(
    session: &SessionSummary,
    interacted: bool,
    transport_attached: bool,
) -> bool {
    transport_attached || session.is_running || (interacted && !session.archived)
}

pub(crate) fn filter_session_tree(
    mut sessions: Vec<SessionSummary>,
    query: &str,
) -> Vec<SessionSummary> {
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return sessions;
    }
    let by_id = sessions
        .iter()
        .map(|session| (session.id.as_str(), session))
        .collect::<HashMap<_, _>>();
    let mut included = sessions
        .iter()
        .filter(|session| session.search_text().contains(&needle))
        .map(|session| session.id.clone())
        .collect::<HashSet<_>>();
    for id in included.clone() {
        let mut current = by_id.get(id.as_str()).copied();
        let mut seen = HashSet::new();
        while let Some(session) = current {
            if !seen.insert(session.id.as_str()) {
                break;
            }
            included.insert(session.id.clone());
            current = session
                .parent_session
                .as_deref()
                .and_then(|parent| by_id.get(parent).copied());
        }
    }
    let by_parent = sessions.iter().fold(
        HashMap::<&str, Vec<&str>>::new(),
        |mut children, session| {
            if let Some(parent) = session.parent_session.as_deref() {
                children
                    .entry(parent)
                    .or_default()
                    .push(session.id.as_str());
            }
            children
        },
    );
    let mut stack = included.iter().cloned().collect::<Vec<_>>();
    let mut expanded = HashSet::new();
    while let Some(parent) = stack.pop() {
        if !expanded.insert(parent.clone()) {
            continue;
        }
        if let Some(children) = by_parent.get(parent.as_str()) {
            for child in children {
                included.insert((*child).to_owned());
                stack.push((*child).to_owned());
            }
        }
    }
    sessions.retain(|session| included.contains(&session.id));
    sessions
}

pub(crate) fn root_sessions(sessions: &[SessionSummary]) -> Vec<&SessionSummary> {
    sessions
        .iter()
        .filter(|session| session.parent_session.is_none())
        .collect()
}

pub(crate) struct SessionRootIndex<'a> {
    by_id: HashMap<&'a str, &'a SessionSummary>,
    by_path: HashMap<&'a Path, &'a SessionSummary>,
}

impl<'a> SessionRootIndex<'a> {
    pub(crate) fn new(sessions: &'a [SessionSummary]) -> Self {
        Self {
            by_id: sessions
                .iter()
                .map(|session| (session.id.as_str(), session))
                .collect(),
            by_path: sessions
                .iter()
                .map(|session| (session.path.as_path(), session))
                .collect(),
        }
    }

    pub(crate) fn root_for_path(&self, selected: Option<&Path>) -> Option<&'a SessionSummary> {
        let mut current = *self.by_path.get(selected?)?;
        for _ in 0..self.by_id.len() {
            let Some(parent) = current.parent_session.as_deref() else {
                break;
            };
            let Some(parent) = self.by_id.get(parent) else {
                break;
            };
            current = *parent;
        }
        Some(current)
    }
}

pub(crate) fn root_session_for_path<'a>(
    sessions: &'a [SessionSummary],
    selected: Option<&Path>,
) -> Option<&'a SessionSummary> {
    SessionRootIndex::new(sessions).root_for_path(selected)
}

pub(crate) fn is_subagent_path(sessions: &[SessionSummary], path: &Path) -> bool {
    sessions
        .iter()
        .any(|session| session.path == path && session.parent_session.is_some())
}

pub(crate) fn descendant_sessions<'a>(
    sessions: &'a [SessionSummary],
    root_id: &str,
) -> Vec<(&'a SessionSummary, usize)> {
    let mut by_parent: HashMap<&str, Vec<&SessionSummary>> = HashMap::new();
    for session in sessions {
        if let Some(parent) = session.parent_session.as_deref() {
            by_parent.entry(parent).or_default().push(session);
        }
    }
    let mut stack = by_parent
        .get(root_id)
        .into_iter()
        .flatten()
        .rev()
        .map(|session| (*session, 1_usize))
        .collect::<Vec<_>>();
    let mut descendants = Vec::new();
    let mut seen = HashSet::new();
    while let Some((session, depth)) = stack.pop() {
        if !seen.insert(session.id.as_str()) {
            continue;
        }
        descendants.push((session, depth));
        if let Some(children) = by_parent.get(session.id.as_str()) {
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

pub(crate) fn session_family_for_path<'a>(
    sessions: &'a [SessionSummary],
    path: &Path,
) -> Option<Vec<&'a SessionSummary>> {
    let root = root_session_for_path(sessions, Some(path))?;
    let mut family = vec![root];
    family.extend(
        descendant_sessions(sessions, &root.id)
            .into_iter()
            .map(|(session, _)| session),
    );
    Some(family)
}

pub(crate) fn archived_root_family_for_path<'a>(
    sessions: &'a [SessionSummary],
    path: &Path,
) -> Option<Vec<&'a SessionSummary>> {
    let requested = sessions.iter().find(|session| session.path == path)?;
    if requested.parent_session.is_some() || !requested.archived {
        return None;
    }
    session_family_for_path(sessions, path)
}
