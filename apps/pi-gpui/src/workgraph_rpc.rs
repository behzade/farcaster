//! Private companion-extension RPC edge for the durable work graph.

use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use serde::{Deserialize, Serialize};
use workgraph::{
    adapter::SqliteAdapter,
    contract::{
        EditAction, EditRequest, EditResult, Evidence, EvidenceKind, NodeDraft, Outcome,
        PlanSnapshot, SearchRequest, SearchResult,
    },
    core::{WorkGraph, WorkGraphError},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum WorkGraphRpcError {
    #[error("unknown workgraph field: {0}")]
    Field(String),
    #[error("workgraph field is missing: {0}")]
    Missing(&'static str),
    #[error("workgraph field is invalid: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Graph(#[from] WorkGraphError),
    #[error("workgraph data could not be encoded")]
    Encode(#[from] serde_json::Error),
}

pub(crate) struct WorkGraphRpcResponse {
    pub(crate) response: crate::protocol::ExtensionUiResponse,
    pub(crate) changed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Output<T> {
    success: bool,
    data: T,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeRequest {
    operation: String,
    project: String,
    #[serde(default)]
    fields: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphOutput {
    root: u64,
    active: Option<u64>,
    nodes: Vec<NodeOutput>,
    edges: Vec<EdgeOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matches: Option<Vec<u64>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeOutput {
    number: u64,
    title: String,
    acceptance: String,
    state: &'static str,
}

#[derive(Serialize)]
struct EdgeOutput {
    from: u64,
    to: u64,
}

pub(crate) fn response(
    request: &crate::protocol::ExtensionUiRequest,
    database: &Path,
) -> Option<WorkGraphRpcResponse> {
    let (id, payload) = request.workgraph_rpc()?;
    let changed = serde_json::from_str::<BridgeRequest>(payload).is_ok_and(|request| {
        request.operation == "workgraph"
            && request
                .fields
                .get("action")
                .is_some_and(|action| action != "search")
    });
    let result = handle(payload, database);
    let changed = changed && result.is_ok();
    let value = result.unwrap_or_else(|error| {
        serde_json::json!({
            "success": false,
            "error": error.to_string(),
        })
        .to_string()
    });
    Some(WorkGraphRpcResponse {
        response: crate::protocol::ExtensionUiResponse::Value {
            id: id.to_owned(),
            value,
        },
        changed,
    })
}

fn handle(payload: &str, database: &Path) -> Result<String, WorkGraphRpcError> {
    let request = serde_json::from_str::<BridgeRequest>(payload)?;
    if request.operation != "workgraph" {
        return Err(WorkGraphRpcError::Invalid("operation"));
    }
    let action = required(&request.fields, "action")?;
    reject_action_fields(action, &request.fields)?;
    let project = canonical_project(&request.project)?;
    let adapter = SqliteAdapter::open(database).map_err(WorkGraphError::Persistence)?;
    let mut graph = WorkGraph::new(adapter);
    let value = match action {
        "search" => {
            let snapshot = attached_snapshot(
                &mut graph,
                &project,
                required(&request.fields, "sessionId")?,
            )?;
            serde_json::to_value(snapshot.map(|snapshot| {
                graph_output(&snapshot, request.fields.get("query").map(String::as_str))
            }))?
        }
        "patch" => {
            let nodes = node_list(&request.fields)?;
            let result = graph.edit(&EditRequest {
                project,
                idempotency_key: required(&request.fields, "idempotencyKey")?.to_owned(),
                action: EditAction::Patch {
                    nodes,
                    after: optional_number(&request.fields, "after")?,
                    before: optional_number(&request.fields, "before")?,
                    session_id: required(&request.fields, "sessionId")?.to_owned(),
                    session_path: required(&request.fields, "sessionPath")?.to_owned(),
                },
            })?;
            let EditResult::Plan(snapshot) = result else {
                return Err(WorkGraphRpcError::Invalid("patch result"));
            };
            serde_json::to_value(graph_output(&snapshot, None))?
        }
        "complete" => {
            let evidence = required(&request.fields, "evidence")?.to_owned();
            let result = graph.edit(&EditRequest {
                project,
                idempotency_key: required(&request.fields, "idempotencyKey")?.to_owned(),
                action: EditAction::Complete {
                    session_id: required(&request.fields, "sessionId")?.to_owned(),
                    next: optional_number(&request.fields, "next")?,
                    outcome: Outcome {
                        note: evidence.clone(),
                        evidence: Evidence {
                            kind: EvidenceKind::Observation,
                            reference: evidence,
                        },
                    },
                },
            })?;
            let EditResult::Plan(snapshot) = result else {
                return Err(WorkGraphRpcError::Invalid("complete result"));
            };
            serde_json::to_value(graph_output(&snapshot, None))?
        }
        _ => return Err(WorkGraphRpcError::Invalid("action")),
    };
    serde_json::to_string(&Output {
        success: true,
        data: value,
    })
    .map_err(Into::into)
}

fn attached_snapshot(
    graph: &mut WorkGraph<SqliteAdapter>,
    project: &str,
    session_id: &str,
) -> Result<Option<PlanSnapshot>, WorkGraphRpcError> {
    let link = match graph.search(&SearchRequest::Session {
        project: project.to_owned(),
        session_id: session_id.to_owned(),
    })? {
        SearchResult::Session(link) => link,
        _ => None,
    };
    let Some(link) = link else {
        return Ok(None);
    };
    match graph.search(&SearchRequest::Plan {
        project: project.to_owned(),
        plan: link.plan_number,
        walk: Some(link.walk_number),
    })? {
        SearchResult::Plan(snapshot) => Ok(Some(snapshot)),
        _ => Err(WorkGraphRpcError::Invalid("search result")),
    }
}

fn graph_output(snapshot: &PlanSnapshot, query: Option<&str>) -> GraphOutput {
    let completed = active_node_numbers(snapshot);
    let active = snapshot.walk.as_ref().and_then(|walk| walk.current_node);
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let matches = query.map(|query| {
        let query = query.to_lowercase();
        snapshot
            .nodes
            .iter()
            .filter(|node| {
                node.title.to_lowercase().contains(&query)
                    || node.acceptance.to_lowercase().contains(&query)
            })
            .map(|node| node.number)
            .collect()
    });
    GraphOutput {
        root: snapshot.plan.root_node,
        active,
        nodes: snapshot
            .nodes
            .iter()
            .map(|node| NodeOutput {
                number: node.number,
                title: node.title.clone(),
                acceptance: if node.acceptance.is_empty() {
                    "Acceptance was not recorded for this legacy node.".into()
                } else {
                    node.acceptance.clone()
                },
                state: if completed.contains(&node.number) {
                    "completed"
                } else if active == Some(node.number) {
                    "active"
                } else {
                    "pending"
                },
            })
            .collect(),
        edges: snapshot
            .edges
            .iter()
            .map(|edge| EdgeOutput {
                from: edge.from,
                to: edge.to,
            })
            .collect(),
        matches,
    }
}

fn active_node_numbers(snapshot: &PlanSnapshot) -> HashSet<u64> {
    let mut result = HashSet::new();
    let mut current = snapshot.walk.as_ref().and_then(|walk| walk.head_step);
    while let Some(id) = current {
        let Some(step) = snapshot.steps.iter().find(|step| step.id == id) else {
            break;
        };
        result.insert(step.node_number);
        current = step.parent_step;
    }
    result
}

fn canonical_project(configured: &str) -> Result<String, WorkGraphRpcError> {
    let path = std::path::PathBuf::from(configured);
    path.canonicalize()
        .map_err(|_| WorkGraphRpcError::Invalid("project"))
        .and_then(|path| {
            path.into_os_string()
                .into_string()
                .map_err(|_| WorkGraphRpcError::Invalid("project"))
        })
}

fn reject_action_fields(
    action: &str,
    fields: &BTreeMap<String, String>,
) -> Result<(), WorkGraphRpcError> {
    let allowed: &[&str] = match action {
        "search" => &["action", "query", "sessionId"],
        "patch" => &[
            "action",
            "nodes",
            "after",
            "before",
            "idempotencyKey",
            "sessionId",
            "sessionPath",
        ],
        "complete" => &["action", "evidence", "next", "idempotencyKey", "sessionId"],
        _ => return Err(WorkGraphRpcError::Invalid("action")),
    };
    reject_unknown(fields, allowed)
}

fn reject_unknown(
    fields: &BTreeMap<String, String>,
    allowed: &[&str],
) -> Result<(), WorkGraphRpcError> {
    if let Some(key) = fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(WorkGraphRpcError::Field(key.clone()));
    }
    Ok(())
}

fn required<'a>(
    fields: &'a BTreeMap<String, String>,
    key: &'static str,
) -> Result<&'a str, WorkGraphRpcError> {
    fields
        .get(key)
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .ok_or(WorkGraphRpcError::Missing(key))
}

fn optional_number(
    fields: &BTreeMap<String, String>,
    key: &'static str,
) -> Result<Option<u64>, WorkGraphRpcError> {
    fields
        .get(key)
        .map(|value| value.parse().map_err(|_| WorkGraphRpcError::Invalid(key)))
        .transpose()
}

fn node_list(fields: &BTreeMap<String, String>) -> Result<Vec<NodeDraft>, WorkGraphRpcError> {
    serde_json::from_str(required(fields, "nodes")?).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(project: &Path, fields: serde_json::Value) -> String {
        serde_json::json!({
            "operation": "workgraph",
            "project": project,
            "fields": fields,
        })
        .to_string()
    }

    #[test]
    fn companion_rpc_patches_searches_and_completes_the_attached_graph() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project = tempfile::tempdir().expect("project");
        let database = directory.path().join("gui-state.sqlite3");
        let nodes = serde_json::json!([
            {"title": "Issue", "acceptance": "Problem is understood"},
            {"title": "Outcome", "acceptance": "Behavior is fixed"}
        ]);
        let output = handle(
            &request(
                project.path(),
                serde_json::json!({
                    "action": "patch",
                    "nodes": nodes.to_string(),
                    "sessionId": "session-1",
                    "sessionPath": "/sessions/one.jsonl",
                    "idempotencyKey": "patch-1",
                }),
            ),
            &database,
        )
        .expect("patch");
        let output: serde_json::Value = serde_json::from_str(&output).expect("patch output");
        assert_eq!(output["data"]["active"], 1);
        assert_eq!(output["data"]["nodes"][0]["acceptance"], "Problem is understood");

        let output = handle(
            &request(
                project.path(),
                serde_json::json!({
                    "action": "search",
                    "query": "fixed",
                    "sessionId": "session-1",
                }),
            ),
            &database,
        )
        .expect("search");
        let output: serde_json::Value = serde_json::from_str(&output).expect("search output");
        assert_eq!(output["data"]["matches"], serde_json::json!([2]));

        let output = handle(
            &request(
                project.path(),
                serde_json::json!({
                    "action": "complete",
                    "evidence": "Focused regression test passed",
                    "sessionId": "session-1",
                    "idempotencyKey": "complete-1",
                }),
            ),
            &database,
        )
        .expect("complete");
        let output: serde_json::Value = serde_json::from_str(&output).expect("complete output");
        assert_eq!(output["data"]["active"], 2);
        assert_eq!(output["data"]["nodes"][0]["state"], "completed");
    }

    #[test]
    fn successful_searches_do_not_report_a_workgraph_change() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project = tempfile::tempdir().expect("project");
        let payload = request(
            project.path(),
            serde_json::json!({"action": "search", "sessionId": "session-1"}),
        );
        let request = crate::protocol::ExtensionUiRequest::Input {
            id: "bridge-search".into(),
            title: crate::protocol::WORKGRAPH_RPC_TITLE.into(),
            placeholder: Some(payload),
            timeout: None,
        };
        let response =
            response(&request, &directory.path().join("state.sqlite")).expect("bridge response");
        assert!(!response.changed);
    }

    #[test]
    fn fields_for_another_action_are_rejected() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project = tempfile::tempdir().expect("project");
        let result = handle(
            &request(
                project.path(),
                serde_json::json!({
                    "action": "search",
                    "sessionId": "one",
                    "nodes": "[]"
                }),
            ),
            &directory.path().join("state.sqlite"),
        );
        assert!(matches!(result, Err(WorkGraphRpcError::Field(field)) if field == "nodes"));
    }

}
