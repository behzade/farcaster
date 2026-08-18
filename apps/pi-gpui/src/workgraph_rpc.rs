//! Private companion-extension RPC edge for the durable work graph.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};
use workgraph::{
    adapter::SqliteAdapter,
    contract::{EditAction, EditRequest, IssueStatus, PlanningView, SearchRequest},
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

pub(crate) fn handle(payload: &str, database: &Path) -> Result<String, WorkGraphRpcError> {
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
    reject_unknown(
        fields,
        &["project", "view", "status", "number", "sessionId"],
    )?;
    match fields.get("view").map(String::as_str).unwrap_or("status") {
        "status" => Ok(SearchRequest::Status {
            project,
            status: fields
                .get("status")
                .map(|value| parse_status(value))
                .transpose()?,
        }),
        "issue" => Ok(SearchRequest::Issue {
            project,
            number: number(fields, "number")?,
        }),
        "ready" => Ok(SearchRequest::Planning {
            project,
            planning: PlanningView::Ready,
        }),
        "blocked" => Ok(SearchRequest::Planning {
            project,
            planning: PlanningView::Blocked,
        }),
        "next" => Ok(SearchRequest::Planning {
            project,
            planning: PlanningView::Next,
        }),
        "graph" => Ok(SearchRequest::Graph { project }),
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
            "project",
            "action",
            "title",
            "body",
            "priority",
            "number",
            "status",
            "dependsOn",
            "expectedVersion",
            "idempotencyKey",
            "sessionId",
            "sessionPath",
        ],
    )?;
    let action = match required(fields, "action")? {
        "create" => EditAction::Create {
            title: required(fields, "title")?.to_owned(),
            body: fields.get("body").cloned().unwrap_or_default(),
            priority: optional_number(fields, "priority")?.unwrap_or(0),
        },
        "set_fields" => EditAction::SetFields {
            number: number(fields, "number")?,
            title: fields.get("title").cloned(),
            body: fields.get("body").cloned(),
            priority: optional_number(fields, "priority")?,
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "set_priority" => EditAction::SetPriority {
            number: number(fields, "number")?,
            priority: number(fields, "priority")?,
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "set_status" => EditAction::SetStatus {
            number: number(fields, "number")?,
            status: parse_status(required(fields, "status")?)?,
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "add_note" => EditAction::AddNote {
            number: number(fields, "number")?,
            body: required(fields, "body")?.to_owned(),
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "add_dependency" => EditAction::AddDependency {
            number: number(fields, "number")?,
            depends_on: number(fields, "dependsOn")?,
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "remove_dependency" => EditAction::RemoveDependency {
            number: number(fields, "number")?,
            depends_on: number(fields, "dependsOn")?,
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "link_session" => EditAction::LinkSession {
            number: number(fields, "number")?,
            session_id: required(fields, "sessionId")?.to_owned(),
            session_path: required(fields, "sessionPath")?.to_owned(),
            expected_version: optional_number(fields, "expectedVersion")?,
        },
        "unlink_session" => EditAction::UnlinkSession {
            session_id: required(fields, "sessionId")?.to_owned(),
        },
        _ => return Err(WorkGraphRpcError::Invalid("action")),
    };
    let idempotency_key = required(fields, "idempotencyKey")?.to_owned();
    Ok(EditRequest {
        project,
        idempotency_key,
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

fn parse_status(value: &str) -> Result<IssueStatus, WorkGraphRpcError> {
    match value {
        "open" => Ok(IssueStatus::Open),
        "in_progress" => Ok(IssueStatus::InProgress),
        "blocked" => Ok(IssueStatus::Blocked),
        "done" => Ok(IssueStatus::Done),
        "cancelled" => Ok(IssueStatus::Cancelled),
        _ => Err(WorkGraphRpcError::Invalid("status")),
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
    fn companion_rpc_creates_edits_and_reads_an_issue() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project = tempfile::tempdir().expect("project");
        let database = directory.path().join("gui-state.sqlite3");
        let output = handle(
            &request(
                "edit",
                project.path(),
                serde_json::json!({
                    "action": "create",
                    "title": "Merge graph",
                    "idempotencyKey": "create-1",
                }),
            ),
            &database,
        )
        .expect("create");
        assert!(output.contains("Merge graph"));
        let output = handle(
            &request(
                "edit",
                project.path(),
                serde_json::json!({
                    "action": "set_fields",
                    "number": "1",
                    "title": "Merge durable graph",
                    "priority": "2",
                    "expectedVersion": "1",
                    "idempotencyKey": "fields-1",
                }),
            ),
            &database,
        )
        .expect("set fields");
        assert!(output.contains("Merge durable graph"));
        let output = handle(
            &request(
                "search",
                project.path(),
                serde_json::json!({ "view": "ready" }),
            ),
            &database,
        )
        .expect("search");
        assert!(output.contains("Merge durable graph"));
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
