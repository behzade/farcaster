use super::*;
use crate::agents::{
    Backend, CallerProfile, CallerRegistry, WorkerEvent, WorkerLaunch, WorkerSession,
    WorkerSessionFactory,
};
use std::{sync::Arc, time::Duration};

struct ResumeFactory(std::sync::mpsc::Sender<WorkerLaunch>);

impl WorkerSessionFactory for ResumeFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        let identity = CallerRegistry::shared().issue_as_with_access(
            &launch.project,
            CallerProfile {
                backend: Backend::Codex,
                provider: launch.provider.clone(),
                model: launch.model.clone(),
                effort: launch.effort.clone(),
            },
            None,
            launch.worker_id.clone(),
            launch.worker_name.clone(),
            launch.parent_worker_id.clone(),
            launch.access_mode,
        )?;
        if let crate::agents::WorkerContext::Resume { session_locator } = &launch.context {
            identity.bind(session_locator.clone());
        }
        self.0.send(launch).map_err(|_| "record resumed launch")?;
        Ok(Box::new(ResumeSession(identity)))
    }
}

struct ResumeSession(crate::agents::CallerIdentity);

impl WorkerSession for ResumeSession {
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
fn persisted_child_reuses_name_session_assignment_and_access_mode() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let assignment = crate::agents::WorkerAssignment {
        profile: "fast".into(),
        execution: crate::agents::WorkerExecution {
            harness: Backend::Codex,
            provider: "openai".into(),
            model: "saved-model".into(),
            effort: Some("high".into()),
        },
    };
    crate::storage::StateStore::open_at(&database)?.save_worker_family(
        &crate::agents::WorkerFamilyLink {
            project: temp.path().to_owned(),
            child_backend: Backend::Codex,
            child_session: "saved-child-session".into(),
            parent_backend: Backend::Pi,
            parent_session: "saved-parent-session".into(),
            execution: Some(assignment.execution.clone()),
            routing: Some(crate::agents::WorkerRouting {
                name: "research".into(),
                assignment,
                access_mode: crate::agents::HarnessAccessMode::Sandboxed,
            }),
        },
    )?;

    let (launches, resumed) = std::sync::mpsc::channel();
    let factory: Arc<dyn WorkerSessionFactory> = Arc::new(ResumeFactory(launches));
    let pool = crate::agents::WorkerPool::new(
        std::collections::BTreeMap::from([(Backend::Codex, factory)]),
        Backend::Codex,
        temp.path().to_owned(),
        1,
    )?;
    pool.restore_families(crate::storage::StateStore::open_at(&database)?.load_worker_routes()?)?;
    let parent = CallerRegistry::shared().issue_with_access(
        temp.path(),
        CallerProfile {
            backend: Backend::Pi,
            provider: None,
            model: None,
            effort: None,
        },
        None,
        crate::agents::HarnessAccessMode::Full,
    );
    parent.bind("saved-parent-session");
    let result = send(
        &pool,
        SendParams {
            to: Some("research".into()),
            message: "continue".into(),
            profile: None,
        },
        Some(parent.token().into()),
        &crate::agents::WorkerProfiles::default(),
        |execution, _, mode| (execution.harness == Backend::Codex).then_some(mode),
    )?;

    assert_eq!(result["created"], false);
    assert_eq!(result["assignment"]["profile"], "fast");
    let launch = resumed
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "resumed worker did not launch")?;
    assert!(matches!(
        launch.context,
        crate::agents::WorkerContext::Resume { session_locator }
            if session_locator == "saved-child-session"
    ));
    assert_eq!(
        launch.access_mode,
        crate::agents::HarnessAccessMode::Sandboxed
    );
    Ok(())
}
