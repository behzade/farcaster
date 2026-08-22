use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionRequirement {
    #[default]
    RevisionOrObservation,
    File,
    Observation,
}

impl CompletionRequirement {
    pub const fn accepts(self, evidence: EvidenceKind) -> bool {
        match self {
            Self::RevisionOrObservation => {
                matches!(evidence, EvidenceKind::Revision | EvidenceKind::Observation)
            }
            Self::File => matches!(evidence, EvidenceKind::File),
            Self::Observation => matches!(evidence, EvidenceKind::Observation),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Revision,
    File,
    Observation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub reference: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub note: String,
    pub evidence: Evidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub project: String,
    pub number: u64,
    pub title: String,
    pub root_node: u64,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub plan_number: u64,
    pub number: u64,
    pub title: String,
    pub files: Vec<String>,
    pub completion: CompletionRequirement,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub plan_number: u64,
    pub from: u64,
    pub to: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Walk {
    pub plan_number: u64,
    pub number: u64,
    pub current_node: Option<u64>,
    pub head_step: Option<u64>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkStep {
    pub id: u64,
    pub walk_number: u64,
    pub node_number: u64,
    pub parent_step: Option<u64>,
    pub outcome: Outcome,
    pub completed_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLink {
    pub session_id: String,
    pub session_path: String,
    pub plan_number: u64,
    pub walk_number: u64,
    pub linked_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSnapshot {
    pub plan: Plan,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub walk: Option<Walk>,
    pub steps: Vec<WalkStep>,
    pub sessions: Vec<SessionLink>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGraph {
    pub plans: Vec<Plan>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub walks: Vec<Walk>,
    pub steps: Vec<WalkStep>,
    pub sessions: Vec<SessionLink>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredProject {
    pub graph: ProjectGraph,
    pub next_plan_number: u64,
    pub next_node_number: u64,
    pub next_walk_number: u64,
    pub next_step_id: u64,
}

impl StoredProject {
    pub fn new() -> Self {
        Self {
            next_plan_number: 1,
            next_node_number: 1,
            next_walk_number: 1,
            next_step_id: 1,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum SearchRequest {
    Project {
        project: String,
    },
    Plan {
        project: String,
        plan: u64,
        walk: Option<u64>,
    },
    Node {
        project: String,
        plan: u64,
        number: u64,
    },
    Session {
        project: String,
        session_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "view", content = "data", rename_all = "snake_case")]
pub enum SearchResult {
    Project(ProjectGraph),
    Plan(PlanSnapshot),
    Node(Node),
    Session(Option<SessionLink>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EditAction {
    CreatePlan {
        title: String,
        root_title: String,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default)]
        completion: CompletionRequirement,
    },
    AddNode {
        plan: u64,
        title: String,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default)]
        completion: CompletionRequirement,
        after: Option<u64>,
    },
    SetNode {
        plan: u64,
        number: u64,
        title: Option<String>,
        files: Option<Vec<String>>,
        completion: Option<CompletionRequirement>,
        expected_version: Option<u64>,
    },
    AddEdge {
        plan: u64,
        from: u64,
        to: u64,
    },
    RemoveEdge {
        plan: u64,
        from: u64,
        to: u64,
    },
    CreateWalk {
        plan: u64,
    },
    Advance {
        walk: u64,
        number: u64,
        next: Option<u64>,
        outcome: Outcome,
        expected_version: Option<u64>,
    },
    Rewind {
        walk: u64,
        number: u64,
        expected_version: Option<u64>,
    },
    LinkSession {
        walk: u64,
        session_id: String,
        session_path: String,
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
    Plan(PlanSnapshot),
    Node(Node),
    Edge(Edge),
    RemovedEdge(Edge),
    Walk(Walk),
    Step(WalkStep),
    Session(SessionLink),
    UnlinkedSession(SessionLink),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdempotencyReceipt {
    pub fingerprint: String,
    pub result: EditResult,
}
