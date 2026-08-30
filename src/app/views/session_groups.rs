use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    primitives::ReorderPosition,
    projects::DraftSession,
    sessions::{SessionSummary, root_sessions},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum SessionRailKind {
    Project,
    Review,
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
    pub(super) review: Vec<SessionRailItem>,
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
    let mut review = Vec::new();
    let mut archived = Vec::new();

    for session in root_sessions(sessions)
        .into_iter()
        .filter(|session| project_filter.is_none_or(|filter| filter == session.project))
    {
        let item = SessionRailItem {
            session: session.clone(),
            kind: if session.archived {
                SessionRailKind::Archived
            } else if session.in_review {
                SessionRailKind::Review
            } else {
                SessionRailKind::Project
            },
        };
        match item.kind {
            SessionRailKind::Project => active.push(ActiveSessionItem::Session(item)),
            SessionRailKind::Review => review.push(item),
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
    sort_sessions(&mut review, manual_order);
    archived.sort_by(|left, right| {
        right
            .session
            .app_session_id
            .cmp(&left.session.app_session_id)
    });

    SessionRailLists {
        active,
        review,
        archived,
    }
}

fn sort_sessions(items: &mut [SessionRailItem], order: &[i64]) {
    items.sort_by(|left, right| {
        right
            .session
            .app_session_id
            .cmp(&left.session.app_session_id)
    });
    apply_manual_order(items, order, |item| item.session.app_session_id);
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
mod tests {
    use std::{path::PathBuf, time::SystemTime};

    use super::*;
    use crate::sessions::UsageSummary;

    #[test]
    fn drafts_and_sessions_share_one_descending_id_order() {
        let alpha = PathBuf::from("/alpha");
        let beta = PathBuf::from("/beta");
        let mut draft = DraftSession::with_id("draft".into(), alpha.clone());
        draft.app_session_id = 2;
        let sessions = vec![
            session("old", 1, &alpha, false),
            session("new", 3, &beta, false),
        ];

        let lists = session_rail_lists(&sessions, &[draft], None, &[]);

        assert_eq!(
            lists
                .active
                .iter()
                .map(ActiveSessionItem::app_session_id)
                .collect::<Vec<_>>(),
            [3, 2, 1]
        );
    }

    #[test]
    fn manual_order_overrides_id_order_and_new_ids_stay_first() {
        let project = PathBuf::from("/project");
        let sessions = vec![
            session("new", 4, &project, false),
            session("three", 3, &project, false),
            session("two", 2, &project, false),
            session("one", 1, &project, false),
        ];

        let lists = session_rail_lists(&sessions, &[], None, &[1, 3, 2]);

        assert_eq!(
            lists
                .active
                .iter()
                .map(ActiveSessionItem::app_session_id)
                .collect::<Vec<_>>(),
            [4, 1, 3, 2]
        );
    }

    #[test]
    fn reorder_uses_before_and_after_insertion_gaps() {
        assert_eq!(
            reordered_session_ids(&[4, 3, 2, 1], 4, 2, ReorderPosition::After),
            Some(vec![3, 2, 4, 1])
        );
        assert_eq!(
            reordered_session_ids(&[4, 3, 2, 1], 1, 3, ReorderPosition::Before),
            Some(vec![4, 1, 3, 2])
        );
        assert_eq!(
            reordered_session_ids(&[4, 3, 2, 1], 3, 3, ReorderPosition::Before),
            None
        );
    }

    #[test]
    fn filtered_reorder_preserves_hidden_row_positions() {
        assert_eq!(
            merge_visible_session_order(&[5, 4, 3, 2, 1], &[1, 3, 5]),
            [1, 4, 3, 2, 5]
        );
    }

    #[test]
    fn project_filter_keeps_a_flat_subset() {
        let alpha = PathBuf::from("/alpha");
        let beta = PathBuf::from("/beta");
        let mut alpha_draft = DraftSession::with_id("alpha-draft".into(), alpha.clone());
        alpha_draft.app_session_id = 4;
        let mut beta_draft = DraftSession::with_id("beta-draft".into(), beta.clone());
        beta_draft.app_session_id = 3;
        let sessions = vec![
            session("alpha-active", 2, &alpha, false),
            session("beta-archived", 1, &beta, true),
        ];

        let lists = session_rail_lists(
            &sessions,
            &[alpha_draft, beta_draft],
            Some(beta.as_path()),
            &[],
        );

        assert_eq!(lists.active.len(), 1);
        assert_eq!(lists.active[0].app_session_id(), 3);
        assert_eq!(lists.archived.len(), 1);
        assert_eq!(lists.archived[0].session.app_session_id, 1);
    }

    #[test]
    fn promotion_identity_is_rendered_once_and_prefers_the_draft() {
        let project = PathBuf::from("/project");
        let mut draft = DraftSession::with_id("draft".into(), project.clone());
        draft.app_session_id = 7;
        draft.submitted = true;
        let persisted = session("persisted", 7, &project, false);

        let lists = session_rail_lists(&[persisted], &[draft], None, &[]);

        assert_eq!(lists.active.len(), 1);
        assert!(matches!(lists.active[0], ActiveSessionItem::Draft(_)));
    }

    #[test]
    fn unassigned_fallback_sessions_are_not_deduplicated() {
        let project = PathBuf::from("/project");
        let sessions = vec![
            session("one", 0, &project, false),
            session("two", 0, &project, false),
        ];

        let lists = session_rail_lists(&sessions, &[], None, &[]);

        assert_eq!(lists.active.len(), 2);
    }

    #[test]
    fn review_is_a_separate_bucket_with_active_manual_order() {
        let project = PathBuf::from("/project");
        let mut first = session("first", 1, &project, false);
        first.in_review = true;
        let mut second = session("second", 2, &project, false);
        second.in_review = true;
        let active = session("active", 3, &project, false);

        let lists = session_rail_lists(&[first, second, active], &[], None, &[1, 2, 3]);

        assert_eq!(lists.active.len(), 1);
        assert_eq!(lists.review.len(), 2);
        assert_eq!(
            lists
                .review
                .iter()
                .map(|item| item.session.app_session_id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(lists.archived.is_empty());
    }

    fn session(id: &str, app_session_id: i64, project: &Path, archived: bool) -> SessionSummary {
        SessionSummary::from_cached(
            id.into(),
            PathBuf::from(format!("/{id}.jsonl")),
            project.to_path_buf(),
            id.into(),
            String::new(),
            String::new(),
            None,
            SystemTime::UNIX_EPOCH,
            0,
            UsageSummary::default(),
            archived,
            false,
            String::new(),
        )
        .with_app_session_id(app_session_id)
    }
}
