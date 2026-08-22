//! Private companion-extension RPC edge for the durable work graph.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};
use workgraph::{
    adapter::SqliteAdapter,
    contract::{
        CompletionRequirement, EditAction, EditRequest, Evidence, EvidenceKind, Outcome,
        SearchRequest,
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
    #[error("workgraph output could not be encoded")]
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

pub(crate) fn response(
    request: &crate::protocol::ExtensionUiRequest,
    database: &Path,
) -> Option<WorkGraphRpcResponse> {
    let (id, payload) = request.workgraph_rpc()?;
    let is_edit = serde_json::from_str::<BridgeRequest>(payload)
        .is_ok_and(|request| request.operation == "edit");
    let result = handle(payload, database);
    let changed = is_edit && result.is_ok();
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
    let project = canonical_project(&request.project)?;
    let adapter = SqliteAdapter::open(database).map_err(WorkGraphError::Persistence)?;
    let mut graph = WorkGraph::new(adapter);
    let value = match request.operation.as_str() {
        "search" => {
            serde_json::to_value(graph.search(&search_request(project, &request.fields)?)?)?
        }
        "edit" => serde_json::to_value(graph.edit(&edit_request(project, &request.fields)?)?)?,
        _ => return Err(WorkGraphRpcError::Invalid("operation")),
    };
    serde_json::to_string(&Output {
        success: true,
        data: value,
    })
    .map_err(Into::into)
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

fn search_request(
    project: String,
    fields: &BTreeMap<String, String>,
) -> Result<SearchRequest, WorkGraphRpcError> {
    reject_unknown(fields, &["view", "plan", "walk", "number", "sessionId"])?;
    match fields.get("view").map(String::as_str).unwrap_or("project") {
        "project" => Ok(SearchRequest::Project { project }),
        "plan" => Ok(SearchRequest::Plan {
            project,
            plan: number(fields, "plan")?,
            walk: optional_number(fields, "walk")?,
        }),
        "node" => Ok(SearchRequest::Node {
            project,
            plan: number(fields, "plan")?,
            number: number(fields, "number")?,
        }),
        "session" => Ok(SearchRequest::Session {
            project,
            session_id: required(fields, "sessionId")?.to_owned(),
        }),
        _ => Err(WorkGraphRpcError::Invalid("view")),
    }
}

fn edit_request(
    project: String,
    fields: &BTreeMap<String, String>,
) -> Result<EditRequest, WorkGraphRpcError> {
    reject_unknown(
        fields,
        &[
            "action",
            "title",
            "rootTitle",
            "files",
            "completion",
            "plan",
            "walk",
            "number",
            "after",
            "from",
            "to",
            "next",
            "note",
            "evidenceKind",
            "evidence",
            "expectedVersion",
            "idempotencyKey",
            "sessionId",
            "sessionPath",
        ],
    )?;
    let action = match required(fields, "action")? {
        "create_plan" => EditAction::CreatePlan {
            title: required(fields, "title")?.to_owned(),
            root_title: required(fields, "rootTitle")?.to_owned(),
            files: string_list(fields, "files")?,
            completion: optional_completion(fields)?.unwrap_or_default(),
        },
        "add_node" => EditAction::AddNode {
            plan: number(fields, "plan")?,
            title: required(fields, "title")?.to_owned(),
            files: string_list(fields, "files")?,
            completion: optional_completion(fields)?.unwrap_or_default(),
            after: optional_number(fields, "after")?,
        },
        "set_node" => EditAction::SetNode {
            plan: number(fields, "plan")?,
            number: number(fields, "number")?,
            title: fields.get("title").cloned(),
            files: fields
                .contains_key("files")
                .then(|| string_list(fields, "files"))
                .transpose()?,
            completion: optional_completion(fields)?,
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "add_edge" => EditAction::AddEdge {
            plan: number(fields, "plan")?,
            from: number(fields, "from")?,
            to: number(fields, "to")?,
        },
        "remove_edge" => EditAction::RemoveEdge {
            plan: number(fields, "plan")?,
            from: number(fields, "from")?,
            to: number(fields, "to")?,
        },
        "create_walk" => EditAction::CreateWalk {
            plan: number(fields, "plan")?,
        },
        "advance" => EditAction::Advance {
            walk: number(fields, "walk")?,
            number: number(fields, "number")?,
            next: optional_number(fields, "next")?,
            outcome: Outcome {
                note: required(fields, "note")?.to_owned(),
                evidence: Evidence {
                    kind: parse_evidence(required(fields, "evidenceKind")?)?,
                    reference: required(fields, "evidence")?.to_owned(),
                },
            },
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "rewind" => EditAction::Rewind {
            walk: number(fields, "walk")?,
            number: number(fields, "number")?,
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "link_session" => EditAction::LinkSession {
            walk: number(fields, "walk")?,
            session_id: required(fields, "sessionId")?.to_owned(),
            session_path: required(fields, "sessionPath")?.to_owned(),
        },
        "unlink_session" => EditAction::UnlinkSession {
            session_id: required(fields, "sessionId")?.to_owned(),
        },
        _ => return Err(WorkGraphRpcError::Invalid("action")),
    };
    Ok(EditRequest {
        project,
        idempotency_key: required(fields, "idempotencyKey")?.to_owned(),
        action,
    })
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

fn number(fields: &BTreeMap<String, String>, key: &'static str) -> Result<u64, WorkGraphRpcError> {
    optional_number(fields, key)?.ok_or(WorkGraphRpcError::Missing(key))
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

fn string_list(
    fields: &BTreeMap<String, String>,
    key: &'static str,
) -> Result<Vec<String>, WorkGraphRpcError> {
    fields
        .get(key)
        .map(|value| serde_json::from_str(value).map_err(|_| WorkGraphRpcError::Invalid(key)))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn optional_completion(
    fields: &BTreeMap<String, String>,
) -> Result<Option<CompletionRequirement>, WorkGraphRpcError> {
    fields
        .get("completion")
        .map(|value| match value.as_str() {
            "revision_or_observation" => Ok(CompletionRequirement::RevisionOrObservation),
            "file" => Ok(CompletionRequirement::File),
            "observation" => Ok(CompletionRequirement::Observation),
            _ => Err(WorkGraphRpcError::Invalid("completion")),
        })
        .transpose()
}

fn parse_evidence(value: &str) -> Result<EvidenceKind, WorkGraphRpcError> {
    match value {
        "revision" => Ok(EvidenceKind::Revision),
        "file" => Ok(EvidenceKind::File),
        "observation" => Ok(EvidenceKind::Observation),
        _ => Err(WorkGraphRpcError::Invalid("evidenceKind")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(operation: &str, project: &Path, fields: serde_json::Value) -> String {
        serde_json::json!({
            "operation": operation,
            "project": project,
            "fields": fields,
        })
        .to_string()
    }

    #[test]
    fn companion_rpc_creates_and_reads_a_plan() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project = tempfile::tempdir().expect("project");
        let database = directory.path().join("gui-state.sqlite3");
        let output = handle(
            &request(
                "edit",
                project.path(),
                serde_json::json!({
                    "action": "create_plan",
                    "title": "Git and jj integration",
                    "rootTitle": "Current product",
                    "idempotencyKey": "create-1",
                }),
            ),
            &database,
        )
        .expect("create");
        assert!(output.contains("Git and jj integration"));
        handle(
            &request(
                "edit",
                project.path(),
                serde_json::json!({
                    "action": "add_node",
                    "plan": "1",
                    "title": "Both backends work",
                    "after": "1",
                    "idempotencyKey": "node-1",
                }),
            ),
            &database,
        )
        .expect("add node");
        handle(
            &request(
                "edit",
                project.path(),
                serde_json::json!({
                    "action": "advance",
                    "walk": "1",
                    "number": "1",
                    "note": "Current product state verified",
                    "evidenceKind": "observation",
                    "evidence": "apps/pi-gpui at git:abc123",
                    "expectedVersion": "1",
                    "idempotencyKey": "advance-1",
                }),
            ),
            &database,
        )
        .expect("advance");
        let output = handle(
            &request(
                "search",
                project.path(),
                serde_json::json!({ "view": "plan", "plan": "1", "walk": "1" }),
            ),
            &database,
        )
        .expect("search");
        assert!(output.contains("Current product state verified"));
        assert!(output.contains("\"currentNode\":2"));
    }

    #[test]
    fn successful_searches_do_not_report_a_workgraph_change() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project = tempfile::tempdir().expect("project");
        let payload = request(
            "search",
            project.path(),
            serde_json::json!({ "view": "project" }),
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
    fn unknown_rpc_fields_are_rejected() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project = tempfile::tempdir().expect("project");
        let result = handle(
            &request("search", project.path(), serde_json::json!({ "wat": "no" })),
            &directory.path().join("state.sqlite"),
        );
        assert!(matches!(result, Err(WorkGraphRpcError::Field(field)) if field == "wat"));
    }
}
