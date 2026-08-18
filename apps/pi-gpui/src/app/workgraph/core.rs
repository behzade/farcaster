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

pub(super) fn issue_matches_board_filter(issue: &Issue, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }
    let searchable = format!("#{} {}", issue.number, issue.title);
    searchable.to_lowercase().contains(&filter.to_lowercase())
}

pub(super) fn matching_project_groups(
    data: &BoardData,
    filter: BoardFilter,
    search: &str,
) -> Vec<IssueGroup> {
    project_groups(data, filter)
        .into_iter()
        .filter_map(|mut group| {
            group
                .rows
                .retain(|row| issue_matches_board_filter(&row.issue, search));
            (!group.rows.is_empty()).then_some(group)
        })
        .collect()
}

pub(super) fn format_relative_issue_time(updated_at: i64, now: i64) -> String {
    let elapsed_seconds = now.saturating_sub(updated_at).max(0) / 1_000;
    match elapsed_seconds {
        0..=59 => "just now".to_owned(),
        60..=3_599 => format!("{}m ago", elapsed_seconds / 60),
        3_600..=86_399 => format!("{}h ago", elapsed_seconds / 3_600),
        86_400..=604_799 => format!("{}d ago", elapsed_seconds / 86_400),
        604_800..=2_629_799 => format!("{}w ago", elapsed_seconds / 604_800),
        2_629_800..=31_557_599 => format!("{}mo ago", elapsed_seconds / 2_629_800),
        _ => format!("{}y ago", elapsed_seconds / 31_557_600),
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
            notes: Vec::new(),
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

    #[test]
    fn text_filter_matches_issue_number_and_title_case_insensitively() {
        let issue = issue(42, IssueStatus::Open, 0);
        assert!(issue_matches_board_filter(&issue, "#42"));
        assert!(issue_matches_board_filter(&issue, "ISSUE 42"));
        assert!(!issue_matches_board_filter(&issue, "unrelated"));
    }

    #[test]
    fn relative_issue_time_uses_the_issues_board_boundaries() {
        let now = 1_800_000_000_000_i64;
        assert_eq!(format_relative_issue_time(now - 30_000, now), "just now");
        assert_eq!(format_relative_issue_time(now - 120_000, now), "2m ago");
        assert_eq!(format_relative_issue_time(now - 7_200_000, now), "2h ago");
        assert_eq!(format_relative_issue_time(now - 259_200_000, now), "3d ago");
        assert_eq!(format_relative_issue_time(now + 30_000, now), "just now");
    }
}
