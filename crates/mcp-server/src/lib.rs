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
            let (profiles, catalogs, harness_profiles) = with_store(&store, |store| {
                Ok((
                    store.load_worker_profiles()?,
                    store.load_configuration_catalogs()?,
                    store.load_harness_profiles()?,
                ))
            })?;
            let launch_profiles = std::sync::Arc::new(crate::agents::HarnessProfiles::default());
            launch_profiles.replace(harness_profiles)?;
            let launch_config = crate::agents::AgentLaunchConfig {
                profiles: launch_profiles,
                ..Default::default()
            };
            workers::send_configurable(
                &pool,
                params,
                caller_token,
                &profiles,
                |model, profile_id, project, parent_access_mode| {
                    workers::launch_access_mode(
                        &launch_config,
                        model,
                        profile_id,
                        project,
                        parent_access_mode,
                        &catalogs,
                    )
                },
                |profile, caller| {
                    profile_prompt::configure(profile, caller, &catalogs, &launch_config, &store)
                },
            )
        })
        .await
        .map_err(|error| format!("worker send task failed: {error}"))??;
        json_object(value)
    }

    #[tool(
        name = "worker_notices",
        description = "Read or wait for project notices; reuse after=cursor with the same paths to get only new messages. Wait timeout defaults to 30 seconds, max 60. Posts return only an acknowledgement; keep your read cursor. Notices are advisory, not locks."
    )]
    async fn worker_notices(
        &self,
        Parameters(params): Parameters<notices::Params>,
        Extension(parts): Extension<axum::http::request::Parts>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<notices::Response>, String> {
        let token = caller_token(&parts)
            .ok_or_else(|| "worker notices require a registered Farcaster caller".to_owned())?;
        let caller = tokio::task::spawn_blocking(move || {
            crate::agents::CallerRegistry::shared().resolve(&token)
        })
        .await
        .map_err(|error| format!("worker notice task failed: {error}"))??;
        let value = tokio::select! {
            biased;
            _ = context.ct.cancelled() => return Err("worker notice request cancelled".into()),
            value = self.notices.access(&caller, params) => value?,
        };
        Ok(Json(value))
    }

    #[tool(
        name = "workgraph_search",
        description = "List task summaries, defaulting to active tasks, 20 per page (max 100). Continue with after=nextAfter and the same filters. Use status=all or completed for history; task=N returns full details."
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
        description = "Create or extend an ordered task chain. Returns only created task summaries; does not claim them."
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
        description = "Atomically claim a ready task for your session. Returns only its summary; conflicts with another owner."
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
        description = "Release your task for another session to claim. Returns only its summary."
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
        description = "Complete your task with evidence. Returns its summary and newly ready task summaries; does not claim them. Read task details to retrieve stored evidence."
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
    instructions = "Use Farcaster to track substantial work and coordinate agents. For code tool calls, print only structuredContent or selected fields."
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
