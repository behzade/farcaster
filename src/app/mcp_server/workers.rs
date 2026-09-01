use rmcp::schemars;
use serde::Deserialize;

use crate::agents::{CallerContext, CallerRegistry, StartWorker, WorkerContext, WorkerPool};

pub(super) const NEW_WORKER: &str = "new";
pub(super) const CHILD_WORKER: &str = "child";
pub(super) const PARENT_WORKER: &str = "parent";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SendParams {
    /// Existing worker id, `new` for a top-level peer, `child` for a nested worker, or `parent` to message the parent.
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
    let child = match params.to.as_str() {
        NEW_WORKER => Some(false),
        CHILD_WORKER => Some(true),
        _ => None,
    };
    if let Some(child) = child {
        let caller = registry.resolve(token)?;
        if caller.parent_worker_id.is_some() {
            return Err("child workers cannot create workers".to_owned());
        }
        pool.allow_project(&caller.project)?;
        let worker = pool.start(new_worker(caller, params.message, child))?;
        return Ok(serde_json::json!({
            "workerId": worker.id,
            "created": true,
            "queued": true,
            "child": child,
        }));
    }
    let to = if params.to == PARENT_WORKER {
        registry
            .resolve(token)?
            .parent_worker_id
            .ok_or_else(|| "only child workers can message their parent".to_owned())?
    } else {
        params.to
    };
    let worker_id = registry.send(token, &to, params.message)?;
    Ok(serde_json::json!({
        "workerId": worker_id,
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

fn new_worker(caller: CallerContext, message: String, child: bool) -> StartWorker {
    let (label, parent_worker_id) = if child {
        ("parent", Some(caller.worker_id.clone()))
    } else {
        ("peer", None)
    };
    StartWorker {
        project: caller.project,
        prompt: format!(
            "Task delegated by Farcaster {label} {}:\n\n{}",
            caller.worker_id, message
        ),
        backend: caller.backend,
        parent_session: caller.session,
        parent_worker_id,
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
    fn new_worker_is_fresh_but_inherits_the_callers_execution_profile() {
        let request = new_worker(caller(), "check the migration".into(), false);

        assert_eq!(request.project, std::path::PathBuf::from("/caller/project"));
        assert_eq!(request.backend, "codex-cli");
        assert_eq!(request.context, WorkerContext::Fresh);
        assert_eq!(request.parent_worker_id, None);
        assert_eq!(request.provider.as_deref(), Some("openai"));
        assert_eq!(request.model.as_deref(), Some("gpt-5"));
        assert_eq!(request.effort.as_deref(), Some("high"));
        assert!(request.prompt.contains("peer"));
        assert!(request.prompt.contains("worker-1"));
        assert!(request.prompt.contains("check the migration"));
    }

    #[test]
    fn child_worker_nests_under_the_caller_without_forking() {
        let request = new_worker(caller(), "review the diff".into(), true);
        assert_eq!(request.context, WorkerContext::Fresh);
        assert_eq!(request.parent_session, "caller-session");
        assert_eq!(request.parent_worker_id.as_deref(), Some("worker-1"));
        assert!(request.prompt.contains("parent"));
    }
}
