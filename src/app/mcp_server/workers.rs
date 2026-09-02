use rmcp::schemars;
use serde::Deserialize;

use crate::agents::{CallerContext, CallerRegistry, StartWorker, WorkerContext, WorkerPool};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SendParams {
    /// Direct child name. Required for top-level workers and omitted by children.
    pub(super) to: Option<String>,
    /// Message or delegated task for the worker.
    pub(super) message: String,
}

pub(super) fn send(
    pool: &WorkerPool,
    params: SendParams,
    caller_token: Option<String>,
) -> Result<serde_json::Value, String> {
    let token = caller_token
        .as_deref()
        .ok_or_else(|| "worker send requires a registered Farcaster caller".to_owned())?;
    let registry = CallerRegistry::shared();
    let caller = registry.resolve(token)?;

    if caller.parent_worker_id.is_some() {
        let worker = registry
            .send(token, "", params.message)?
            .ok_or_else(|| "parent worker is unavailable".to_owned())?;
        return Ok(serde_json::json!({
            "worker": worker,
            "created": false,
            "queued": true,
        }));
    }

    let to = params
        .to
        .ok_or_else(|| "top-level workers must provide a child name in `to`".to_owned())?;
    if !crate::agents::valid_worker_name(&to) {
        return Err("child name must be 1-48 ASCII letters, numbers, '-' or '_' and cannot start with punctuation".into());
    }
    if let Some(worker) = registry.send(token, &to, params.message.clone())? {
        return Ok(serde_json::json!({
            "worker": worker,
            "created": false,
            "queued": true,
        }));
    }

    pool.allow_project(&caller.project)?;
    let name = to;
    pool.start(new_worker(caller, name.clone(), params.message))?;
    Ok(serde_json::json!({
        "worker": name,
        "created": true,
        "queued": true,
    }))
}

fn new_worker(caller: CallerContext, name: String, message: String) -> StartWorker {
    StartWorker {
        project: caller.project,
        name: name.clone(),
        prompt: format!(
            "Task delegated by Farcaster parent {} to child {name}:\n\n{message}",
            caller.worker_name
        ),
        backend: caller.backend,
        parent_session: caller.session,
        parent_worker_id: Some(caller.worker_id),
        context: WorkerContext::Fresh,
        provider: caller.provider,
        model: caller.model,
        effort: caller.effort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller() -> CallerContext {
        CallerContext {
            worker_id: "worker-1".into(),
            worker_name: "OrangeCoyote".into(),
            project: "/caller/project".into(),
            session: "caller-session".into(),
            backend: "codex-cli".into(),
            provider: Some("openai".into()),
            model: Some("gpt-5".into()),
            effort: Some("high".into()),
            parent_worker_id: None,
        }
    }

    #[test]
    fn named_child_is_fresh_and_inherits_the_parent_execution_profile() {
        let request = new_worker(caller(), "diff-review".into(), "review the diff".into());

        assert_eq!(request.project, std::path::PathBuf::from("/caller/project"));
        assert_eq!(request.name, "diff-review");
        assert_eq!(request.backend, "codex-cli");
        assert_eq!(request.context, WorkerContext::Fresh);
        assert_eq!(request.parent_session, "caller-session");
        assert_eq!(request.parent_worker_id.as_deref(), Some("worker-1"));
        assert_eq!(request.provider.as_deref(), Some("openai"));
        assert_eq!(request.model.as_deref(), Some("gpt-5"));
        assert_eq!(request.effort.as_deref(), Some("high"));
        assert!(request.prompt.contains("OrangeCoyote"));
        assert!(request.prompt.contains("diff-review"));
        assert!(request.prompt.contains("review the diff"));
    }
}
