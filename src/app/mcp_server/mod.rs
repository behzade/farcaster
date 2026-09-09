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
    // Per-request protocol metadata (SEP-2575) is deliberately not required:
    // the harnesses served here (OpenCode, Codex, ACP, Pi) are 2025-era MCP
    // clients. Re-enable rmcp's stateless_protocol_metadata_required once
    // they adopt 2026-07-28 era negotiation.
    StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
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
            let store = crate::app::persistence::StateStore::open_at(&database)?;
            let profiles = store.load_worker_profiles()?;
            let catalogs = store.load_configuration_catalogs()?;
            let backends = crate::agents::backend_statuses()
                .into_iter()
                .filter(|backend| backend.available)
                .map(|backend| backend.id)
                .collect::<Vec<_>>();
            workers::send(&pool, params, caller_token, &profiles, |model, project| {
                workers::model_available(model, project, &backends, &catalogs)
            })
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

fn tools_for_role(child: bool, tasks: &crate::agents::WorkerProfiles) -> Vec<rmcp::model::Tool> {
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
                properties.remove("profile");
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
                let descriptions = tasks
                    .profiles
                    .iter()
                    .map(|profile| format!("{}: {}", profile.name, profile.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                properties.insert("profile".into(), if tasks.profiles.is_empty() { serde_json::json!(false) } else { serde_json::json!({
                    "type": "string", "enum": tasks.profiles.iter().map(|profile| profile.name.as_str()).collect::<Vec<_>>(),
                    "description": format!("Worker profile; required on creation, omitted on reuse. The first available model in the profile's ordered list is selected. That model and effort stay fixed for the child's lifetime.\n{descriptions}")
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
    instructions = "You are running inside Farcaster, a GUI app for multiple agent harnesses. Use Farcaster MCP by default to keep substantial work in a persistent task graph the user can inspect, coordinate with concurrent agents through workspace notices, and delegate independent work to predefined subagent profiles across harnesses for cost and visibility."
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
            .and_then(|store| store.load_worker_profiles())
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
#[path = "mod_tests.rs"]
mod tests;
