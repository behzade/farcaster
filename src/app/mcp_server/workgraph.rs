use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};
use workgraph::{
    EditAction, EditRequest, Evidence, EvidenceKind, NodeDraft, Outcome, ProjectGraph,
    SearchRequest, SearchResult, SqliteAdapter, WorkGraph,
};

use crate::agents::CallerContext;

static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchParams {
    #[serde(default)]
    pub(super) query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PatchNode {
    pub(super) title: String,
    pub(super) acceptance: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PatchParams {
    pub(super) nodes: Vec<PatchNode>,
    pub(super) after: Option<u64>,
    pub(super) before: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct TaskParams {
    pub(super) task: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CompleteParams {
    pub(super) task: u64,
    pub(super) evidence: String,
}

fn session_identity(database: &Path, caller: &CallerContext) -> Result<(String, String), String> {
    let store = crate::app::persistence::StateStore::open_at(database)?;
    let sessions = store.cached_sessions("")?;
    let session = sessions
        .iter()
        .find(|session| {
            session.project == caller.project
                && session.harness == caller.backend
                && (session.id == caller.session || session.path == Path::new(&caller.session))
        })
        .ok_or_else(|| {
            "authenticated session is not indexed yet; retry after session discovery".to_owned()
        })?;
    if sessions.iter().any(|other| {
        other.project == session.project
            && other.id == session.id
            && (other.path != session.path || other.harness != session.harness)
    }) {
        return Err(
            "session ID is ambiguous across indexed sessions; cannot safely link workgraph".into(),
        );
    }
    Ok((
        session.id.clone(),
        session.path.to_string_lossy().into_owned(),
    ))
}

fn project_graph(database: &Path, caller: &CallerContext) -> Result<ProjectGraph, String> {
    let adapter = SqliteAdapter::open(database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let SearchResult::Project(project) = graph
        .search(&SearchRequest::Project {
            project: project_key(caller)?,
        })
        .map_err(|error| error.to_string())?
    else {
        return Err("work graph returned an unexpected search result".into());
    };
    Ok(project)
}

fn task_views(
    graph: &ProjectGraph,
    query: &str,
    identity: Option<&(String, String)>,
) -> Vec<Value> {
    let query = query.trim().to_lowercase();
    graph
        .nodes
        .iter()
        .filter(|node| {
            query.is_empty()
                || node.title.to_lowercase().contains(&query)
                || node.acceptance.to_lowercase().contains(&query)
        })
        .map(|node| {
            let state = graph.task_state(node.number);
            let completed = state.as_ref().and_then(|state| state.completion.as_ref());
            let owner = state.as_ref().and_then(|state| state.owner.as_ref());
            let predecessors: Vec<_> = graph
                .edges
                .iter()
                .filter(|edge| edge.to == node.number)
                .map(|edge| edge.from)
                .collect();
            let blockers: Vec<_> = predecessors
                .iter()
                .copied()
                .filter(|number| {
                    graph
                        .task_state(*number)
                        .is_none_or(|state| state.completion.is_none())
                })
                .collect();
            let successors: Vec<_> = graph
                .edges
                .iter()
                .filter(|edge| edge.from == node.number)
                .map(|edge| edge.to)
                .collect();
            let status = if completed.is_some() {
                "completed"
            } else if owner.is_some() {
                "claimed"
            } else if blockers.is_empty() {
                "ready"
            } else {
                "blocked"
            };
            json!({
                "task": node.number,
                "plan": node.plan_number,
                "title": node.title,
                "acceptance": node.acceptance,
                "owner": owner.map(|owner| &owner.session_id),
                "ownedByYou": owner.is_some_and(|owner| identity.is_some_and(|(id, path)| owner.session_id == *id && owner.session_path == *path)),
                "status": status,
                "blockers": blockers,
                "predecessors": predecessors,
                "successors": successors,
                "completion": completed,
            })
        })
        .collect()
}

pub(super) fn search(
    database: &Path,
    caller: &CallerContext,
    params: SearchParams,
) -> Result<Value, String> {
    let identity = session_identity(database, caller).ok();
    Ok(
        json!({ "tasks": task_views(&project_graph(database, caller)?, &params.query, identity.as_ref()) }),
    )
}

pub(super) fn patch(
    database: &Path,
    caller: &CallerContext,
    params: PatchParams,
) -> Result<Value, String> {
    edit(
        database,
        caller,
        EditAction::CreateTasks {
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
        },
    )
}

pub(super) fn claim(
    database: &Path,
    caller: &CallerContext,
    params: TaskParams,
) -> Result<Value, String> {
    let (session_id, session_path) = session_identity(database, caller)?;
    edit(
        database,
        caller,
        EditAction::ClaimTask {
            task: params.task,
            session_id,
            session_path,
        },
    )
}

pub(super) fn release(
    database: &Path,
    caller: &CallerContext,
    params: TaskParams,
) -> Result<Value, String> {
    let (session_id, _) = session_identity(database, caller)?;
    edit(
        database,
        caller,
        EditAction::ReleaseTask {
            task: params.task,
            session_id,
        },
    )
}

pub(super) fn complete(
    database: &Path,
    caller: &CallerContext,
    params: CompleteParams,
) -> Result<Value, String> {
    let (session_id, _) = session_identity(database, caller)?;
    let graph = project_graph(database, caller)?;
    let node = graph
        .nodes
        .iter()
        .find(|node| node.number == params.task)
        .ok_or_else(|| "task not found".to_owned())?;
    let evidence_kind = match node.completion {
        workgraph::CompletionRequirement::File => EvidenceKind::File,
        workgraph::CompletionRequirement::RevisionOrObservation
        | workgraph::CompletionRequirement::Observation => EvidenceKind::Observation,
    };
    let mut result = edit(
        database,
        caller,
        EditAction::CompleteTask {
            task: params.task,
            session_id,
            outcome: Outcome {
                note: params.evidence.clone(),
                evidence: Evidence {
                    kind: evidence_kind,
                    reference: params.evidence,
                },
            },
        },
    )?;
    let newly_ready: Vec<_> = result["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|task| {
            task["status"] == "ready"
                && task["predecessors"]
                    .as_array()
                    .is_some_and(|dependencies| dependencies.contains(&json!(params.task)))
        })
        .cloned()
        .collect();
    result["newlyReady"] = json!(newly_ready);
    Ok(result)
}

fn edit(database: &Path, caller: &CallerContext, action: EditAction) -> Result<Value, String> {
    let adapter = SqliteAdapter::open(database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    graph
        .edit(&EditRequest {
            project: project_key(caller)?,
            idempotency_key: operation_id()?,
            action,
        })
        .map_err(|error| error.to_string())?;
    search(
        database,
        caller,
        SearchParams {
            query: String::new(),
        },
    )
}

fn project_key(caller: &CallerContext) -> Result<String, String> {
    caller
        .project
        .canonicalize()
        .map_err(|error| format!("resolve work graph project: {error}"))?
        .into_os_string()
        .into_string()
        .map_err(|_| "work graph project path is not valid UTF-8".to_owned())
}

fn operation_id() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is unavailable".to_owned())?
        .as_nanos();
    let sequence = OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("mcp-workgraph-{nanos}-{sequence}"))
}

#[cfg(test)]
mod tests;
