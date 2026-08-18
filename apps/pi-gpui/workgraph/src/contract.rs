use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Open,
    InProgress,
    Blocked,
    Done,
    Cancelled,
}

impl IssueStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "in_progress" => Some(Self::InProgress),
            "blocked" => Some(Self::Blocked),
            "done" => Some(Self::Done),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub project: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub status: IssueStatus,
    pub priority: u64,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: i64,
    pub issue_number: u64,
    pub body: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub issue: Issue,
    pub dependencies: Vec<u64>,
    pub dependents: Vec<u64>,
    pub notes: Vec<Note>,
    pub sessions: Vec<SessionLink>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub issue_number: u64,
    pub depends_on: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLink {
    pub session_id: String,
    pub session_path: String,
    pub issue_number: u64,
    pub linked_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGraph {
    pub issues: Vec<Issue>,
    pub dependencies: Vec<Dependency>,
    pub notes: Vec<Note>,
    pub sessions: Vec<SessionLink>,
    pub ready: Vec<u64>,
    pub blocked: Vec<u64>,
    pub next: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningView {
    Ready,
    Blocked,
    Next,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum SearchRequest {
    Status {
        project: String,
        status: Option<IssueStatus>,
    },
    Issue {
        project: String,
        number: u64,
    },
    Planning {
        project: String,
        planning: PlanningView,
    },
    Graph {
        project: String,
    },
    Session {
        project: String,
        session_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "view", content = "data", rename_all = "snake_case")]
pub enum SearchResult {
    Status(Vec<Issue>),
    Issue(IssueDetail),
    Planning(Vec<Issue>),
    Graph(ProjectGraph),
    Session(Option<SessionLink>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EditAction {
    Create {
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        priority: u64,
    },
    SetStatus {
        number: u64,
        status: IssueStatus,
        expected_version: Option<u64>,
    },
    AddNote {
        number: u64,
        body: String,
        expected_version: Option<u64>,
    },
    AddDependency {
        number: u64,
        depends_on: u64,
        expected_version: Option<u64>,
    },
    RemoveDependency {
        number: u64,
        depends_on: u64,
        expected_version: Option<u64>,
    },
    LinkSession {
        number: u64,
        session_id: String,
        session_path: String,
        expected_version: Option<u64>,
    },
    UnlinkSession {
        session_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditRequest {
    pub project: String,
    pub idempotency_key: String,
    pub action: EditAction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum EditResult {
    Issue(Issue),
    Note(Note),
    Session(SessionLink),
    UnlinkedSession(SessionLink),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectRecordId(i64);

impl ProjectRecordId {
    pub const fn from_storage(value: i64) -> Self {
        Self(value)
    }

    pub const fn as_storage(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionedUpdate {
    Updated,
    Missing,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdempotencyReceipt {
    pub fingerprint: String,
    pub result: EditResult,
}
