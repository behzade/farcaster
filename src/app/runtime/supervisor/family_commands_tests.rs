use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime},
};

use super::*;
use crate::agents::{
    HarnessAccessMode, StartWorker, WorkerContext, WorkerEvent, WorkerInputResponse, WorkerLaunch,
    WorkerPool, WorkerSendMode, WorkerSession, WorkerSessionFactory, WorkerStatus,
};
use crate::sessions::UsageSummary;
use serde_json::json;

#[derive(Default)]
struct LifecycleFactory {
    aborts: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    fail_close: Arc<AtomicBool>,
    close_gate: Arc<(Mutex<bool>, Condvar)>,
}

struct LifecycleSession {
    aborts: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    fail_close: Arc<AtomicBool>,
    close_gate: Arc<(Mutex<bool>, Condvar)>,
}

impl WorkerSessionFactory for LifecycleFactory {
    fn create(&self, _launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        Ok(Box::new(LifecycleSession {
            aborts: self.aborts.clone(),
            closes: self.closes.clone(),
            drops: self.drops.clone(),
            fail_close: self.fail_close.clone(),
            close_gate: self.close_gate.clone(),
        }))
    }
}

impl WorkerSession for LifecycleSession {
    fn send(&mut self, _message: String, _mode: WorkerSendMode) -> Result<(), String> {
        Ok(())
    }

    fn respond(&mut self, _response: WorkerInputResponse) -> Result<(), String> {
        Ok(())
    }

    fn abort(&mut self) -> Result<(), String> {
        self.aborts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        None
    }

    fn close(&mut self) -> Result<(), String> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        let (gate, changed) = &*self.close_gate;
        let mut blocked = gate.lock().map_err(|_| "close gate")?;
        while *blocked {
            blocked = changed.wait(blocked).map_err(|_| "close gate")?;
        }
        if self.fail_close.load(Ordering::SeqCst) {
            Err("gated close failed".into())
        } else {
            Ok(())
        }
    }
}

impl Drop for LifecycleSession {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn summary(project: &Path, id: &str, parent: Option<&str>) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        project.join(format!("{id}.jsonl")),
        project.to_owned(),
        id.into(),
        String::new(),
        String::new(),
        parent.map(str::to_owned),
        SystemTime::now(),
        0,
        UsageSummary::default(),
        false,
        true,
        String::new(),
    )
}

fn lifecycle_pool(project: &Path, factory: Arc<LifecycleFactory>) -> Result<WorkerPool, String> {
    let factory: Arc<dyn WorkerSessionFactory> = factory;
    WorkerPool::new(
        BTreeMap::from([("pi".into(), factory)]),
        "pi".into(),
        project.to_owned(),
        4,
    )
}

fn start_worker(
    pool: &WorkerPool,
    project: &Path,
    name: &str,
    parent: &Path,
) -> Result<(), String> {
    pool.start_assigned(
        StartWorker {
            project: project.to_owned(),
            name: name.into(),
            prompt: "stay alive".into(),
            backend: "pi".into(),
            parent_session: parent.to_string_lossy().into_owned(),
            parent_worker_id: None,
            context: WorkerContext::Fresh,
            provider: None,
            model: None,
            effort: None,
            access_mode: HarnessAccessMode::Auto,
        },
        None,
    )?;
    Ok(())
}

fn supervisor_for_family(
    state: StateStore,
    sessions: Vec<SessionSummary>,
) -> (Supervisor, mpsc::Receiver<RuntimeEvent>) {
    let (_commands, command_rx) = mpsc::channel();
    let (events_tx, events) = mpsc::channel();
    let (wake, _) = async_channel::bounded(1);
    let (_, configuration_rx) = mpsc::channel();
    (
        Supervisor {
            process_command: AgentLaunchConfig::default(),
            command_rx,
            event_tx: UiEventSender {
                events: events_tx,
                wake,
            },
            supervisor_thread: thread::current(),
            catalog_key: "catalog".into(),
            actors: HashMap::new(),
            selected: "catalog".into(),
            selected_project: sessions[0].project.clone(),
            selected_session: None,
            generation: 0,
            latest: HashMap::new(),
            catalog_sessions: sessions,
            catalog_generation: 7,
            actor_paths: HashMap::new(),
            failed_actor_shutdowns: HashMap::new(),
            interacted: HashSet::new(),
            document_revisions: HashMap::new(),
            pending_extensions: HashMap::new(),
            active_dialogs: HashMap::new(),
            needs_input: HashSet::new(),
            clock: 0,
            last_touch: HashMap::new(),
            configurations: HarnessConfigurationStore::default(),
            catalog_state: Some(state),
            configuration_catalogs: Vec::new(),
            configuration_rx,
            configuration_tx: None,
            configuration_requests: HashSet::new(),
            published_statuses: HashMap::new(),
            recovery: Default::default(),
            published_recovery_selection: None,
        },
        events,
    )
}

fn archived(database: &Path, path: &Path) -> Result<bool, String> {
    let state = StateStore::open_at(database)?;
    let path = crate::sessions::normalize_session_path(path);
    Ok(state
        .cached_sessions("")?
        .into_iter()
        .find(|session| session.path == path)
        .ok_or_else(|| format!("missing session {}", path.display()))?
        .archived)
}

fn wait_for(counter: &AtomicUsize, expected: usize) {
    for _ in 0..100 {
        if counter.load(Ordering::SeqCst) >= expected {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("counter did not reach {expected}");
}

#[test]
fn retrying_actor_blocks_destructive_family_commands() {
    let mut snapshot = RuntimeSnapshot::default();
    assert!(!session_actor_has_active_work(&snapshot, false));
    Arc::make_mut(&mut snapshot.conversation).reduce(&json!({
        "type": "auto_retry_start",
        "attempt": 1,
    }));

    assert!(!snapshot.conversation.running);
    assert!(snapshot.conversation.retrying);
    assert!(session_actor_has_active_work(&snapshot, false));
}

#[test]
fn supervisor_waits_for_pool_shutdown_before_archiving_and_leaves_other_families_alive()
-> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let root = summary(temp.path(), "root", None);
    let child = summary(temp.path(), "child", Some("root"));
    let unrelated = summary(temp.path(), "unrelated", None);
    let mut state = StateStore::open_at(&database)?;
    state.replace_sessions(&[root.clone(), child.clone(), unrelated.clone()])?;

    let factory = Arc::new(LifecycleFactory::default());
    *factory.close_gate.0.lock().map_err(|_| "close gate")? = true;
    let pool = lifecycle_pool(temp.path(), factory.clone())?;
    start_worker(&pool, temp.path(), "family-child", &root.path)?;
    start_worker(&pool, temp.path(), "other-child", &unrelated.path)?;
    let (supervisor, events) =
        supervisor_for_family(state, vec![root.clone(), child, unrelated.clone()]);

    crate::app::mcp_server::with_test_worker_pool(pool.clone(), || {
        let path = root.path.clone();
        let handle = thread::spawn(move || {
            let mut supervisor = supervisor;
            assert!(
                supervisor
                    .handle_session_family_command(&RuntimeCommand::StopSessionFamily { path })
            );
            supervisor
        });
        wait_for(&factory.closes, 1);
        assert!(!archived(&database, &root.path)?);
        assert!(events.try_iter().all(|event| !matches!(
            event,
            RuntimeEvent::SessionStatus { ref status, .. } if status == "Stopped"
        )));
        assert_eq!(factory.aborts.load(Ordering::SeqCst), 1);

        *factory.close_gate.0.lock().map_err(|_| "close gate")? = false;
        factory.close_gate.1.notify_all();
        let supervisor = handle.join().map_err(|_| "supervisor test panicked")?;
        assert!(archived(&database, &root.path)?);
        assert_eq!(factory.aborts.load(Ordering::SeqCst), 1);
        assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
        assert_eq!(factory.drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            pool.snapshots()?
                .into_iter()
                .filter(|worker| worker.status == WorkerStatus::Running)
                .count(),
            1,
            "an unrelated family must remain usable after the target shutdown"
        );
        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::SessionStatus { ref status, .. } if status == "Stopped"
        )));
        assert!(
            supervisor
                .catalog_sessions
                .iter()
                .find(|session| session.path == root.path)
                .is_some_and(|session| session.archived)
        );

        pool.stop_session_family(temp.path(), &[("pi".into(), unrelated.path.clone())])?;
        Ok(())
    })
}

#[test]
fn supervisor_does_not_archive_or_report_stopped_when_pool_close_fails() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let root = summary(temp.path(), "root", None);
    let mut state = StateStore::open_at(&database)?;
    state.replace_sessions(std::slice::from_ref(&root))?;
    let factory = Arc::new(LifecycleFactory::default());
    factory.fail_close.store(true, Ordering::SeqCst);
    let pool = lifecycle_pool(temp.path(), factory.clone())?;
    start_worker(&pool, temp.path(), "family-child", &root.path)?;
    let (mut supervisor, events) = supervisor_for_family(state, vec![root.clone()]);

    crate::app::mcp_server::with_test_worker_pool(pool, || {
        assert!(
            supervisor.handle_session_family_command(&RuntimeCommand::StopSessionFamily {
                path: root.path.clone(),
            })
        );
        assert!(!archived(&database, &root.path)?);
        let first_events = events.try_iter().collect::<Vec<_>>();
        assert!(first_events.iter().any(|event| matches!(
            event,
            RuntimeEvent::SessionsFailed { message, .. } if message.contains("gated close failed")
        )));
        assert!(first_events.iter().all(|event| !matches!(
            event,
            RuntimeEvent::SessionStatus { status, .. } if status == "Stopped"
        )));
        assert_eq!(factory.aborts.load(Ordering::SeqCst), 1);
        assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
        assert_eq!(factory.drops.load(Ordering::SeqCst), 1);
        Ok(())
    })
}

#[test]
fn supervisor_does_not_archive_or_report_stopped_when_actor_close_fails() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let root = summary(temp.path(), "root", None);
    let mut state = StateStore::open_at(&database)?;
    state.replace_sessions(std::slice::from_ref(&root))?;
    let pool = lifecycle_pool(temp.path(), Arc::new(LifecycleFactory::default()))?;
    let (mut supervisor, events) = supervisor_for_family(state, vec![root.clone()]);
    let key = format!("session:{}", root.path.display());
    let (commands, _command_rx) = mpsc::channel();
    let (_event_tx, actor_events) = mpsc::channel();
    let join = thread::spawn(|| Err("actor transport close failed".to_owned()));
    supervisor
        .actor_paths
        .insert(root.path.clone(), key.clone());
    supervisor.actors.insert(
        key,
        SessionRuntimeHandle {
            commands,
            events: actor_events,
            thread: join.thread().clone(),
            join,
        },
    );

    crate::app::mcp_server::with_test_worker_pool(pool, || {
        assert!(
            supervisor.handle_session_family_command(&RuntimeCommand::StopSessionFamily {
                path: root.path.clone(),
            })
        );
        assert!(!archived(&database, &root.path)?);
        let first_events = events.try_iter().collect::<Vec<_>>();
        assert!(first_events.iter().any(|event| matches!(
            event,
            RuntimeEvent::SessionsFailed { message, .. }
                if message.contains("actor transport close failed")
        )));
        assert!(first_events.iter().all(|event| !matches!(
            event,
            RuntimeEvent::SessionStatus { status, .. } if status == "Stopped"
        )));
        assert!(
            supervisor.handle_session_family_command(&RuntimeCommand::StopSessionFamily {
                path: root.path.clone(),
            })
        );
        assert!(!archived(&database, &root.path)?);
        let retry_events = events.try_iter().collect::<Vec<_>>();
        assert!(retry_events.iter().any(|event| matches!(
            event,
            RuntimeEvent::SessionsFailed { message, .. }
                if message.contains("actor transport close failed")
        )));
        assert!(retry_events.iter().all(|event| !matches!(
            event,
            RuntimeEvent::SessionStatus { status, .. } if status == "Stopped"
        )));
        Ok(())
    })
}
