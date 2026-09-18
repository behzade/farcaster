use std::{cmp::Reverse, collections::HashMap, path::Path};

use crate::{
    app::ui::primitives::ReorderPosition,
    sessions::{DraftSession, SessionSummary, root_sessions},
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

    pub(super) fn project(&self) -> &Path {
        match self {
            Self::Draft(draft) => draft.project.as_path(),
            Self::Session(item) => item.session.project.as_path(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct SessionRailLists {
    pub(super) active: Vec<ActiveSessionItem>,
    pub(super) archived: Vec<SessionRailItem>,
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
    active.sort_by_key(active_kind_rank);
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
#[path = "groups_scale_tests.rs"]
mod scale_tests;
#[cfg(test)]
#[path = "groups_tests.rs"]
mod tests;
