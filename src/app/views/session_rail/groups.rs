use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    app::ui::primitives::ReorderPosition,
    projects::DraftSession,
    sessions::{SessionSummary, root_sessions},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum SessionRailKind {
    Project,
    Archived,
}

#[derive(Clone, Debug)]
pub(super) struct SessionRailItem {
    pub(super) session: SessionSummary,
    pub(super) kind: SessionRailKind,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub(super) enum ActiveSessionItem {
    Draft(DraftSession),
    Session(SessionRailItem),
}

impl ActiveSessionItem {
    pub(super) fn app_session_id(&self) -> i64 {
        match self {
            Self::Draft(draft) => draft.app_session_id,
            Self::Session(item) => item.session.app_session_id,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct SessionRailLists {
    pub(super) active: Vec<ActiveSessionItem>,
    pub(super) archived: Vec<SessionRailItem>,
}

pub(in crate::app) fn roots_waiting_for_descendants(
    sessions: &[SessionSummary],
) -> HashSet<String> {
    let parent_by_id = sessions
        .iter()
        .filter_map(|session| {
            session
                .parent_session
                .as_ref()
                .map(|parent| (session.id.as_str(), parent.as_str()))
        })
        .collect::<HashMap<_, _>>();
    let mut waiting = HashSet::new();
    for session in sessions.iter().filter(|session| session.is_running) {
        let mut current = session.id.as_str();
        let mut seen = HashSet::new();
        while seen.insert(current) {
            let Some(parent) = parent_by_id.get(current).copied() else {
                break;
            };
            waiting.insert(parent.to_owned());
            current = parent;
        }
    }
    waiting
}

pub(super) fn session_rail_lists(
    sessions: &[SessionSummary],
    drafts: &[DraftSession],
    project_filter: Option<&Path>,
    manual_order: &[i64],
) -> SessionRailLists {
    let mut active = drafts
        .iter()
        .filter(|draft| project_filter.is_none_or(|filter| filter == draft.project))
        .cloned()
        .map(ActiveSessionItem::Draft)
        .collect::<Vec<_>>();
    let mut archived = Vec::new();

    for session in root_sessions(sessions)
        .into_iter()
        .filter(|session| project_filter.is_none_or(|filter| filter == session.project))
    {
        let item = SessionRailItem {
            session: session.clone(),
            kind: if session.archived {
                SessionRailKind::Archived
            } else {
                SessionRailKind::Project
            },
        };
        match item.kind {
            SessionRailKind::Project => active.push(ActiveSessionItem::Session(item)),
            SessionRailKind::Archived => archived.push(item),
        }
    }

    active.sort_by(|left, right| {
        right
            .app_session_id()
            .cmp(&left.app_session_id())
            .then_with(|| active_kind_rank(left).cmp(&active_kind_rank(right)))
    });
    active.dedup_by(|left, right| {
        let id = left.app_session_id();
        id > 0 && id == right.app_session_id()
    });
    apply_manual_order(&mut active, manual_order, ActiveSessionItem::app_session_id);
    archived.sort_by_key(|item| Reverse((item.session.modified, item.session.app_session_id)));

    SessionRailLists { active, archived }
}

fn apply_manual_order<T>(items: &mut [T], order: &[i64], app_session_id: impl Fn(&T) -> i64) {
    let rank = order
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect::<HashMap<_, _>>();
    items.sort_by(|left, right| {
        match (
            rank.get(&app_session_id(left)),
            rank.get(&app_session_id(right)),
        ) {
            (Some(left), Some(right)) => left.cmp(right),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
}

pub(super) fn merge_visible_session_order(all: &[i64], visible: &[i64]) -> Vec<i64> {
    let visible_ids = visible
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let mut reordered = visible.iter().copied();
    all.iter()
        .map(|id| {
            if visible_ids.contains(id) {
                reordered.next().unwrap_or(*id)
            } else {
                *id
            }
        })
        .collect()
}

pub(super) fn reordered_session_ids(
    visible: &[i64],
    source: i64,
    target: i64,
    position: ReorderPosition,
) -> Option<Vec<i64>> {
    if source == target {
        return None;
    }
    let mut order = visible.to_vec();
    let source_index = order.iter().position(|id| *id == source)?;
    order.remove(source_index);
    let target_index = order.iter().position(|id| *id == target)?;
    let insertion = target_index + usize::from(position == ReorderPosition::After);
    order.insert(insertion, source);
    (order != visible).then_some(order)
}

const fn active_kind_rank(item: &ActiveSessionItem) -> u8 {
    match item {
        ActiveSessionItem::Draft(_) => 0,
        ActiveSessionItem::Session(_) => 1,
    }
}

#[cfg(test)]
#[path = "groups_tests.rs"]
mod tests;
