use std::path::PathBuf;

use rmcp::schemars;
use serde::Deserialize;

use crate::workers::{StartWorker, WorkerContext, WorkerMessageMode, WorkerPool};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct StartParams {
    /// Absolute project directory for the worker.
    pub(super) project: String,
    /// Task for the worker.
    pub(super) prompt: String,
    /// Agent backend. Currently `pi`.
    #[serde(default)]
    pub(super) backend: Option<String>,
    /// Existing backend session to use as context.
    #[serde(default)]
    pub(super) source_session: Option<String>,
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

pub(super) fn start(pool: &WorkerPool, params: StartParams) -> Result<serde_json::Value, String> {
    let context = params
        .source_session
        .map_or(WorkerContext::Fresh, |session| WorkerContext::Session {
            session_locator: session,
        });
    encode(
        &pool.start(StartWorker {
            project: PathBuf::from(params.project),
            prompt: params.prompt,
            backend: params
                .backend
                .unwrap_or_else(|| pool.default_backend().to_owned()),
            context,
            provider: params.provider,
            model: params.model,
            effort: params.effort,
        })?,
    )
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

fn encode(value: &impl serde::Serialize) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|error| format!("encode worker result: {error}"))
}
