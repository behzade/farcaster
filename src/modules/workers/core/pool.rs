use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::super::{
    StartWorker, WorkerContext, WorkerInputResponse, WorkerMessageMode, WorkerSnapshot,
    WorkerStatus,
};
use super::port::{WorkerLaunch, WorkerSendMode, WorkerSessionFactory};
use super::run::{self, RunCommand};

const STOP_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_TERMINAL_HISTORY: usize = 64;

#[derive(Clone)]
pub(crate) struct WorkerPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    factories: BTreeMap<String, Arc<dyn WorkerSessionFactory>>,
    default_backend: String,
    allowed_projects: BTreeSet<std::path::PathBuf>,
    maximum: usize,
    state: Mutex<PoolState>,
}

#[derive(Default)]
struct PoolState {
    sequence: u64,
    starting: usize,
    records: BTreeMap<String, WorkerRecord>,
}

struct WorkerRecord {
    snapshot: Arc<Mutex<WorkerSnapshot>>,
    commands: mpsc::Sender<RunCommand>,
    thread: Option<thread::JoinHandle<()>>,
}

impl WorkerPool {
    pub(crate) fn new(
        factories: BTreeMap<String, Arc<dyn WorkerSessionFactory>>,
        default_backend: String,
        allowed_project: std::path::PathBuf,
        maximum: usize,
    ) -> Result<Self, String> {
        if maximum == 0 {
            return Err("worker pool capacity must be positive".into());
        }
        if !factories.contains_key(&default_backend) {
            return Err(format!("unknown default worker backend: {default_backend}"));
        }
        let allowed_project = canonical_directory(&allowed_project)?;
        Ok(Self {
            inner: Arc::new(PoolInner {
                factories,
                default_backend,
                allowed_projects: BTreeSet::from([allowed_project]),
                maximum,
                state: Mutex::new(PoolState::default()),
            }),
        })
    }

    pub(crate) fn default_backend(&self) -> &str {
        &self.inner.default_backend
    }

    pub(crate) fn backends(&self) -> Vec<String> {
        self.inner.factories.keys().cloned().collect()
    }

    pub(crate) fn start(&self, request: StartWorker) -> Result<WorkerSnapshot, String> {
        validate_start(&request)?;
        let project = canonical_directory(&request.project)?;
        if !self.inner.allowed_projects.contains(&project) {
            return Err(format!(
                "worker project is outside this Farcaster instance: {}",
                project.display()
            ));
        }
        let context = match request.context {
            WorkerContext::Fresh => WorkerContext::Fresh,
            WorkerContext::Session { session_locator } => {
                if session_locator.trim().is_empty() {
                    return Err("worker source session locator must not be empty".into());
                }
                WorkerContext::Session { session_locator }
            }
        };
        let factory = self
            .inner
            .factories
            .get(&request.backend)
            .ok_or_else(|| format!("unsupported worker backend: {}", request.backend))?
            .clone();

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        reap_terminal(&mut state);
        let active = state
            .records
            .values()
            .filter(|record| match snapshot(record) {
                Ok(snapshot) => !snapshot.status.terminal(),
                Err(_) => true,
            })
            .count()
            + state.starting;
        if active >= self.inner.maximum {
            return Err(format!(
                "worker pool is full ({active}/{})",
                self.inner.maximum
            ));
        }
        state.sequence = state.sequence.saturating_add(1);
        let id = worker_id(state.sequence)?;
        state.starting = state.starting.saturating_add(1);
        drop(state);

        let prepared = (|| {
            let mut session = factory.create(WorkerLaunch {
                project: project.clone(),
                context,
                provider: request.provider,
                model: request.model,
                effort: request.effort,
            })?;
            session.send(request.prompt, WorkerSendMode::Prompt)?;
            Ok::<_, String>(session)
        })();
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        state.starting = state.starting.saturating_sub(1);
        let session = prepared?;
        let initial = WorkerSnapshot {
            id: id.clone(),
            backend: request.backend,
            project,
            session_locator: None,
            status: WorkerStatus::Running,
            output: None,
            error: None,
            pending_input: None,
        };
        let shared = Arc::new(Mutex::new(initial.clone()));
        let (commands, handle) = run::spawn(&id, session, shared.clone())?;
        state.records.insert(
            id,
            WorkerRecord {
                snapshot: shared,
                commands,
                thread: Some(handle),
            },
        );
        Ok(initial)
    }

    pub(crate) fn send(
        &self,
        id: &str,
        message: String,
        requested_mode: WorkerMessageMode,
    ) -> Result<WorkerSnapshot, String> {
        if message.trim().is_empty() {
            return Err("worker message must not be empty".into());
        }
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        let record = state
            .records
            .get(id)
            .ok_or_else(|| format!("unknown worker: {id}"))?;
        let mut current = record
            .snapshot
            .lock()
            .map_err(|_| format!("worker {id} state is unavailable"))?;
        let mode = match (current.status, requested_mode) {
            (WorkerStatus::Running, WorkerMessageMode::Auto | WorkerMessageMode::Steer) => {
                WorkerSendMode::Steer
            }
            (WorkerStatus::Running, WorkerMessageMode::Prompt) => WorkerSendMode::Queue,
            (WorkerStatus::Idle, WorkerMessageMode::Auto | WorkerMessageMode::Prompt) => {
                WorkerSendMode::Prompt
            }
            (WorkerStatus::Idle, WorkerMessageMode::Steer) => {
                return Err(format!(
                    "cannot steer idle worker {id}; send a prompt instead"
                ));
            }
            (WorkerStatus::Stopping, _) => return Err(format!("worker {id} is stopping")),
            (WorkerStatus::NeedsInput, _) => {
                return Err(format!("worker {id} is awaiting input; use worker_respond"));
            }
            (WorkerStatus::Failed, _) => return Err(format!("worker {id} has failed")),
            (WorkerStatus::Stopped, _) => return Err(format!("worker {id} is stopped")),
        };
        record
            .commands
            .send(RunCommand::Send { message, mode })
            .map_err(|_| format!("worker {id} has exited"))?;
        if current.status == WorkerStatus::Idle {
            current.status = WorkerStatus::Running;
            current.output = None;
            current.error = None;
            current.pending_input = None;
        }
        Ok(current.clone())
    }

    pub(crate) fn respond(
        &self,
        id: &str,
        value: Option<String>,
        cancel: bool,
    ) -> Result<WorkerSnapshot, String> {
        if !cancel && value.is_none() {
            return Err("worker response requires a value or cancellation".into());
        }
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        let record = state
            .records
            .get(id)
            .ok_or_else(|| format!("unknown worker: {id}"))?;
        let mut current = record
            .snapshot
            .lock()
            .map_err(|_| format!("worker {id} state is unavailable"))?;
        if current.status == WorkerStatus::Stopping {
            return Err(format!("worker {id} is stopping"));
        }
        let input = current
            .pending_input
            .as_ref()
            .ok_or_else(|| format!("worker {id} is not awaiting input"))?;
        record
            .commands
            .send(RunCommand::Respond(WorkerInputResponse {
                id: input.id.clone(),
                value,
                cancel,
            }))
            .map_err(|_| format!("worker {id} has exited"))?;
        current.status = WorkerStatus::Running;
        current.pending_input = None;
        Ok(current.clone())
    }

    pub(crate) fn list(&self) -> Result<Vec<WorkerSnapshot>, String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        state.records.values().map(snapshot).collect()
    }

    pub(crate) fn status(&self, id: &str) -> Result<WorkerSnapshot, String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        snapshot(
            state
                .records
                .get(id)
                .ok_or_else(|| format!("unknown worker: {id}"))?,
        )
    }

    pub(crate) fn stop(&self, id: &str) -> Result<WorkerSnapshot, String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        let record = state
            .records
            .get(id)
            .ok_or_else(|| format!("unknown worker: {id}"))?;
        let current = snapshot(record)?;
        if current.status.terminal() {
            return Ok(current);
        }
        if current.status != WorkerStatus::Stopping {
            record
                .snapshot
                .lock()
                .map_err(|_| format!("worker {id} state is unavailable"))?
                .status = WorkerStatus::Stopping;
            if record.commands.send(RunCommand::Stop).is_err() {
                let mut snapshot = record
                    .snapshot
                    .lock()
                    .map_err(|_| format!("worker {id} state is unavailable"))?;
                snapshot.status = WorkerStatus::Failed;
                snapshot.error = Some("worker process exited before stop".into());
                return Ok(snapshot.clone());
            }
        }
        drop(state);
        let deadline = Instant::now() + STOP_TIMEOUT;
        loop {
            let current = self.status(id)?;
            if current.status == WorkerStatus::Stopped {
                return Ok(current);
            }
            if Instant::now() >= deadline {
                return Err(format!("timed out while stopping worker {id}"));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for PoolInner {
    fn drop(&mut self) {
        let Ok(state) = self.state.get_mut() else {
            return;
        };
        for record in state.records.values() {
            let _ = record.commands.send(RunCommand::Stop);
        }
        for record in state.records.values_mut() {
            if let Some(handle) = record.thread.take() {
                let _ = handle.join();
            }
        }
    }
}

fn reap_terminal(state: &mut PoolState) {
    let remove = state
        .records
        .values()
        .filter(|record| snapshot(record).is_ok_and(|snapshot| snapshot.status.terminal()))
        .count()
        .saturating_sub(MAX_TERMINAL_HISTORY);
    let ids = state
        .records
        .iter()
        .filter(|(_, record)| snapshot(record).is_ok_and(|snapshot| snapshot.status.terminal()))
        .map(|(id, _)| id.clone())
        .take(remove)
        .collect::<Vec<_>>();
    for id in ids {
        state.records.remove(&id);
    }
}

fn snapshot(record: &WorkerRecord) -> Result<WorkerSnapshot, String> {
    record
        .snapshot
        .lock()
        .map(|snapshot| snapshot.clone())
        .map_err(|_| "worker state is unavailable".to_owned())
}

fn validate_start(request: &StartWorker) -> Result<(), String> {
    if request.prompt.trim().is_empty() {
        return Err("worker prompt must not be empty".into());
    }
    Ok(())
}

fn canonical_directory(path: &Path) -> Result<std::path::PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve worker project {}: {error}", path.display()))?;
    if !path.is_dir() {
        return Err(format!(
            "worker project is not a directory: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn worker_id(sequence: u64) -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is unavailable".to_owned())?
        .as_nanos();
    Ok(format!("worker-{nanos}-{sequence}"))
}
