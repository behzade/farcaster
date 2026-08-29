//! Stateless MCP delivery adapter for Farcaster-owned capabilities.

use std::{
    borrow::Cow,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::ProtocolVersion,
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::Deserialize;
use workgraph::{
    adapter::SqliteAdapter,
    contract::{
        EditAction, EditRequest, EditResult, Evidence, EvidenceKind, NodeDraft, Outcome,
        SearchRequest, SearchResult,
    },
    core::WorkGraph,
};

const BIND_ADDRESS: &str = "127.0.0.1:8765";
const MCP_PATH: &str = "/mcp";
static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) struct McpServer {
    _thread: JoinHandle<()>,
}

pub(crate) fn start(database: PathBuf) -> Result<McpServer, String> {
    let listener = TcpListener::bind(BIND_ADDRESS)
        .map_err(|error| format!("bind http://{BIND_ADDRESS}{MCP_PATH}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure MCP listener: {error}"))?;
    let thread = std::thread::Builder::new()
        .name("farcaster-mcp".into())
        .spawn(move || {
            if let Err(error) = serve(listener, database) {
                zlog::error!("MCP server stopped: {error}");
            }
        })
        .map_err(|error| format!("spawn MCP server: {error}"))?;
    Ok(McpServer { _thread: thread })
}

fn serve(listener: TcpListener, database: PathBuf) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("create MCP runtime: {error}"))?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener)
            .map_err(|error| format!("open MCP listener: {error}"))?;
        let config = server_config();
        let service = StreamableHttpService::new(
            move || Ok(WorkGraphMcp::new(database.clone())),
            LocalSessionManager::default().into(),
            config,
        );
        let router = axum::Router::new().nest_service(MCP_PATH, service);
        axum::serve(listener, router)
            .await
            .map_err(|error| format!("serve MCP requests: {error}"))
    })
}

fn server_config() -> StreamableHttpServerConfig {
    StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_stateless_protocol_metadata_required(true)
}

#[derive(Clone)]
struct WorkGraphMcp {
    database: PathBuf,
}

impl WorkGraphMcp {
    fn new(database: PathBuf) -> Self {
        Self { database }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SearchParams {
    /// Absolute project directory.
    project: String,
    /// Text matched case-insensitively against node titles and acceptance conditions.
    #[serde(default)]
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PatchNode {
    /// Concise task title.
    title: String,
    /// Observable condition that proves the task is complete.
    acceptance: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PatchParams {
    /// Absolute project directory.
    project: String,
    /// Backend session identifier that owns this walk.
    session_id: String,
    /// Backend session path or stable session locator.
    session_path: String,
    /// Ordered task chain to insert.
    nodes: Vec<PatchNode>,
    /// Existing node before the inserted chain.
    #[serde(default)]
    after: Option<u64>,
    /// Existing node after the inserted chain.
    #[serde(default)]
    before: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct CompleteParams {
    /// Absolute project directory.
    project: String,
    /// Backend session identifier attached to the active walk.
    session_id: String,
    /// Evidence that the active task's acceptance condition was met.
    evidence: String,
    /// Successor node when the active node branches.
    #[serde(default)]
    next: Option<u64>,
}

#[tool_router]
impl WorkGraphMcp {
    #[tool(
        name = "workgraph_search",
        description = "Search Farcaster's durable work graph by node title or acceptance condition"
    )]
    async fn search(&self, Parameters(params): Parameters<SearchParams>) -> Result<String, String> {
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || search_workgraph(&database, params))
            .await
            .map_err(|error| format!("work graph search task failed: {error}"))?
    }

    #[tool(
        name = "workgraph_patch",
        description = "Create or extend an ordered task chain in Farcaster's durable work graph"
    )]
    async fn patch(&self, Parameters(params): Parameters<PatchParams>) -> Result<String, String> {
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || patch_workgraph(&database, params))
            .await
            .map_err(|error| format!("work graph patch task failed: {error}"))?
    }

    #[tool(
        name = "workgraph_complete",
        description = "Complete the active work-graph node for a backend session"
    )]
    async fn complete(
        &self,
        Parameters(params): Parameters<CompleteParams>,
    ) -> Result<String, String> {
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || complete_workgraph(&database, params))
            .await
            .map_err(|error| format!("work graph completion task failed: {error}"))?
    }
}

#[tool_handler(name = "farcaster", version = "0.1.0")]
impl ServerHandler for WorkGraphMcp {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28])
    }
}

fn search_workgraph(database: &Path, params: SearchParams) -> Result<String, String> {
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

fn patch_workgraph(database: &Path, params: PatchParams) -> Result<String, String> {
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

fn complete_workgraph(database: &Path, params: CompleteParams) -> Result<String, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_only_workgraph_tools() {
        let tools = WorkGraphMcp::tool_router().list_all();
        let names = tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["workgraph_complete", "workgraph_patch", "workgraph_search"]
        );
    }

    #[test]
    fn accepts_only_modern_stateless_requests() {
        let server = WorkGraphMcp::new(PathBuf::from("unused"));
        assert_eq!(
            server.supported_protocol_versions().as_ref(),
            [ProtocolVersion::V_2026_07_28]
        );
        let config = server_config();
        assert!(!config.legacy_session_mode);
        assert!(config.stateless_protocol_metadata_required);
        assert!(config.json_response);
    }

    #[test]
    fn patch_search_and_complete_share_the_application_database() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let project = temp.path().join("project");
        std::fs::create_dir(&project).map_err(|error| error.to_string())?;
        let database = temp.path().join("state.sqlite3");
        let project = project.to_string_lossy().into_owned();

        let patched = patch_workgraph(
            &database,
            PatchParams {
                project: project.clone(),
                session_id: "session-1".into(),
                session_path: "backend://session-1".into(),
                nodes: vec![
                    PatchNode {
                        title: "Add MCP server".into(),
                        acceptance: "Server answers modern MCP requests".into(),
                    },
                    PatchNode {
                        title: "Verify client".into(),
                        acceptance: "Client can call every exposed tool".into(),
                    },
                ],
                after: None,
                before: None,
            },
        )?;
        assert!(patched.contains("Add MCP server"));

        let found = search_workgraph(
            &database,
            SearchParams {
                project: project.clone(),
                query: "modern mcp".into(),
            },
        )?;
        assert!(found.contains("Add MCP server"));
        assert!(!found.contains("Verify client"));

        let completed = complete_workgraph(
            &database,
            CompleteParams {
                project,
                session_id: "session-1".into(),
                evidence: "MCP contract test passed".into(),
                next: None,
            },
        )?;
        assert!(completed.contains("MCP contract test passed"));
        assert!(completed.contains("Verify client"));
        Ok(())
    }
}
