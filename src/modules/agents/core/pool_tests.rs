use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
};

use super::*;
use crate::agents::{
    WorkerEvent, WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory,
};
use crate::modules::agents::contract::{StartWorker, WorkerContext, WorkerInputResponse};

#[derive(Default)]
struct FakeFactory {
    sends: Mutex<Vec<Arc<Mutex<Vec<WorkerSendMode>>>>>,
}

struct FakeSession {
    events: mpsc::Receiver<WorkerEvent>,
    sent: Arc<Mutex<Vec<WorkerSendMode>>>,
}

impl WorkerSessionFactory for FakeFactory {
    fn create(&self, _launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        let (_events, receiver) = mpsc::channel();
        let sent = Arc::new(Mutex::new(Vec::new()));
        self.sends
            .lock()
            .map_err(|_| "fake sessions unavailable".to_owned())?
            .push(sent.clone());
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

#[test]
fn starts_with_an_initial_prompt_and_enforces_capacity() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory = Arc::new(FakeFactory::default());
    let pool = pool(factory.clone(), project.path(), 1)?;

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
