use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use super::*;
use crate::agents::{
    WorkerEvent, WorkerInput, WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory,
};
use crate::modules::agents::contract::{
    StartWorker, WorkerContext, WorkerInputResponse, WorkerMessageMode, WorkerSnapshot,
    WorkerStatus,
};

#[derive(Default)]
struct FakeFactory {
    sessions: Mutex<Vec<FakeHandle>>,
}

#[derive(Clone)]
struct FakeHandle {
    events: mpsc::Sender<WorkerEvent>,
    sent: Arc<Mutex<Vec<WorkerSendMode>>>,
}

struct FakeSession {
    events: mpsc::Receiver<WorkerEvent>,
    sent: Arc<Mutex<Vec<WorkerSendMode>>>,
}

impl WorkerSessionFactory for FakeFactory {
    fn create(&self, _launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        let (events, receiver) = mpsc::channel();
        let sent = Arc::new(Mutex::new(Vec::new()));
        self.sessions
            .lock()
            .map_err(|_| "fake sessions unavailable".to_owned())?
            .push(FakeHandle {
                events,
                sent: sent.clone(),
            });
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
) -> Result<WorkerPool, String> {
    let factory: Arc<dyn WorkerSessionFactory> = factory;
    WorkerPool::new(
        BTreeMap::from([("pi".into(), factory)]),
        "pi".into(),
        project.to_owned(),
        maximum,
    )
}

fn request(project: &std::path::Path) -> StartWorker {
    StartWorker {
        project: project.to_owned(),
        prompt: "work".into(),
        backend: "pi".into(),
        parent_session: "backend://parent".into(),
        context: WorkerContext::Fresh,
        provider: None,
        model: None,
        effort: None,
    }
}

fn wait_for(pool: &WorkerPool, id: &str, status: WorkerStatus) -> WorkerSnapshot {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = pool.status(id).expect("worker status");
        if snapshot.status == status {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "worker did not reach {status:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn workers_settle_and_can_be_prompted_again() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let pool = pool(factory.clone(), project.path(), 2)?;
    assert_eq!(pool.default_backend(), "pi");
    assert_eq!(pool.backends(), ["pi"]);
    let started = pool.start(request(project.path()))?;
    let handle = factory
        .sessions
        .lock()
        .map_err(|_| "fake sessions unavailable".to_owned())?[0]
        .clone();
    assert_eq!(
        handle
            .sent
            .lock()
            .map_err(|_| "fake sends unavailable".to_owned())?
            .as_slice(),
        [WorkerSendMode::Prompt]
    );

    handle
        .events
        .send(WorkerEvent::SessionChanged {
            locator: "backend://worker-1".into(),
        })
        .map_err(|error| error.to_string())?;
    handle
        .events
        .send(WorkerEvent::Settled {
            output: "done".into(),
        })
        .map_err(|error| error.to_string())?;
    let idle = wait_for(&pool, &started.id, WorkerStatus::Idle);
    assert_eq!(idle.output.as_deref(), Some("done"));
    assert_eq!(idle.session_locator.as_deref(), Some("backend://worker-1"));

    let running = pool.send(&started.id, "more".into(), WorkerMessageMode::Auto)?;
    assert_eq!(running.status, WorkerStatus::Running);
    let deadline = Instant::now() + Duration::from_secs(2);
    while handle
        .sent
        .lock()
        .map_err(|_| "fake sends unavailable".to_owned())?
        .len()
        < 2
    {
        if Instant::now() >= deadline {
            return Err("worker did not receive second prompt".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        handle
            .sent
            .lock()
            .map_err(|_| "fake sends unavailable".to_owned())?[1],
        WorkerSendMode::Prompt
    );
    handle
        .events
        .send(WorkerEvent::NeedsInput(WorkerInput {
            id: "question-1".into(),
            prompt: "Choose".into(),
            options: vec!["A".into(), "B".into()],
            secret: false,
        }))
        .map_err(|error| error.to_string())?;
    let pending = wait_for(&pool, &started.id, WorkerStatus::NeedsInput);
    assert_eq!(
        pending
            .pending_input
            .as_ref()
            .map(|input| input.id.as_str()),
        Some("question-1")
    );
    assert_eq!(
        pool.respond(&started.id, Some("A".into()), false)?.status,
        WorkerStatus::Running
    );
    Ok(())
}

#[test]
fn running_workers_are_steered_and_capacity_is_bounded() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let pool = pool(factory.clone(), project.path(), 1)?;
    let started = pool.start(request(project.path()))?;
    assert!(pool.start(request(project.path())).is_err());
    let other = tempfile::tempdir().map_err(|error| error.to_string())?;
    assert!(pool.start(request(other.path())).is_err());

    pool.send(&started.id, "redirect".into(), WorkerMessageMode::Auto)?;
    let handle = factory
        .sessions
        .lock()
        .map_err(|_| "fake sessions unavailable".to_owned())?[0]
        .clone();
    let deadline = Instant::now() + Duration::from_secs(2);
    while handle
        .sent
        .lock()
        .map_err(|_| "fake sends unavailable".to_owned())?
        .len()
        < 2
    {
        if Instant::now() >= deadline {
            return Err("worker did not receive steering message".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        handle
            .sent
            .lock()
            .map_err(|_| "fake sends unavailable".to_owned())?[1],
        WorkerSendMode::Steer
    );
    assert_eq!(pool.stop(&started.id)?.status, WorkerStatus::Stopped);
    Ok(())
}

#[test]
fn projects_from_later_calling_sessions_can_be_allowed() -> Result<(), String> {
    let startup_project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let later_project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let pool = pool(factory, startup_project.path(), 1)?;

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
