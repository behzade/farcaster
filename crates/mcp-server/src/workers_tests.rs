use super::*;
use crate::agents::Backend;
use crate::agents::{CallerIdentity, CallerProfile};
use crate::agents::{WorkerEvent, WorkerLaunch, WorkerSession, WorkerSessionFactory};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

struct Factory {
    launches: Arc<Mutex<Vec<WorkerLaunch>>>,
}
struct Session(CallerIdentity);
impl WorkerSessionFactory for Factory {
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
        std::collections::BTreeMap::from([(Backend::Codex, factory)]),
        Backend::Codex,
        temp.path().to_owned(),
        1,
    )?;
    let registry = CallerRegistry::shared();
    let parent = registry.issue_with_access(
        temp.path(),
        CallerProfile {
            backend: Backend::Pi,
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
        super::send(pool, params, token, profiles, |model, _, mode| {
            (model.harness == Backend::Codex).then_some(mode)
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
    assert_eq!(result["pending"], true);
    assert_eq!(result["assignment"]["execution"]["harness"], "codex-cli");
    wait_for_launches(&launches, 1)?;
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
        assert_eq!(launch.access_mode, crate::agents::HarnessAccessMode::Auto);
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

struct BlockingFactory {
    creates: Arc<AtomicUsize>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    messages: Arc<Mutex<Vec<(String, crate::agents::WorkerSendMode)>>>,
}

struct BlockingSession {
    identity: CallerIdentity,
    messages: Arc<Mutex<Vec<(String, crate::agents::WorkerSendMode)>>>,
}

impl WorkerSessionFactory for BlockingFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        let (blocked, changed) = &*self.gate;
        let mut blocked = blocked.lock().map_err(|_| "setup gate")?;
        while *blocked {
            blocked = changed.wait(blocked).map_err(|_| "setup gate")?;
        }
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
            launch.worker_name,
            launch.parent_worker_id,
            launch.access_mode,
        )?;
        identity.bind(format!("session-{}", launch.worker_id));
        Ok(Box::new(BlockingSession {
            identity,
            messages: self.messages.clone(),
        }))
    }
}

impl WorkerSession for BlockingSession {
    fn send(&mut self, message: String, mode: crate::agents::WorkerSendMode) -> Result<(), String> {
        self.messages
            .lock()
            .map_err(|_| "messages")?
            .push((message, mode));
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
        None
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn worker_send_retry_reuses_a_pending_named_child_reservation() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let creates = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new((Mutex::new(true), Condvar::new()));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let factory: Arc<dyn WorkerSessionFactory> = Arc::new(BlockingFactory {
        creates: creates.clone(),
        gate: gate.clone(),
        messages: messages.clone(),
    });
    let pool = WorkerPool::new(
        std::collections::BTreeMap::from([(Backend::Codex, factory)]),
        Backend::Codex,
        temp.path().to_owned(),
        1,
    )?;
    let parent = CallerRegistry::shared().issue(
        temp.path(),
        CallerProfile {
            backend: Backend::Pi,
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("pending-parent");
    let token = Some(parent.token().to_owned());
    let profiles = crate::agents::WorkerProfiles::default();
    let send = |message: &str, profile| {
        super::send(
            &pool,
            SendParams {
                to: Some("slow-child".into()),
                message: message.into(),
                profile,
            },
            token.clone(),
            &profiles,
            |model, _, mode| (model.harness == Backend::Codex).then_some(mode),
        )
    };

    let first = send("inspect", Some("oracle".into()))?;
    assert_eq!(first["created"], true);
    assert_eq!(first["pending"], true);
    wait_until("factory setup", || creates.load(Ordering::SeqCst) == 1)?;
    let retry = send("inspect", None)?;
    assert_eq!(retry["created"], false);
    assert_eq!(retry["pending"], true);
    assert_eq!(retry["assignment"]["profile"], "oracle");
    assert_eq!(creates.load(Ordering::SeqCst), 1);
    let follow_up = send("also inspect tests", None)?;
    assert_eq!(follow_up["pending"], true);

    let (blocked, changed) = &*gate;
    *blocked.lock().map_err(|_| "setup gate")? = false;
    changed.notify_all();
    wait_until("worker running", || {
        pool.snapshots().is_ok_and(|snapshots| {
            snapshots
                .iter()
                .any(|snapshot| snapshot.status == crate::agents::WorkerStatus::Running)
        })
    })?;
    assert_eq!(creates.load(Ordering::SeqCst), 1);
    let messages = messages.lock().map_err(|_| "messages")?;
    assert_eq!(
        messages.len(),
        3,
        "every acknowledged message must be delivered"
    );
    assert_eq!(messages[0].1, crate::agents::WorkerSendMode::Prompt);
    assert_eq!(messages[1].1, crate::agents::WorkerSendMode::Queue);
    assert!(messages[1].0.contains("inspect"));
    assert_eq!(messages[2].1, crate::agents::WorkerSendMode::Queue);
    assert!(messages[2].0.contains("also inspect tests"));
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
        std::collections::BTreeMap::from([(Backend::Codex, factory)]),
        Backend::Codex,
        temp.path().to_owned(),
        1,
    )?;
    let registry = CallerRegistry::shared();
    let parent = registry.issue_with_access(
        temp.path(),
        CallerProfile {
            backend: Backend::Codex,
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
            backend: Backend::Codex,
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
            harness: Backend::Codex,
            provider: "openai".into(),
            model: "test-model".into(),
            effort: None,
        },
    };

    pool.start_assigned(
        new_worker(
            child,
            "grandchild".into(),
            "work".into(),
            &assignment,
            crate::agents::HarnessAccessMode::Full,
        ),
        Some(assignment),
    )?;
    assert_eq!(
        launches.lock().map_err(|_| "launches")?[0].access_mode,
        crate::agents::HarnessAccessMode::Full
    );
    Ok(())
}

#[test]
fn restricted_parent_cannot_reuse_a_running_full_child_after_session_rebind() -> Result<(), String>
{
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let launches = Arc::new(Mutex::new(Vec::new()));
    let factory: Arc<dyn WorkerSessionFactory> = Arc::new(Factory { launches });
    let pool = WorkerPool::new(
        std::collections::BTreeMap::from([(Backend::Codex, factory)]),
        Backend::Codex,
        temp.path().to_owned(),
        1,
    )?;
    let registry = CallerRegistry::shared();
    let profile = CallerProfile {
        backend: Backend::Codex,
        provider: None,
        model: None,
        effort: None,
    };
    let full_parent = registry.issue_with_access(
        temp.path(),
        profile.clone(),
        None,
        crate::agents::HarnessAccessMode::Full,
    );
    full_parent.bind("rebound-parent-session");
    let full_parent_context = registry.resolve(full_parent.token())?;
    let child = registry.issue_as_with_access(
        temp.path(),
        profile.clone(),
        None,
        "full-child-id".into(),
        "full-child".into(),
        Some(full_parent_context.worker_id),
        crate::agents::HarnessAccessMode::Full,
    )?;
    child.bind("full-child-session");
    registry.set_assignment(
        "full-child-id",
        crate::agents::WorkerAssignment {
            profile: "oracle".into(),
            execution: crate::agents::WorkerExecution {
                harness: Backend::Codex,
                provider: "openai".into(),
                model: "model".into(),
                effort: None,
            },
        },
    )?;

    let restricted_parent = registry.issue_with_access(
        temp.path(),
        profile,
        None,
        crate::agents::HarnessAccessMode::Sandboxed,
    );
    restricted_parent.bind("rebound-parent-session");
    let error = super::send(
        &pool,
        SendParams {
            to: Some("full-child".into()),
            message: "do not deliver".into(),
            profile: None,
        },
        Some(restricted_parent.token().into()),
        &crate::agents::WorkerProfiles::default(),
        |model, _, mode| (model.harness == Backend::Codex).then_some(mode),
    )
    .expect_err("restricted parent must not reuse a Full child");
    assert!(error.contains("restricted parent cannot reuse"), "{error}");
    assert!(
        child.try_recv().is_none(),
        "message reached unrestricted child"
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
    let pi = factories.remove(&Backend::Pi).ok_or("Pi factory missing")?;
    let pool = WorkerPool::new(
        std::collections::BTreeMap::from([(Backend::Pi, pi)]),
        Backend::Pi,
        temp.path().to_owned(),
        1,
    )?;
    let parent = CallerRegistry::shared().issue_with_access(
        temp.path(),
        CallerProfile {
            backend: Backend::Codex,
            provider: None,
            model: None,
            effort: None,
        },
        None,
        crate::agents::HarnessAccessMode::Sandboxed,
    );
    parent.bind("codex-parent");
    let result = super::send(
        &pool,
        SendParams {
            to: Some("pi-child".into()),
            message: "work".into(),
            profile: Some("oracle".into()),
        },
        Some(parent.token().into()),
        &crate::agents::WorkerProfiles::default(),
        |model, _, mode| (model.harness == Backend::Pi).then_some(mode),
    )?;
    assert_eq!(result["pending"], true);
    let failure = wait_worker_failed(&pool)?;
    let error = failure.error.ok_or("worker failure missing error")?;
    assert!(
        error.contains("cannot confirm the requested access mode"),
        "{error}"
    );
    let report = parent
        .try_recv()
        .ok_or("missing async setup failure report")?;
    assert!(report.message.contains("Worker failed to start"));
    assert!(
        report
            .message
            .contains("cannot confirm the requested access mode")
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
        std::collections::BTreeMap::from([(Backend::Codex, factory)]),
        Backend::Codex,
        temp.path().to_owned(),
        1,
    )?;
    let registry = CallerRegistry::shared();
    let parent = registry.issue(
        temp.path(),
        CallerProfile {
            backend: Backend::Pi,
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
            |model, _, mode| (model.harness == Backend::Codex).then_some(mode),
        )?;
        assert_eq!(result["created"], true);
        wait_for_event_senders(&events, index + 1)?;
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
        |model, _, mode| (model.harness == Backend::Codex).then_some(mode),
    )?;
    assert_eq!(result["created"], false);
    assert_eq!(result["pending"], true);
    wait_for_launches(&launches, 10)?;
    assert!(matches!(
        &launches.lock().map_err(|_| "launches")?.last().ok_or("resume launch")?.context,
        WorkerContext::Resume { session_locator }
            if session_locator == "/sessions/child-0.jsonl"
    ));
    Ok(())
}

fn wait_for_launches(launches: &Mutex<Vec<WorkerLaunch>>, expected: usize) -> Result<(), String> {
    wait_until("worker launch", || {
        launches
            .lock()
            .is_ok_and(|launches| launches.len() >= expected)
    })
}

fn wait_for_event_senders(
    events: &Mutex<Vec<mpsc::Sender<WorkerEvent>>>,
    expected: usize,
) -> Result<(), String> {
    wait_until("worker event sender", || {
        events.lock().is_ok_and(|events| events.len() >= expected)
    })
}

fn wait_worker_failed(pool: &WorkerPool) -> Result<crate::agents::WorkerSnapshot, String> {
    let mut failed = None;
    wait_until("worker failure", || {
        failed = pool.snapshots().ok().and_then(|snapshots| {
            snapshots
                .into_iter()
                .find(|snapshot| snapshot.status == crate::agents::WorkerStatus::Failed)
        });
        failed.is_some()
    })?;
    failed.ok_or("worker failure missing".into())
}

fn wait_until(label: &str, mut ready: impl FnMut() -> bool) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !ready() {
        if std::time::Instant::now() >= deadline {
            return Err(format!("timed out waiting for {label}"));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Ok(())
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
    let backends = vec![Backend::Pi];
    let catalog = crate::storage::CachedConfigurationCatalog {
        harness: Backend::Pi,
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
            sandbox_adapter: None,
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

#[test]
fn auto_parent_prefers_an_auto_candidate_within_the_profile() {
    let mut profiles = crate::agents::WorkerProfiles::default();
    profiles.profiles[0].models = vec![
        crate::agents::WorkerExecution {
            harness: Backend::OpenCode,
            provider: "openai".into(),
            model: "sandboxed".into(),
            effort: None,
        },
        crate::agents::WorkerExecution {
            harness: Backend::Codex,
            provider: "openai".into(),
            model: "auto".into(),
            effort: None,
        },
    ];
    let profile_name = profiles.profiles[0].name.clone();

    let (assignment, mode) = resolve_child(
        &profiles,
        &profile_name,
        std::path::Path::new("/project"),
        crate::agents::HarnessAccessMode::Auto,
        |model, _, _| match model.harness {
            Backend::OpenCode => Some(crate::agents::HarnessAccessMode::Sandboxed),
            Backend::Codex => Some(crate::agents::HarnessAccessMode::Auto),
            _ => None,
        },
    )
    .expect("an Auto-capable candidate should be selected");

    assert_eq!(assignment.execution.harness, Backend::Codex);
    assert_eq!(mode, crate::agents::HarnessAccessMode::Auto);
}

#[test]
fn sandboxed_pi_parent_prefers_an_auto_cursor_child() {
    let mut profiles = crate::agents::WorkerProfiles::default();
    profiles.profiles[0].models = vec![
        crate::agents::WorkerExecution {
            harness: Backend::OpenCode,
            provider: "openai".into(),
            model: "sandboxed".into(),
            effort: None,
        },
        crate::agents::WorkerExecution {
            harness: Backend::Cursor,
            provider: "cursor-cli".into(),
            model: "auto".into(),
            effort: None,
        },
    ];
    let profile_name = profiles.profiles[0].name.clone();
    let requested = delegated_access_mode(Backend::Pi, crate::agents::HarnessAccessMode::Sandboxed);

    let (assignment, mode) = resolve_child(
        &profiles,
        &profile_name,
        std::path::Path::new("/project"),
        requested,
        |model, _, _| match model.harness {
            Backend::OpenCode => Some(crate::agents::HarnessAccessMode::Sandboxed),
            Backend::Cursor => Some(crate::agents::HarnessAccessMode::Auto),
            _ => None,
        },
    )
    .expect("Cursor Auto should be preferred for a sandboxed Pi parent");

    assert_eq!(assignment.execution.harness, Backend::Cursor);
    assert_eq!(mode, crate::agents::HarnessAccessMode::Auto);
}

#[test]
fn auto_parent_degrades_to_sandboxed_when_no_auto_candidate_exists() {
    let mut profiles = crate::agents::WorkerProfiles::default();
    profiles.profiles[0].models = vec![crate::agents::WorkerExecution {
        harness: Backend::OpenCode,
        provider: "openai".into(),
        model: "sandboxed".into(),
        effort: None,
    }];
    let profile_name = profiles.profiles[0].name.clone();

    let (assignment, mode) = resolve_child(
        &profiles,
        &profile_name,
        std::path::Path::new("/project"),
        crate::agents::HarnessAccessMode::Auto,
        |_, _, _| Some(crate::agents::HarnessAccessMode::Sandboxed),
    )
    .expect("a sandboxed candidate should be used as the fallback");

    assert_eq!(assignment.execution.harness, Backend::OpenCode);
    assert_eq!(mode, crate::agents::HarnessAccessMode::Sandboxed);
}

#[test]
fn restricted_parent_never_routes_to_unsandboxed_pi() {
    let project = std::path::Path::new("/project");
    let auto = crate::agents::HarnessAccessMode::Auto;
    let sandboxed = crate::agents::HarnessAccessMode::Sandboxed;
    let pi = crate::agents::WorkerExecution {
        harness: Backend::Pi,
        provider: "openai".into(),
        model: "model".into(),
        effort: None,
    };
    let opencode = crate::agents::WorkerExecution {
        harness: Backend::OpenCode,
        provider: "openai".into(),
        model: "model".into(),
        effort: None,
    };

    assert_eq!(
        child_access_mode(&pi, project, auto, &[Backend::Pi], &[]),
        None
    );
    assert_eq!(
        child_access_mode(&pi, project, sandboxed, &[Backend::Pi], &[]),
        None
    );
    assert_eq!(
        child_access_mode(&opencode, project, auto, &[Backend::OpenCode], &[]),
        Some(sandboxed)
    );
    assert_eq!(
        child_access_mode(
            &pi,
            project,
            crate::agents::HarnessAccessMode::Full,
            &[Backend::Pi],
            &[],
        ),
        Some(crate::agents::HarnessAccessMode::Full),
        "only a Full parent may route to an unsandboxed Pi child"
    );

    let mut profiles = crate::agents::WorkerProfiles::default();
    profiles.profiles[0].models = vec![pi];
    let profile_name = profiles.profiles[0].name.clone();
    assert!(
        resolve_child(
            &profiles,
            &profile_name,
            project,
            auto,
            |model, project, mode| { child_access_mode(model, project, mode, &[Backend::Pi], &[]) }
        )
        .is_err(),
        "creation must fail before launching when no protected candidate exists"
    );
}

#[test]
fn auto_parent_can_route_to_pi_when_its_sandbox_adapter_is_configured() {
    let project = std::path::Path::new("/project");
    let pi = crate::agents::WorkerExecution {
        harness: Backend::Pi,
        provider: "openai".into(),
        model: "model".into(),
        effort: None,
    };
    let catalogs = [crate::storage::CachedConfigurationCatalog {
        harness: Backend::Pi,
        project: project.into(),
        catalog: crate::agents::ConfigurationCatalog {
            models: vec![crate::protocol::Model {
                id: "model".into(),
                name: "Model".into(),
                provider: "openai".into(),
                context_window: 0,
                reasoning: false,
                resolved_model: None,
                access_modes: None,
                efforts: None,
            }],
            efforts: Vec::new(),
            sandbox_adapter: Some("pi-nono".into()),
        },
    }];

    assert_eq!(
        child_access_mode(
            &pi,
            project,
            crate::agents::HarnessAccessMode::Auto,
            &[Backend::Pi],
            &catalogs,
        ),
        Some(crate::agents::HarnessAccessMode::Sandboxed)
    );
}
