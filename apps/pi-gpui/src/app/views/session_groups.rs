//! Pure session-rail ordering and project-filter policy.

use std::path::Path;

use crate::{
    projects::DraftSession,
    sessions::{SessionSummary, root_sessions},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SessionRailKind {
    Project,
    Settled,
}

#[derive(Clone, Debug)]
pub(super) struct SessionRailItem {
    pub(super) session: SessionSummary,
    pub(super) kind: SessionRailKind,
}

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

pub(super) fn recent_archived_sessions(
    sessions: &[SessionRailItem],
    limit: usize,
) -> Vec<SessionRailItem> {
    sessions.iter().take(limit).cloned().collect()
}

pub(super) fn session_rail_lists(
    sessions: &[SessionSummary],
    drafts: &[DraftSession],
    project_filter: Option<&Path>,
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
            kind: if session.settled {
                SessionRailKind::Settled
            } else {
                SessionRailKind::Project
            },
        };
        if session.settled {
            archived.push(item);
        } else {
            active.push(ActiveSessionItem::Session(item));
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
    archived.sort_by(|left, right| {
        right
            .session
            .app_session_id
            .cmp(&left.session.app_session_id)
    });

    SessionRailLists { active, archived }
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

        let lists = session_rail_lists(&sessions, &[draft], None);

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

        let lists = session_rail_lists(&sessions, &[alpha_draft, beta_draft], Some(beta.as_path()));

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

        let lists = session_rail_lists(&[persisted], &[draft], None);

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

        let lists = session_rail_lists(&sessions, &[], None);

        assert_eq!(lists.active.len(), 2);
    }

    #[test]
    fn archived_preview_uses_the_same_descending_id_order() {
        let project = PathBuf::from("/project");
        let sessions = vec![
            session("one", 1, &project, true),
            session("three", 3, &project, true),
            session("two", 2, &project, true),
        ];
        let lists = session_rail_lists(&sessions, &[], None);

        assert_eq!(
            recent_archived_sessions(&lists.archived, 2)
                .iter()
                .map(|item| item.session.app_session_id)
                .collect::<Vec<_>>(),
            [3, 2]
        );
    }

    fn session(id: &str, app_session_id: i64, project: &Path, settled: bool) -> SessionSummary {
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
            settled,
            false,
            String::new(),
        )
        .with_app_session_id(app_session_id)
    }
}
