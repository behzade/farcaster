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
    /// Classification of already-delegated work. Required only when creating a child.
    pub(super) task: Option<String>,
    /// Judgment delegated: specified procedure, guided local decisions, or independent approach. Defaults to guided on creation; omit on reuse.
    #[schemars(with = "Option<String>")]
    pub(super) judgment: Option<crate::agents::WorkerJudgment>,
}

pub(super) fn send(
    pool: &WorkerPool,
    params: SendParams,
    caller_token: Option<String>,
    tasks: &crate::agents::WorkerTasks,
) -> Result<serde_json::Value, String> {
    if params.message.trim().is_empty() {
        return Err("worker message must not be empty".into());
    }
    let token = caller_token
        .as_deref()
        .ok_or_else(|| "worker send requires a registered Farcaster caller".to_owned())?;
    let registry = CallerRegistry::shared();
    let caller = registry.resolve(token)?;

    if caller.parent_worker_id.is_some() {
        if params.task.is_some() || params.judgment.is_some() {
            return Err("children cannot select task routing".into());
        }
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
    if let Some(assignment) = registry.child_assignment(&caller, &to)? {
        validate_reuse(&assignment, params.task.as_deref(), params.judgment)?;
        let worker = registry
            .send(token, &to, params.message.clone())?
            .ok_or("child became unavailable; retry with a new child name")?;
        return Ok(serde_json::json!({
            "worker": worker,
            "created": false,
            "queued": true,
            "assignment": assignment,
        }));
    }

    pool.allow_project(&caller.project)?;
    let name = to;
    let task = params
        .task
        .as_deref()
        .ok_or("new children require a configured `task`; omit task only when reusing a child")?;
    let assignment = tasks.resolve(task, params.judgment.unwrap_or_default())?;
    pool.start_assigned(
        new_worker(caller, name.clone(), params.message, &assignment),
        Some(assignment.clone()),
    )?;
    Ok(serde_json::json!({
        "worker": name,
        "created": true,
        "queued": true,
        "assignment": assignment,
    }))
}

fn validate_reuse(
    assignment: &crate::agents::WorkerAssignment,
    task: Option<&str>,
    judgment: Option<crate::agents::WorkerJudgment>,
) -> Result<(), String> {
    if task.is_some_and(|task| task != assignment.task)
        || judgment.is_some_and(|judgment| judgment != assignment.judgment)
    {
        return Err("child task and judgment are fixed at creation; use a new child name for different routing".into());
    }
    Ok(())
}

fn new_worker(
    caller: CallerContext,
    name: String,
    message: String,
    assignment: &crate::agents::WorkerAssignment,
) -> StartWorker {
    StartWorker {
        project: caller.project,
        name: name.clone(),
        prompt: format!(
            "Task delegated by Farcaster parent {} to child {name}:\n\n{message}",
            caller.worker_name
        ),
        backend: assignment.execution.harness.clone(),
        parent_session: caller.session,
        parent_worker_id: Some(caller.worker_id),
        context: WorkerContext::Fresh,
        provider: Some(assignment.execution.provider.clone()),
        model: Some(assignment.execution.model.clone()),
        effort: assignment.execution.effort.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{CallerIdentity, CallerProfile};
    use crate::agents::{WorkerEvent, WorkerLaunch, WorkerSession, WorkerSessionFactory};
    use std::sync::{Arc, Mutex};

    struct Factory {
        launches: Arc<Mutex<Vec<WorkerLaunch>>>,
    }
    struct Session(CallerIdentity);
    impl WorkerSessionFactory for Factory {
        fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
            let identity = CallerRegistry::shared().issue_as(
                &launch.project,
                CallerProfile {
                    backend: "codex-cli".into(),
                    provider: launch.provider.clone(),
                    model: launch.model.clone(),
                    effort: launch.effort.clone(),
                },
                None,
                launch.worker_id.clone(),
                launch.worker_name.clone(),
                launch.parent_worker_id.clone(),
            )?;
            identity.bind(format!("session-{}", launch.worker_id));
            self.launches.lock().unwrap().push(launch);
            Ok(Box::new(Session(identity)))
        }
    }
    impl WorkerSession for Session {
        fn send(&mut self, _: String, _: crate::agents::WorkerSendMode) -> Result<(), String> {
            Ok(())
        }
        fn respond(&mut self, _: crate::agents::WorkerInputResponse) -> Result<(), String> {
            Ok(())
        }
        fn abort(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<WorkerEvent> {
            let _ = self.0.token();
            None
        }
        fn close(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn worker_send_routes_across_harnesses_and_reuses_the_original_assignment() -> Result<(), String>
    {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let launches = Arc::new(Mutex::new(Vec::new()));
        let factory: Arc<dyn WorkerSessionFactory> = Arc::new(Factory {
            launches: launches.clone(),
        });
        let pool = WorkerPool::new(
            std::collections::BTreeMap::from([("codex-cli".into(), factory)]),
            "codex-cli".into(),
            temp.path().to_owned(),
            1,
        )?;
        let registry = CallerRegistry::shared();
        let parent = registry.issue(
            temp.path(),
            CallerProfile {
                backend: "pi".into(),
                provider: Some("parent-provider".into()),
                model: Some("expensive-parent".into()),
                effort: Some("max".into()),
            },
            None,
        );
        parent.bind("/sessions/parent.jsonl");
        let token = Some(parent.token().to_owned());
        let mut tasks = crate::agents::WorkerTasks::default();
        tasks.tasks[0].specified.harness = "codex-cli".into();
        tasks.tasks[0].specified.provider = "openai".into();
        let params = |task, judgment| SendParams {
            to: Some("inspect".into()),
            message: "inspect these files".into(),
            task,
            judgment,
        };
        assert!(send(&pool, params(None, None), token.clone(), &tasks).is_err());
        assert!(
            send(
                &pool,
                params(Some("missing".into()), None),
                token.clone(),
                &tasks
            )
            .is_err()
        );
        assert!(launches.lock().unwrap().is_empty());
        let result = send(
            &pool,
            params(
                Some("read".into()),
                Some(crate::agents::WorkerJudgment::Specified),
            ),
            token.clone(),
            &tasks,
        )?;
        assert_eq!(result["created"], true);
        assert_eq!(result["assignment"]["execution"]["harness"], "codex-cli");
        assert_eq!(
            launches.lock().unwrap()[0].model.as_deref(),
            Some("gpt-5.6-luna")
        );
        {
            let launches = launches.lock().unwrap();
            let launch = &launches[0];
            assert_eq!(launch.context, WorkerContext::Fresh);
            assert_eq!(launch.provider.as_deref(), Some("openai"));
            assert_eq!(launch.effort.as_deref(), Some("high"));
            assert_eq!(launch.parent_session, "/sessions/parent.jsonl");
            assert_eq!(launch.project, temp.path().canonicalize().unwrap());
        }
        assert!(
            send(
                &pool,
                params(
                    Some("read".into()),
                    Some(crate::agents::WorkerJudgment::Specified)
                ),
                token.clone(),
                &tasks
            )
            .is_ok()
        );
        assert!(
            send(
                &pool,
                params(None, Some(crate::agents::WorkerJudgment::Independent)),
                token.clone(),
                &tasks
            )
            .is_err()
        );
        // Deleting a task does not change a live child's assignment.
        tasks.tasks.clear();
        let result = send(&pool, params(None, None), token.clone(), &tasks)?;
        assert_eq!(result["created"], false);
        assert_eq!(result["assignment"]["judgment"], "specified");
        assert!(send(&pool, params(Some("review".into()), None), token, &tasks).is_err());
        assert_eq!(launches.lock().unwrap().len(), 1);
        Ok(())
    }
}
