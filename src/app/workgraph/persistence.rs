use std::path::PathBuf;

use workgraph::{
    CompletionRequirement, EditAction, EditRequest, EditResult, SearchRequest, SearchResult,
    SqliteAdapter, WorkGraph,
};

use super::contract::PlanData;

pub(super) fn create_plan(
    database: PathBuf,
    project: PathBuf,
    title: String,
    root_title: String,
) -> Result<(PlanData, u64), String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let result = graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-create-plan-{}", operation_id()?),
            action: EditAction::CreatePlan {
                title,
                root_title,
                files: Vec::new(),
                completion: CompletionRequirement::RevisionOrObservation,
            },
        })
        .map_err(|error| error.to_string())?;
    let EditResult::Plan(snapshot) = result else {
        return Err("work graph returned an unexpected plan result".into());
    };
    let number = snapshot.plan.root_node;
    Ok((load_plan(database, project, None)?, number))
}

pub(super) fn add_node(
    database: PathBuf,
    project: PathBuf,
    plan: u64,
    title: String,
    files: Vec<String>,
    after: Option<u64>,
    session_id: Option<String>,
) -> Result<(PlanData, u64), String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let result = graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-add-node-{}", operation_id()?),
            action: EditAction::AddNode {
                plan,
                title,
                files,
                completion: CompletionRequirement::RevisionOrObservation,
                after,
            },
        })
        .map_err(|error| error.to_string())?;
    let EditResult::Node(node) = result else {
        return Err("work graph returned an unexpected node result".into());
    };
    Ok((
        load_plan(database, project, session_id.as_deref())?,
        node.number,
    ))
}

pub(super) fn link_session(
    database: PathBuf,
    project: PathBuf,
    walk: u64,
    session_id: String,
    session_path: String,
) -> Result<PlanData, String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(&database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: format!("gui-link-walk-{walk}-{}", operation_id()?),
            action: EditAction::LinkSession {
                walk,
                session_id: session_id.clone(),
                session_path,
            },
        })
        .map_err(|error| error.to_string())?;
    load_plan(database, project, Some(&session_id))
}

pub(super) fn load_plan(
    database: PathBuf,
    project: PathBuf,
    session_id: Option<&str>,
) -> Result<PlanData, String> {
    let project_key = canonical_project(&project)?;
    let adapter = SqliteAdapter::open(database).map_err(|error| error.to_string())?;
    let mut graph = WorkGraph::new(adapter);
    let project_graph = match graph
        .search(&SearchRequest::Project {
            project: project_key.clone(),
        })
        .map_err(|error| error.to_string())?
    {
        SearchResult::Project(graph) => graph,
        _ => return Err("work graph returned an unexpected project result".into()),
    };
    let session_link = session_id
        .map(|session_id| {
            graph
                .search(&SearchRequest::Session {
                    project: project_key.clone(),
                    session_id: session_id.to_owned(),
                })
                .map_err(|error| error.to_string())
        })
        .transpose()?
        .and_then(|result| match result {
            SearchResult::Session(link) => link,
            _ => None,
        });
    let selected = session_link
        .as_ref()
        .map(|link| (link.plan_number, Some(link.walk_number)))
        .or_else(|| {
            project_graph
                .plans
                .iter()
                .max_by_key(|plan| plan.number)
                .map(|plan| {
                    let walk = project_graph
                        .walks
                        .iter()
                        .filter(|walk| walk.plan_number == plan.number)
                        .max_by_key(|walk| walk.number)
                        .map(|walk| walk.number);
                    (plan.number, walk)
                })
        });
    let snapshot = selected
        .map(|(plan, walk)| {
            graph
                .search(&SearchRequest::Plan {
                    project: project_key,
                    plan,
                    walk,
                })
                .map_err(|error| error.to_string())
        })
        .transpose()?
        .and_then(|result| match result {
            SearchResult::Plan(snapshot) => Some(snapshot),
            _ => None,
        });
    Ok(PlanData {
        plans: project_graph.plans,
        snapshot,
        session_link,
    })
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
