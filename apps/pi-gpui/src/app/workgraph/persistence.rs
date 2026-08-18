use std::path::PathBuf;

use workgraph::{
    adapter::SqliteAdapter,
    contract::{EditAction, EditRequest, EditResult, SearchRequest, SearchResult},
    core::WorkGraph,
};

use super::contract::BoardData;

pub(super) fn create_issue(
    database: PathBuf,
    project: PathBuf,
    title: String,
    body: String,
) -> Result<(BoardData, u64), String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let operation = operation_id()?;
    let result = graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-create-{operation}"),
            action: EditAction::Create {
                title,
                body,
                priority: 0,
            },
        })
        .map_err(|error| error.to_string())?;
    let EditResult::Issue(issue) = result else {
        return Err("work graph returned an unexpected create result".into());
    };
    Ok((load_issues(database, project)?, issue.number))
}

pub(super) fn update_issue_fields(
    database: PathBuf,
    project: PathBuf,
    number: u64,
    title: String,
    body: String,
    priority: u64,
    expected_version: u64,
) -> Result<BoardData, String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-fields-{number}-{}", operation_id()?),
            action: EditAction::SetFields {
                number,
                title: Some(title),
                body: Some(body),
                priority: Some(priority),
                expected_version: Some(expected_version),
            },
        })
        .map_err(|error| error.to_string())?;
    load_issues(database, project)
}

pub(super) fn add_dependency(
    database: PathBuf,
    project: PathBuf,
    number: u64,
    depends_on: u64,
    expected_version: u64,
) -> Result<BoardData, String> {
    change_dependency(
        database,
        project,
        number,
        depends_on,
        expected_version,
        true,
    )
}

pub(super) fn remove_dependency(
    database: PathBuf,
    project: PathBuf,
    number: u64,
    depends_on: u64,
    expected_version: u64,
) -> Result<BoardData, String> {
    change_dependency(
        database,
        project,
        number,
        depends_on,
        expected_version,
        false,
    )
}

fn change_dependency(
    database: PathBuf,
    project: PathBuf,
    number: u64,
    depends_on: u64,
    expected_version: u64,
    add: bool,
) -> Result<BoardData, String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let action = if add {
        EditAction::AddDependency {
            number,
            depends_on,
            expected_version: Some(expected_version),
        }
    } else {
        EditAction::RemoveDependency {
            number,
            depends_on,
            expected_version: Some(expected_version),
        }
    };
    graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-dependency-{number}-{depends_on}-{}", operation_id()?),
            action,
        })
        .map_err(|error| error.to_string())?;
    load_issues(database, project)
}

pub(super) fn add_issue_note(
    database: PathBuf,
    project: PathBuf,
    number: u64,
    expected_version: u64,
    body: String,
) -> Result<BoardData, String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-note-{number}-{}", operation_id()?),
            action: EditAction::AddNote {
                number,
                body,
                expected_version: Some(expected_version),
            },
        })
        .map_err(|error| error.to_string())?;
    load_issues(database, project)
}

pub(super) fn update_issue_status(
    database: PathBuf,
    project: PathBuf,
    number: u64,
    status: workgraph::contract::IssueStatus,
    expected_version: u64,
) -> Result<BoardData, String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-status-{number}-{}", operation_id()?),
            action: EditAction::SetStatus {
                number,
                status,
                expected_version: Some(expected_version),
            },
        })
        .map_err(|error| error.to_string())?;
    load_issues(database, project)
}

pub(super) fn link_session(
    database: PathBuf,
    project: PathBuf,
    number: u64,
    session_id: String,
    session_path: String,
) -> Result<BoardData, String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-link-{number}-{}", operation_id()?),
            action: EditAction::LinkSession {
                number,
                session_id,
                session_path,
                expected_version: None,
            },
        })
        .map_err(|error| error.to_string())?;
    load_issues(database, project)
}

fn operation_id() -> Result<u128, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system clock is unavailable".to_owned())
        .map(|duration| duration.as_nanos())
}

fn canonical_project(project: &std::path::Path) -> Result<String, String> {
    project
        .canonicalize()
        .map_err(|error| format!("resolve work graph project: {error}"))?
        .into_os_string()
        .into_string()
        .map_err(|_| "work graph project path is not valid UTF-8".to_owned())
}

pub(super) fn load_issues(database: PathBuf, project: PathBuf) -> Result<BoardData, String> {
    let project = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    match graph
        .search(&SearchRequest::Graph { project })
        .map_err(|error| error.to_string())?
    {
        SearchResult::Graph(graph) => Ok(BoardData {
            issues: graph.issues,
            dependencies: graph.dependencies,
            notes: graph.notes,
            sessions: graph.sessions,
            ready: graph.ready.into_iter().collect(),
            blocked: graph.blocked.into_iter().collect(),
            next: graph.next,
        }),
        _ => Err("work graph returned an unexpected graph result".into()),
    }
}
