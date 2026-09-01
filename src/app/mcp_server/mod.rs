//! Stateless MCP delivery adapter for Farcaster-owned capabilities.

mod workers;
mod workgraph;

use std::{borrow::Cow, net::TcpListener, path::PathBuf, thread::JoinHandle};

use rmcp::{
    ServerHandler,
    handler::server::{
        tool::Extension,
        wrapper::{Json, Parameters},
    },
    model::ProtocolVersion,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};

const BIND_ADDRESS: &str = "127.0.0.1:8765";
const MCP_PATH: &str = "/mcp";
const CALLER_HEADER: &str = "farcaster-caller";

pub(crate) struct McpServer {
    _thread: JoinHandle<()>,
}

pub(crate) fn start(
    database: PathBuf,
    worker_pool: crate::agents::WorkerPool,
    workgraph_updates: async_channel::Sender<()>,
) -> Result<McpServer, String> {
    let listener = TcpListener::bind(BIND_ADDRESS)
        .map_err(|error| format!("bind http://{BIND_ADDRESS}{MCP_PATH}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure MCP listener: {error}"))?;
    let thread = std::thread::Builder::new()
        .name("farcaster-mcp".into())
        .spawn(move || {
            if let Err(error) = serve(listener, database, worker_pool, workgraph_updates) {
                zlog::error!("MCP server stopped: {error}");
            }
        })
        .map_err(|error| format!("spawn MCP server: {error}"))?;
    Ok(McpServer { _thread: thread })
}

fn serve(
    listener: TcpListener,
    database: PathBuf,
    worker_pool: crate::agents::WorkerPool,
    workgraph_updates: async_channel::Sender<()>,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("create MCP runtime: {error}"))?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener)
            .map_err(|error| format!("open MCP listener: {error}"))?;
        let config = server_config();
        let service = StreamableHttpService::new(
            move || {
                Ok(FarcasterMcp::new(
                    database.clone(),
                    worker_pool.clone(),
                    workgraph_updates.clone(),
                ))
            },
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
struct FarcasterMcp {
    database: PathBuf,
    workers: crate::agents::WorkerPool,
    workgraph_updates: async_channel::Sender<()>,
}

impl FarcasterMcp {
    fn new(
        database: PathBuf,
        workers: crate::agents::WorkerPool,
        workgraph_updates: async_channel::Sender<()>,
    ) -> Self {
        Self {
            database,
            workers,
            workgraph_updates,
        }
    }
}

#[tool_router]
impl FarcasterMcp {
    #[tool(
        name = "worker_send",
        description = "Send a message to an existing top-level peer. Set `to` to `new` to create a fresh top-level agent using your current harness and model. New workers are independent, visible Farcaster sessions—not subagents—and should only be created for substantial independent work. Use the harness's native subagent facilities for delegated subtasks."
    )]
    async fn worker_send(
        &self,
        Parameters(params): Parameters<workers::SendParams>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<serde_json::Value>, String> {
        let caller_token = caller_token(&parts);
        let pool = self.workers.clone();
        let value = tokio::task::spawn_blocking(move || workers::send(&pool, params, caller_token))
            .await
            .map_err(|error| format!("worker send task failed: {error}"))??;
        Ok(Json(value))
    }

    #[tool(
        name = "worker_list",
        description = "List active top-level Farcaster peers in this project"
    )]
    async fn worker_list(
        &self,
        Parameters(_params): Parameters<workers::ListParams>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<serde_json::Value>, String> {
        let caller_token = caller_token(&parts);
        let value = tokio::task::spawn_blocking(move || workers::list(caller_token))
            .await
            .map_err(|error| format!("worker list task failed: {error}"))??;
        Ok(Json(value))
    }

    #[tool(
        name = "workgraph_search",
        description = "Search Farcaster's durable work graph by node title or acceptance condition"
    )]
    async fn search(
        &self,
        Parameters(params): Parameters<workgraph::SearchParams>,
    ) -> Result<String, String> {
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || workgraph::search(&database, params))
            .await
            .map_err(|error| format!("work graph search task failed: {error}"))?
    }

    #[tool(
        name = "workgraph_patch",
        description = "Create or extend an ordered task chain in Farcaster's durable work graph"
    )]
    async fn patch(
        &self,
        Parameters(params): Parameters<workgraph::PatchParams>,
    ) -> Result<String, String> {
        let database = self.database.clone();
        let result = tokio::task::spawn_blocking(move || workgraph::patch(&database, params))
            .await
            .map_err(|error| format!("work graph patch task failed: {error}"))??;
        notify_workgraph_changed(&self.workgraph_updates);
        Ok(result)
    }

    #[tool(
        name = "workgraph_complete",
        description = "Complete the active work-graph node for a backend session"
    )]
    async fn complete(
        &self,
        Parameters(params): Parameters<workgraph::CompleteParams>,
    ) -> Result<String, String> {
        let database = self.database.clone();
        let result = tokio::task::spawn_blocking(move || workgraph::complete(&database, params))
            .await
            .map_err(|error| format!("work graph completion task failed: {error}"))??;
        notify_workgraph_changed(&self.workgraph_updates);
        Ok(result)
    }
}

fn caller_token(parts: &axum::http::request::Parts) -> Option<String> {
    parts
        .headers
        .get(CALLER_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn notify_workgraph_changed(updates: &async_channel::Sender<()>) {
    let _ = updates.try_send(());
}

#[tool_handler(
    name = "farcaster",
    version = "0.1.0",
    instructions = "Farcaster provides communication between top-level peer workers and durable work graphs. Use worker_send with `to: new` only for substantial independent work; use the harness's native subagents for delegated subtasks."
)]
impl ServerHandler for FarcasterMcp {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28])
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    fn worker_pool(project: &std::path::Path) -> crate::agents::WorkerPool {
        let (factories, default_backend) =
            crate::agents::worker_factories(crate::agents::AgentLaunchConfig::default());
        crate::agents::WorkerPool::new(factories, default_backend, project.to_owned(), 4)
            .expect("worker pool")
    }

    #[test]
    fn exposes_only_farcaster_tools() {
        let tools = FarcasterMcp::tool_router().list_all();
        let names = tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "worker_list",
                "worker_send",
                "workgraph_complete",
                "workgraph_patch",
                "workgraph_search"
            ]
        );
        assert!(
            tools
                .iter()
                .filter(|tool| tool.name.starts_with("worker_"))
                .all(|tool| tool.output_schema.is_some())
        );
    }

    #[test]
    fn accepts_only_modern_stateless_requests() {
        let temp = tempfile::tempdir().expect("temp directory");
        let project = temp.path().join("project");
        std::fs::create_dir(&project).expect("project directory");
        let (workgraph_updates, _) = async_channel::bounded(1);
        let server = FarcasterMcp::new(
            PathBuf::from("unused"),
            worker_pool(&project),
            workgraph_updates,
        );
        assert_eq!(
            server.supported_protocol_versions().as_ref(),
            [ProtocolVersion::V_2026_07_28]
        );
        let instructions = server.get_info().instructions.expect("server instructions");
        assert!(instructions.contains("top-level peer workers"));
        assert!(instructions.contains("worker_send"));
        assert!(instructions.contains("native subagents"));
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

        let patched = workgraph::patch(
            &database,
            workgraph::PatchParams {
                project: project.clone(),
                session_id: "session-1".into(),
                session_path: "backend://session-1".into(),
                nodes: vec![
                    workgraph::PatchNode {
                        title: "Add MCP server".into(),
                        acceptance: "Server answers modern MCP requests".into(),
                    },
                    workgraph::PatchNode {
                        title: "Verify client".into(),
                        acceptance: "Client can call every exposed tool".into(),
                    },
                ],
                after: None,
                before: None,
            },
        )?;
        assert!(patched.contains("Add MCP server"));

        let found = workgraph::search(
            &database,
            workgraph::SearchParams {
                project: project.clone(),
                query: "modern mcp".into(),
            },
        )?;
        assert!(found.contains("Add MCP server"));
        assert!(!found.contains("Verify client"));

        let completed = workgraph::complete(
            &database,
            workgraph::CompleteParams {
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
