//! Sessionless Streamable HTTP adapter for Farcaster-owned MCP capabilities.

mod lifecycle;
mod notices;
pub(crate) use lifecycle::{set_enabled, start};
mod workers;
mod workgraph;

use std::{borrow::Cow, path::PathBuf};

use rmcp::{
    ServerHandler,
    handler::server::{
        tool::Extension,
        wrapper::{Json, Parameters},
    },
    model::ProtocolVersion,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::StreamableHttpServerConfig,
};

const BIND_ADDRESS: &str = "127.0.0.1:8765";
const MCP_PATH: &str = "/mcp";
const CALLER_HEADER: &str = "farcaster-caller";

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
    notices: notices::NoticeBoard,
}

impl FarcasterMcp {
    fn new(
        database: PathBuf,
        workers: crate::agents::WorkerPool,
        workgraph_updates: async_channel::Sender<()>,
        notices: notices::NoticeBoard,
    ) -> Self {
        Self {
            database,
            workers,
            workgraph_updates,
            notices,
        }
    }

    async fn workgraph_call<P: Send + 'static>(
        &self,
        parts: axum::http::request::Parts,
        params: P,
        operation: fn(
            &std::path::Path,
            &crate::agents::CallerContext,
            P,
        ) -> Result<serde_json::Value, String>,
        mutates: bool,
    ) -> Result<Json<serde_json::Value>, String> {
        let token = caller_token(&parts)
            .ok_or_else(|| "workgraph requires a registered Farcaster caller".to_owned())?;
        let database = self.database.clone();
        let result = tokio::task::spawn_blocking(move || {
            let caller = crate::agents::CallerRegistry::shared().resolve(&token)?;
            operation(&database, &caller, params)
        })
        .await
        .map_err(|error| format!("work graph task failed: {error}"))??;
        if mutates {
            notify_workgraph_changed(&self.workgraph_updates);
        }
        Ok(Json(result))
    }
}

#[tool_router]
impl FarcasterMcp {
    #[tool(
        name = "worker_send",
        description = "Send work within your worker family. Top-level workers provide a direct child name in `to`; first use creates the child and subsequent messages reuse it. Children omit `to` and always send to their parent."
    )]
    async fn worker_send(
        &self,
        Parameters(params): Parameters<workers::SendParams>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<serde_json::Value>, String> {
        let caller_token = caller_token(&parts);
        let pool = self.workers.clone();
        let database = self.database.clone();
        let value = tokio::task::spawn_blocking(move || {
            let tasks =
                crate::app::persistence::StateStore::open_at(&database)?.load_worker_tasks()?;
            workers::send(&pool, params, caller_token, &tasks)
        })
        .await
        .map_err(|error| format!("worker send task failed: {error}"))??;
        Ok(Json(value))
    }

    #[tool(
        name = "worker_notices",
        description = "Read or post a non-intrusive project notice board for top-level worker coordination. Use only when shared-worktree changes overlap or may conflict with your work, or when change ownership is unclear. Do not use it for unrelated changes. Posting also returns matching notices."
    )]
    async fn worker_notices(
        &self,
        Parameters(params): Parameters<notices::Params>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<notices::Response>, String> {
        let token = caller_token(&parts)
            .ok_or_else(|| "worker notices require a registered Farcaster caller".to_owned())?;
        let board = self.notices.clone();
        let value = tokio::task::spawn_blocking(move || {
            let caller = crate::agents::CallerRegistry::shared().resolve(&token)?;
            board.access(&caller, params)
        })
        .await
        .map_err(|error| format!("worker notice task failed: {error}"))??;
        Ok(Json(value))
    }

    #[tool(
        name = "workgraph_search",
        description = "Find tasks in your project with owners, blockers, and readiness. Omit query to list all tasks."
    )]
    async fn search(
        &self,
        Parameters(params): Parameters<workgraph::SearchParams>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<serde_json::Value>, String> {
        self.workgraph_call(parts, params, workgraph::search, false)
            .await
    }

    #[tool(
        name = "workgraph_patch",
        description = "Create or extend an ordered task chain in your project. Creating tasks does not claim them."
    )]
    async fn patch(
        &self,
        Parameters(params): Parameters<workgraph::PatchParams>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<serde_json::Value>, String> {
        self.workgraph_call(parts, params, workgraph::patch, true)
            .await
    }

    #[tool(
        name = "workgraph_claim",
        description = "Atomically claim a ready task for your authenticated session. Conflicts if already owned by another session."
    )]
    async fn claim(
        &self,
        Parameters(params): Parameters<workgraph::TaskParams>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<serde_json::Value>, String> {
        self.workgraph_call(parts, params, workgraph::claim, true)
            .await
    }

    #[tool(
        name = "workgraph_release",
        description = "Release a task owned by your authenticated session so another session can claim it."
    )]
    async fn release(
        &self,
        Parameters(params): Parameters<workgraph::TaskParams>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<serde_json::Value>, String> {
        self.workgraph_call(parts, params, workgraph::release, true)
            .await
    }

    #[tool(
        name = "workgraph_complete",
        description = "Complete a task owned by your authenticated session with evidence. Returns newly ready tasks; does not claim them."
    )]
    async fn complete(
        &self,
        Parameters(params): Parameters<workgraph::CompleteParams>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<serde_json::Value>, String> {
        self.workgraph_call(parts, params, workgraph::complete, true)
            .await
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

fn tools_for_role(child: bool, tasks: &crate::agents::WorkerTasks) -> Vec<rmcp::model::Tool> {
    let mut tools = FarcasterMcp::tool_router().list_all();
    if child {
        tools.retain(|tool| tool.name != "worker_notices");
    }
    if let Some(tool) = tools.iter_mut().find(|tool| tool.name == "worker_send") {
        let mut schema = (*tool.input_schema).clone();
        if child {
            if let Some(properties) = schema
                .get_mut("properties")
                .and_then(serde_json::Value::as_object_mut)
            {
                properties.remove("to");
                properties.remove("task");
                properties.remove("judgment");
            }
            schema.insert("required".into(), serde_json::json!(["message"]));
            tool.description = Some(Cow::Borrowed(
                "Send a message to your parent worker. The parent is implicit; use this tool for all communication, including final results.",
            ));
        } else {
            if let Some(properties) = schema
                .get_mut("properties")
                .and_then(serde_json::Value::as_object_mut)
            {
                properties.insert("judgment".into(), serde_json::json!({
                    "type": "string", "enum": crate::agents::WorkerJudgment::ALL.map(|judgment| judgment.label()),
                    "description": "Judgment delegated: specified procedure, guided local decisions, or independent approach. Defaults to guided on creation; omit on reuse."
                }));
                properties.insert("task".into(), if tasks.tasks.is_empty() { serde_json::json!(false) } else { serde_json::json!({
                    "type": "string", "enum": tasks.tasks.iter().map(|task| task.name.as_str()).collect::<Vec<_>>(),
                    "description": "Classification of already-delegated work; required on creation, omitted on reuse."
                }) });
            }
            schema.insert("required".into(), serde_json::json!(["to", "message"]));
            tool.description = Some(Cow::Borrowed(
                "Send a message or delegated task to a named direct child. First use creates the child; later uses reuse it.",
            ));
        }
        tool.input_schema = std::sync::Arc::new(schema);
    }
    tools
}

#[tool_handler(
    name = "farcaster",
    version = "0.1.0",
    instructions = "Farcaster provides worker coordination and durable work graphs."
)]
impl ServerHandler for FarcasterMcp {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28])
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let token = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(caller_token)
            .ok_or_else(|| rmcp::ErrorData::internal_error("missing Farcaster caller", None))?;
        let child = crate::agents::CallerRegistry::shared()
            .is_child(&token)
            .map_err(|error| rmcp::ErrorData::internal_error(error, None))?;
        let tasks = crate::app::persistence::StateStore::open_at(&self.database)
            .and_then(|store| store.load_worker_tasks())
            .map_err(|error| rmcp::ErrorData::internal_error(error, None))?;
        Ok(rmcp::model::ListToolsResult {
            result_type: Some(rmcp::model::ResultType::COMPLETE),
            tools: tools_for_role(child, &tasks),
            meta: None,
            next_cursor: None,
            ttl_ms: Some(0),
            cache_scope: Some(rmcp::model::CacheScope::Private),
        })
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
                "worker_notices",
                "worker_send",
                "workgraph_claim",
                "workgraph_complete",
                "workgraph_patch",
                "workgraph_release",
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
    fn tool_schemas_follow_the_caller_role() {
        let parent = tools_for_role(false, &crate::agents::WorkerTasks::default());
        let parent_send = parent
            .iter()
            .find(|tool| tool.name == "worker_send")
            .expect("parent worker_send");
        assert!(parent.iter().any(|tool| tool.name == "worker_notices"));
        assert_eq!(
            parent_send.input_schema.get("required"),
            Some(&serde_json::json!(["to", "message"]))
        );

        let child = tools_for_role(true, &crate::agents::WorkerTasks::default());
        let child_send = child
            .iter()
            .find(|tool| tool.name == "worker_send")
            .expect("child worker_send");
        assert!(!child.iter().any(|tool| tool.name == "worker_notices"));
        assert_eq!(
            child_send.input_schema.get("required"),
            Some(&serde_json::json!(["message"]))
        );
        assert!(
            child_send.input_schema["properties"]
                .as_object()
                .is_some_and(|properties| !properties.contains_key("to"))
        );
    }

    #[test]
    fn worker_task_schema_tracks_customization_and_empty_definitions() {
        let mut tasks = crate::agents::WorkerTasks::default();
        tasks.tasks[0].name = "audit".into();
        tasks.tasks.truncate(1);
        let tools = tools_for_role(false, &tasks);
        let send = tools
            .iter()
            .find(|tool| tool.name == "worker_send")
            .unwrap();
        assert_eq!(
            send.input_schema["properties"]["task"]["enum"],
            serde_json::json!(["audit"])
        );
        assert_eq!(
            send.input_schema["properties"]["judgment"]["enum"],
            serde_json::json!(["specified", "guided", "independent"])
        );
        let child = tools_for_role(true, &tasks);
        let properties = child
            .iter()
            .find(|tool| tool.name == "worker_send")
            .unwrap()
            .input_schema["properties"]
            .as_object()
            .unwrap();
        for name in ["to", "task", "judgment"] {
            assert!(!properties.contains_key(name));
        }
        tasks.tasks.clear();
        let tools = tools_for_role(false, &tasks);
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.name == "worker_send")
                .unwrap()
                .input_schema["properties"]["task"],
            serde_json::json!(false)
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
            notices::NoticeBoard::default(),
        );
        assert_eq!(
            server.supported_protocol_versions().as_ref(),
            [ProtocolVersion::V_2026_07_28]
        );
        let config = server_config();
        assert!(!config.legacy_session_mode);
        assert!(config.stateless_protocol_metadata_required);
        assert!(config.json_response);
    }

    #[tokio::test]
    async fn workgraph_rejects_missing_authenticated_caller() {
        let temp = tempfile::tempdir().expect("project");
        let (updates, _) = async_channel::bounded(1);
        let server = FarcasterMcp::new(
            temp.path().join("unused.db"),
            worker_pool(temp.path()),
            updates,
            notices::NoticeBoard::default(),
        );
        let (parts, _) = axum::http::Request::new(()).into_parts();
        let result = server
            .search(
                Parameters(workgraph::SearchParams {
                    query: String::new(),
                }),
                Extension(parts),
            )
            .await;
        assert!(matches!(result, Err(error) if error.contains("registered Farcaster caller")));
        assert!(!temp.path().join("unused.db").exists());
    }

    #[test]
    fn workgraph_schemas_do_not_accept_caller_identity() {
        let tools = FarcasterMcp::tool_router().list_all();
        for tool in tools
            .iter()
            .filter(|tool| tool.name.starts_with("workgraph_"))
        {
            let properties = tool.input_schema["properties"].as_object().unwrap();
            for forbidden in ["project", "sessionId", "sessionPath", "next"] {
                assert!(
                    !properties.contains_key(forbidden),
                    "{} exposes {forbidden}",
                    tool.name
                );
            }
            assert_eq!(tool.input_schema["additionalProperties"], false);
            assert!(tool.output_schema.is_some());
        }
    }
}
