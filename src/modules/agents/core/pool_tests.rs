use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use super::*;
use crate::agents::{
    WorkerEvent, WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory,
};
use crate::modules::agents::contract::{StartWorker, WorkerContext, WorkerInputResponse};

#[derive(Default)]
struct FakeFactory {
    slots: Mutex<Vec<super::WorkerSlot>>,
    sends: Mutex<Vec<Arc<Mutex<Vec<WorkerSendMode>>>>>,
    events: Mutex<Vec<mpsc::Sender<WorkerEvent>>>,
}

struct FakeSession {
    events: mpsc::Receiver<WorkerEvent>,
    sent: Arc<Mutex<Vec<WorkerSendMode>>>,
}

impl WorkerSessionFactory for FakeFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
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
        Ok(Box::new(FakeSession {
            events: receiver,
            sent,
        }))
    }
}

impl WorkerSession for FakeSession {
    fn send(&mut self, _message: String, mode: WorkerSendMode) -> Result<(), String> {
        self.sent
            .lock()
            .map_err(|_| "fake sends unavailable".to_owned())?
            .push(mode);
        Ok(())
    }

    fn respond(&mut self, _response: WorkerInputResponse) -> Result<(), String> {
        Ok(())
    }

    fn abort(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        self.events.try_recv().ok()
    }

    fn close(&mut self) -> Result<(), String> {
        Ok(())
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
fn child_settlement_notifies_the_ui_without_messaging_parent() -> Result<(), String> {
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
    child_request.parent_worker_id = Some(parent_id.clone());

    let child = pool.start(child_request)?;
    let child_identity = CallerRegistry::shared().issue_as(
        project.path(),
        CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
        child.id.clone(),
        "implementation".into(),
        Some(parent_id),
    )?;
    child_identity.bind("pi-child");
    wait_for_update(&updates)?;
    factory
        .events
        .lock()
        .map_err(|_| "fake events unavailable".to_owned())?[0]
        .send(WorkerEvent::Settled {
            output: "done".into(),
        })
        .map_err(|_| "fake worker stopped".to_owned())?;

    wait_for_update(&updates)?;
    assert!(parent.try_recv().is_none());
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
    pool.start(request(project.path()))?;
    wait_for_update(&updates)?;
    let first = factory.slots.lock().map_err(|_| "slots")?[0].clone();
    factory.events.lock().map_err(|_| "events")?[0]
        .send(WorkerEvent::Settled {
            output: "done".into(),
        })
        .map_err(|_| "send event")?;
    wait_for_update(&updates)?;

    pool.start(request(project.path()))?;
    wait_for_update(&updates)?;
    assert!(
        !first.try_activate(),
        "reactivation cannot bypass a running worker"
    );
    factory.events.lock().map_err(|_| "events")?[1]
        .send(WorkerEvent::Settled {
            output: "also done".into(),
        })
        .map_err(|_| "send event")?;
    wait_for_update(&updates)?;
    assert!(first.try_activate());
    assert!(pool.start(request(project.path())).is_err());
    first.release();
    assert!(pool.start(request(project.path())).is_ok());
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
