use crate::agents::Backend;
use std::{
    collections::BTreeMap,
    fs,
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

struct CloseGateGuard(Arc<(Mutex<bool>, Condvar)>);

impl CloseGateGuard {
    fn block(factory: &LifecycleFactory) -> Self {
        *factory
            .close_gate
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
        Self(factory.close_gate.clone())
    }
}

impl Drop for CloseGateGuard {
    fn drop(&mut self) {
        *self.0.0.lock().unwrap_or_else(|error| error.into_inner()) = false;
        self.0.1.notify_all();
    }
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
        BTreeMap::from([(Backend::Pi, factory)]),
        Backend::Pi,
        project.to_owned(),
        4,
    )
}

fn start_worker(
    pool: &WorkerPool,
    project: &Path,
    name: &str,
    parent: &Path,
) -> Result<crate::agents::CallerIdentity, String> {
    let identity = crate::agents::CallerRegistry::shared().issue(
        project,
        crate::agents::CallerProfile {
            backend: Backend::Pi,
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    identity.bind(parent.to_string_lossy().into_owned());
    let context = crate::agents::CallerRegistry::shared().resolve(identity.token())?;
    pool.start_assigned(
        StartWorker {
            project: project.to_owned(),
            name: name.into(),
            prompt: "stay alive".into(),
            backend: Backend::Pi,
            parent_session: parent.to_string_lossy().into_owned(),
            parent_worker_id: Some(context.worker_id),
            context: WorkerContext::Fresh,
            provider: None,
            model: None,
            effort: None,
            access_mode: HarnessAccessMode::Auto,
        },
        None,
    )?;
    Ok(identity)
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
            account_usage: HashMap::new(),
            host: crate::test_support::host(),
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
            catalog_state: Some(state.into()),
            configuration_catalogs: Vec::new(),
            configuration_rx,
            configuration_tx: None,
            configuration_requests: HashSet::new(),
            requested_access_modes: HashMap::new(),
            published_statuses: HashMap::new(),
        },
        events,
    )
}

#[test]
fn archive_intent_resolves_a_draft_bound_and_promoted_before_command_execution()
-> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut state = StateStore::open_at(&database)?;
    let mut draft = crate::sessions::DraftSession::with_id(
        Some(Backend::Pi),
        "draft".into(),
        temp.path().into(),
    );
    draft.submitted = true;
    draft.app_session_id = state.allocate_app_session_id(&draft)?;
    let command = RuntimeCommand::SetAppSessionArchived {
        app_session_id: crate::sessions::AppSessionId::new(draft.app_session_id).expect("identity"),
        archived: true,
    };
    draft.session_path = Some(temp.path().join("bound.jsonl"));
    state.allocate_app_session_id(&draft)?;
    state.remove_draft(&draft.id)?;
    let sessions = state.cached_sessions("")?;
    let path = sessions[0].path.clone();
    let (mut supervisor, events) = supervisor_for_family(state, sessions);
    assert!(supervisor.handle_session_family_command(&command));
    assert!(archived(&database, &path)?);
    assert!(supervisor.catalog_sessions[0].archived);
    assert!(events.try_iter().next().is_none());

    supervisor
        .catalog_state
        .as_ref()
        .expect("store")
        .with(|store| store.delete_session_state(&[path.clone()]))?;
    assert!(supervisor.handle_session_family_command(&command));
    assert!(matches!(
        events.try_recv(),
        Ok(RuntimeEvent::SessionsFailed { generation: 7, .. })
    ));
    Ok(())
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
fn stopping_for_move_keeps_family_unarchived_and_discards_only_its_queue() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let root = summary(temp.path(), "root", None);
    let unrelated = summary(temp.path(), "unrelated", None);
    let mut state = StateStore::open_at(&database)?;
    state.replace_sessions(&[root.clone(), unrelated.clone()])?;
    let pool = lifecycle_pool(temp.path(), Arc::new(LifecycleFactory::default()))?;
    let (mut supervisor, _) = supervisor_for_family(state, vec![root.clone(), unrelated.clone()]);
    let host_state = supervisor.host.state_store()?;
    let (root_prompt, unrelated_prompt) = host_state.with(|store| {
        store.replace_sessions(&[root.clone(), unrelated.clone()])?;
        let root_prompt = store.enqueue_prompt(
            &format!("session:{}", root.path.display()),
            Backend::Pi,
            temp.path(),
            Some(&root.path),
            crate::protocol::PromptMode::Normal,
            "move me",
            &[],
        )?;
        let unrelated_prompt = store.enqueue_prompt(
            &format!("session:{}", unrelated.path.display()),
            Backend::Pi,
            temp.path(),
            Some(&unrelated.path),
            crate::protocol::PromptMode::Normal,
            "keep me",
            &[],
        )?;
        Ok((root_prompt, unrelated_prompt))
    })?;

    farcaster_mcp_server::with_test_worker_pool(pool, || {
        supervisor.stop_session_family_work(&root.path, false)?;
        supervisor.discard_family_queue(&root.path)?;
        assert!(!archived(&database, &root.path)?);
        let remaining = host_state.with(|store| store.queued_prompts())?;
        assert_eq!(remaining.len(), 1, "{remaining:?}");
        assert_eq!(remaining[0].id, unrelated_prompt);
        assert_ne!(remaining[0].id, root_prompt);
        Ok(())
    })
}

#[test]
fn failed_move_restores_selected_actor_and_command_route() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = summary(temp.path(), "root", None);
    fs::write(
        &root.path,
        format!(
            "{}\n",
            json!({"type":"session","version":3,"id":"root","cwd":temp.path()})
        ),
    )
    .map_err(|error| error.to_string())?;
    let mut state = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    state.replace_sessions(std::slice::from_ref(&root))?;
    let pool = lifecycle_pool(temp.path(), Arc::new(LifecycleFactory::default()))?;
    let (mut supervisor, events) = supervisor_for_family(state, vec![root.clone()]);
    let key = format!("session:{}", root.path.display());
    let (actor_commands, actor_command_rx) = mpsc::channel();
    let (_actor_events_tx, actor_events) = mpsc::channel();
    let join = thread::spawn(move || {
        while let Ok(command) = actor_command_rx.recv() {
            if matches!(command, RuntimeCommand::Shutdown) {
                break;
            }
        }
        Ok(())
    });
    supervisor.actors.insert(
        key.clone(),
        SessionRuntimeHandle {
            commands: actor_commands,
            events: actor_events,
            thread: join.thread().clone(),
            join,
        },
    );
    supervisor
        .actor_paths
        .insert(root.path.clone(), key.clone());
    supervisor.selected = key.clone();
    supervisor.selected_session = Some(root.path.clone());
    let host_state = supervisor.host.state_store()?;
    host_state.with(|store| {
        store.replace_sessions(std::slice::from_ref(&root))?;
        store.enqueue_prompt(
            &key,
            Backend::Pi,
            temp.path(),
            Some(&root.path),
            crate::protocol::PromptMode::Normal,
            "do not replay",
            &[],
        )?;
        Ok(())
    })?;

    farcaster_mcp_server::with_test_worker_pool(pool, || {
        assert!(
            supervisor.handle_session_family_command(&RuntimeCommand::StopAndMoveSession {
                path: root.path.clone(),
                target_project: temp.path().join("missing-project"),
            },)
        );
        assert_eq!(supervisor.selected, key);
        assert_eq!(supervisor.actor_paths.get(&root.path), Some(&key));
        assert!(supervisor.actors.contains_key(&key));
        assert!(root.path.is_file());
        assert!(host_state.with(|store| store.queued_prompts())?.is_empty());
        let failure_events = events.try_iter().collect::<Vec<_>>();
        assert!(failure_events.iter().any(|event| matches!(
            event,
            RuntimeEvent::SessionsFailed { message, .. }
                if message.contains("resolve target project")
        )));
        assert!(
            failure_events
                .iter()
                .all(|event| !matches!(event, RuntimeEvent::SessionMoved { .. }))
        );

        let (commands, command_rx) = mpsc::channel();
        supervisor.command_rx = command_rx;
        commands
            .send(RuntimeCommand::SelectSession {
                path: root.path.clone(),
                harness: root.harness,
                session_id: root.id.clone(),
                project: root.project.clone(),
            })
            .map_err(|error| error.to_string())?;
        assert!(supervisor.process_next_command());
        assert_eq!(supervisor.selected, key);
        assert!(supervisor.actors.contains_key(&key));
        let mut selected_snapshot = false;
        for _ in 0..100 {
            supervisor.drain_actor_events();
            selected_snapshot |= events.try_iter().any(|event| {
                matches!(
                    event,
                    RuntimeEvent::Snapshot { snapshot, .. }
                        if snapshot.harness == Some(root.harness)
                            && snapshot.selected_session.as_deref() == Some(root.path.as_path())
                )
            });
            if selected_snapshot {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            selected_snapshot,
            "the restored actor must load the selected chat"
        );
        Ok(())
    })
}

#[test]
fn failed_stop_keeps_an_archived_session_and_its_pending_message() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut root = summary(temp.path(), "root", None);
    root.archived = true;
    let mut state = StateStore::open_at(&database)?;
    state.replace_sessions(std::slice::from_ref(&root))?;
    let factory = Arc::new(LifecycleFactory::default());
    factory.fail_close.store(true, Ordering::SeqCst);
    let pool = lifecycle_pool(temp.path(), factory)?;
    let _parent = start_worker(&pool, temp.path(), "family-child", &root.path)?;
    let (mut supervisor, events) = supervisor_for_family(state, vec![root.clone()]);
    let host_state = supervisor.host.state_store()?;
    let prompt = host_state.with(|store| {
        store.replace_sessions(std::slice::from_ref(&root))?;
        store.enqueue_prompt(
            &format!("session:{}", root.path.display()),
            Backend::Pi,
            temp.path(),
            Some(&root.path),
            crate::protocol::PromptMode::Normal,
            "keep this message",
            &[],
        )
    })?;

    farcaster_mcp_server::with_test_worker_pool(pool, || {
        assert!(supervisor.handle_session_family_command(
            &RuntimeCommand::StopAndDeleteSessionFamily {
                path: root.path.clone(),
            },
        ));
        assert!(
            supervisor
                .catalog_sessions
                .iter()
                .any(|session| session.path == root.path)
        );
        assert!(archived(&database, &root.path)?);
        assert!(
            host_state
                .with(|store| store.queued_prompts())?
                .iter()
                .any(|row| row.id == prompt)
        );
        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::SessionsFailed { message, .. } if message.contains("gated close failed")
        )));
        Ok(())
    })
}

#[test]
fn direct_delete_preserves_files_and_queue_when_a_worker_cannot_stop() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut root = summary(temp.path(), "root", None);
    root.archived = true;
    std::fs::write(&root.path, "keep this transcript").map_err(|error| error.to_string())?;
    let mut state = StateStore::open_at(&database)?;
    state.replace_sessions(std::slice::from_ref(&root))?;
    let factory = Arc::new(LifecycleFactory::default());
    factory.fail_close.store(true, Ordering::SeqCst);
    let pool = lifecycle_pool(temp.path(), factory)?;
    let _parent = start_worker(&pool, temp.path(), "family-child", &root.path)?;
    let (mut supervisor, events) = supervisor_for_family(state, vec![root.clone()]);
    let host_state = supervisor.host.state_store()?;
    let prompt = host_state.with(|store| {
        store.replace_sessions(std::slice::from_ref(&root))?;
        store.enqueue_prompt(
            &format!("session:{}", root.path.display()),
            Backend::Pi,
            temp.path(),
            Some(&root.path),
            crate::protocol::PromptMode::Normal,
            "keep this message",
            &[],
        )
    })?;

    farcaster_mcp_server::with_test_worker_pool(pool, || {
        assert!(
            supervisor.handle_session_family_command(&RuntimeCommand::DeleteSessionFamily {
                path: root.path.clone(),
            },)
        );
        assert!(
            supervisor
                .catalog_sessions
                .iter()
                .any(|session| session.path == root.path)
        );
        assert!(archived(&database, &root.path)?);
        assert_eq!(
            std::fs::read_to_string(&root.path).map_err(|error| error.to_string())?,
            "keep this transcript"
        );
        assert!(
            host_state
                .with(|store| store.queued_prompts())?
                .iter()
                .any(|row| row.id == prompt)
        );
        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::SessionsFailed { message, .. } if message.contains("gated close failed")
        )));
        Ok(())
    })
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
    let pool = lifecycle_pool(temp.path(), factory.clone())?;
    let release_close_gate = CloseGateGuard::block(&factory);
    let _parent = start_worker(&pool, temp.path(), "family-child", &root.path)?;
    let _other_parent = start_worker(&pool, temp.path(), "other-child", &unrelated.path)?;
    let (supervisor, events) =
        supervisor_for_family(state, vec![root.clone(), child, unrelated.clone()]);

    farcaster_mcp_server::with_test_worker_pool(pool.clone(), || {
        let _release_close_gate = release_close_gate;
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
            RuntimeEvent::SessionStatus { ref status, .. } if *status == RunStatus::Stopped
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
            RuntimeEvent::SessionStatus { ref status, .. } if *status == RunStatus::Stopped
        )));
        assert!(
            supervisor
                .catalog_sessions
                .iter()
                .find(|session| session.path == root.path)
                .is_some_and(|session| session.archived)
        );

        pool.stop_session_family(temp.path(), &[unrelated.key()])?;
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
    let _parent = start_worker(&pool, temp.path(), "family-child", &root.path)?;
    let (mut supervisor, events) = supervisor_for_family(state, vec![root.clone()]);

    farcaster_mcp_server::with_test_worker_pool(pool, || {
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
            RuntimeEvent::SessionStatus { status, .. } if *status == RunStatus::Stopped
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

    farcaster_mcp_server::with_test_worker_pool(pool, || {
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
            RuntimeEvent::SessionStatus { status, .. } if *status == RunStatus::Stopped
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
            RuntimeEvent::SessionStatus { status, .. } if *status == RunStatus::Stopped
        )));
        Ok(())
    })
}

#[test]
fn stale_catalog_identity_cannot_authorize_archive_after_bindings_already_merged()
-> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut state = StateStore::open_at(&database)?;
    state.replace_sessions(&[summary(temp.path(), "root", None)])?;
    let old = state.cached_sessions("")?.remove(0);
    let mut draft = crate::sessions::DraftSession::with_id(
        Some(Backend::Pi),
        "draft".into(),
        temp.path().into(),
    );
    draft.submitted = true;
    draft.app_session_id = state.allocate_app_session_id(&draft)?;
    draft.session_path = Some(old.path.clone());
    state.allocate_app_session_id(&draft)?;
    let current = state.cached_sessions("")?.remove(0);
    assert_ne!(old.key(), current.key());
    let factory = Arc::new(LifecycleFactory::default());
    let pool = lifecycle_pool(temp.path(), factory.clone())?;
    let parent = start_worker(&pool, temp.path(), "family-child", &old.path)?;
    parent.bind_execution_for_test(crate::agents::ExecutionBinding {
        session_record: current.app_session_id,
        turn_id: "after-merge".into(),
        prompt_id: None,
    });
    let (mut supervisor, _) = supervisor_for_family(state, vec![old.clone()]);
    farcaster_mcp_server::with_test_worker_pool(pool.clone(), || {
        let error = supervisor
            .stop_session_family_work(&old.path, true)
            .unwrap_err();
        assert!(error.contains("identity changed"), "{error}");
        assert!(!archived(&database, &old.path)?);
        assert_eq!(factory.aborts.load(Ordering::SeqCst), 0);
        assert_eq!(pool.snapshots()?[0].status, WorkerStatus::Running);
        supervisor.catalog_sessions = vec![current];
        supervisor.stop_session_family_work(&old.path, true)?;
        assert_eq!(pool.snapshots()?[0].status, WorkerStatus::Stopped);
        assert!(archived(&database, &old.path)?);
        Ok(())
    })
}

#[test]
fn merge_during_stop_releases_original_fence_before_retry() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut state = StateStore::open_at(&database)?;
    state.replace_sessions(&[summary(temp.path(), "root", None)])?;
    let old = state.cached_sessions("")?.remove(0);
    let factory = Arc::new(LifecycleFactory::default());
    let pool = lifecycle_pool(temp.path(), factory.clone())?;
    let release_close_gate = CloseGateGuard::block(&factory);
    let parent = start_worker(&pool, temp.path(), "family-child", &old.path)?;
    parent.bind_execution_for_test(crate::agents::ExecutionBinding {
        session_record: old.app_session_id,
        turn_id: "before-merge".into(),
        prompt_id: None,
    });
    let (mut supervisor, _) = supervisor_for_family(state, vec![old.clone()]);
    let store = supervisor.catalog_state.as_ref().unwrap().clone();
    farcaster_mcp_server::with_test_worker_pool(pool.clone(), || {
        let _release_close_gate = release_close_gate;
        let path = old.path.clone();
        let stop = thread::spawn(move || {
            let result = supervisor.stop_session_family_work(&path, true);
            (supervisor, result)
        });
        wait_for(&factory.closes, 1);
        let current = store.with(|state| {
            let mut draft = crate::sessions::DraftSession::with_id(
                Some(Backend::Pi),
                "draft".into(),
                temp.path().into(),
            );
            draft.submitted = true;
            draft.app_session_id = state.allocate_app_session_id(&draft)?;
            draft.session_path = Some(old.path.clone());
            state.allocate_app_session_id(&draft)?;
            Ok(state.cached_sessions("")?.remove(0))
        })?;
        assert_ne!(old.key(), current.key());
        parent.bind_execution_for_test(crate::agents::ExecutionBinding {
            session_record: current.app_session_id,
            turn_id: "after-merge".into(),
            prompt_id: None,
        });
        *factory.close_gate.0.lock().map_err(|_| "close gate")? = false;
        factory.close_gate.1.notify_all();
        let (mut supervisor, result) = stop.join().map_err(|_| "stop panicked")?;
        assert!(result.unwrap_err().contains("identity changed"));
        assert!(!archived(&database, &old.path)?);
        supervisor.catalog_sessions = vec![current];
        supervisor.stop_session_family_work(&old.path, false)?;
        let context = crate::agents::CallerRegistry::shared().resolve(parent.token())?;
        let child = pool.start(StartWorker {
            project: temp.path().into(),
            name: "after-retry".into(),
            prompt: "stay alive".into(),
            backend: Backend::Pi,
            parent_session: context.session,
            parent_worker_id: Some(context.worker_id),
            context: WorkerContext::Fresh,
            provider: None,
            model: None,
            effort: None,
            access_mode: HarnessAccessMode::Auto,
        })?;
        assert_eq!(child.status, WorkerStatus::Running);
        Ok(())
    })
}
