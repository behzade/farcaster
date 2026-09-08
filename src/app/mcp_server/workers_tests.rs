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
fn worker_send_routes_across_harnesses_and_reuses_the_original_assignment() -> Result<(), String> {
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
    tasks.tasks.clear();
    let result = send(&pool, params(None, None), token.clone(), &tasks)?;
    assert_eq!(result["created"], false);
    assert_eq!(result["assignment"]["judgment"], "specified");
    assert!(send(&pool, params(Some("review".into()), None), token, &tasks).is_err());
    assert_eq!(launches.lock().unwrap().len(), 1);
    Ok(())
}
