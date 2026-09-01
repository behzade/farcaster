use rmcp::schemars;
use serde::Deserialize;

use crate::agents::{CallerContext, CallerRegistry, StartWorker, WorkerContext, WorkerPool};

pub(super) const NEW_WORKER: &str = "new";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SendParams {
    /// Existing worker identifier, or `new` to create an independent top-level agent.
    pub(super) to: String,
    /// Message or independent task for the worker.
    pub(super) message: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(super) struct ListParams {}

pub(super) fn send(
    pool: &WorkerPool,
    params: SendParams,
    caller_token: Option<String>,
) -> Result<serde_json::Value, String> {
    let token = caller_token
        .as_deref()
        .ok_or_else(|| "worker send requires a registered Farcaster caller".to_owned())?;
    let registry = CallerRegistry::shared();
    if params.to == NEW_WORKER {
        let caller = registry.resolve(token)?;
        pool.allow_project(&caller.project)?;
        let worker = pool.start(new_worker(caller, params.message))?;
        return Ok(serde_json::json!({
            "workerId": worker.id,
            "created": true,
            "queued": true,
        }));
    }
    registry.send(token, &params.to, params.message)?;
    Ok(serde_json::json!({
        "workerId": params.to,
        "created": false,
        "queued": true,
    }))
}

pub(super) fn list(caller_token: Option<String>) -> Result<serde_json::Value, String> {
    let token = caller_token
        .as_deref()
        .ok_or_else(|| "worker list requires a registered Farcaster caller".to_owned())?;
    let (self_id, workers) = CallerRegistry::shared().list(token)?;
    Ok(serde_json::json!({
        "self": self_id,
        "workers": workers,
    }))
}

fn new_worker(caller: CallerContext, message: String) -> StartWorker {
    StartWorker {
        project: caller.project,
        prompt: format!(
            "Task delegated by Farcaster peer {}:\n\n{}",
            caller.worker_id, message
        ),
        backend: caller.backend,
        parent_session: caller.session,
        context: WorkerContext::Fresh,
        provider: caller.provider,
        model: caller.model,
        effort: caller.effort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_worker_is_fresh_but_inherits_the_callers_execution_profile() {
        let request = new_worker(
            CallerContext {
                worker_id: "worker-1".into(),
                project: "/caller/project".into(),
                session: "caller-session".into(),
                backend: "codex-cli".into(),
                provider: Some("openai".into()),
                model: Some("gpt-5".into()),
                effort: Some("high".into()),
            },
            "check the migration".into(),
        );

        assert_eq!(request.project, std::path::PathBuf::from("/caller/project"));
        assert_eq!(request.backend, "codex-cli");
        assert_eq!(request.context, WorkerContext::Fresh);
        assert_eq!(request.provider.as_deref(), Some("openai"));
        assert_eq!(request.model.as_deref(), Some("gpt-5"));
        assert_eq!(request.effort.as_deref(), Some("high"));
        assert!(request.prompt.contains("worker-1"));
        assert!(request.prompt.contains("check the migration"));
    }
}
