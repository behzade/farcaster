use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::schemars;
use serde::Deserialize;
use workgraph::{
    EditAction, EditRequest, EditResult, Evidence, EvidenceKind, NodeDraft, Outcome, SearchRequest,
    SearchResult, SqliteAdapter, WorkGraph,
};

static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SearchParams {
    /// Absolute project directory.
    pub(super) project: String,
    /// Text matched case-insensitively against node titles and acceptance conditions.
    #[serde(default)]
    pub(super) query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PatchNode {
    /// Concise task title.
    pub(super) title: String,
    /// Observable condition that proves the task is complete.
    pub(super) acceptance: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PatchParams {
    /// Absolute project directory.
    pub(super) project: String,
    /// Backend session identifier that owns this walk.
    pub(super) session_id: String,
    /// Backend session path or stable session locator.
    pub(super) session_path: String,
    /// Ordered task chain to insert.
    pub(super) nodes: Vec<PatchNode>,
    /// Existing node before the inserted chain.
    #[serde(default)]
    pub(super) after: Option<u64>,
    /// Existing node after the inserted chain.
    #[serde(default)]
    pub(super) before: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct CompleteParams {
    /// Absolute project directory.
    pub(super) project: String,
    /// Backend session identifier attached to the active walk.
    pub(super) session_id: String,
    /// Evidence that the active task's acceptance condition was met.
    pub(super) evidence: String,
    /// Successor node when the active node branches.
    #[serde(default)]
    pub(super) next: Option<u64>,
}

pub(super) fn search(database: &Path, params: SearchParams) -> Result<String, String> {
    let project = canonical_project(&params.project)?;
    let adapter = SqliteAdapter::open(database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let SearchResult::Project(project_graph) = graph
        .search(&SearchRequest::Project { project })
        .map_err(|error| error.to_string())?
    else {
        return Err("work graph returned an unexpected search result".into());
    };
    let query = params.query.trim().to_lowercase();
    let nodes = project_graph
        .nodes
        .into_iter()
        .filter(|node| {
            query.is_empty()
                || node.title.to_lowercase().contains(&query)
                || node.acceptance.to_lowercase().contains(&query)
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&nodes)
        .map_err(|error| format!("encode work graph search: {error}"))
}

pub(super) fn patch(database: &Path, params: PatchParams) -> Result<String, String> {
    let project = canonical_project(&params.project)?;
    let adapter = SqliteAdapter::open(database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let result = graph
        .edit(&EditRequest {
            project,
            idempotency_key: operation_id("mcp-patch")?,
            action: EditAction::Patch {
                nodes: params
                    .nodes
                    .into_iter()
                    .map(|node| NodeDraft {
                        title: node.title,
                        acceptance: node.acceptance,
                    })
                    .collect(),
                after: params.after,
                before: params.before,
                session_id: params.session_id,
                session_path: params.session_path,
            },
        })
        .map_err(|error| error.to_string())?;
    let EditResult::Plan(snapshot) = result else {
        return Err("work graph returned an unexpected patch result".into());
    };
    serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("encode work graph patch: {error}"))
}

pub(super) fn complete(database: &Path, params: CompleteParams) -> Result<String, String> {
    let project = canonical_project(&params.project)?;
    let adapter = SqliteAdapter::open(database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let result = graph
        .edit(&EditRequest {
            project,
            idempotency_key: operation_id("mcp-complete")?,
            action: EditAction::Complete {
                session_id: params.session_id,
                next: params.next,
                outcome: Outcome {
                    note: params.evidence.clone(),
                    evidence: Evidence {
                        kind: EvidenceKind::Observation,
                        reference: params.evidence,
                    },
                },
            },
        })
        .map_err(|error| error.to_string())?;
    let EditResult::Plan(snapshot) = result else {
        return Err("work graph returned an unexpected completion result".into());
    };
    serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("encode work graph completion: {error}"))
}

fn canonical_project(project: &str) -> Result<String, String> {
    Path::new(project)
        .canonicalize()
        .map_err(|error| format!("resolve work graph project: {error}"))?
        .into_os_string()
        .into_string()
        .map_err(|_| "work graph project path is not valid UTF-8".to_owned())
}

fn operation_id(prefix: &str) -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is unavailable".to_owned())?
        .as_nanos();
    let sequence = OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{prefix}-{nanos}-{sequence}"))
}
