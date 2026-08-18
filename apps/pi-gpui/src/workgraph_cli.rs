//! Same-binary command edge for the durable work graph.

use std::{collections::BTreeMap, ffi::OsString, path::Path};

use serde::Serialize;
use workgraph::{
    adapter::SqliteAdapter,
    contract::{EditAction, EditRequest, IssueStatus, PlanningView, SearchRequest},
    core::{WorkGraph, WorkGraphError},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    #[error("usage: pi-gpui workgraph search|edit key=value...")]
    Usage,
    #[error("unknown or duplicate workgraph field: {0}")]
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

pub(crate) fn run(
    arguments: impl IntoIterator<Item = OsString>,
    database: &Path,
) -> Result<String, CliError> {
    let mut arguments = arguments.into_iter();
    let operation = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or(CliError::Usage)?;
    let fields = parse_fields(arguments)?;
    let project = canonical_project(fields.get("project").map(String::as_str))?;
    let adapter = SqliteAdapter::open(database).map_err(WorkGraphError::Persistence)?;
    let mut graph = WorkGraph::new(adapter);
    let value = match operation.as_str() {
        "search" => serde_json::to_value(graph.search(&search_request(project, &fields)?)?)?,
        "edit" => serde_json::to_value(graph.edit(&edit_request(project, &fields)?)?)?,
        _ => return Err(CliError::Usage),
    };
    Ok(format!(
        "{}\n",
        serde_json::to_string(&Output {
            success: true,
            data: value
        })?
    ))
}

fn parse_fields(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<BTreeMap<String, String>, CliError> {
    let mut fields = BTreeMap::new();
    for argument in arguments {
        let argument = argument
            .into_string()
            .map_err(|_| CliError::Invalid("argument"))?;
        let (key, value) = argument
            .split_once('=')
            .ok_or(CliError::Invalid("argument"))?;
        if key.is_empty() || fields.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(CliError::Field(key.to_owned()));
        }
    }
    Ok(fields)
}

fn canonical_project(configured: Option<&str>) -> Result<String, CliError> {
    let path = configured
        .map_or_else(std::env::current_dir, |value| Ok(value.into()))
        .map_err(|_| CliError::Invalid("project"))?;
    path.canonicalize()
        .map_err(|_| CliError::Invalid("project"))
        .and_then(|path| {
            path.into_os_string()
                .into_string()
                .map_err(|_| CliError::Invalid("project"))
        })
}

fn search_request(
    project: String,
    fields: &BTreeMap<String, String>,
) -> Result<SearchRequest, CliError> {
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
        _ => Err(CliError::Invalid("view")),
    }
}

fn edit_request(
    project: String,
    fields: &BTreeMap<String, String>,
) -> Result<EditRequest, CliError> {
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
        _ => return Err(CliError::Invalid("action")),
    };
    let idempotency_key = required(fields, "idempotencyKey")?.to_owned();
    Ok(EditRequest {
        project,
        idempotency_key,
        action,
    })
}

fn reject_unknown(fields: &BTreeMap<String, String>, allowed: &[&str]) -> Result<(), CliError> {
    if let Some(key) = fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(CliError::Field(key.clone()));
    }
    Ok(())
}

fn required<'a>(
    fields: &'a BTreeMap<String, String>,
    key: &'static str,
) -> Result<&'a str, CliError> {
    fields
        .get(key)
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .ok_or(CliError::Missing(key))
}

fn number(fields: &BTreeMap<String, String>, key: &'static str) -> Result<u64, CliError> {
    optional_number(fields, key)?.ok_or(CliError::Missing(key))
}

fn optional_number(
    fields: &BTreeMap<String, String>,
    key: &'static str,
) -> Result<Option<u64>, CliError> {
    fields
        .get(key)
        .map(|value| value.parse().map_err(|_| CliError::Invalid(key)))
        .transpose()
}

fn parse_status(value: &str) -> Result<IssueStatus, CliError> {
    match value {
        "open" => Ok(IssueStatus::Open),
        "in_progress" => Ok(IssueStatus::InProgress),
        "blocked" => Ok(IssueStatus::Blocked),
        "done" => Ok(IssueStatus::Done),
        "cancelled" => Ok(IssueStatus::Cancelled),
        _ => Err(CliError::Invalid("status")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_database_cli_creates_and_reads_an_issue() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project = tempfile::tempdir().expect("project");
        let database = directory.path().join("gui-state.sqlite3");
        let project_field = format!("project={}", project.path().display());
        let output = run(
            [
                "edit".into(),
                project_field.clone().into(),
                "action=create".into(),
                "title=Merge graph".into(),
                "idempotencyKey=create-1".into(),
            ],
            &database,
        )
        .expect("create");
        assert!(output.contains("Merge graph"));
        let output = run(
            ["search".into(), project_field.into(), "view=ready".into()],
            &database,
        )
        .expect("search");
        assert!(output.contains("Merge graph"));
        let _gui_state = crate::state::StateStore::open_at(&database)
            .expect("GUI state opens after work graph migration");
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project = tempfile::tempdir().expect("project");
        let result = run(
            [
                "search".into(),
                format!("project={}", project.path().display()).into(),
                "wat=no".into(),
            ],
            &directory.path().join("state.sqlite"),
        );
        assert!(matches!(result, Err(CliError::Field(field)) if field == "wat"));
    }
}
