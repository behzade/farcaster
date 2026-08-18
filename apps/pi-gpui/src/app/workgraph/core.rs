//! Pure workgraph board projection independent of GPUI and SQLite.

use workgraph::contract::{Issue, IssueStatus};

use super::contract::{BoardData, BoardFilter, BoardGroup, IssueGroup, IssueRow};

pub(super) fn filter_count(data: &BoardData, filter: BoardFilter) -> usize {
    data.issues
        .iter()
        .filter(|issue| filter_matches(data, filter, issue))
        .count()
}

pub(super) fn project_groups(data: &BoardData, filter: BoardFilter) -> Vec<IssueGroup> {
    let order = [
        BoardGroup::Attention,
        BoardGroup::InProgress,
        BoardGroup::ReadyNext,
        BoardGroup::Completed,
        BoardGroup::Cancelled,
    ];
    order
        .into_iter()
        .filter_map(|group| {
            let mut rows = data
                .issues
                .iter()
                .filter(|issue| {
                    filter_matches(data, filter, issue) && group_for(data, issue) == group
                })
                .cloned()
                .map(|issue| issue_row(data, issue))
                .collect::<Vec<_>>();
            rows.sort_by_key(|row| (row.issue.priority, row.issue.created_at, row.issue.number));
            (!rows.is_empty()).then_some(IssueGroup { group, rows })
        })
        .collect()
}

fn filter_matches(data: &BoardData, filter: BoardFilter, issue: &Issue) -> bool {
    match filter {
        BoardFilter::Active => !matches!(issue.status, IssueStatus::Done | IssueStatus::Cancelled),
        BoardFilter::Open => data.ready.contains(&issue.number),
        BoardFilter::InProgress => {
            issue.status == IssueStatus::InProgress && !data.blocked.contains(&issue.number)
        }
        BoardFilter::Blocked => data.blocked.contains(&issue.number),
        BoardFilter::Done => issue.status == IssueStatus::Done,
        BoardFilter::Cancelled => issue.status == IssueStatus::Cancelled,
    }
}

fn issue_row(data: &BoardData, issue: Issue) -> IssueRow {
    IssueRow {
        status_label: if data.blocked.contains(&issue.number)
            && issue.status != IssueStatus::Blocked
        {
            "Blocked by dependency"
        } else {
            status_label(issue.status)
        },
        priority_label: if issue.priority == 0 {
            "Normal".into()
        } else {
            format!("P{}", issue.priority)
        },
        issue,
    }
}

fn group_for(data: &BoardData, issue: &Issue) -> BoardGroup {
    if data.blocked.contains(&issue.number) {
        return BoardGroup::Attention;
    }
    match issue.status {
        IssueStatus::Blocked => BoardGroup::Attention,
        IssueStatus::InProgress => BoardGroup::InProgress,
        IssueStatus::Open if data.ready.contains(&issue.number) => BoardGroup::ReadyNext,
        IssueStatus::Open => BoardGroup::Attention,
        IssueStatus::Done => BoardGroup::Completed,
        IssueStatus::Cancelled => BoardGroup::Cancelled,
    }
}

pub(super) const fn status_label(status: IssueStatus) -> &'static str {
    match status {
        IssueStatus::Open => "Ready",
        IssueStatus::InProgress => "In progress",
        IssueStatus::Blocked => "Blocked",
        IssueStatus::Done => "Done",
        IssueStatus::Cancelled => "Cancelled",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn issue(number: u64, status: IssueStatus, priority: u64) -> Issue {
        Issue {
            project: "/project".into(),
            number,
            title: format!("Issue {number}"),
            body: String::new(),
            status,
            priority,
            version: 1,
            created_at: i64::try_from(number).expect("small fixture"),
            updated_at: 0,
        }
    }

    fn data(issues: Vec<Issue>, ready: &[u64], blocked: &[u64], next: Option<u64>) -> BoardData {
        BoardData {
            issues,
            dependencies: Vec::new(),
            sessions: Vec::new(),
            ready: ready.iter().copied().collect::<HashSet<_>>(),
            blocked: blocked.iter().copied().collect::<HashSet<_>>(),
            next,
        }
    }

    #[test]
    fn dependency_blocked_open_issue_is_attention_not_ready() {
        let board = data(
            vec![
                issue(1, IssueStatus::Open, 0),
                issue(2, IssueStatus::Open, 1),
                issue(3, IssueStatus::InProgress, 2),
            ],
            &[1],
            &[2, 3],
            Some(1),
        );
        let groups = project_groups(&board, BoardFilter::Active);
        assert_eq!(
            groups.iter().map(|group| group.group).collect::<Vec<_>>(),
            vec![BoardGroup::Attention, BoardGroup::ReadyNext]
        );
        assert_eq!(
            groups[0]
                .rows
                .iter()
                .map(|row| row.issue.number)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(filter_count(&board, BoardFilter::Open), 1);
        assert_eq!(filter_count(&board, BoardFilter::Blocked), 2);
    }

    #[test]
    fn ready_rows_match_canonical_next_priority_order() {
        let board = data(
            vec![
                issue(1, IssueStatus::Open, 2),
                issue(2, IssueStatus::Open, 0),
                issue(3, IssueStatus::Open, 1),
                issue(4, IssueStatus::Done, 0),
            ],
            &[1, 2, 3],
            &[],
            Some(2),
        );
        let groups = project_groups(&board, BoardFilter::Open);
        let rows = &groups[0].rows;
        assert_eq!(
            rows.iter().map(|row| row.issue.number).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
        assert_eq!(rows.first().map(|row| row.issue.number), board.next);
    }

    #[test]
    fn closed_filters_keep_their_own_empty_and_group_semantics() {
        let board = data(
            vec![
                issue(1, IssueStatus::Done, 0),
                issue(2, IssueStatus::Cancelled, 0),
            ],
            &[],
            &[],
            None,
        );
        assert_eq!(
            project_groups(&board, BoardFilter::Done)[0].group,
            BoardGroup::Completed
        );
        assert_eq!(
            project_groups(&board, BoardFilter::Cancelled)[0].group,
            BoardGroup::Cancelled
        );
        assert!(project_groups(&board, BoardFilter::Active).is_empty());
    }
}
