use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, Instant},
};

use super::*;
use crate::agents::{
    WorkerEvent, WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory,
};
use crate::modules::agents::contract::{
    StartWorker, WorkerContext, WorkerInputResponse, WorkerStatus,
};

#[derive(Default)]
struct FakeFactory {
    slots: Mutex<Vec<super::WorkerSlot>>,
    sends: Mutex<Vec<Arc<Mutex<Vec<WorkerSendMode>>>>>,
    events: Mutex<Vec<mpsc::Sender<WorkerEvent>>>,
    responses: Arc<Mutex<Vec<WorkerInputResponse>>>,
    aborts: Arc<std::sync::atomic::AtomicUsize>,
    closes: Arc<std::sync::atomic::AtomicUsize>,
    drops: Arc<std::sync::atomic::AtomicUsize>,
    launches: Mutex<Vec<WorkerLaunch>>,
    fail_close: Arc<std::sync::atomic::AtomicBool>,
    fail_send: Arc<std::sync::atomic::AtomicBool>,
    fail_new_session_close: Arc<std::sync::atomic::AtomicBool>,
    close_gate: Arc<(Mutex<bool>, Condvar)>,
    creates: Arc<std::sync::atomic::AtomicUsize>,
    create_gate: Arc<(Mutex<bool>, Condvar)>,
}

struct FakeSession {
    events: mpsc::Receiver<WorkerEvent>,
    sent: Arc<Mutex<Vec<WorkerSendMode>>>,
    responses: Arc<Mutex<Vec<WorkerInputResponse>>>,
    aborts: Arc<std::sync::atomic::AtomicUsize>,
    closes: Arc<std::sync::atomic::AtomicUsize>,
    drops: Arc<std::sync::atomic::AtomicUsize>,
    identity: Option<crate::agents::CallerIdentity>,
    fail_close: Arc<std::sync::atomic::AtomicBool>,
    fail_send: Arc<std::sync::atomic::AtomicBool>,
    fail_created_session_close: bool,
    close_gate: Arc<(Mutex<bool>, Condvar)>,
}

impl WorkerSessionFactory for FakeFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        self.creates
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (gate, changed) = &*self.create_gate;
        let mut blocked = gate.lock().map_err(|_| "create gate")?;
        while *blocked {
            blocked = changed.wait(blocked).map_err(|_| "create gate")?;
        }
        self.launches
            .lock()
            .map_err(|_| "fake launches unavailable")?
            .push(launch.clone());
        if let Some(slot) = launch.slot {
            self.slots
                .lock()
                .map_err(|_| "fake slots unavailable")?
                .push(slot);
        }
        let (events, receiver) = mpsc::channel();
        let sent = Arc::new(Mutex::new(Vec::new()));
        self.sends
            .lock()
            .map_err(|_| "fake sessions unavailable".to_owned())?
            .push(sent.clone());
        self.events
            .lock()
            .map_err(|_| "fake events unavailable".to_owned())?
            .push(events);
        let identity = launch
            .parent_worker_id
            .clone()
            .map(|parent_id| {
                CallerRegistry::shared().issue_as_with_access(
                    &launch.project,
                    CallerProfile {
                        backend: "pi".into(),
                        provider: None,
                        model: None,
                        effort: None,
                    },
                    None,
                    launch.worker_id.clone(),
                    launch.worker_name.clone(),
                    Some(parent_id),
                    launch.access_mode,
                )
            })
            .transpose()?;
        if let (
            Some(identity),
            WorkerContext::Session { session_locator } | WorkerContext::Resume { session_locator },
        ) = (&identity, &launch.context)
        {
            identity.bind(session_locator.clone());
        }
        Ok(Box::new(FakeSession {
            events: receiver,
            sent,
            responses: self.responses.clone(),
            aborts: self.aborts.clone(),
            closes: self.closes.clone(),
            drops: self.drops.clone(),
            identity,
            fail_close: self.fail_close.clone(),
            fail_send: self.fail_send.clone(),
            fail_created_session_close: self
                .fail_new_session_close
                .load(std::sync::atomic::Ordering::SeqCst),
            close_gate: self.close_gate.clone(),
        }))
    }
}

impl WorkerSession for FakeSession {
    fn send(&mut self, _message: String, mode: WorkerSendMode) -> Result<(), String> {
        if self.fail_send.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("send failed".into());
        }
        self.sent
            .lock()
            .map_err(|_| "fake sends unavailable".to_owned())?
            .push(mode);
        Ok(())
    }

    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String> {
        self.responses
            .lock()
            .map_err(|_| "responses")?
            .push(response);
        Ok(())
    }

    fn abort(&mut self) -> Result<(), String> {
        self.aborts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        let event = self.events.try_recv().ok()?;
        if let (Some(identity), WorkerEvent::SessionChanged { locator }) = (&self.identity, &event)
        {
            identity.bind(locator.clone());
        }
        Some(event)
    }

    fn close(&mut self) -> Result<(), String> {
        self.closes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (gate, changed) = &*self.close_gate;
        let mut blocked = gate.lock().map_err(|_| "close gate")?;
        while *blocked {
            blocked = changed.wait(blocked).map_err(|_| "close gate")?;
        }
        if self.fail_created_session_close
            || self.fail_close.load(std::sync::atomic::Ordering::SeqCst)
        {
            Err("close failed".into())
        } else {
            Ok(())
        }
    }
}

impl Drop for FakeSession {
    fn drop(&mut self) {
        self.drops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn pool(
    factory: Arc<FakeFactory>,
    project: &std::path::Path,
    maximum: usize,
) -> Result<(WorkerPool, async_channel::Receiver<()>), String> {
    let factory: Arc<dyn WorkerSessionFactory> = factory;
    let pool = WorkerPool::new(
        BTreeMap::from([("pi".into(), factory)]),
        "pi".into(),
        project.to_owned(),
        maximum,
    )?;
    let receiver = pool.updates();
    Ok((pool, receiver))
}

fn request(project: &std::path::Path) -> StartWorker {
    StartWorker {
        project: project.to_owned(),
        name: "implementation".into(),
        prompt: "work".into(),
        backend: "pi".into(),
        parent_session: "backend://parent".into(),
        parent_worker_id: None,
        context: WorkerContext::Fresh,
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Auto,
    }
}

fn assignment() -> super::WorkerAssignment {
    super::WorkerAssignment {
        profile: "test-profile".into(),
        execution: super::WorkerExecution {
            harness: "pi".into(),
            provider: "test-provider".into(),
            model: "test-model".into(),
            effort: None,
        },
    }
}

#[test]
fn starts_with_an_initial_prompt_and_enforces_capacity() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, _) = pool(factory.clone(), project.path(), 1)?;

    let started = pool.start(request(project.path()))?;
    assert_eq!(started.backend, "pi");
    assert_eq!(
        factory.sends.lock().map_err(|_| "fake sends unavailable")?[0]
            .lock()
            .map_err(|_| "fake sends unavailable")?
            .as_slice(),
        [WorkerSendMode::Prompt]
    );
    assert!(pool.start(request(project.path())).is_err());
    Ok(())
}

#[test]
fn future_launches_use_current_proxy_and_requested_access_mode() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, _) = pool(factory.clone(), project.path(), 2)?;
    let proxy = "http://127.0.0.1:8118";
    pool.set_app_proxy(Some(proxy.into()))?;
    let mut first = request(project.path());
    first.access_mode = crate::agents::HarnessAccessMode::Sandboxed;
    pool.start(first)?;

    pool.set_app_proxy(None)?;
    let mut second = request(project.path());
    second.name = "after-clear".into();
    second.access_mode = crate::agents::HarnessAccessMode::Full;
    pool.start(second)?;

    let launches = factory.launches.lock().map_err(|_| "launches")?;
    assert_eq!(launches[0].app_proxy.as_deref(), Some(proxy));
    assert_eq!(
        launches[0].access_mode,
        crate::agents::HarnessAccessMode::Sandboxed
    );
    assert_eq!(launches[1].app_proxy, None);
    assert_eq!(
        launches[1].access_mode,
        crate::agents::HarnessAccessMode::Full
    );
    Ok(())
}

#[test]
fn projects_from_later_calling_sessions_can_be_allowed() -> Result<(), String> {
    let startup_project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let later_project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, _) = pool(factory, startup_project.path(), 1)?;

    assert!(pool.start(request(later_project.path())).is_err());
    pool.allow_project(later_project.path())?;
    assert_eq!(
        pool.start(request(later_project.path()))?.project,
        later_project
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
    );
    Ok(())
}

#[test]
fn child_settlement_sends_one_final_message_per_turn() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let parent = CallerRegistry::shared().issue(
        project.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("pi-parent");
    let parent_id = CallerRegistry::shared().resolve(parent.token())?.worker_id;
    let mut child_request = request(project.path());
    child_request.parent_worker_id = Some(parent_id);
    pool.start(child_request)?;
    wait_for_update(&updates)?;
    let events = factory.events.lock().map_err(|_| "events")?[0].clone();
    for (starts_turn, output, delivers) in [
        (false, "done", true),
        (false, "done", false), // Duplicate completion.
        (true, "done", true),   // Same answer on a later turn.
        (true, "  ", false),    // Empty completion.
    ] {
        if starts_turn {
            events
                .send(WorkerEvent::Started)
                .map_err(|_| "start turn")?;
        }
        events
            .send(WorkerEvent::Settled {
                output: output.into(),
            })
            .map_err(|_| "finish turn")?;
        wait_for_update(&updates)?;
        if delivers {
            let report = parent.try_recv().ok_or("missing final message")?;
            assert_eq!(report.from, "implementation");
            assert_eq!(report.message, output);
        }
        assert!(parent.try_recv().is_none(), "unexpected parent message");
    }
    Ok(())
}

#[test]
fn worker_parent_reports_and_inputs_follow_a_replacement_parent_process() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let registry = CallerRegistry::shared();
    let parent = registry.issue(
        project.path(),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("/sessions/stable-parent.jsonl");
    let original = registry.resolve(parent.token())?;
    let mut child_request = request(project.path());
    child_request.parent_session = original.session.clone();
    child_request.parent_worker_id = Some(original.worker_id);
    pool.start(child_request)?;
    wait_for_update(&updates)?;
    drop(parent);

    let replacement = registry.issue(
        project.path(),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    replacement.bind("/sessions/stable-parent.jsonl");
    let events = factory.events.lock().map_err(|_| "events")?[0].clone();
    events
        .send(WorkerEvent::Settled {
            output: "finished after restart".into(),
        })
        .map_err(|_| "settle")?;
    wait_for_update(&updates)?;
    assert_eq!(
        replacement
            .try_recv()
            .ok_or("replacement missed report")?
            .message,
        "finished after restart"
    );

    events.send(WorkerEvent::Started).map_err(|_| "start")?;
    events
        .send(WorkerEvent::NeedsInput(crate::agents::WorkerInput {
            id: "approval".into(),
            prompt: "Proceed?".into(),
            options: vec!["Yes".into(), "No".into()],
            secret: false,
        }))
        .map_err(|_| "input")?;
    wait_for_update(&updates)?;
    let inputs =
        registry.take_child_inputs(project.path(), "codex-cli", "/sessions/stable-parent.jsonl");
    assert_eq!(inputs.len(), 1);
    assert!(
        registry
            .take_child_inputs(project.path(), "codex-cli", "/sessions/stable-parent.jsonl")
            .is_empty(),
        "the replacement parent must not receive the same input twice"
    );
    assert!(
        registry
            .take_child_inputs(project.path(), "pi", "/sessions/stable-parent.jsonl")
            .is_empty()
    );
    registry.respond_to_child_input(WorkerInputResponse {
        id: inputs[0].id.clone(),
        value: Some("Yes".into()),
        cancel: false,
    })?;
    wait_for_update(&updates)?;
    assert_eq!(
        *factory.responses.lock().map_err(|_| "responses")?,
        vec![WorkerInputResponse {
            id: "approval".into(),
            value: Some("Yes".into()),
            cancel: false,
        }],
        "the replacement parent's answer must reach the child once with its backend id"
    );
    assert!(
        replacement.try_recv().is_none(),
        "unexpected duplicate report"
    );
    Ok(())
}

fn wait_for_update(receiver: &async_channel::Receiver<()>) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match receiver.try_recv() {
            Ok(()) => return Ok(()),
            Err(async_channel::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(format!("worker update was not delivered: {error}")),
        }
    }
}

#[test]
fn completed_workers_release_capacity_and_reactivation_waits_for_a_slot() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let first_started = pool.start(request(project.path()))?;
    wait_for_update(&updates)?;
    let first = factory.slots.lock().map_err(|_| "slots")?[0].clone();
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::Settled {
            output: "done".into(),
        })
        .map_err(|_| "send event")?;
    wait_for_worker_status(&pool, &first_started.id, WorkerStatus::Idle)?;

    let second_started = pool.start(request(project.path()))?;
    wait_for_update(&updates)?;
    let second = factory.slots.lock().map_err(|_| "slots")?[1].clone();
    assert_eq!(factory.drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        !first.try_activate(),
        "reactivation cannot bypass a running worker"
    );
    factory.events.lock().map_err(|_| "events")?[1]
        .send(WorkerEvent::Settled {
            output: "also done".into(),
        })
        .map_err(|_| "send event")?;
    wait_for_worker_status(&pool, &second_started.id, WorkerStatus::Idle)?;
    assert!(second.try_activate());
    assert!(pool.start(request(project.path())).is_err());
    second.release();
    assert!(pool.start(request(project.path())).is_ok());
    Ok(())
}

#[test]
fn an_idle_worker_reactivated_by_peer_input_is_not_retired() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let started = pool.start(request(project.path()))?;
    wait_for_update(&updates)?;
    let slot = factory.slots.lock().map_err(|_| "slots")?[0].clone();
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::Settled {
            output: "done".into(),
        })
        .map_err(|_| "settle")?;
    wait_for_worker_status(&pool, &started.id, WorkerStatus::Idle)?;
    assert!(slot.try_activate(), "peer input reserves the worker slot");

    let mut second = request(project.path());
    second.name = "second".into();
    assert!(pool.start(second.clone()).is_err());
    assert_eq!(factory.drops.load(std::sync::atomic::Ordering::SeqCst), 0);

    slot.release();
    assert!(pool.start(second).is_ok());
    assert_eq!(factory.drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn failure_releases_capacity_and_notifies_parent_without_a_child_registration() -> Result<(), String>
{
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let parent = CallerRegistry::shared().issue(
        project.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("failure-parent");
    let mut child_request = request(project.path());
    child_request.parent_worker_id =
        Some(CallerRegistry::shared().resolve(parent.token())?.worker_id);
    pool.start(child_request)?;
    wait_for_update(&updates)?;
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::Failed("connection lost".into()))
        .map_err(|_| "send event")?;
    wait_for_update(&updates)?;
    let report = parent.try_recv().ok_or("missing failure report")?;
    assert_eq!(report.from, "implementation");
    assert_eq!(report.message, "Worker failed: connection lost");
    assert!(parent.try_recv().is_none());
    assert!(pool.start(request(project.path())).is_ok());
    Ok(())
}

#[test]
fn child_input_reaches_parent_and_answer_returns_to_worker() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let registry = CallerRegistry::shared();
    let parent = registry.issue(
        project.path(),
        CallerProfile {
            backend: "parent-backend".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("input-parent");
    let context = registry.resolve(parent.token())?;
    let mut child_request = request(project.path());
    child_request.parent_worker_id = Some(context.worker_id);
    pool.start(child_request)?;
    wait_for_update(&updates)?;
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::NeedsInput(crate::agents::WorkerInput {
            id: "original".into(),
            prompt: "Which?".into(),
            options: vec!["A".into(), "B".into()],
            secret: false,
        }))
        .map_err(|_| "send event")?;
    wait_for_update(&updates)?;
    let inputs = registry.take_child_inputs(&context.project, "parent-backend", "input-parent");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].options, ["A", "B"]);
    assert!(
        parent.try_recv().is_none(),
        "questions must go to the user, not the parent model"
    );
    registry.respond_to_child_input(WorkerInputResponse {
        id: inputs[0].id.clone(),
        value: Some("B".into()),
        cancel: false,
    })?;
    wait_for_update(&updates)?;
    assert_eq!(
        *factory.responses.lock().map_err(|_| "responses")?,
        vec![WorkerInputResponse {
            id: "original".into(),
            value: Some("B".into()),
            cancel: false,
        }]
    );
    Ok(())
}

#[test]
fn stopping_a_session_family_aborts_closes_and_joins_its_children() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 3)?;
    let registry = CallerRegistry::shared();
    let parent = registry.issue(
        project.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("/sessions/parent.jsonl");
    let parent_context = registry.resolve(parent.token())?;
    let mut child = request(project.path());
    child.name = "child".into();
    child.parent_session = parent_context.session;
    child.parent_worker_id = Some(parent_context.worker_id);
    pool.start(child)?;
    wait_for_update(&updates)?;
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::SessionChanged {
            locator: "/sessions/child.jsonl".into(),
        })
        .map_err(|_| "child locator")?;
    wait_for_update(&updates)?;
    let child_id = factory.launches.lock().map_err(|_| "launches")?[0]
        .worker_id
        .clone();
    let mut grandchild = request(project.path());
    grandchild.name = "grandchild".into();
    grandchild.parent_session = "/sessions/child.jsonl".into();
    grandchild.parent_worker_id = Some(child_id);
    pool.start(grandchild)?;
    wait_for_update(&updates)?;
    let mut unrelated = request(project.path());
    unrelated.name = "unrelated".into();
    unrelated.parent_session = "/sessions/other.jsonl".into();
    pool.start(unrelated)?;
    wait_for_update(&updates)?;
    let event_senders = factory.events.lock().map_err(|_| "events")?;
    let child_events = event_senders[0].clone();
    let grandchild_events = event_senders[1].clone();
    let unrelated_events = event_senders[2].clone();
    drop(event_senders);

    let stopped = pool.stop_session_family(
        project.path(),
        &[(
            "pi".into(),
            std::path::PathBuf::from("/sessions/parent.jsonl"),
        )],
    )?;

    assert_eq!(stopped, 2);
    assert_eq!(factory.aborts.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(factory.closes.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(factory.drops.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(
        child_events
            .send(WorkerEvent::Settled {
                output: "late work".into()
            })
            .is_err(),
        "joined worker must not accept later backend work"
    );
    assert!(grandchild_events.send(WorkerEvent::Started).is_err());
    assert!(unrelated_events.send(WorkerEvent::Started).is_ok());
    let snapshots = pool.snapshots()?;
    assert_eq!(
        snapshots
            .iter()
            .filter(|snapshot| snapshot.status == WorkerStatus::Stopped)
            .count(),
        2
    );
    assert_eq!(
        snapshots
            .iter()
            .filter(|snapshot| snapshot.status == WorkerStatus::Running)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn session_family_stop_reports_close_failure_instead_of_confirming_stop() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let parent = CallerRegistry::shared().issue(
        project.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("/sessions/parent.jsonl");
    let parent_context = CallerRegistry::shared().resolve(parent.token())?;
    let mut child = request(project.path());
    child.parent_session = parent_context.session.clone();
    child.parent_worker_id = Some(parent_context.worker_id.clone());
    pool.start(child)?;
    wait_for_update(&updates)?;
    factory
        .fail_close
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let error = pool
        .stop_session_family(
            project.path(),
            &[(
                "pi".into(),
                std::path::PathBuf::from("/sessions/parent.jsonl"),
            )],
        )
        .expect_err("failed close cannot confirm a stopped family");

    assert!(error.contains("close failed"));
    assert_eq!(factory.aborts.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(factory.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(factory.drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(pool.snapshots()?[0].status, WorkerStatus::Failed);
    factory
        .fail_close
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let repeated = pool
        .stop_session_family(
            project.path(),
            &[(
                "pi".into(),
                std::path::PathBuf::from("/sessions/parent.jsonl"),
            )],
        )
        .expect_err("a repeated stop cannot confirm cleanup that already failed");
    assert!(repeated.contains("close failed"));
    assert_eq!(factory.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
    let mut retry = request(project.path());
    retry.name = "retry".into();
    retry.parent_session = parent_context.session;
    retry.parent_worker_id = Some(parent_context.worker_id);
    let retry_error = pool
        .start(retry)
        .expect_err("an unconfirmed old process must keep consuming the process limit");
    assert!(retry_error.contains("close failed"));
    Ok(())
}

#[test]
fn failed_idle_retirement_does_not_admit_an_unbounded_replacement() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let started = pool.start(request(project.path()))?;
    wait_for_update(&updates)?;
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::Settled {
            output: "done".into(),
        })
        .map_err(|_| "settle")?;
    wait_for_worker_status(&pool, &started.id, WorkerStatus::Idle)?;
    factory
        .fail_close
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let mut replacement = request(project.path());
    replacement.name = "replacement".into();
    let first = pool
        .start(replacement.clone())
        .expect_err("failed retirement must reject the replacement");
    assert!(first.contains("close failed"));
    factory
        .fail_close
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let second = pool
        .start(replacement)
        .expect_err("unconfirmed cleanup must still hold the process limit");
    assert!(second.contains("close failed"));
    assert_eq!(factory.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(factory.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(factory.drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(pool.snapshots()?[0].status, WorkerStatus::Failed);
    Ok(())
}

#[test]
fn failed_initial_send_cleanup_keeps_its_process_capacity_owned() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    factory
        .fail_send
        .store(true, std::sync::atomic::Ordering::SeqCst);
    factory
        .fail_close
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (pool, _) = pool(factory.clone(), project.path(), 1)?;

    let error = pool
        .start(request(project.path()))
        .expect_err("the initial send must fail");
    assert!(error.contains("send failed"));
    assert!(error.contains("close failed"));
    assert_eq!(pool.snapshots()?[0].status, WorkerStatus::Failed);

    factory
        .fail_send
        .store(false, std::sync::atomic::Ordering::SeqCst);
    factory
        .fail_close
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let mut retry = request(project.path());
    retry.name = "retry".into();
    let retry_error = pool
        .start(retry)
        .expect_err("unconfirmed setup cleanup must keep the process limit full");
    assert!(retry_error.contains("close failed"));
    assert_eq!(factory.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(factory.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(factory.drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn failed_resume_send_cleanup_keeps_its_process_capacity_owned() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let parent = CallerRegistry::shared().issue(
        project.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("/sessions/parent.jsonl");
    let context = CallerRegistry::shared().resolve(parent.token())?;
    let mut first = request(project.path());
    first.parent_session = context.session.clone();
    first.parent_worker_id = Some(context.worker_id.clone());
    let started = pool.start_assigned(first, Some(assignment()))?;
    wait_for_update(&updates)?;
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::SessionChanged {
            locator: "/sessions/child.jsonl".into(),
        })
        .map_err(|_| "session")?;
    wait_for_update(&updates)?;
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::Settled {
            output: "done".into(),
        })
        .map_err(|_| "settle")?;
    wait_for_worker_status(&pool, &started.id, WorkerStatus::Idle)?;

    let mut second = request(project.path());
    second.name = "second".into();
    second.parent_session = context.session.clone();
    second.parent_worker_id = Some(context.worker_id.clone());
    let second = pool.start_assigned(second, Some(assignment()))?;
    wait_for_update(&updates)?;
    factory.events.lock().map_err(|_| "events")?[1]
        .send(WorkerEvent::SessionChanged {
            locator: "/sessions/second.jsonl".into(),
        })
        .map_err(|_| "second session")?;
    factory.events.lock().map_err(|_| "events")?[1]
        .send(WorkerEvent::Settled {
            output: "done".into(),
        })
        .map_err(|_| "second settle")?;
    wait_for_worker_status(&pool, &second.id, WorkerStatus::Idle)?;

    factory
        .fail_send
        .store(true, std::sync::atomic::Ordering::SeqCst);
    factory
        .fail_new_session_close
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let error = pool
        .resume_child(&context, "implementation", "again".into(), None)
        .expect_err("the resumed send must fail");
    assert!(error.contains("send failed"));
    assert!(error.contains("close failed"));
    assert!(
        pool.snapshots()?.iter().any(|snapshot| {
            snapshot.id == started.id && snapshot.status == WorkerStatus::Failed
        })
    );

    factory
        .fail_send
        .store(false, std::sync::atomic::Ordering::SeqCst);
    factory
        .fail_new_session_close
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let mut retry = request(project.path());
    retry.name = "retry".into();
    let retry_error = pool
        .start(retry)
        .expect_err("unconfirmed resumed cleanup must keep the process limit full");
    assert!(retry_error.contains("close failed"));
    assert_eq!(factory.creates.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(factory.closes.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(factory.drops.load(std::sync::atomic::Ordering::SeqCst), 3);
    Ok(())
}

#[test]
fn family_stop_fence_blocks_new_children_until_shutdown_finishes() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 2)?;
    let parent = CallerRegistry::shared().issue(
        project.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("/sessions/parent.jsonl");
    let context = CallerRegistry::shared().resolve(parent.token())?;
    let mut child = request(project.path());
    child.parent_session = context.session.clone();
    child.parent_worker_id = Some(context.worker_id.clone());
    pool.start(child)?;
    wait_for_update(&updates)?;
    let (gate, _) = &*factory.close_gate;
    *gate.lock().map_err(|_| "close gate")? = true;

    let stopping_pool = pool.clone();
    let stopping_project = project.path().to_owned();
    let stop = std::thread::spawn(move || {
        stopping_pool.stop_session_family(
            &stopping_project,
            &[(
                "pi".into(),
                std::path::PathBuf::from("/sessions/parent.jsonl"),
            )],
        )
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    while factory.closes.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        if Instant::now() >= deadline {
            return Err("worker close did not start".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut same_family = request(project.path());
    same_family.name = "late-child".into();
    same_family.parent_session = context.session.clone();
    same_family.parent_worker_id = Some(context.worker_id.clone());
    let late_pool = pool.clone();
    let late = std::thread::spawn(move || late_pool.start(same_family));
    std::thread::sleep(Duration::from_millis(20));
    assert!(
        factory.launches.lock().map_err(|_| "launches")?.len() == 1,
        "a late child must not launch while family close is blocked"
    );

    let (gate, changed) = &*factory.close_gate;
    *gate.lock().map_err(|_| "close gate")? = false;
    changed.notify_all();
    assert_eq!(stop.join().map_err(|_| "stop thread")??, 1);
    assert!(
        late.join().map_err(|_| "late start thread")?.is_err(),
        "the family fence must reject a late child after close completes"
    );
    let mut unrelated = request(project.path());
    unrelated.name = "unrelated".into();
    unrelated.parent_session = "/sessions/other.jsonl".into();
    unrelated.parent_worker_id = Some(context.worker_id);
    assert!(pool.start(unrelated).is_ok());
    Ok(())
}

#[test]
fn family_stop_waits_for_an_in_flight_child_creation_then_joins_it() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, _) = pool(factory.clone(), project.path(), 1)?;
    let parent = CallerRegistry::shared().issue(
        project.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("/sessions/parent.jsonl");
    let context = CallerRegistry::shared().resolve(parent.token())?;
    let mut child = request(project.path());
    child.parent_session = context.session;
    child.parent_worker_id = Some(context.worker_id);
    let (gate, _) = &*factory.create_gate;
    *gate.lock().map_err(|_| "create gate")? = true;

    let starting_pool = pool.clone();
    let start = std::thread::spawn(move || starting_pool.start(child));
    let deadline = Instant::now() + Duration::from_secs(1);
    while factory.creates.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        if Instant::now() >= deadline {
            return Err("worker creation did not start".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let stopping_pool = pool.clone();
    let stopping_project = project.path().to_owned();
    let stop = std::thread::spawn(move || {
        stopping_pool.stop_session_family(
            &stopping_project,
            &[(
                "pi".into(),
                std::path::PathBuf::from("/sessions/parent.jsonl"),
            )],
        )
    });
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(factory.aborts.load(std::sync::atomic::Ordering::SeqCst), 0);

    let (gate, changed) = &*factory.create_gate;
    *gate.lock().map_err(|_| "create gate")? = false;
    changed.notify_all();
    start.join().map_err(|_| "start thread")??;
    assert_eq!(stop.join().map_err(|_| "stop thread")??, 1);
    assert_eq!(factory.aborts.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(factory.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(factory.drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        factory.events.lock().map_err(|_| "events")?[0]
            .send(WorkerEvent::Started)
            .is_err()
    );
    Ok(())
}

#[test]
fn stopping_a_family_expires_its_delivered_child_input() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let registry = CallerRegistry::shared();
    let parent = registry.issue(
        project.path(),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("/sessions/parent.jsonl");
    let context = registry.resolve(parent.token())?;
    let mut child = request(project.path());
    child.parent_session = context.session.clone();
    child.parent_worker_id = Some(context.worker_id);
    pool.start(child)?;
    wait_for_update(&updates)?;
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::NeedsInput(crate::agents::WorkerInput {
            id: "backend-input".into(),
            prompt: "Proceed?".into(),
            options: Vec::new(),
            secret: false,
        }))
        .map_err(|_| "input")?;
    wait_for_update(&updates)?;
    let shown = registry.take_child_inputs(project.path(), "codex-cli", "/sessions/parent.jsonl");
    assert_eq!(shown.len(), 1);

    pool.stop_session_family(
        project.path(),
        &[(
            "codex-cli".into(),
            std::path::PathBuf::from("/sessions/parent.jsonl"),
        )],
    )?;

    assert_eq!(
        registry.take_expired_child_inputs(project.path(), "codex-cli", "/sessions/parent.jsonl"),
        vec![shown[0].id.clone()]
    );
    assert!(
        registry
            .respond_to_child_input(WorkerInputResponse {
                id: shown[0].id.clone(),
                value: Some("yes".into()),
                cancel: false,
            })
            .is_err()
    );
    Ok(())
}

#[test]
fn idle_processes_are_bounded_and_a_retired_child_resumes_its_session() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let (pool, updates) = pool(factory.clone(), project.path(), 1)?;
    let parent = CallerRegistry::shared().issue_with_access(
        project.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
        crate::agents::HarnessAccessMode::Full,
    );
    parent.bind("/sessions/parent.jsonl");
    let parent_context = CallerRegistry::shared().resolve(parent.token())?;

    for index in 0..10 {
        let mut child_request = request(project.path());
        child_request.name = format!("worker-{index}");
        child_request.parent_session = parent_context.session.clone();
        child_request.parent_worker_id = Some(parent_context.worker_id.clone());
        let started = pool.start_assigned(child_request, Some(assignment()))?;
        wait_for_update(&updates)?;
        factory.events.lock().map_err(|_| "events")?[index]
            .send(WorkerEvent::SessionChanged {
                locator: format!("/sessions/child-{index}.jsonl"),
            })
            .map_err(|_| "session change")?;
        wait_for_update(&updates)?;
        factory.events.lock().map_err(|_| "events")?[index]
            .send(WorkerEvent::Settled {
                output: "done".into(),
            })
            .map_err(|_| "settle")?;
        wait_for_worker_status(&pool, &started.id, WorkerStatus::Idle)?;
    }

    assert_eq!(
        factory.drops.load(std::sync::atomic::Ordering::SeqCst),
        9,
        "capacity one retains one live session and retires every older idle session"
    );
    let wrong_parent = CallerRegistry::shared().issue(
        project.path(),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    wrong_parent.bind("/sessions/parent.jsonl");
    let wrong_context = CallerRegistry::shared().resolve(wrong_parent.token())?;
    assert!(
        pool.resume_child(&wrong_context, "worker-0", "wrong parent".into(), None)?
            .is_none(),
        "an equal native session string from another backend must not resume the child"
    );
    pool.set_app_proxy(Some("https://resume-proxy.example:8443".into()))?;
    let resumed = pool
        .resume_child(&parent_context, "worker-0", "follow up".into(), None)?
        .ok_or("retired child was not found")?;
    assert_eq!(resumed.profile, "test-profile");
    let launches = factory.launches.lock().map_err(|_| "launches")?;
    assert!(matches!(
        &launches.last().ok_or("missing resume launch")?.context,
        WorkerContext::Resume { session_locator }
            if session_locator == "/sessions/child-0.jsonl"
    ));
    assert_eq!(
        launches.last().ok_or("missing resume launch")?.access_mode,
        crate::agents::HarnessAccessMode::Full
    );
    assert_eq!(
        launches
            .last()
            .ok_or("missing resume launch")?
            .app_proxy
            .as_deref(),
        Some("https://resume-proxy.example:8443")
    );
    drop(launches);
    assert_eq!(
        factory
            .sends
            .lock()
            .map_err(|_| "sends")?
            .last()
            .ok_or("missing resumed send")?
            .lock()
            .map_err(|_| "send modes")?
            .as_slice(),
        [WorkerSendMode::Prompt]
    );
    assert_eq!(
        factory.creates.load(std::sync::atomic::Ordering::SeqCst),
        11,
        "resuming reopens the retired child instead of creating a new identity"
    );
    assert_eq!(
        factory.closes.load(std::sync::atomic::Ordering::SeqCst),
        10,
        "resuming at capacity must retire the prior idle process"
    );
    assert_eq!(
        factory.drops.load(std::sync::atomic::Ordering::SeqCst),
        10,
        "only the resumed process may remain live at capacity one"
    );
    assert!(
        factory.events.lock().map_err(|_| "events")?[9]
            .send(WorkerEvent::Started)
            .is_err(),
        "the prior idle process must be joined before the retired child resumes"
    );
    Ok(())
}

fn wait_for_worker_status(pool: &WorkerPool, id: &str, status: WorkerStatus) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let reached = pool
            .snapshots()?
            .iter()
            .any(|snapshot| snapshot.id == id && snapshot.status == status);
        if reached {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("worker {id} did not reach {status:?}"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
