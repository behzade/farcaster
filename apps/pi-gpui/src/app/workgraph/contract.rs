//! UI-owned workgraph view contract.

use std::collections::HashSet;

use workgraph::contract::Issue;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum BoardFilter {
    #[default]
    Active,
    Open,
    InProgress,
    Blocked,
    Done,
    Cancelled,
}

impl BoardFilter {
    pub(super) const ALL: [Self; 6] = [
        Self::Active,
        Self::Open,
        Self::InProgress,
        Self::Blocked,
        Self::Done,
        Self::Cancelled,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Active => "All active",
            Self::Open => "Ready",
            Self::InProgress => "In progress",
            Self::Blocked => "Needs attention",
            Self::Done => "Done",
            Self::Cancelled => "Cancelled",
        }
    }

    pub(super) const fn empty_message(self) -> &'static str {
        match self {
            Self::Active => "No active issues.",
            Self::Open => "No issues are ready.",
            Self::InProgress => "No issues are in progress.",
            Self::Blocked => "No issues need attention.",
            Self::Done => "No completed issues.",
            Self::Cancelled => "No cancelled issues.",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BoardGroup {
    Attention,
    InProgress,
    ReadyNext,
    Completed,
    Cancelled,
}

impl BoardGroup {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Attention => "Needs attention",
            Self::InProgress => "In progress",
            Self::ReadyNext => "Ready next",
            Self::Completed => "Completed",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IssueRow {
    pub issue: Issue,
    pub status_label: &'static str,
    pub priority_label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IssueGroup {
    pub group: BoardGroup,
    pub rows: Vec<IssueRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BoardLoadState {
    Loading,
    Ready(BoardData),
    Failed(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct BoardData {
    pub issues: Vec<Issue>,
    pub ready: HashSet<u64>,
    pub blocked: HashSet<u64>,
    pub next: Option<u64>,
}
