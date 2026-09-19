use crate::Backend;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use self::setup::{
    canonical_directory, validate_fixed_profile, validate_start, wait_for_setup, worker_id,
};
use super::{
    concurrency::WorkerConcurrency,
    run::RunCommand,
    worker::{WorkerLaunch, WorkerSessionFactory},
};
use crate::contract::{StartWorker, WorkerContext, WorkerSnapshot, WorkerStatus};

#[path = "pool_setup.rs"]
mod setup;

const MAX_TERMINAL_HISTORY: usize = 64;

#[derive(Clone)]
pub struct WorkerPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    factories: BTreeMap<Backend, Arc<dyn WorkerSessionFactory>>,
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
    stopping_families: BTreeSet<(std::path::PathBuf, Backend, String)>,
}

struct WorkerRecord {
    snapshot: Arc<Mutex<WorkerSnapshot>>,
    commands: Option<mpsc::Sender<RunCommand>>,
    thread: Option<thread::JoinHandle<()>>,
    factory: Arc<dyn WorkerSessionFactory>,
    launch: WorkerLaunch,
    assignment: Option<super::WorkerAssignment>,
    restored_access_mode: Option<crate::HarnessAccessMode>,
    parent_backend: Option<Backend>,
    cleanup_confirmed: Arc<AtomicBool>,
    setup_done: Arc<(Mutex<bool>, Condvar)>,
    setup_cancelled: Arc<Mutex<bool>>,
    setup_stop_requested: Arc<AtomicBool>,
    pending_messages: Vec<crate::PeerMessage>,
}

struct ReservedStart {
    id: String,
    prompt: String,
    parent: Option<super::caller::WorkerParent>,
    assignment: Option<super::WorkerAssignment>,
}

enum ReservedStartResult {
    Reserved(ReservedStart),
    Existing {
        assignment: super::WorkerAssignment,
        pending: bool,
    },
}

impl WorkerPool {
    pub fn new(
        factories: BTreeMap<Backend, Arc<dyn WorkerSessionFactory>>,
        default_backend: Backend,
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

    pub fn updates(&self) -> async_channel::Receiver<()> {
        self.inner.update_receiver.clone()
    }

    pub fn restore_families(
        &self,
        families: impl IntoIterator<Item = super::WorkerFamilyLink>,
    ) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        for family in families {
            let Some(routing) = family.routing else {
                continue;
            };
            if !crate::valid_worker_name(&routing.name)
                || family.child_session.trim().is_empty()
                || family.parent_session.trim().is_empty()
                || routing.assignment.execution.harness != family.child_backend
            {
                zlog::warn!("Ignore invalid saved worker route for {}", routing.name);
                continue;
            }
            let project = match canonical_directory(&family.project) {
                Ok(project) => project,
                Err(error) => {
                    zlog::warn!("Ignore saved worker route {}: {error}", routing.name);
                    continue;
                }
            };
            if state.records.values().any(|record| {
                record.launch.project == project
                    && record.launch.parent_session == family.parent_session
                    && record.parent_backend == Some(family.parent_backend)
                    && record
                        .launch
                        .worker_name
                        .eq_ignore_ascii_case(&routing.name)
            }) {
                return Err(format!(
                    "ambiguous saved worker route for child {}",
                    routing.name
                ));
            }
            let Some(factory) = self.inner.factories.get(&family.child_backend).cloned() else {
                zlog::warn!(
                    "Ignore saved worker route {} for unavailable backend {}",
                    routing.name,
                    family.child_backend
                );
                continue;
            };
            state.sequence = state.sequence.saturating_add(1);
            let id = worker_id(state.sequence)?;
            let snapshot = WorkerSnapshot {
                id: id.clone(),
                backend: family.child_backend,
                project: project.clone(),
                session_locator: Some(family.child_session.clone()),
                status: WorkerStatus::Idle,
                output: None,
                error: None,
                pending_input: None,
            };
            state.records.insert(
                id.clone(),
                WorkerRecord {
                    snapshot: Arc::new(Mutex::new(snapshot)),
                    commands: None,
                    thread: None,
                    factory,
                    launch: WorkerLaunch {
                        slot: None,
                        worker_id: id,
                        worker_name: routing.name,
                        project,
                        parent_session: family.parent_session,
                        parent_worker_id: None,
                        context: WorkerContext::Resume {
                            session_locator: family.child_session,
                        },
                        provider: Some(routing.assignment.execution.provider.clone()),
                        model: Some(routing.assignment.execution.model.clone()),
                        effort: routing.assignment.execution.effort.clone(),
                        access_mode: routing.access_mode,
                        app_proxy: None,
                        ephemeral: false,
                    },
                    assignment: Some(routing.assignment),
                    restored_access_mode: Some(routing.access_mode),
                    parent_backend: Some(family.parent_backend),
                    cleanup_confirmed: Arc::new(AtomicBool::new(true)),
                    setup_done: Arc::new((Mutex::new(true), Condvar::new())),
                    setup_cancelled: Arc::new(Mutex::new(false)),
                    setup_stop_requested: Arc::new(AtomicBool::new(false)),
                    pending_messages: Vec::new(),
                },
            );
        }
        Ok(())
    }

    pub fn set_app_proxy(&self, proxy: Option<String>) -> Result<(), String> {
        *self
            .inner
            .app_proxy
            .lock()
            .map_err(|_| "worker proxy configuration is unavailable".to_owned())? = proxy;
        Ok(())
    }

    #[cfg(test)]
    pub fn fence_family(&self, parent: &super::CallerContext) -> Result<(), String> {
        self.inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?
            .stopping_families
            .insert((
                parent.project.clone(),
                parent.backend,
                parent.session.clone(),
            ));
        Ok(())
    }

    #[cfg(test)]
    pub fn app_proxy(&self) -> Result<Option<String>, String> {
        self.inner
            .app_proxy
            .lock()
            .map(|proxy| proxy.clone())
            .map_err(|_| "worker proxy configuration is unavailable".to_owned())
    }

    pub fn allow_project(&self, project: &Path) -> Result<(), String> {
        let project = canonical_directory(project)?;
        self.inner
            .allowed_projects
            .lock()
            .map_err(|_| "worker project registry is unavailable".to_owned())?
            .insert(project);
        Ok(())
    }

    #[cfg(test)]
    pub fn start(&self, request: StartWorker) -> Result<WorkerSnapshot, String> {
        self.start_assigned(request, None)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn start_assigned(
        &self,
        request: StartWorker,
        assignment: Option<super::WorkerAssignment>,
    ) -> Result<WorkerSnapshot, String> {
        let reserved = self.reserve_start(request, assignment, false, None)?;
        self.provision_reserved(reserved)
    }

    pub fn queue_pending_child(
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
            parent.backend,
            parent.session.clone(),
        )) {
            return Err("worker session family is stopping".into());
        }
        let Some(record) = find_pending_child(&mut state, parent, name) else {
            return Ok(None);
        };
        let assignment = record
            .assignment
            .clone()
            .ok_or("pending child has no saved worker assignment")?;
        validate_fixed_profile(&assignment, profile)?;
        crate::validate_child_access(parent.access_mode, record.launch.access_mode)?;
        record.pending_messages.push(crate::PeerMessage {
            from: parent.worker_name.clone(),
            message,
        });
        Ok(Some(assignment))
    }

    pub fn queue_assigned(
        &self,
        request: StartWorker,
        assignment: super::WorkerAssignment,
        concurrent_message: crate::PeerMessage,
    ) -> Result<(super::WorkerAssignment, bool, bool), String> {
        let reserved =
            match self.reserve_start(request, Some(assignment), true, Some(concurrent_message))? {
                ReservedStartResult::Existing {
                    assignment,
                    pending,
                } => return Ok((assignment, false, pending)),
                ReservedStartResult::Reserved(reserved) => reserved,
            };
        let queued_assignment = reserved
            .assignment
            .clone()
            .ok_or("queued child has no saved worker assignment")?;
        let worker_id = reserved.id.clone();
        let pool = self.clone();
        if let Err(error) = thread::Builder::new()
            .name(format!("farcaster-worker-setup-{worker_id}"))
            .spawn(move || {
                let _ = pool.provision_reserved(ReservedStartResult::Reserved(reserved));
            })
        {
            let error = format!("queue worker setup: {error}");
            self.fail_reserved_setup(&worker_id, error.clone(), true);
            return Err(error);
        }
        notify(&self.inner.updates);
        Ok((queued_assignment, true, true))
    }

    fn reserve_start(
        &self,
        request: StartWorker,
        assignment: Option<super::WorkerAssignment>,
        deduplicate_child: bool,
        concurrent_message: Option<crate::PeerMessage>,
    ) -> Result<ReservedStartResult, String> {
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
        let app_proxy = self
            .inner
            .app_proxy
            .lock()
            .map_err(|_| "worker proxy configuration is unavailable".to_owned())?
            .clone();
        let parent = request.parent_worker_id.as_ref().map(|parent_id| {
            super::caller::WorkerParent::new(
                parent_id.clone(),
                project.clone(),
                request.name.clone(),
                request.parent_session.clone(),
            )
        });
        let parent_backend = parent.as_ref().and_then(|parent| parent.backend);
        let prompt = if parent.is_some() {
            format!(
                "You are a Farcaster child worker. Your final answer is automatically sent to your parent after each turn. Farcaster MCP is not available in this child session.\n\n{}",
                request.prompt
            )
        } else {
            request.prompt
        };

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        if parent_backend.as_ref().is_some_and(|backend| {
            state.stopping_families.contains(&(
                project.clone(),
                *backend,
                request.parent_session.clone(),
            ))
        }) {
            return Err("worker session family is stopping".into());
        }
        if deduplicate_child
            && let Some(parent_backend) = parent_backend
            && let Some(record) = state.records.values_mut().find(|record| {
                record.launch.project == project
                    && record.launch.parent_session == request.parent_session
                    && record.parent_backend == Some(parent_backend)
                    && record
                        .launch
                        .worker_name
                        .eq_ignore_ascii_case(&request.name)
                    && snapshot(record).is_ok_and(|snapshot| !snapshot.status.terminal())
            })
        {
            let existing = record
                .assignment
                .clone()
                .ok_or("existing child has no saved worker assignment")?;
            if let Some(requested) = assignment.as_ref() {
                validate_fixed_profile(&existing, Some(&requested.profile))?;
            }
            let pending = snapshot(record)?.status == WorkerStatus::Pending;
            if pending && let Some(message) = concurrent_message {
                record.pending_messages.push(message);
            }
            return Ok(ReservedStartResult::Existing {
                assignment: existing,
                pending,
            });
        }
        join_terminal_runs(&mut state)?;
        reap_terminal(&mut state);
        retire_idle_for_new_worker(&mut state, self.inner.process_limit)?;
        let slot = self.inner.concurrency.reserve()?;
        state.sequence = state.sequence.saturating_add(1);
        let id = worker_id(state.sequence)?;
        let launch = WorkerLaunch {
            slot: Some(slot),
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
            app_proxy,
            ephemeral: false,
        };
        let initial = WorkerSnapshot {
            id: id.clone(),
            backend: request.backend,
            project,
            session_locator: None,
            status: WorkerStatus::Pending,
            output: None,
            error: None,
            pending_input: None,
        };
        state.records.insert(
            id.clone(),
            WorkerRecord {
                snapshot: Arc::new(Mutex::new(initial.clone())),
                commands: None,
                thread: None,
                factory,
                launch,
                assignment: assignment.clone(),
                restored_access_mode: None,
                parent_backend,
                cleanup_confirmed: Arc::new(AtomicBool::new(false)),
                setup_done: Arc::new((Mutex::new(false), Condvar::new())),
                setup_cancelled: Arc::new(Mutex::new(false)),
                setup_stop_requested: Arc::new(AtomicBool::new(false)),
                pending_messages: Vec::new(),
            },
        );
        Ok(ReservedStartResult::Reserved(ReservedStart {
            id,
            prompt,
            parent,
            assignment,
        }))
    }

    pub fn snapshots(&self) -> Result<Vec<WorkerSnapshot>, String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        state.records.values().map(snapshot).collect()
    }

    pub fn stop_session_family(
        &self,
        project: &Path,
        sessions: &[(crate::Backend, std::path::PathBuf)],
    ) -> Result<usize, String> {
        let project = canonical_directory(project)?;
        let mut sessions = sessions
            .iter()
            .map(|(backend, path)| (*backend, path.to_string_lossy().into_owned()))
            .collect::<BTreeSet<_>>();
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        expand_family_sessions(&state, &project, &mut sessions);
        let family_keys = sessions
            .iter()
            .map(|(backend, locator)| (project.clone(), *backend, locator.clone()))
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
                            sessions.contains(&(*backend, record.launch.parent_session.clone()))
                        },
                    ) || current.session_locator.as_ref().is_some_and(|locator| {
                        sessions.contains(&(current.backend, locator.clone()))
                    })))
                .then(|| id.clone())
            })
            .collect::<Vec<_>>();
        let setup_waiters = ids
            .iter()
            .filter_map(|id| {
                let record = state.records.get(id)?;
                let (done, _) = &*record.setup_done;
                let unfinished = done.lock().ok().is_some_and(|done| !*done);
                unfinished.then(|| {
                    (
                        id.clone(),
                        record.setup_done.clone(),
                        record.setup_cancelled.clone(),
                        record.setup_stop_requested.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        if !setup_waiters.is_empty() {
            drop(state);
            for (_, _, setup_cancelled, setup_stop_requested) in &setup_waiters {
                setup_stop_requested.store(true, Ordering::SeqCst);
                *setup_cancelled
                    .lock()
                    .map_err(|_| "worker setup cancellation state is unavailable".to_owned())? =
                    true;
            }
            state = self
                .inner
                .state
                .lock()
                .map_err(|_| "worker pool state is unavailable".to_owned())?;
            for (id, _, _, _) in &setup_waiters {
                let Some(record) = state.records.get_mut(id) else {
                    continue;
                };
                if let Ok(mut current) = record.snapshot.lock()
                    && current.status == WorkerStatus::Pending
                {
                    current.status = WorkerStatus::Stopped;
                    current.pending_input = None;
                }
            }
            notify(&self.inner.updates);
            drop(state);
            for (_, setup_done, _, _) in &setup_waiters {
                wait_for_setup(setup_done)?;
            }
            state = self
                .inner
                .state
                .lock()
                .map_err(|_| "worker pool state is unavailable".to_owned())?;
        }
        let mut failures = Vec::new();
        for id in &ids {
            let Some(record) = state.records.get_mut(id) else {
                continue;
            };
            if record.thread.is_some() {
                if let Err(error) = finish_run(record, RunCommand::Stop) {
                    failures.push(format!("{}: {error}", record.launch.worker_name));
                    continue;
                }
            } else if let Ok(mut current) = record.snapshot.lock()
                && (current.status == WorkerStatus::Pending
                    || record.cleanup_confirmed.load(Ordering::SeqCst))
            {
                current.status = WorkerStatus::Stopped;
                current.pending_input = None;
            }
            let current = snapshot(record)?;
            if current.status != WorkerStatus::Stopped
                || !record.cleanup_confirmed.load(Ordering::SeqCst)
            {
                failures.push(current.error.unwrap_or_else(|| {
                    format!("worker {} process cleanup was not confirmed", current.id)
                }));
            }
        }
        if !ids.is_empty() {
            notify(&self.inner.updates);
        }
        if failures.is_empty() {
            Ok(ids.len())
        } else {
            Err(failures.join("; "))
        }
    }

    pub fn finish_session_family_stop(
        &self,
        project: &Path,
        sessions: &[(crate::Backend, std::path::PathBuf)],
    ) -> Result<(), String> {
        let project = canonical_directory(project)?;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "worker pool state is unavailable".to_owned())?;
        let mut sessions = sessions
            .iter()
            .map(|(backend, locator)| (*backend, locator.to_string_lossy().into_owned()))
            .collect::<BTreeSet<_>>();
        expand_family_sessions(&state, &project, &mut sessions);
        for (backend, locator) in sessions {
            state
                .stopping_families
                .remove(&(project.clone(), backend, locator));
        }
        Ok(())
    }

    pub fn resume_child(
        &self,
        parent: &super::CallerContext,
        name: &str,
        message: String,
        profile: Option<&str>,
        route: impl Fn(
            &super::WorkerAssignment,
            crate::HarnessAccessMode,
        ) -> Option<crate::HarnessAccessMode>,
    ) -> Result<Option<super::WorkerAssignment>, String> {
        let (reserved, assignment) = {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| "worker pool state is unavailable".to_owned())?;
            if state.stopping_families.contains(&(
                parent.project.clone(),
                parent.backend,
                parent.session.clone(),
            )) {
                return Err("worker session family is stopping".into());
            }
            let Some(id) = state.records.iter().find_map(|(id, record)| {
                (record.launch.project == parent.project
                    && record.launch.parent_session == parent.session
                    && record.parent_backend == Some(parent.backend)
                    && record.launch.worker_name.eq_ignore_ascii_case(name)
                    && record.thread.is_none()
                    && snapshot(record).is_ok_and(|snapshot| snapshot.status == WorkerStatus::Idle))
                .then(|| id.clone())
            }) else {
                return Ok(None);
            };
            retire_idle_for_new_worker(&mut state, self.inner.process_limit)?;
            let app_proxy = self
                .inner
                .app_proxy
                .lock()
                .map_err(|_| "worker proxy configuration is unavailable".to_owned())?
                .clone();
            let record = state.records.get_mut(&id).expect("selected worker exists");
            let locator = snapshot(record)?
                .session_locator
                .ok_or("retired child has no resumable session locator")?;
            let assignment = record
                .assignment
                .clone()
                .ok_or("retired child has no saved worker assignment")?;
            validate_fixed_profile(&assignment, profile)?;
            let requested_access = record.restored_access_mode.unwrap_or(parent.access_mode);
            crate::validate_child_access(parent.access_mode, requested_access)?;
            let access_mode = route(&assignment, requested_access)
                .ok_or("saved child assignment has no protected access mode for this parent")?;
            if record
                .restored_access_mode
                .is_some_and(|saved| access_mode != saved)
            {
                return Err(
                    "saved child access mode is no longer available for this parent".into(),
                );
            }
            let slot = self.inner.concurrency.reserve()?;
            record.launch.slot = Some(slot);
            record.launch.parent_worker_id = Some(parent.worker_id.clone());
            record.launch.parent_session.clone_from(&parent.session);
            record.launch.context = WorkerContext::Resume {
                session_locator: locator,
            };
            record.launch.access_mode = access_mode;
            record.launch.app_proxy = app_proxy;
            record.cleanup_confirmed.store(false, Ordering::SeqCst);
            record.pending_messages.clear();
            record.setup_stop_requested.store(false, Ordering::SeqCst);
            if let Ok(mut cancelled) = record.setup_cancelled.lock() {
                *cancelled = false;
            }
            let (setup_done, _) = &*record.setup_done;
            if let Ok(mut setup_done) = setup_done.lock() {
                *setup_done = false;
            }
            if let Ok(mut current) = record.snapshot.lock() {
                current.status = WorkerStatus::Pending;
                current.error = None;
                current.pending_input = None;
            }
            let worker_parent = super::caller::WorkerParent::new(
                parent.worker_id.clone(),
                parent.project.clone(),
                record.launch.worker_name.clone(),
                parent.session.clone(),
            );
            let prompt = crate::PeerMessage {
                from: parent.worker_name.clone(),
                message,
            }
            .prompt();
            (
                ReservedStartResult::Reserved(ReservedStart {
                    id,
                    prompt,
                    parent: Some(worker_parent),
                    assignment: Some(assignment.clone()),
                }),
                assignment,
            )
        };
        let ReservedStartResult::Reserved(ref setup) = reserved else {
            return Err("retired child reservation unexpectedly reused another worker".into());
        };
        let worker_id = setup.id.clone();
        let pool = self.clone();
        if let Err(error) = thread::Builder::new()
            .name(format!("farcaster-worker-setup-{worker_id}"))
            .spawn(move || {
                let _ = pool.provision_reserved(reserved);
            })
        {
            let error = format!("queue worker resume: {error}");
            self.fail_reserved_setup(&worker_id, error.clone(), true);
            return Err(error);
        }
        notify(&self.inner.updates);
        Ok(Some(assignment))
    }
}

fn find_pending_child<'a>(
    state: &'a mut PoolState,
    parent: &super::CallerContext,
    name: &str,
) -> Option<&'a mut WorkerRecord> {
    state.records.values_mut().find(|record| {
        record.launch.project == parent.project
            && record.launch.parent_session == parent.session
            && record.parent_backend == Some(parent.backend)
            && record.launch.worker_name.eq_ignore_ascii_case(name)
            && snapshot(record).is_ok_and(|snapshot| snapshot.status == WorkerStatus::Pending)
    })
}

fn expand_family_sessions(
    state: &PoolState,
    project: &Path,
    sessions: &mut BTreeSet<(Backend, String)>,
) {
    loop {
        let descendants = state
            .records
            .values()
            .filter_map(|record| {
                let current = snapshot(record).ok()?;
                (current.project == project
                    && record.parent_backend.as_ref().is_some_and(|backend| {
                        sessions.contains(&(*backend, record.launch.parent_session.clone()))
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
