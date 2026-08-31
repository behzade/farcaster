use std::path::PathBuf;

use rmcp::schemars;
use serde::Deserialize;

use crate::agents::{CallerContext, StartWorker, WorkerContext, WorkerMessageMode, WorkerPool};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct StartParams {
    /// Absolute project directory for the worker.
    pub(super) project: String,
    /// Task for the worker.
    pub(super) prompt: String,
    /// Agent backend. Use `worker_backends` to list available backends.
    #[serde(default)]
    pub(super) backend: Option<String>,
    /// Parent backend session. Normally supplied automatically by Farcaster.
    #[serde(default)]
    pub(super) parent_session: Option<String>,
    /// Existing backend session to use as context. Omit to fork the parent.
    #[serde(default)]
    pub(super) source_session: Option<String>,
    /// Start with automatic, inherited, or blank context. Automatic forks only on the default backend.
    #[serde(default)]
    pub(super) context: StartContext,
    /// Model provider; must be paired with `model`.
    #[serde(default)]
    pub(super) provider: Option<String>,
    /// Model identifier; must be paired with `provider`.
    #[serde(default)]
    pub(super) model: Option<String>,
    /// Backend reasoning or effort level.
    #[serde(default)]
    pub(super) effort: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum StartContext {
    #[default]
    Auto,
    Fresh,
    Fork,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SendParams {
    /// Worker identifier returned by `worker_start`.
    pub(super) id: String,
    /// Message for the worker.
    pub(super) message: String,
    /// `auto` steers running workers and prompts idle workers.
    #[serde(default)]
    pub(super) mode: MessageMode,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum MessageMode {
    #[default]
    Auto,
    Prompt,
    Steer,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(super) struct ListParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(super) struct RespondParams {
    /// Worker identifier returned by `worker_start`.
    pub(super) id: String,
    /// Selected option or entered value.
    #[serde(default)]
    pub(super) value: Option<String>,
    /// Cancel the pending input instead of answering it.
    #[serde(default)]
    pub(super) cancel: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(super) struct WorkerParams {
    /// Worker identifier returned by `worker_start`.
    pub(super) id: String,
}

pub(super) fn backends(pool: &WorkerPool) -> serde_json::Value {
    serde_json::json!({
        "default": pool.default_backend(),
        "available": pool.backends(),
    })
}

pub(super) fn start(
    pool: &WorkerPool,
    params: StartParams,
    caller: Option<CallerContext>,
) -> Result<serde_json::Value, String> {
    let project = PathBuf::from(&params.project);
    let caller_parent = if let Some(caller) = caller {
        let requested = canonical_directory(&project)?;
        let caller_project = canonical_directory(&caller.project)?;
        if requested != caller_project {
            return Err(format!(
                "worker project does not match its calling session: {}",
                requested.display()
            ));
        }
        pool.allow_project(&caller_project)?;
        Some(caller.session)
    } else {
        None
    };
    let backend = params
        .backend
        .unwrap_or_else(|| pool.default_backend().to_owned());
    let source_session = params.source_session;
    let parent_session = params
        .parent_session
        .or(caller_parent)
        .or_else(|| source_session.clone())
        .ok_or_else(|| "worker start requires a parent session".to_owned())?;
    let context = match params.context {
        StartContext::Auto if backend != pool.default_backend() => WorkerContext::Fresh,
        StartContext::Auto | StartContext::Fork => WorkerContext::Session {
            session_locator: source_session.unwrap_or_else(|| parent_session.clone()),
        },
        StartContext::Fresh => WorkerContext::Fresh,
    };
    encode(&pool.start(StartWorker {
        project,
        prompt: params.prompt,
        backend,
        parent_session,
        context,
        provider: params.provider,
        model: params.model,
        effort: params.effort,
    })?)
}

pub(super) fn send(pool: &WorkerPool, params: SendParams) -> Result<serde_json::Value, String> {
    let mode = match params.mode {
        MessageMode::Auto => WorkerMessageMode::Auto,
        MessageMode::Prompt => WorkerMessageMode::Prompt,
        MessageMode::Steer => WorkerMessageMode::Steer,
    };
    encode(&pool.send(&params.id, params.message, mode)?)
}

pub(super) fn respond(
    pool: &WorkerPool,
    params: RespondParams,
) -> Result<serde_json::Value, String> {
    encode(&pool.respond(&params.id, params.value, params.cancel)?)
}

pub(super) fn list(pool: &WorkerPool) -> Result<serde_json::Value, String> {
    encode(&pool.list()?)
}

pub(super) fn status(pool: &WorkerPool, params: WorkerParams) -> Result<serde_json::Value, String> {
    encode(&pool.status(&params.id)?)
}

pub(super) fn stop(pool: &WorkerPool, params: WorkerParams) -> Result<serde_json::Value, String> {
    encode(&pool.stop(&params.id)?)
}

fn canonical_directory(path: &std::path::Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("resolve worker project {}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "worker project is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn encode(value: &impl serde::Serialize) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|error| format!("encode worker result: {error}"))
}
