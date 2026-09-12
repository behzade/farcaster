use super::*;
use crate::agents::{CallerIdentity, CallerProfile};
use crate::agents::{WorkerEvent, WorkerLaunch, WorkerSession, WorkerSessionFactory};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

struct Factory {
    launches: Arc<Mutex<Vec<WorkerLaunch>>>,
}
struct Session(CallerIdentity);
impl WorkerSessionFactory for Factory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        let identity = CallerRegistry::shared().issue_as_with_access(
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
            launch.access_mode,
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
    let parent = registry.issue_with_access(
        temp.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: Some("parent-provider".into()),
            model: Some("expensive-parent".into()),
            effort: Some("max".into()),
        },
        None,
        crate::agents::HarnessAccessMode::Sandboxed,
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
        assert_eq!(
            launch.access_mode,
            crate::agents::HarnessAccessMode::Sandboxed
        );
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
fn nested_parent_policy_reaches_the_grandchild_factory_launch() -> Result<(), String> {
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
    let parent = registry.issue_with_access(
        temp.path(),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
        crate::agents::HarnessAccessMode::Full,
    );
    parent.bind("parent-session");
    let parent = registry.resolve(parent.token())?;
    let child = registry.issue_as_with_access(
        temp.path(),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
        "nested-parent-id".into(),
        "nested-parent".into(),
        Some(parent.worker_id),
        parent.access_mode,
    )?;
    child.bind("nested-parent-session");
    let child = registry.resolve(child.token())?;
    let assignment = crate::agents::WorkerAssignment {
        profile: "nested".into(),
        execution: crate::agents::WorkerExecution {
            harness: "codex-cli".into(),
            provider: "openai".into(),
            model: "test-model".into(),
            effort: None,
        },
    };

    pool.start_assigned(
        new_worker(child, "grandchild".into(), "work".into(), &assignment),
        Some(assignment),
    )?;
    assert_eq!(
        launches.lock().map_err(|_| "launches")?[0].access_mode,
        crate::agents::HarnessAccessMode::Full
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn restrictive_cross_backend_launch_errors_instead_of_using_auto() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let script = temp.path().join("fake-pi.sh");
    std::fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))
        .map_err(|error| error.to_string())?;
    let command = crate::agents::AgentLaunchConfig::test_script(&script, vec!["normal".into()]);
    let (mut factories, _) = crate::agents::worker_factories(command);
    let pi = factories.remove("pi").ok_or("Pi factory missing")?;
    let pool = WorkerPool::new(
        std::collections::BTreeMap::from([("pi".into(), pi)]),
        "pi".into(),
        temp.path().to_owned(),
        1,
    )?;
    let parent = CallerRegistry::shared().issue_with_access(
        temp.path(),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
        crate::agents::HarnessAccessMode::Sandboxed,
    );
    parent.bind("codex-parent");
    let error = super::send(
        &pool,
        SendParams {
            to: Some("pi-child".into()),
            message: "work".into(),
            profile: Some("oracle".into()),
        },
        Some(parent.token().into()),
        &crate::agents::WorkerProfiles::default(),
        |model, _| model.harness == "pi",
    )
    .expect_err("unsupported restrictive launch must fail");
    assert!(
        error.contains("cannot confirm the requested access mode"),
        "{error}"
    );
    Ok(())
}

struct RetiringFactory {
    launches: Arc<Mutex<Vec<WorkerLaunch>>>,
    events: Arc<Mutex<Vec<mpsc::Sender<WorkerEvent>>>>,
    closes: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

struct RetiringSession {
    identity: CallerIdentity,
    events: mpsc::Receiver<WorkerEvent>,
    closes: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl WorkerSessionFactory for RetiringFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        let identity = CallerRegistry::shared().issue_as_with_access(
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
            launch.access_mode,
        )?;
        let locator = match &launch.context {
            WorkerContext::Fresh => format!("/sessions/{}.jsonl", launch.worker_name),
            WorkerContext::Session { session_locator }
            | WorkerContext::Resume { session_locator } => session_locator.clone(),
        };
        identity.bind(locator);
        let (events, receiver) = mpsc::channel();
        self.events.lock().map_err(|_| "events")?.push(events);
        self.launches.lock().map_err(|_| "launches")?.push(launch);
        Ok(Box::new(RetiringSession {
            identity,
            events: receiver,
            closes: self.closes.clone(),
            drops: self.drops.clone(),
        }))
    }
}

impl WorkerSession for RetiringSession {
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
        let _ = self.identity.token();
        self.events.try_recv().ok()
    }
    fn close(&mut self) -> Result<(), String> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for RetiringSession {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn worker_send_resumes_a_named_child_after_idle_process_retirement() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let launches = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let closes = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let factory: Arc<dyn WorkerSessionFactory> = Arc::new(RetiringFactory {
        launches: launches.clone(),
        events: events.clone(),
        closes: closes.clone(),
        drops: drops.clone(),
    });
    let pool = WorkerPool::new(
        std::collections::BTreeMap::from([("codex-cli".into(), factory)]),
        "codex-cli".into(),
        temp.path().to_owned(),
        1,
    )?;
    let updates = pool.updates();
    let registry = CallerRegistry::shared();
    let parent = registry.issue(
        temp.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("/sessions/parent.jsonl");
    let token = Some(parent.token().to_owned());
    let tasks = crate::agents::WorkerProfiles::default();

    for index in 0..9 {
        let result = super::send(
            &pool,
            SendParams {
                to: Some(format!("child-{index}")),
                message: "first turn".into(),
                profile: Some("oracle".into()),
            },
            token.clone(),
            &tasks,
            |model, _| model.harness == "codex-cli",
        )?;
        assert_eq!(result["created"], true);
        wait_worker_update(&updates)?;
        let event = events.lock().map_err(|_| "events")?[index].clone();
        event
            .send(WorkerEvent::SessionChanged {
                locator: format!("/sessions/child-{index}.jsonl"),
            })
            .map_err(|_| "session")?;
        event
            .send(WorkerEvent::Settled {
                output: "done".into(),
            })
            .map_err(|_| "settle")?;
        wait_worker_idle(&pool)?;
    }
    assert_eq!(closes.load(Ordering::SeqCst), 8);
    assert_eq!(drops.load(Ordering::SeqCst), 8);

    let result = super::send(
        &pool,
        SendParams {
            to: Some("child-0".into()),
            message: "second turn".into(),
            profile: None,
        },
        token,
        &tasks,
        |model, _| model.harness == "codex-cli",
    )?;
    assert_eq!(result["created"], false);
    assert!(matches!(
        &launches.lock().map_err(|_| "launches")?.last().ok_or("resume launch")?.context,
        WorkerContext::Resume { session_locator }
            if session_locator == "/sessions/child-0.jsonl"
    ));
    Ok(())
}

fn wait_worker_update(updates: &async_channel::Receiver<()>) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match updates.try_recv() {
            Ok(()) => return Ok(()),
            Err(async_channel::TryRecvError::Empty) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(error) => return Err(format!("worker update missing: {error}")),
        }
    }
}

fn wait_worker_idle(pool: &WorkerPool) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        if pool
            .snapshots()?
            .last()
            .is_some_and(|snapshot| snapshot.status == crate::agents::WorkerStatus::Idle)
        {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("worker did not settle".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
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
                resolved_model: None,
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
