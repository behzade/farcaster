use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    concurrency::WorkerConcurrency,
    run::{self, RunCommand},
    worker::{WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory},
};
use crate::modules::agents::contract::{StartWorker, WorkerContext, WorkerSnapshot, WorkerStatus};

const MAX_TERMINAL_HISTORY: usize = 64;

#[derive(Clone)]
pub(crate) struct WorkerPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    factories: BTreeMap<String, Arc<dyn WorkerSessionFactory>>,
    allowed_projects: Mutex<BTreeSet<std::path::PathBuf>>,
    app_proxy: Mutex<Option<String>>,
    concurrency: WorkerConcurrency,
    process_limit: usize,
    updates: async_channel::Sender<()>,
    update_receiver: async_channel::Receiver<()>,
    state: Mutex<PoolState>,
}

#[derive(Default)]
struct PoolState {
    sequence: u64,
    records: BTreeMap<String, WorkerRecord>,
    stopping_families: BTreeSet<(std::path::PathBuf, String, String)>,
}

struct WorkerRecord {
    snapshot: Arc<Mutex<WorkerSnapshot>>,
    commands: Option<mpsc::Sender<RunCommand>>,
    thread: Option<thread::JoinHandle<()>>,
    factory: Arc<dyn WorkerSessionFactory>,
    launch: WorkerLaunch,
    assignment: Option<super::WorkerAssignment>,
    parent_backend: Option<String>,
    cleanup_confirmed: Arc<AtomicBool>,
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
        let (updates, update_receiver) = async_channel::unbounded();
        Ok(Self {
            inner: Arc::new(PoolInner {
                factories,
                allowed_projects: Mutex::new(BTreeSet::from([allowed_project])),
                app_proxy: Mutex::new(None),
                concurrency: WorkerConcurrency::new(maximum),
                process_limit: maximum,
                updates,
                update_receiver,
                state: Mutex::new(PoolState::default()),
            }),
        })
    }

    pub(crate) fn updates(&self) -> async_channel::Receiver<()> {
        self.inner.update_receiver.clone()
    }

    pub(crate) fn set_app_proxy(&self, proxy: Option<String>) -> Result<(), String> {
        *self
            .inner
            .app_proxy
            .lock()
            .map_err(|_| "worker proxy configuration is unavailable".to_owned())? = proxy;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn app_proxy(&self) -> Result<Option<String>, String> {
        self.inner
            .app_proxy
            .lock()
            .map(|proxy| proxy.clone())
            .map_err(|_| "worker proxy configuration is unavailable".to_owned())
    }

    pub(crate) fn allow_project(&self, project: &Path) -> Result<(), String> {
        let project = canonical_directory(project)?;
        self.inner
            .allowed_projects
            .lock()
            .map_err(|_| "worker project registry is unavailable".to_owned())?
            .insert(project);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn start(&self, request: StartWorker) -> Result<WorkerSnapshot, String> {
        self.start_assigned(request, None)
    }

    pub(crate) fn start_assigned(
        &self,
        request: StartWorker,
        assignment: Option<super::WorkerAssignment>,
    ) -> Result<WorkerSnapshot, String> {
        validate_start(&request)?;
        let project = canonical_directory(&request.project)?;
        if !self
            .inner
            .allowed_projects
            .lock()
            .map_err(|_| "worker project registry is unavailable".to_owned())?
            .contains(&project)
        {
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
            WorkerContext::Resume { session_locator } => {
                return Err(format!(
                    "new workers cannot request a retired session resume: {session_locator}"
                ));
            }
        };
        let factory = self
            .inner
            .factories
            .get(&request.backend)
            .ok_or_else(|| format!("unsupported worker backend: {}", request.backend))?
            .clone();

        let parent = request.parent_worker_id.as_ref().map(|parent_id| {
            super::caller::WorkerParent::new(
                parent_id.clone(),
                project.clone(),
                request.name.clone(),
                request.parent_session.clone(),
            )
        });
        let parent_backend = parent.as_ref().and_then(|parent| parent.backend.clone());

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        if parent_backend.as_ref().is_some_and(|backend| {
            state.stopping_families.contains(&(
                project.clone(),
                backend.clone(),
                request.parent_session.clone(),
            ))
        }) {
            return Err("worker session family is stopping".into());
        }
        join_terminal_runs(&mut state)?;
        reap_terminal(&mut state);
        retire_idle_for_new_worker(&mut state, self.inner.process_limit)?;
        let slot = self.inner.concurrency.reserve()?;
        state.sequence = state.sequence.saturating_add(1);
        let id = worker_id(state.sequence)?;

        let mut launch = WorkerLaunch {
            slot: Some(slot.clone()),
            worker_id: id.clone(),
            worker_name: request.name,
            project: project.clone(),
            parent_session: request.parent_session,
            parent_worker_id: request.parent_worker_id,
            context,
            provider: request.provider,
            model: request.model,
            effort: request.effort,
            access_mode: request.access_mode,
            app_proxy: self
                .inner
                .app_proxy
                .lock()
                .map_err(|_| "worker proxy configuration is unavailable".to_owned())?
                .clone(),
            ephemeral: false,
        };
        let mut session = factory.create(launch.clone())?;
        if let Some(assignment) = assignment.clone()
            && let Err(error) =
                super::CallerRegistry::shared().set_assignment(&id, assignment.clone())
        {
            let (error, cleanup_confirmed) = close_setup_session(&mut *session, error);
            if !cleanup_confirmed {
                slot.release();
                launch.slot = None;
                retain_failed_setup(
                    &mut state,
                    &id,
                    &request.backend,
                    &project,
                    factory,
                    launch,
                    Some(assignment),
                    parent_backend,
                    error.clone(),
                );
                notify(&self.inner.updates);
            }
            return Err(error);
        }
        let prompt = if parent.is_some() {
            format!(
                "You are a Farcaster child worker. Your final answer is automatically sent to your parent after each turn. Farcaster MCP is not available in this child session.\n\n{}",
                request.prompt
            )
        } else {
            request.prompt
        };
        if let Err(error) = session.send(prompt, WorkerSendMode::Prompt) {
            let (error, cleanup_confirmed) = close_setup_session(&mut *session, error);
            if !cleanup_confirmed {
                slot.release();
                launch.slot = None;
                retain_failed_setup(
                    &mut state,
                    &id,
                    &request.backend,
                    &project,
                    factory,
                    launch,
                    assignment,
                    parent_backend,
                    error.clone(),
                );
                notify(&self.inner.updates);
            }
            return Err(error);
        }
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
        let cleanup_confirmed = Arc::new(AtomicBool::new(false));
        let (commands, handle) = run::spawn(
            &id,
            session,
            shared.clone(),
            slot.clone(),
            parent,
            self.inner.updates.clone(),
            cleanup_confirmed.clone(),
        )?;
        state.records.insert(
            id,
            WorkerRecord {
                snapshot: shared,
                commands: Some(commands),
                thread: Some(handle),
                factory,
                launch,
                assignment,
                parent_backend,
                cleanup_confirmed,
            },
        );
        notify(&self.inner.updates);
        Ok(initial)
    }

    pub(crate) fn snapshots(&self) -> Result<Vec<WorkerSnapshot>, String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        state.records.values().map(snapshot).collect()
    }

    pub(crate) fn stop_session_family(
        &self,
        project: &Path,
        sessions: &[(String, std::path::PathBuf)],
    ) -> Result<usize, String> {
        let project = canonical_directory(project)?;
        let mut sessions = sessions
            .iter()
            .map(|(backend, path)| (backend.clone(), path.to_string_lossy().into_owned()))
            .collect::<BTreeSet<_>>();
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        expand_family_sessions(&state, &project, &mut sessions);
        let family_keys = sessions
            .iter()
            .map(|(backend, locator)| (project.clone(), backend.clone(), locator.clone()))
            .collect::<BTreeSet<_>>();
        state.stopping_families.extend(family_keys.iter().cloned());
        let ids = state
            .records
            .iter()
            .filter_map(|(id, record)| {
                let current = snapshot(record).ok()?;
                (current.project == project
                    && (record.parent_backend.as_ref().map_or_else(
                        || {
                            sessions
                                .iter()
                                .any(|(_, locator)| locator == &record.launch.parent_session)
                        },
                        |backend| {
                            sessions
                                .contains(&(backend.clone(), record.launch.parent_session.clone()))
                        },
                    ) || current.session_locator.as_ref().is_some_and(|locator| {
                        sessions.contains(&(current.backend.clone(), locator.clone()))
                    })))
                .then(|| id.clone())
            })
            .collect::<Vec<_>>();
        let mut failures = Vec::new();
        for id in &ids {
            let record = state.records.get_mut(id).expect("selected worker exists");
            if record.thread.is_some() {
                if let Err(error) = finish_run(record, RunCommand::Stop) {
                    failures.push(format!("{}: {error}", record.launch.worker_name));
                    continue;
                }
            } else if record.cleanup_confirmed.load(Ordering::SeqCst) {
                if let Ok(mut current) = record.snapshot.lock() {
                    current.status = WorkerStatus::Stopped;
                    current.pending_input = None;
                }
            }
            let current = snapshot(record)?;
            if current.status != WorkerStatus::Stopped {
                failures.push(
                    current
                        .error
                        .unwrap_or_else(|| format!("worker {} did not stop", current.id)),
                );
            }
        }
        if !ids.is_empty() {
            notify(&self.inner.updates);
        }
        if failures.is_empty() {
            Ok(ids.len())
        } else {
            for key in family_keys {
                state.stopping_families.remove(&key);
            }
            Err(failures.join("; "))
        }
    }

    pub(crate) fn finish_session_family_stop(
        &self,
        project: &Path,
        sessions: &[(String, std::path::PathBuf)],
    ) -> Result<(), String> {
        let project = canonical_directory(project)?;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        let mut sessions = sessions
            .iter()
            .map(|(backend, locator)| (backend.clone(), locator.to_string_lossy().into_owned()))
            .collect::<BTreeSet<_>>();
        expand_family_sessions(&state, &project, &mut sessions);
        for (backend, locator) in sessions {
            state
                .stopping_families
                .remove(&(project.clone(), backend, locator));
        }
        Ok(())
    }

    pub(crate) fn resume_child(
        &self,
        parent: &super::CallerContext,
        name: &str,
        message: String,
        profile: Option<&str>,
    ) -> Result<Option<super::WorkerAssignment>, String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        if state.stopping_families.contains(&(
            parent.project.clone(),
            parent.backend.clone(),
            parent.session.clone(),
        )) {
            return Err("worker session family is stopping".into());
        }
        let Some(id) = state.records.iter().find_map(|(id, record)| {
            (record.launch.project == parent.project
                && record.launch.parent_session == parent.session
                && record.parent_backend.as_deref() == Some(parent.backend.as_str())
                && record.launch.worker_name.eq_ignore_ascii_case(name)
                && record.thread.is_none()
                && snapshot(record).is_ok_and(|snapshot| snapshot.status == WorkerStatus::Idle))
            .then(|| id.clone())
        }) else {
            return Ok(None);
        };
        retire_idle_for_new_worker(&mut state, self.inner.process_limit)?;
        let record = state.records.get_mut(&id).expect("selected worker exists");
        let locator = snapshot(record)?
            .session_locator
            .ok_or("retired child has no resumable session locator")?;
        let assignment = record
            .assignment
            .clone()
            .ok_or("retired child has no saved worker assignment")?;
        if profile.is_some_and(|profile| profile != assignment.profile) {
            return Err(
                "a child's profile is fixed at creation; use a new child name for a different profile"
                    .into(),
            );
        }
        let slot = self.inner.concurrency.reserve()?;
        record.launch.slot = Some(slot.clone());
        record.launch.parent_worker_id = Some(parent.worker_id.clone());
        record.launch.parent_session.clone_from(&parent.session);
        record.launch.context = WorkerContext::Resume {
            session_locator: locator,
        };
        record.launch.access_mode = parent.access_mode;
        record.launch.app_proxy = self
            .inner
            .app_proxy
            .lock()
            .map_err(|_| "worker proxy configuration is unavailable".to_owned())?
            .clone();
        let mut session = match record.factory.create(record.launch.clone()) {
            Ok(session) => session,
            Err(error) => {
                record.launch.slot = None;
                slot.release();
                return Err(error);
            }
        };
        if let Err(error) = super::CallerRegistry::shared()
            .set_assignment(&record.launch.worker_id, assignment.clone())
        {
            let (error, cleanup_confirmed) = close_setup_session(&mut *session, error);
            record.launch.slot = None;
            slot.release();
            record
                .cleanup_confirmed
                .store(cleanup_confirmed, Ordering::SeqCst);
            if !cleanup_confirmed && let Ok(mut current) = record.snapshot.lock() {
                current.status = WorkerStatus::Failed;
                current.error = Some(error.clone());
                current.pending_input = None;
            }
            return Err(error);
        }
        let prompt = crate::agents::PeerMessage {
            from: parent.worker_name.clone(),
            message,
        }
        .prompt();
        if let Err(error) = session.send(prompt, WorkerSendMode::Prompt) {
            let (error, cleanup_confirmed) = close_setup_session(&mut *session, error);
            record.launch.slot = None;
            slot.release();
            record
                .cleanup_confirmed
                .store(cleanup_confirmed, Ordering::SeqCst);
            if !cleanup_confirmed && let Ok(mut current) = record.snapshot.lock() {
                current.status = WorkerStatus::Failed;
                current.error = Some(error.clone());
                current.pending_input = None;
            }
            return Err(error);
        }
        let parent = super::caller::WorkerParent::new(
            parent.worker_id.clone(),
            parent.project.clone(),
            record.launch.worker_name.clone(),
            parent.session.clone(),
        );
        if let Ok(mut current) = record.snapshot.lock() {
            current.status = WorkerStatus::Running;
            current.error = None;
            current.pending_input = None;
        }
        record.cleanup_confirmed.store(false, Ordering::SeqCst);
        let spawned = run::spawn(
            &record.launch.worker_id,
            session,
            record.snapshot.clone(),
            slot.clone(),
            Some(parent),
            self.inner.updates.clone(),
            record.cleanup_confirmed.clone(),
        );
        let (commands, thread) = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                record.launch.slot = None;
                slot.release();
                if let Ok(mut current) = record.snapshot.lock() {
                    current.status = WorkerStatus::Failed;
                    current.error = Some(error.clone());
                }
                return Err(error);
            }
        };
        record.commands = Some(commands);
        record.thread = Some(thread);
        notify(&self.inner.updates);
        Ok(Some(assignment))
    }
}

fn expand_family_sessions(
    state: &PoolState,
    project: &Path,
    sessions: &mut BTreeSet<(String, String)>,
) {
    loop {
        let descendants = state
            .records
            .values()
            .filter_map(|record| {
                let current = snapshot(record).ok()?;
                (current.project == project
                    && record.parent_backend.as_ref().is_some_and(|backend| {
                        sessions.contains(&(backend.clone(), record.launch.parent_session.clone()))
                    }))
                .then(|| {
                    current
                        .session_locator
                        .map(|locator| (current.backend, locator))
                })
                .flatten()
            })
            .collect::<Vec<_>>();
        let before = sessions.len();
        sessions.extend(descendants);
        if sessions.len() == before {
            break;
        }
    }
}

fn notify(updates: &async_channel::Sender<()>) {
    let _ = updates.try_send(());
}

impl Drop for PoolInner {
    fn drop(&mut self) {
        let Ok(state) = self.state.get_mut() else {
            return;
        };
        for record in state.records.values() {
            if let Some(commands) = &record.commands {
                let _ = commands.send(RunCommand::Stop);
            }
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
        .filter(|record| {
            record.cleanup_confirmed.load(Ordering::SeqCst)
                && snapshot(record).is_ok_and(|snapshot| snapshot.status.terminal())
        })
        .count()
        .saturating_sub(MAX_TERMINAL_HISTORY);
    let ids = state
        .records
        .iter()
        .filter(|(_, record)| {
            record.cleanup_confirmed.load(Ordering::SeqCst)
                && snapshot(record).is_ok_and(|snapshot| snapshot.status.terminal())
        })
        .map(|(id, _)| id.clone())
        .take(remove)
        .collect::<Vec<_>>();
    for id in ids {
        state.records.remove(&id);
    }
}

fn join_terminal_runs(state: &mut PoolState) -> Result<(), String> {
    for record in state.records.values_mut() {
        if record.thread.is_none()
            || !snapshot(record).is_ok_and(|snapshot| snapshot.status.terminal())
        {
            continue;
        }
        record.commands.take();
        if record
            .thread
            .take()
            .expect("selected worker thread exists")
            .join()
            .is_err()
        {
            return Err("worker thread panicked during cleanup".into());
        }
        if !record.cleanup_confirmed.load(Ordering::SeqCst) {
            return Err(snapshot(record)?
                .error
                .unwrap_or_else(|| "worker process cleanup was not confirmed".into()));
        }
    }
    Ok(())
}

fn retire_idle_for_new_worker(state: &mut PoolState, process_limit: usize) -> Result<(), String> {
    let live = state
        .records
        .values()
        .filter(|record| {
            record.thread.is_some() || !record.cleanup_confirmed.load(Ordering::SeqCst)
        })
        .count();
    let retire = live.saturating_add(1).saturating_sub(process_limit);
    let ids = state
        .records
        .iter()
        .filter(|(_, record)| {
            record.thread.is_some()
                && snapshot(record).is_ok_and(|snapshot| snapshot.status == WorkerStatus::Idle)
                && record
                    .launch
                    .slot
                    .as_ref()
                    .is_some_and(|slot| !slot.is_active())
        })
        .map(|(id, _)| id.clone())
        .take(retire)
        .collect::<Vec<_>>();
    if ids.len() < retire {
        let unresolved = state.records.values().find_map(|record| {
            (record.thread.is_none() && !record.cleanup_confirmed.load(Ordering::SeqCst))
                .then(|| snapshot(record).ok()?.error)
                .flatten()
        });
        return Err(unresolved.unwrap_or_else(|| {
            format!("worker process limit reached ({process_limit}); no idle worker can retire")
        }));
    }
    for id in ids {
        finish_run(
            state.records.get_mut(&id).expect("selected worker exists"),
            RunCommand::Retire,
        )?;
    }
    Ok(())
}

fn finish_run(record: &mut WorkerRecord, command: RunCommand) -> Result<(), String> {
    let send_error = record.commands.take().and_then(|commands| {
        commands
            .send(command)
            .err()
            .map(|_| "worker command channel is unavailable".to_owned())
    });
    let join_error = record.thread.take().and_then(|thread| {
        thread
            .join()
            .err()
            .map(|_| "worker thread panicked during shutdown".to_owned())
    });
    if let Some(join) = join_error {
        return Err(send_error.map_or(join.clone(), |send| format!("{send}; {join}")));
    }
    if record.cleanup_confirmed.load(Ordering::SeqCst) {
        Ok(())
    } else {
        let cleanup = snapshot(record)?
            .error
            .unwrap_or_else(|| "worker process cleanup was not confirmed".into());
        Err(send_error.map_or(cleanup.clone(), |send| format!("{send}; {cleanup}")))
    }
}

fn snapshot(record: &WorkerRecord) -> Result<WorkerSnapshot, String> {
    record
        .snapshot
        .lock()
        .map(|snapshot| snapshot.clone())
        .map_err(|_| "worker state is unavailable".to_owned())
}

fn close_setup_session(session: &mut dyn WorkerSession, mut error: String) -> (String, bool) {
    match session.close() {
        Ok(()) => (error, true),
        Err(close_error) => {
            error.push_str(&format!("; worker cleanup failed: {close_error}"));
            (error, false)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn retain_failed_setup(
    state: &mut PoolState,
    id: &str,
    backend: &str,
    project: &Path,
    factory: Arc<dyn WorkerSessionFactory>,
    launch: WorkerLaunch,
    assignment: Option<super::WorkerAssignment>,
    parent_backend: Option<String>,
    error: String,
) {
    state.records.insert(
        id.to_owned(),
        WorkerRecord {
            snapshot: Arc::new(Mutex::new(WorkerSnapshot {
                id: id.to_owned(),
                backend: backend.to_owned(),
                project: project.to_owned(),
                session_locator: None,
                status: WorkerStatus::Failed,
                output: None,
                error: Some(error),
                pending_input: None,
            })),
            commands: None,
            thread: None,
            factory,
            launch,
            assignment,
            parent_backend,
            cleanup_confirmed: Arc::new(AtomicBool::new(false)),
        },
    );
}

fn validate_start(request: &StartWorker) -> Result<(), String> {
    if !crate::agents::valid_worker_name(&request.name) {
        return Err("worker name must be 1-48 ASCII letters, numbers, '-' or '_' and cannot start with punctuation".into());
    }
    if request.prompt.trim().is_empty() {
        return Err("worker prompt must not be empty".into());
    }
    if request.parent_session.trim().is_empty() {
        return Err("worker parent session must not be empty".into());
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
