//! Pure session-rail grouping and project-filter policy.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

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
pub(super) enum ActiveProjectItem {
    Draft(DraftSession),
    Session(SessionRailItem),
}

#[derive(Clone, Debug)]
pub(super) struct ProjectGroup<T> {
    pub(super) project: PathBuf,
    pub(super) items: Vec<T>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct SessionRailGroups {
    pub(super) active: Vec<ProjectGroup<ActiveProjectItem>>,
    pub(super) archived: Vec<ProjectGroup<SessionRailItem>>,
}

pub(super) fn recent_archived_sessions(
    groups: &[ProjectGroup<SessionRailItem>],
    limit: usize,
) -> Vec<SessionRailItem> {
    let mut sessions = groups
        .iter()
        .flat_map(|group| group.items.iter().cloned())
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .session
            .modified
            .cmp(&left.session.modified)
            .then_with(|| left.session.id.cmp(&right.session.id))
    });
    sessions.truncate(limit);
    sessions
}

pub(super) fn session_rail_groups(
    sessions: &[SessionSummary],
    drafts: &[DraftSession],
    order: &[String],
    project_filter: Option<&Path>,
    _active_projects: &HashSet<PathBuf>,
) -> SessionRailGroups {
    let rank = order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut roots = root_sessions(sessions);
    roots.sort_by(|left, right| {
        rank.get(left.id.as_str())
            .cmp(&rank.get(right.id.as_str()))
            .then_with(|| right.timestamp.cmp(&left.timestamp))
    });

    let mut active = Vec::new();
    for draft in drafts
        .iter()
        .filter(|draft| project_filter.is_none_or(|filter| filter == draft.project))
    {
        push_grouped(
            &mut active,
            draft.project.clone(),
            ActiveProjectItem::Draft(draft.clone()),
        );
    }

    let mut archived = Vec::new();
    for session in roots
        .into_iter()
        .filter(|session| project_filter.is_none_or(|filter| filter == session.project))
    {
        if session.settled {
            push_grouped(
                &mut archived,
                session.project.clone(),
                SessionRailItem {
                    session: session.clone(),
                    kind: SessionRailKind::Settled,
                },
            );
        } else {
            push_grouped(
                &mut active,
                session.project.clone(),
                ActiveProjectItem::Session(SessionRailItem {
                    session: session.clone(),
                    kind: SessionRailKind::Project,
                }),
            );
        }
    }

    SessionRailGroups { active, archived }
}

pub(in crate::app) fn session_move_allowed(
    sessions: &[SessionSummary],
    source: &str,
    target: &str,
) -> bool {
    let project_for = |id: &str| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.project.as_path())
    };
    matches!(
        (project_for(source), project_for(target)),
        (Some(source), Some(target)) if source == target
    )
}

fn push_grouped<T>(groups: &mut Vec<ProjectGroup<T>>, project: PathBuf, item: T) {
    if let Some(group) = groups.iter_mut().find(|group| group.project == project) {
        group.items.push(item);
    } else {
        groups.push(ProjectGroup {
            project,
            items: vec![item],
        });
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    use crate::sessions::UsageSummary;

    #[test]
    fn groups_active_drafts_and_sessions_once_per_project() {
        let alpha = PathBuf::from("/alpha");
        let beta = PathBuf::from("/beta");
        let drafts = vec![DraftSession::with_id("draft".into(), alpha.clone())];
        let sessions = vec![
            session("alpha-one", &alpha, false),
            session("beta-one", &beta, false),
            session("alpha-two", &alpha, false),
        ];

        let groups = session_rail_groups(
            &sessions,
            &drafts,
            &["alpha-two".into(), "beta-one".into(), "alpha-one".into()],
            None,
            &HashSet::new(),
        );

        assert_eq!(groups.active.len(), 2);
        assert_eq!(groups.active[0].project, alpha);
        assert_eq!(groups.active[0].items.len(), 3);
        assert_eq!(groups.active[1].project, beta);
        assert_eq!(groups.active[1].items.len(), 1);
    }

    #[test]
    fn project_filter_applies_to_active_drafts_and_archived_sessions() {
        let alpha = PathBuf::from("/alpha");
        let beta = PathBuf::from("/beta");
        let drafts = vec![
            DraftSession::with_id("alpha-draft".into(), alpha.clone()),
            DraftSession::with_id("beta-draft".into(), beta.clone()),
        ];
        let sessions = vec![
            session("alpha-active", &alpha, false),
            session("alpha-archived", &alpha, true),
            session("beta-active", &beta, false),
            session("beta-archived", &beta, true),
        ];

        let groups = session_rail_groups(&sessions, &drafts, &[], Some(&beta), &HashSet::new());

        assert_eq!(groups.active.len(), 1);
        assert_eq!(groups.active[0].project, beta);
        assert_eq!(groups.active[0].items.len(), 2);
        assert_eq!(groups.archived.len(), 1);
        assert_eq!(groups.archived[0].project, beta);
        assert_eq!(groups.archived[0].items.len(), 1);
    }

    #[test]
    fn manual_order_is_preserved_inside_project_groups() {
        let project = PathBuf::from("/project");
        let sessions = vec![
            session("one", &project, false),
            session("two", &project, false),
            session("three", &project, false),
        ];

        let groups = session_rail_groups(
            &sessions,
            &[],
            &["three".into(), "one".into(), "two".into()],
            None,
            &HashSet::new(),
        );
        let ids = groups.active[0]
            .items
            .iter()
            .filter_map(|item| match item {
                ActiveProjectItem::Session(item) => Some(item.session.id.as_str()),
                ActiveProjectItem::Draft(_) => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["three", "one", "two"]);
    }

    #[test]
    fn project_group_order_does_not_change_with_runtime_status() {
        let inactive_one = PathBuf::from("/inactive-one");
        let running_one = PathBuf::from("/running-one");
        let inactive_two = PathBuf::from("/inactive-two");
        let running_two = PathBuf::from("/running-two");
        let sessions = vec![
            session("inactive-one", &inactive_one, false),
            running_session("running-one", &running_one),
            session("inactive-two", &inactive_two, false),
            running_session("running-two", &running_two),
        ];

        let active_projects = HashSet::from([running_one.clone(), running_two.clone()]);
        let groups = session_rail_groups(
            &sessions,
            &[],
            &[
                "inactive-one".into(),
                "running-one".into(),
                "inactive-two".into(),
                "running-two".into(),
            ],
            None,
            &active_projects,
        );
        let projects = groups
            .active
            .iter()
            .map(|group| group.project.as_path())
            .collect::<Vec<_>>();

        assert_eq!(
            projects,
            vec![
                inactive_one.as_path(),
                running_one.as_path(),
                inactive_two.as_path(),
                running_two.as_path(),
            ]
        );
    }

    #[test]
    fn session_drag_order_is_limited_to_one_project() {
        let alpha = PathBuf::from("/alpha");
        let beta = PathBuf::from("/beta");
        let sessions = vec![
            session("alpha-one", &alpha, false),
            session("alpha-two", &alpha, false),
            session("beta-one", &beta, false),
        ];

        assert!(session_move_allowed(&sessions, "alpha-one", "alpha-two"));
        assert!(!session_move_allowed(&sessions, "alpha-one", "beta-one"));
        assert!(!session_move_allowed(&sessions, "missing", "alpha-one"));
    }

    #[test]
    fn archived_sessions_are_grouped_separately() {
        let alpha = PathBuf::from("/alpha");
        let beta = PathBuf::from("/beta");
        let sessions = vec![
            session("alpha-one", &alpha, true),
            session("beta-one", &beta, true),
            session("alpha-two", &alpha, true),
        ];

        let groups = session_rail_groups(&sessions, &[], &[], None, &HashSet::new());

        assert!(groups.active.is_empty());
        assert_eq!(groups.archived.len(), 2);
        assert_eq!(groups.archived[0].project, alpha);
        assert_eq!(groups.archived[0].items.len(), 2);
        assert_eq!(groups.archived[1].project, beta);
    }

    fn session(id: &str, project: &Path, settled: bool) -> SessionSummary {
        session_with_running(id, project, settled, false)
    }

    fn running_session(id: &str, project: &Path) -> SessionSummary {
        session_with_running(id, project, false, true)
    }

    fn session_with_running(
        id: &str,
        project: &Path,
        settled: bool,
        is_running: bool,
    ) -> SessionSummary {
        SessionSummary::from_cached(
            id.into(),
            PathBuf::from(format!("/{id}.jsonl")),
            project.to_path_buf(),
            id.into(),
            String::new(),
            String::new(),
            None,
            if is_running {
                SystemTime::now()
            } else {
                SystemTime::UNIX_EPOCH
            },
            0,
            UsageSummary::default(),
            settled,
            is_running,
            String::new(),
        )
    }
}
