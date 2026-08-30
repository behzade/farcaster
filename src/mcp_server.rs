//! Stateless MCP delivery adapter for Farcaster-owned capabilities.

mod workers;
mod workgraph;

use std::{borrow::Cow, net::TcpListener, path::PathBuf, thread::JoinHandle};

use rmcp::{
    ServerHandler,
    handler::server::wrapper::{Json, Parameters},
    model::ProtocolVersion,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};

const BIND_ADDRESS: &str = "127.0.0.1:8765";
const MCP_PATH: &str = "/mcp";

pub(crate) struct McpServer {
    _thread: JoinHandle<()>,
}

pub(crate) fn start(
    database: PathBuf,
    approvals: crate::sandbox::approval::ApprovalService,
    worker_pool: crate::workers::WorkerPool,
) -> Result<McpServer, String> {
    let listener = TcpListener::bind(BIND_ADDRESS)
        .map_err(|error| format!("bind http://{BIND_ADDRESS}{MCP_PATH}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure MCP listener: {error}"))?;
    let thread = std::thread::Builder::new()
        .name("farcaster-mcp".into())
        .spawn(move || {
            if let Err(error) = serve(listener, database, approvals, worker_pool) {
                zlog::error!("MCP server stopped: {error}");
            }
        })
        .map_err(|error| format!("spawn MCP server: {error}"))?;
    Ok(McpServer { _thread: thread })
}

fn serve(
    listener: TcpListener,
    database: PathBuf,
    approvals: crate::sandbox::approval::ApprovalService,
    worker_pool: crate::workers::WorkerPool,
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
                    approvals.clone(),
                    worker_pool.clone(),
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
    approvals: crate::sandbox::approval::ApprovalService,
    workers: crate::workers::WorkerPool,
}

impl FarcasterMcp {
    fn new(
        database: PathBuf,
        approvals: crate::sandbox::approval::ApprovalService,
        workers: crate::workers::WorkerPool,
    ) -> Self {
        Self {
            database,
            approvals,
            workers,
        }
    }
}

#[tool_router]
impl FarcasterMcp {
    #[tool(
        name = "request_access",
        description = "Ask the user to grant exact filesystem or network rights to Farcaster's outer sandbox. The grant activates after the current agent turn ends; never retry in the same turn"
    )]
    async fn request_access(
        &self,
        Parameters(params): Parameters<crate::sandbox::approval::RequestAccessParams>,
    ) -> Result<String, String> {
        self.approvals.request_access(params).await
    }

    #[tool(
        name = "worker_backends",
        description = "List worker backends available from this Farcaster instance"
    )]
    async fn worker_backends(
        &self,
        Parameters(_params): Parameters<workers::ListParams>,
    ) -> Json<serde_json::Value> {
        Json(workers::backends(&self.workers))
    }

    #[tool(
        name = "worker_start",
        description = "Start an independent worker in Farcaster's bounded backend-neutral pool"
    )]
    async fn worker_start(
        &self,
        Parameters(params): Parameters<workers::StartParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let pool = self.workers.clone();
        let value = tokio::task::spawn_blocking(move || workers::start(&pool, params))
            .await
            .map_err(|error| format!("worker start task failed: {error}"))??;
        Ok(Json(value))
    }

    #[tool(
        name = "worker_send",
        description = "Prompt or steer an existing Farcaster worker"
    )]
    async fn worker_send(
        &self,
        Parameters(params): Parameters<workers::SendParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let pool = self.workers.clone();
        let value = tokio::task::spawn_blocking(move || workers::send(&pool, params))
            .await
            .map_err(|error| format!("worker send task failed: {error}"))??;
        Ok(Json(value))
    }

    #[tool(
        name = "worker_respond",
        description = "Answer or cancel an interactive request from a Farcaster worker"
    )]
    async fn worker_respond(
        &self,
        Parameters(params): Parameters<workers::RespondParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let pool = self.workers.clone();
        let value = tokio::task::spawn_blocking(move || workers::respond(&pool, params))
            .await
            .map_err(|error| format!("worker response task failed: {error}"))??;
        Ok(Json(value))
    }

    #[tool(name = "worker_list", description = "List Farcaster workers")]
    async fn worker_list(
        &self,
        Parameters(_params): Parameters<workers::ListParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let pool = self.workers.clone();
        let value = tokio::task::spawn_blocking(move || workers::list(&pool))
            .await
            .map_err(|error| format!("worker list task failed: {error}"))??;
        Ok(Json(value))
    }

    #[tool(name = "worker_status", description = "Inspect one Farcaster worker")]
    async fn worker_status(
        &self,
        Parameters(params): Parameters<workers::WorkerParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let pool = self.workers.clone();
        let value = tokio::task::spawn_blocking(move || workers::status(&pool, params))
            .await
            .map_err(|error| format!("worker status task failed: {error}"))??;
        Ok(Json(value))
    }

    #[tool(name = "worker_stop", description = "Stop one Farcaster worker")]
    async fn worker_stop(
        &self,
        Parameters(params): Parameters<workers::WorkerParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let pool = self.workers.clone();
        let value = tokio::task::spawn_blocking(move || workers::stop(&pool, params))
            .await
            .map_err(|error| format!("worker stop task failed: {error}"))??;
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
        tokio::task::spawn_blocking(move || workgraph::patch(&database, params))
            .await
            .map_err(|error| format!("work graph patch task failed: {error}"))?
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
        tokio::task::spawn_blocking(move || workgraph::complete(&database, params))
            .await
            .map_err(|error| format!("work graph completion task failed: {error}"))?
    }
}

#[tool_handler(name = "farcaster", version = "0.1.0")]
impl ServerHandler for FarcasterMcp {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28])
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use super::*;

    fn worker_pool(project: &std::path::Path) -> crate::workers::WorkerPool {
        let factory: Arc<dyn crate::workers::WorkerSessionFactory> = Arc::new(
            crate::agents::PiWorkerFactory::new(crate::agents::PiProcessCommand::default()),
        );
        crate::workers::WorkerPool::new(
            BTreeMap::from([("pi".into(), factory)]),
            "pi".into(),
            project.to_owned(),
            4,
        )
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
                "request_access",
                "worker_backends",
                "worker_list",
                "worker_respond",
                "worker_send",
                "worker_start",
                "worker_status",
                "worker_stop",
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
        let home = temp.path().join("home");
        std::fs::create_dir(&project).expect("project directory");
        std::fs::create_dir_all(home.join(".pi/agent")).expect("agent state");
        let temporary = temp.path().join("tmp");
        std::fs::create_dir(&temporary).expect("temporary directory");
        let (approvals, _) = crate::sandbox::approval::channel(
            &project,
            &home,
            temp.path(),
            &home.join(".pi/agent"),
            &temporary,
            crate::sandbox::test_nono_bypass(),
        )
        .expect("approval channel");
        let server = FarcasterMcp::new(PathBuf::from("unused"), approvals, worker_pool(&project));
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
