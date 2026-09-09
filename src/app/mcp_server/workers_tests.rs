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
        self.launches
            .lock()
            .expect("test operation should succeed")
            .push(launch);
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
    let send = |pool, params, token, profiles: &crate::agents::WorkerProfiles| {
        super::send(pool, params, token, profiles, |model, _| {
            model.harness == "codex-cli"
        })
    };
    let mut tasks = crate::agents::WorkerProfiles::default();
    // Cursor is preferred here, but only Codex is available to this pool.
    tasks.profiles[0].models.rotate_right(1);
    let params = |profile| SendParams {
        to: Some("inspect".into()),
        message: "inspect these files".into(),
        profile,
    };
    assert!(send(&pool, params(None), token.clone(), &tasks).is_err());
    assert!(send(&pool, params(Some("missing".into())), token.clone(), &tasks).is_err());
    assert!(
        launches
            .lock()
            .expect("test operation should succeed")
            .is_empty()
    );
    let result = send(&pool, params(Some("oracle".into())), token.clone(), &tasks)?;
    assert_eq!(result["created"], true);
    assert_eq!(result["assignment"]["execution"]["harness"], "codex-cli");
    assert_eq!(
        launches.lock().expect("test operation should succeed")[0]
            .model
            .as_deref(),
        Some("gpt-6-astra")
    );
    {
        let launches = launches.lock().expect("test operation should succeed");
        let launch = &launches[0];
        assert_eq!(launch.context, WorkerContext::Fresh);
        assert_eq!(launch.provider.as_deref(), Some("openai"));
        assert_eq!(launch.effort.as_deref(), Some("medium"));
        assert_eq!(launch.parent_session, "/sessions/parent.jsonl");
        assert_eq!(
            launch.project,
            temp.path()
                .canonicalize()
                .expect("test operation should succeed")
        );
    }
    tasks.profiles[0].models[1].model = "changed-model".into();
    assert!(send(&pool, params(Some("oracle".into())), token.clone(), &tasks).is_ok());
    assert!(
        send(
            &pool,
            params(Some("thorough".into())),
            token.clone(),
            &tasks
        )
        .is_err()
    );
    tasks.profiles.clear();
    let result = send(&pool, params(None), token.clone(), &tasks)?;
    assert_eq!(result["created"], false);
    assert_eq!(result["assignment"]["profile"], "oracle");
    assert_eq!(result["assignment"]["execution"]["model"], "gpt-6-astra");
    assert!(send(&pool, params(Some("thorough".into())), token, &tasks).is_err());
    assert_eq!(
        launches
            .lock()
            .expect("test operation should succeed")
            .len(),
        1
    );
    Ok(())
}

#[test]
fn worker_send_rejects_old_routing_and_model_overrides() {
    for field in ["task", "judgment", "model", "effort"] {
        let mut value =
            serde_json::json!({"to": "inspect", "message": "review", "profile": "oracle"});
        value[field] = "override".into();
        assert!(serde_json::from_value::<SendParams>(value).is_err());
    }
}

#[test]
fn worker_model_selection_uses_installed_harnesses_and_project_catalogs() {
    let profiles = crate::agents::WorkerProfiles::default();
    let project = std::path::Path::new("/project");
    let backends = vec!["pi".to_owned()];
    let catalog = crate::app::persistence::CachedConfigurationCatalog {
        harness: "pi".into(),
        project: project.into(),
        catalog: crate::agents::ConfigurationCatalog {
            models: vec![crate::protocol::Model {
                id: "gpt-5.6-luna".into(),
                name: "Luna".into(),
                provider: "openai-codex".into(),
                context_window: 0,
                reasoning: true,
                access_modes: None,
                efforts: Some(vec!["high".into()]),
            }],
            efforts: vec![],
        },
    };
    let catalogs = [catalog];
    let assignment = profiles
        .resolve("cheap", |model| {
            model_available(model, project, &backends, &catalogs)
        })
        .expect("test operation should succeed");
    assert_eq!(assignment.execution.provider, "openai-codex");
    assert_eq!(assignment.execution.model, "gpt-5.6-luna");
    assert!(
        profiles
            .resolve("oracle", |model| model_available(
                model, project, &backends, &catalogs
            ))
            .is_err()
    );
    assert!(!model_available(
        &assignment.execution,
        project,
        &[],
        &catalogs
    ));
    let preferred_pi = &profiles.profiles[3].models[1];
    assert!(!model_available(
        preferred_pi,
        project,
        &backends,
        &catalogs
    ));
    assert!(model_available(
        preferred_pi,
        std::path::Path::new("/other"),
        &backends,
        &catalogs
    ));
    assert!(model_available(preferred_pi, project, &backends, &[]));
}
