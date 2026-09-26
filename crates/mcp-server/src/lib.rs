#[cfg(test)]
use farcaster_agent_protocol::extensions as protocol;
use farcaster_agents as agents;
use farcaster_agents::builtin_mcp;
use farcaster_reviews as review_domain;
use farcaster_sessions as sessions;
use farcaster_storage as storage;

mod lifecycle;
mod notice_board;
mod notices;
mod profile_prompt;
mod reviews;
#[cfg(any(test, feature = "test-support"))]
pub use lifecycle::with_test_worker_pool;
pub use lifecycle::{
    McpServer, finish_session_family_worker_stop, install_listener, set_enabled,
    set_worker_app_proxy, start, stop_session_family_workers, worker_snapshots,
};
pub use notice_board::{NoticeBoard, NoticeView};
pub use workers::{SendParams, send};
mod workers;
mod workgraph;

use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};

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

type JsonObject = serde_json::Map<String, serde_json::Value>;
type SharedStore = Arc<Mutex<storage::StateStore>>;

fn with_store<T>(
    store: &SharedStore,
    operation: impl FnOnce(&mut storage::StateStore) -> Result<T, String>,
) -> Result<T, String> {
    let mut store = store
        .lock()
        .map_err(|_| "State database lock is poisoned")?;
    operation(&mut store)
}

fn json_object(value: serde_json::Value) -> Result<Json<JsonObject>, String> {
    match value {
        serde_json::Value::Object(object) => Ok(Json(object)),
        _ => Err("MCP tool output must be an object".into()),
    }
}

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
    store: SharedStore,
    workers: crate::agents::WorkerPool,
    workgraph_updates: async_channel::Sender<()>,
    notices: notices::NoticeBoard,
}

impl FarcasterMcp {
    fn new(
        store: SharedStore,
        workers: crate::agents::WorkerPool,
        workgraph_updates: async_channel::Sender<()>,
        notices: notices::NoticeBoard,
    ) -> Self {
        Self {
            store,
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
            &mut storage::StateStore,
            &crate::agents::CallerContext,
            P,
        ) -> Result<serde_json::Value, String>,
        mutates: bool,
    ) -> Result<Json<JsonObject>, String> {
        let token = caller_token(&parts)
            .ok_or_else(|| "workgraph requires a registered Farcaster caller".to_owned())?;
        let store = self.store.clone();
        let result = tokio::task::spawn_blocking(move || {
            let caller = crate::agents::CallerRegistry::shared().resolve(&token)?;
            with_store(&store, |store| operation(store, &caller, params))
        })
        .await
        .map_err(|error| format!("work graph task failed: {error}"))??;
        if mutates {
            notify_workgraph_changed(&self.workgraph_updates);
        }
        json_object(result)
    }
}

#[tool_router]
impl FarcasterMcp {
    #[tool(
        name = "submit_review",
        description = "Submit suggested review locations for the user as a transcript review card. Supply project-relative files, optional inclusive line bands, and short notes. This is advisory, not a verified changeset. The user can open the list in their editor; submitting never opens it automatically."
    )]
    async fn submit_review(
        &self,
        Parameters(params): Parameters<reviews::Params>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<JsonObject>, String> {
        let token = caller_token(&parts)
            .ok_or_else(|| "review requires a registered Farcaster caller".to_owned())?;
        let store = self.store.clone();
        let (caller, execution) =
            crate::agents::CallerRegistry::shared().resolve_execution(&token)?;
        let result = tokio::task::spawn_blocking(move || {
            let artifact = reviews::submit(&caller, params)?;
            with_store(&store, |store| {
                store.save_review(&caller, &execution, &artifact)
            })?;
            crate::review_domain::delivery::notify();
            Ok::<_, String>(artifact)
        })
        .await
        .map_err(|error| format!("review task failed: {error}"))??;
        json_object(result)
    }

    #[tool(
        name = "worker_send",
        description = "Send work within your worker family. Top-level workers provide a direct child name in `to`; first use creates the child and subsequent messages reuse it. Children omit `to` and always send to their parent."
    )]
    async fn worker_send(
        &self,
        Parameters(params): Parameters<workers::SendParams>,
        Extension(parts): Extension<axum::http::request::Parts>,
    ) -> Result<Json<JsonObject>, String> {
        let caller_token = caller_token(&parts);
        let pool = self.workers.clone();
        let store = self.store.clone();
        let value = tokio::task::spawn_blocking(move || {
            let (profiles, catalogs) = with_store(&store, |store| {
                Ok((
                    store.load_worker_profiles()?,
                    store.load_configuration_catalogs()?,
                ))
            })?;
            let backends = crate::agents::backend_statuses()
                .into_iter()
                .filter(|backend| backend.available)
                .map(|backend| backend.id)
                .collect::<Vec<_>>();
            workers::send_configurable(
                &pool,
                params,
                caller_token,
                &profiles,
                |model, project, parent_access_mode| {
                    workers::child_access_mode(
                        model,
                        project,
                        parent_access_mode,
                        &backends,
                        &catalogs,
                    )
                },
                |profile, caller| {
                    profile_prompt::configure(profile, caller, &catalogs, &backends, &store)
                },
            )
        })
        .await
        .map_err(|error| format!("worker send task failed: {error}"))??;
        json_object(value)
    }

    #[tool(
        name = "worker_notices",
        description = "Read or post project notices only when coordinating potentially overlapping work with other top-level workers."
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
    ) -> Result<Json<JsonObject>, String> {
        self.workgraph_call(parts, params, workgraph::search_store, false)
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
    ) -> Result<Json<JsonObject>, String> {
        self.workgraph_call(parts, params, workgraph::patch_store, true)
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
    ) -> Result<Json<JsonObject>, String> {
        self.workgraph_call(parts, params, workgraph::claim_store, true)
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
    ) -> Result<Json<JsonObject>, String> {
        self.workgraph_call(parts, params, workgraph::release_store, true)
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
    ) -> Result<Json<JsonObject>, String> {
        self.workgraph_call(parts, params, workgraph::complete_store, true)
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
                    .filter(|profile| profile.enabled)
                    .map(|profile| format!("{}: {}", profile.name, profile.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                properties.insert("profile".into(), serde_json::json!({
                    "type": "string", "enum": (tasks.inherit_enabled.then_some("inherit")).into_iter().chain(tasks.profiles.iter().filter(|profile| profile.enabled).map(|profile| profile.name.as_str())).collect::<Vec<_>>(),
                    "description": format!("Worker profile. {} Each named profile has one model and its own worker limit. Selection stays fixed for the child's lifetime; omit on reuse to keep it.\n{descriptions}",
                        if tasks.inherit_enabled { "Omit on creation or use inherit to copy the caller's harness, provider, model, and effort." } else { "Choose an enabled named profile on creation." })
                }));
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
    instructions = "You are running inside Farcaster, a GUI app for multiple agent harnesses. Use Farcaster MCP by default to keep substantial work in a persistent task graph the user can inspect, coordinate with concurrent agents through workspace notices, and delegate independent work to workers using inherit (Same as caller) or configured profiles across harnesses for cost and visibility."
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
        let tasks = with_store(&self.store, |store| store.load_worker_profiles())
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
