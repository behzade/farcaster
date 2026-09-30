use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};
use workgraph::{
    EditAction, EditRequest, EditResult, Evidence, EvidenceKind, Node, NodeDraft, Outcome,
    ProjectGraph, SearchRequest, SearchResult, SqliteAdapter, WorkGraph,
};

use crate::agents::CallerContext;

static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

const DEFAULT_SEARCH_LIMIT: usize = 20;
const MAX_SEARCH_LIMIT: usize = 100;

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum SearchStatus {
    #[default]
    Active,
    All,
    Ready,
    Claimed,
    Blocked,
    Completed,
}

impl SearchStatus {
    fn matches(&self, status: &str) -> bool {
        match self {
            Self::Active => status != "completed",
            Self::All => true,
            Self::Ready => status == "ready",
            Self::Claimed => status == "claimed",
            Self::Blocked => status == "blocked",
            Self::Completed => status == "completed",
        }
    }
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchParams {
    #[serde(default)]
    pub(super) query: String,
    #[schemars(
        description = "Get full details for one task, including completed tasks. Cannot combine with query or after."
    )]
    pub(super) task: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "List filter; defaults to active. Use all or completed for history. Ignored for task lookup."
    )]
    pub(super) status: SearchStatus,
    #[schemars(description = "List page size, default 20, maximum 100.")]
    pub(super) limit: Option<usize>,
    #[schemars(
        description = "Continue a list after the task number returned in nextAfter. Keep the same query and status."
    )]
    pub(super) after: Option<u64>,
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

fn session_identity_store(
    store: &crate::storage::StateStore,
    caller: &CallerContext,
) -> Result<(String, String), String> {
    let session = store.resolve_caller_session(caller)?;
    Ok((
        crate::sessions::AppSessionId::new(session.app_session_id)
            .ok_or_else(|| "authenticated session has no application identity".to_owned())?
            .to_key(),
        session.path.to_string_lossy().into_owned(),
    ))
}

fn project_graph(
    store: &mut crate::storage::StateStore,
    caller: &CallerContext,
) -> Result<ProjectGraph, String> {
    store.with_connection(|connection| {
        let adapter = SqliteAdapter::borrow(connection);
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
    })
}

fn task_view(
    graph: &ProjectGraph,
    node: &Node,
    identity: Option<&(String, String)>,
    details: bool,
) -> Value {
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
    let status = if completed.is_some() {
        "completed"
    } else if owner.is_some() {
        "claimed"
    } else if blockers.is_empty() {
        "ready"
    } else {
        "blocked"
    };
    let mut view = json!({
        "task": node.number,
        "plan": node.plan_number,
        "title": node.title,
        "owner": owner.map(|owner| &owner.session_id),
        "ownedByYou": owner.is_some_and(|owner| identity.is_some_and(|(id, _)| owner.session_id == *id)),
        "status": status,
        "blockers": blockers,
    });
    if details {
        let successors: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.from == node.number)
            .map(|edge| edge.to)
            .collect();
        let mut completion = json!(completed);
        if let Some(outcome) = completion.get_mut("outcome").and_then(Value::as_object_mut)
            && outcome.get("note")
                == outcome
                    .get("evidence")
                    .and_then(|value| value.get("reference"))
        {
            outcome.remove("note");
        }
        view["acceptance"] = json!(node.acceptance);
        view["predecessors"] = json!(predecessors);
        view["successors"] = json!(successors);
        view["completion"] = completion;
    }
    view
}

fn task_views(
    graph: &ProjectGraph,
    tasks: &[u64],
    identity: Option<&(String, String)>,
) -> Vec<Value> {
    tasks
        .iter()
        .filter_map(|task| graph.nodes.iter().find(|node| node.number == *task))
        .map(|node| task_view(graph, node, identity, false))
        .collect()
}

pub(super) fn search_store(
    store: &mut crate::storage::StateStore,
    caller: &CallerContext,
    params: SearchParams,
) -> Result<Value, String> {
    let limit = params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
    if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
        return Err(format!(
            "search limit must be between 1 and {MAX_SEARCH_LIMIT}"
        ));
    }
    if params.task.is_some() && (params.after.is_some() || !params.query.trim().is_empty()) {
        return Err("task lookup cannot use query or after".into());
    }
    let identity = session_identity_store(store, caller).ok();
    let graph = project_graph(store, caller)?;
    if let Some(task) = params.task {
        let node = graph
            .nodes
            .iter()
            .find(|node| node.number == task)
            .ok_or_else(|| "task not found".to_owned())?;
        return Ok(
            json!({"tasks": [task_view(&graph, node, identity.as_ref(), true)], "nextAfter": null}),
        );
    }
    let query = params.query.trim().to_lowercase();
    let mut nodes = graph
        .nodes
        .iter()
        .filter(|node| {
            params.after.is_none_or(|after| node.number > after)
                && (query.is_empty()
                    || node.title.to_lowercase().contains(&query)
                    || node.acceptance.to_lowercase().contains(&query))
        })
        .collect::<Vec<_>>();
    nodes.sort_unstable_by_key(|node| node.number);
    let mut tasks = nodes
        .into_iter()
        .map(|node| task_view(&graph, node, identity.as_ref(), false))
        .filter(|task| {
            params
                .status
                .matches(task["status"].as_str().unwrap_or_default())
        })
        .take(limit + 1)
        .collect::<Vec<_>>();
    let next_after = if tasks.len() > limit {
        Some(tasks[limit - 1]["task"].clone())
    } else {
        None
    };
    tasks.truncate(limit);
    Ok(json!({"tasks": tasks, "nextAfter": next_after}))
}

pub(super) fn patch_store(
    store: &mut crate::storage::StateStore,
    caller: &CallerContext,
    params: PatchParams,
) -> Result<Value, String> {
    let count = params.nodes.len();
    let result = edit(
        store,
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
    )?;
    let EditResult::Tasks(snapshot) = result else {
        return Err("work graph returned an unexpected edit result".into());
    };
    // CreateTasks appends nodes with increasing numbers, including when extending a plan.
    let mut tasks = snapshot
        .nodes
        .iter()
        .map(|node| node.number)
        .collect::<Vec<_>>();
    tasks.sort_unstable();
    let created = tasks.split_off(tasks.len().saturating_sub(count));
    mutation_response(store, caller, &created)
}

pub(super) fn claim_store(
    store: &mut crate::storage::StateStore,
    caller: &CallerContext,
    params: TaskParams,
) -> Result<Value, String> {
    let (session_id, session_path) = session_identity_store(store, caller)?;
    edit(
        store,
        caller,
        EditAction::ClaimTask {
            task: params.task,
            session_id,
            session_path,
        },
    )?;
    mutation_response(store, caller, &[params.task])
}

pub(super) fn release_store(
    store: &mut crate::storage::StateStore,
    caller: &CallerContext,
    params: TaskParams,
) -> Result<Value, String> {
    let (session_id, _) = session_identity_store(store, caller)?;
    edit(
        store,
        caller,
        EditAction::ReleaseTask {
            task: params.task,
            session_id,
        },
    )?;
    mutation_response(store, caller, &[params.task])
}

pub(super) fn complete_store(
    store: &mut crate::storage::StateStore,
    caller: &CallerContext,
    params: CompleteParams,
) -> Result<Value, String> {
    let identity = session_identity_store(store, caller)?;
    let graph = project_graph(store, caller)?;
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
    edit(
        store,
        caller,
        EditAction::CompleteTask {
            task: params.task,
            session_id: identity.0.clone(),
            outcome: Outcome {
                note: params.evidence.clone(),
                evidence: Evidence {
                    kind: evidence_kind,
                    reference: params.evidence,
                },
            },
        },
    )?;
    let updated = project_graph(store, caller)?;
    let newly_ready = updated
        .nodes
        .iter()
        .filter(|node| {
            updated
                .edges
                .iter()
                .any(|edge| edge.from == params.task && edge.to == node.number)
        })
        .map(|node| task_view(&updated, node, Some(&identity), false))
        .filter(|task| task["status"] == "ready")
        .collect::<Vec<_>>();
    Ok(json!({
        "tasks": task_views(&updated, &[params.task], Some(&identity)),
        "newlyReady": newly_ready,
    }))
}

fn mutation_response(
    store: &mut crate::storage::StateStore,
    caller: &CallerContext,
    tasks: &[u64],
) -> Result<Value, String> {
    let graph = project_graph(store, caller)?;
    let identity = session_identity_store(store, caller).ok();
    Ok(json!({"tasks": task_views(&graph, tasks, identity.as_ref())}))
}

fn edit(
    store: &mut crate::storage::StateStore,
    caller: &CallerContext,
    action: EditAction,
) -> Result<EditResult, String> {
    store.with_connection(|connection| {
        let adapter = SqliteAdapter::borrow(connection);
        let mut graph = WorkGraph::new(adapter);
        graph
            .edit(&EditRequest {
                project: project_key(caller)?,
                idempotency_key: operation_id()?,
                action,
            })
            .map_err(|error| error.to_string())
    })
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
