use crate::Backend;
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::{super::contract::PeerMessage, names, worker::WorkerActivityState};

mod inputs;
pub use inputs::is_child_input_id;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WorkerFamilyLink {
    pub project: PathBuf,
    pub child_backend: Backend,
    pub child_session: String,
    pub parent_backend: Backend,
    pub parent_session: String,
    #[serde(default)]
    pub execution: Option<super::WorkerExecution>,
    #[serde(default)]
    pub routing: Option<WorkerRouting>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WorkerRouting {
    pub name: String,
    pub assignment: super::WorkerAssignment,
    pub access_mode: crate::HarnessAccessMode,
}

pub type WorkerFamilySink = Arc<dyn Fn(&WorkerFamilyLink) -> Result<(), String> + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionBinding {
    pub session_record: i64,
    pub turn_id: String,
    pub prompt_id: Option<String>,
}

pub type SessionRecordSink = Arc<dyn Fn(&CallerContext) -> Result<i64, String> + Send + Sync>;
pub type ExecutionSink =
    Arc<dyn Fn(&CallerContext, &ExecutionBinding) -> Result<i64, String> + Send + Sync>;

#[derive(Clone, Default)]
pub struct CallerRegistry {
    callers: Arc<Mutex<HashMap<String, RegisteredCaller>>>,
    family_sink: Arc<Mutex<Option<WorkerFamilySink>>>,
    inputs: Arc<Mutex<Vec<inputs::PendingInput>>>,
    expired_inputs: Arc<Mutex<Vec<inputs::ExpiredInput>>>,
    session_sink: Arc<Mutex<Option<SessionRecordSink>>>,
    execution_sink: Arc<Mutex<Option<ExecutionSink>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallerProfile {
    pub backend: Backend,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallerContext {
    pub worker_id: String,
    pub worker_name: String,
    pub project: PathBuf,
    pub session: String,
    pub backend: Backend,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub access_mode: crate::HarnessAccessMode,
    pub parent_worker_id: Option<String>,
}

struct RegisteredCaller {
    persist_session: bool,
    session_record: Option<i64>,
    execution: Option<ExecutionBinding>,
    worker_id: String,
    worker_name: String,
    project: PathBuf,
    session: Option<String>,
    backend: Backend,
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    access_mode: crate::HarnessAccessMode,
    parent_worker_id: Option<String>,
    parent_session: Option<CallerSession>,
    assignment: Option<super::WorkerAssignment>,
    activity: WorkerActivityState,
    inbox: mpsc::Sender<PeerMessage>,
    wake: Option<thread::Thread>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CallerSession {
    project: PathBuf,
    backend: Backend,
    session: String,
}

pub struct CallerIdentity {
    token: String,
    inbox: mpsc::Receiver<PeerMessage>,
    registry: CallerRegistry,
    slot: Option<super::WorkerSlot>,
    pending_message: RefCell<Option<PeerMessage>>,
}

impl CallerRegistry {
    pub fn set_execution_sinks(
        &self,
        session: Option<SessionRecordSink>,
        execution: Option<ExecutionSink>,
    ) {
        *self.session_sink.lock().unwrap_or_else(|e| e.into_inner()) = session;
        *self
            .execution_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = execution;
    }

    pub fn resolve_execution(
        &self,
        token: &str,
    ) -> Result<(CallerContext, ExecutionBinding), String> {
        let callers = self
            .callers
            .lock()
            .map_err(|_| "caller registry unavailable")?;
        let caller = callers.get(token).ok_or("unknown Farcaster caller")?;
        let context = caller.context().ok_or("caller session is not bound")?;
        let execution = caller
            .execution
            .clone()
            .ok_or("review requires a registered executing turn")?;
        Ok((context, execution))
    }

    fn bind_record(&self, token: &str) {
        let persistent = self
            .callers
            .lock()
            .ok()
            .and_then(|callers| callers.get(token).map(|caller| caller.persist_session))
            .unwrap_or(false);
        if !persistent {
            return;
        }
        let sink = self.session_sink.lock().ok().and_then(|sink| sink.clone());
        let Some(sink) = sink else { return };
        let result = self.resolve(token).and_then(|context| sink(&context));
        match result {
            Ok(record) => {
                if let Ok(mut callers) = self.callers.lock()
                    && let Some(caller) = callers.get_mut(token)
                {
                    caller.session_record = Some(record);
                }
            }
            Err(error) => {
                zlog::error!("Register caller session: {error}");
            }
        }
    }
    pub fn shared() -> &'static Self {
        static REGISTRY: OnceLock<CallerRegistry> = OnceLock::new();
        REGISTRY.get_or_init(Self::default)
    }

    pub fn set_family_sink(&self, sink: Option<WorkerFamilySink>) {
        if let Ok(mut current) = self.family_sink.lock() {
            *current = sink;
        }
    }

    fn persist_family(&self, token: &str) {
        let link = (|| {
            let callers = self.callers.lock().ok()?;
            let child = callers.get(token)?;
            if !child.persist_session {
                return None;
            }
            let parent = child.parent_session.as_ref()?;
            Some(WorkerFamilyLink {
                project: child.project.clone(),
                child_backend: child.backend,
                child_session: child.session.clone()?,
                parent_backend: parent.backend,
                parent_session: parent.session.clone(),
                execution: child.provider.as_ref().zip(child.model.as_ref()).map(
                    |(provider, model)| super::WorkerExecution {
                        harness: child.backend,
                        provider: provider.clone(),
                        model: model.clone(),
                        effort: child.effort.clone(),
                        service_tier: None,
                    },
                ),
                routing: child.assignment.clone().map(|assignment| WorkerRouting {
                    name: child.worker_name.clone(),
                    assignment,
                    access_mode: child.access_mode,
                }),
            })
        })();
        let sink = self.family_sink.lock().ok().and_then(|sink| sink.clone());
        if let (Some(link), Some(sink)) = (link, sink)
            && let Err(error) = sink(&link)
        {
            zlog::warn!("Persist worker family: {error}");
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn issue(
        &self,
        project: &Path,
        profile: CallerProfile,
        wake: Option<thread::Thread>,
    ) -> CallerIdentity {
        self.issue_with_access(project, profile, wake, crate::HarnessAccessMode::Auto)
    }

    pub fn issue_with_access(
        &self,
        project: &Path,
        profile: CallerProfile,
        wake: Option<thread::Thread>,
        access_mode: crate::HarnessAccessMode,
    ) -> CallerIdentity {
        let token = new_identity("caller");
        let worker_id = new_worker_id();
        let project = canonical_project(project);
        let (inbox, receiver) = mpsc::channel();
        if let Ok(mut callers) = self.callers.lock() {
            let worker_name = names::generated_name(|candidate| {
                callers.values().any(|caller| {
                    caller.project == project
                        && caller.parent_worker_id.is_none()
                        && caller.worker_name.eq_ignore_ascii_case(candidate)
                })
            });
            callers.insert(
                token.clone(),
                RegisteredCaller {
                    persist_session: true,
                    session_record: None,
                    execution: None,
                    worker_id,
                    worker_name,
                    project,
                    session: None,
                    backend: profile.backend,
                    provider: profile.provider,
                    model: profile.model,
                    effort: profile.effort,
                    access_mode,
                    parent_worker_id: None,
                    parent_session: None,
                    assignment: None,
                    activity: WorkerActivityState::Starting,
                    inbox,
                    wake,
                },
            );
        }
        CallerIdentity {
            token,
            inbox: receiver,
            registry: self.clone(),
            slot: None,
            pending_message: RefCell::new(None),
        }
    }

    #[cfg(test)]
    pub fn issue_as(
        &self,
        project: &Path,
        profile: CallerProfile,
        wake: Option<thread::Thread>,
        worker_id: String,
        worker_name: String,
        parent_worker_id: Option<String>,
    ) -> Result<CallerIdentity, String> {
        self.issue_as_with_access(
            project,
            profile,
            wake,
            worker_id,
            worker_name,
            parent_worker_id,
            crate::HarnessAccessMode::Auto,
        )
    }

    // Keep the explicit identity and access fields together at caller registration.
    #[allow(clippy::too_many_arguments)]
    pub fn issue_as_with_access(
        &self,
        project: &Path,
        profile: CallerProfile,
        wake: Option<thread::Thread>,
        worker_id: String,
        worker_name: String,
        parent_worker_id: Option<String>,
        access_mode: crate::HarnessAccessMode,
    ) -> Result<CallerIdentity, String> {
        if !crate::valid_worker_name(&worker_name) {
            return Err("worker name must be 1-48 ASCII letters, numbers, '-' or '_' and cannot start with punctuation".into());
        }
        let token = new_identity("caller");
        let project = canonical_project(project);
        let (inbox, receiver) = mpsc::channel();
        let mut callers = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable".to_owned())?;
        let duplicate = callers.values().any(|caller| {
            caller.project == project
                && caller.parent_worker_id == parent_worker_id
                && caller.worker_name.eq_ignore_ascii_case(&worker_name)
        });
        if duplicate {
            return Err(format!("worker name is already in use: {worker_name}"));
        }
        let parent_session = parent_worker_id.as_deref().and_then(|parent_id| {
            callers
                .values()
                .find(|caller| caller.worker_id == parent_id && caller.project == project)
                .and_then(RegisteredCaller::session_key)
        });
        callers.insert(
            token.clone(),
            RegisteredCaller {
                persist_session: true,
                session_record: None,
                execution: None,
                worker_id,
                worker_name,
                project,
                session: None,
                backend: profile.backend,
                provider: profile.provider,
                model: profile.model,
                effort: profile.effort,
                access_mode,
                parent_worker_id,
                parent_session,
                assignment: None,
                activity: WorkerActivityState::Starting,
                inbox,
                wake,
            },
        );
        drop(callers);
        Ok(CallerIdentity {
            token,
            inbox: receiver,
            registry: self.clone(),
            slot: None,
            pending_message: RefCell::new(None),
        })
    }

    pub fn resolve(&self, token: &str) -> Result<CallerContext, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(context) = self
                .callers
                .lock()
                .map_err(|_| "worker caller registry is unavailable".to_owned())?
                .get(token)
                .and_then(RegisteredCaller::context)
            {
                return Ok(context);
            }
            if std::time::Instant::now() >= deadline {
                return Err("worker caller has not established a persistent session".to_owned());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub fn is_child(&self, token: &str) -> Result<bool, String> {
        self.callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable".to_owned())?
            .get(token)
            .map(|caller| caller.parent_worker_id.is_some())
            .ok_or_else(|| "unknown Farcaster caller".to_owned())
    }

    pub fn session_caller(
        &self,
        project: &Path,
        backend: Backend,
        session: &str,
    ) -> Option<(String, CallerProfile)> {
        let callers = self.callers.lock().ok()?;
        let caller = callers.values().find(|caller| {
            caller.project == project
                && caller.backend == backend
                && caller.session.as_deref() == Some(session)
        })?;
        Some((
            caller.worker_name.clone(),
            CallerProfile {
                backend: caller.backend,
                provider: caller.provider.clone(),
                model: caller.model.clone(),
                effort: caller.effort.clone(),
            },
        ))
    }

    pub fn session_parent(&self, backend: Backend, session: &str) -> Option<String> {
        let callers = self.callers.lock().ok()?;
        let child = callers.values().find(|caller| {
            caller.backend == backend && caller.session.as_deref() == Some(session)
        })?;
        let parent = child.parent_session.as_ref()?;
        (parent.backend == child.backend).then(|| parent.session.clone())
    }

    pub fn set_assignment(
        &self,
        worker_id: &str,
        assignment: super::WorkerAssignment,
    ) -> Result<(), String> {
        let token = {
            let mut callers = self
                .callers
                .lock()
                .map_err(|_| "worker caller registry is unavailable")?;
            let (token, caller) = callers
                .iter_mut()
                .find(|(_, caller)| caller.worker_id == worker_id)
                .ok_or("worker is not registered")?;
            caller.assignment = Some(assignment);
            token.clone()
        };
        self.persist_family(&token);
        Ok(())
    }

    pub fn child_assignment(
        &self,
        parent: &CallerContext,
        name: &str,
    ) -> Result<Option<(super::WorkerAssignment, crate::HarnessAccessMode)>, String> {
        let callers = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable")?;
        Ok(callers
            .values()
            .find(|child| child.belongs_to(parent) && child.worker_name.eq_ignore_ascii_case(name))
            .and_then(|child| {
                child
                    .assignment
                    .clone()
                    .map(|assignment| (assignment, child.access_mode))
            }))
    }

    pub fn native_parent_session(&self, worker_id: &str, backend: Backend) -> Option<String> {
        self.callers
            .lock()
            .ok()?
            .values()
            .find(|caller| caller.worker_id == worker_id && caller.backend == backend)?
            .session
            .clone()
    }

    pub fn send(&self, token: &str, to: &str, message: String) -> Result<Option<String>, String> {
        if message.trim().is_empty() {
            return Err("worker message must not be empty".into());
        }
        let callers = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable".to_owned())?;
        let caller = callers
            .get(token)
            .ok_or_else(|| "unknown Farcaster caller".to_owned())?;
        let recipient = match caller.parent_worker_id.as_deref() {
            Some(parent_id) => callers
                .values()
                .find(|candidate| {
                    candidate.worker_id == parent_id
                        && candidate.project == caller.project
                        && candidate.session.is_some()
                })
                .or_else(|| {
                    caller.parent_session.as_ref().and_then(|parent| {
                        callers
                            .values()
                            .find(|candidate| candidate.session_key().as_ref() == Some(parent))
                    })
                }),
            None => callers.values().find(|candidate| {
                candidate.belongs_to_registered(caller)
                    && candidate.worker_name.eq_ignore_ascii_case(to)
            }),
        };
        let Some(recipient) = recipient else {
            if caller.parent_worker_id.is_some() {
                return Err("parent worker is unavailable".into());
            }
            return Ok(None);
        };
        let recipient_name = recipient.worker_name.clone();
        recipient.send_message(caller.worker_name.clone(), message)?;
        Ok(Some(recipient_name))
    }
}

impl RegisteredCaller {
    fn context(&self) -> Option<CallerContext> {
        Some(CallerContext {
            worker_id: self.worker_id.clone(),
            worker_name: self.worker_name.clone(),
            project: self.project.clone(),
            session: self.session.clone()?,
            backend: self.backend,
            provider: self.provider.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            access_mode: self.access_mode,
            parent_worker_id: self.parent_worker_id.clone(),
        })
    }

    fn session_key(&self) -> Option<CallerSession> {
        Some(CallerSession {
            project: self.project.clone(),
            backend: self.backend,
            session: self.session.clone()?,
        })
    }

    fn belongs_to(&self, parent: &CallerContext) -> bool {
        self.parent_worker_id.as_deref() == Some(parent.worker_id.as_str())
            || self.parent_session.as_ref().is_some_and(|session| {
                session.project == parent.project
                    && session.backend == parent.backend
                    && session.session == parent.session
            })
    }

    fn belongs_to_registered(&self, parent: &RegisteredCaller) -> bool {
        self.parent_worker_id.as_deref() == Some(parent.worker_id.as_str())
            || parent
                .session_key()
                .is_some_and(|session| self.parent_session.as_ref() == Some(&session))
    }

    fn send_message(&self, from: String, message: String) -> Result<(), String> {
        self.inbox
            .send(PeerMessage { from, message })
            .map_err(|_| format!("worker {} is unavailable", self.worker_name))?;
        if let Some(wake) = &self.wake {
            wake.unpark();
        }
        Ok(())
    }
}

impl CallerIdentity {
    /// Keep a backend locator available for in-memory routing without recording it as a session.
    pub fn without_session_persistence(self) -> Self {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(caller) = callers.get_mut(&self.token)
        {
            caller.persist_session = false;
            caller.session_record = None;
            caller.execution = None;
        }
        self
    }

    pub fn ensure_execution(&self) {
        let missing = self
            .registry
            .callers
            .lock()
            .ok()
            .and_then(|callers| {
                callers
                    .get(&self.token)
                    .map(|caller| caller.execution.is_none())
            })
            .unwrap_or(false);
        if missing {
            self.begin_execution(None);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn bind_execution_for_test(&self, execution: ExecutionBinding) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(caller) = callers.get_mut(&self.token)
        {
            caller.session_record = Some(execution.session_record);
            caller.execution = Some(execution);
        }
    }

    /// Called at execution dispatch, never when a queued prompt is admitted.
    /// A missing sink is normal for standalone adapters and isolated tests.
    pub fn begin_execution(&self, prompt_id: Option<&str>) {
        self.registry.bind_record(&self.token);
        let execution = (|| {
            let mut callers = self.registry.callers.lock().ok()?;
            let caller = callers.get_mut(&self.token)?;
            if prompt_id.is_some()
                && caller
                    .execution
                    .as_ref()
                    .is_some_and(|execution| execution.prompt_id.as_deref() == prompt_id)
            {
                return None;
            }
            caller.execution = None;
            let binding = ExecutionBinding {
                session_record: caller.session_record?,
                turn_id: uuid::Uuid::new_v4().to_string(),
                prompt_id: prompt_id.map(str::to_owned),
            };
            let context = caller.context()?;
            Some((binding, context))
        })();
        let Some((mut binding, context)) = execution else {
            return;
        };
        let sink = self
            .registry
            .execution_sink
            .lock()
            .ok()
            .and_then(|sink| sink.clone());
        if let Some(sink) = sink {
            match sink(&context, &binding) {
                Ok(session_record) => binding.session_record = session_record,
                Err(error) => {
                    zlog::error!("Register execution turn: {error}");
                    return;
                }
            }
        }
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(caller) = callers.get_mut(&self.token)
            && caller.session.as_deref() == Some(context.session.as_str())
        {
            caller.session_record = Some(binding.session_record);
            caller.execution = Some(binding);
        }
    }
    pub fn with_slot(mut self, slot: Option<super::WorkerSlot>) -> Self {
        self.slot = slot;
        self
    }

    pub fn set_slot(&mut self, slot: Option<super::WorkerSlot>) {
        self.slot = slot;
    }

    pub fn try_activate(&self) -> bool {
        self.slot
            .as_ref()
            .is_none_or(super::WorkerSlot::try_activate)
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn worker_identity(&self) -> Option<(String, String)> {
        let callers = self.registry.callers.lock().ok()?;
        let caller = callers.get(&self.token)?;
        Some((caller.worker_id.clone(), caller.worker_name.clone()))
    }

    pub fn bind(&self, session_locator: impl Into<String>) {
        let session_locator = session_locator.into();
        let mut changed = false;
        let mut rebound = None;
        if let Ok(mut callers) = self.registry.callers.lock() {
            let session_key = if let Some(context) = callers.get_mut(&self.token) {
                changed = context.session.as_deref() != Some(session_locator.as_str());
                if changed {
                    context.session_record = None;
                    context.execution = None;
                }
                context.session = Some(session_locator);
                context.activity = WorkerActivityState::Idle;
                (context.parent_worker_id.is_none()).then(|| {
                    (
                        context.worker_id.clone(),
                        context.session_key().expect("bound caller has a session"),
                    )
                })
            } else {
                None
            };
            if let Some((worker_id, session_key)) = session_key {
                let mut old_ids = Vec::new();
                for child in callers.values_mut().filter(|caller| {
                    caller.parent_session.as_ref() == Some(&session_key)
                        && caller.parent_worker_id.as_deref() != Some(worker_id.as_str())
                }) {
                    if let Some(old_id) = child.parent_worker_id.replace(worker_id.clone()) {
                        old_ids.push(old_id);
                    }
                }
                rebound = Some((old_ids, worker_id));
            }
        }
        if let Some((old_ids, worker_id)) = rebound
            && let Ok(mut inputs) = self.registry.inputs.lock()
        {
            for input in inputs
                .iter_mut()
                .filter(|input| old_ids.contains(&input.parent_id))
            {
                input.parent_id.clone_from(&worker_id);
            }
        }
        if changed {
            self.registry.persist_family(&self.token);
            self.registry.bind_record(&self.token);
        }
    }

    pub fn set_activity(&self, activity: WorkerActivityState) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            context.activity = activity;
            if activity == WorkerActivityState::Idle {
                context.execution = None;
            }
        }
    }

    pub fn set_access_mode(&self, access_mode: crate::HarnessAccessMode) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            context.access_mode = access_mode;
        }
    }

    pub fn select_model(&self, provider: &str, model: &str) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            context.provider = Some(provider.to_owned());
            context.model = Some(model.to_owned());
        }
        self.registry.persist_family(&self.token);
    }

    pub fn select_effort(&self, effort: &str) {
        self.set_effort(Some(effort));
    }

    pub fn set_effort(&self, effort: Option<&str>) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            context.effort = effort.map(str::to_owned);
        }
        self.registry.persist_family(&self.token);
    }

    pub fn try_recv(&self) -> Option<PeerMessage> {
        let message = self
            .pending_message
            .borrow_mut()
            .take()
            .or_else(|| self.inbox.try_recv().ok())?;
        if self.try_activate() {
            Some(message)
        } else {
            *self.pending_message.borrow_mut() = Some(message);
            None
        }
    }

    pub fn discard_pending_messages(&self) {
        self.pending_message.borrow_mut().take();
        while self.inbox.try_recv().is_ok() {}
    }
}

pub(super) struct WorkerParent {
    pub(super) id: String,
    pub(super) project: PathBuf,
    pub(super) child_name: String,
    pub(super) backend: Option<Backend>,
    session: String,
}

impl WorkerParent {
    pub(super) fn new(id: String, project: PathBuf, child_name: String, session: String) -> Self {
        let backend = CallerRegistry::shared()
            .callers
            .lock()
            .ok()
            .and_then(|callers| {
                callers
                    .values()
                    .find(|caller| caller.worker_id == id && caller.project == project)
                    .map(|caller| caller.backend)
            });
        Self {
            id,
            project,
            child_name,
            backend,
            session,
        }
    }

    fn matches(&self, caller: &RegisteredCaller) -> bool {
        (caller.worker_id == self.id && caller.project == self.project)
            || (caller.project == self.project
                && caller.session.as_deref() == Some(self.session.as_str())
                && self
                    .backend
                    .as_ref()
                    .is_some_and(|backend| caller.backend == *backend))
    }

    pub(super) fn report(&self, message: String) {
        let registry = CallerRegistry::shared();
        let Ok(callers) = registry.callers.lock() else {
            return;
        };
        let Some(parent) = callers.values().find(|caller| self.matches(caller)) else {
            zlog::warn!("Parent unavailable for worker {} report", self.child_name);
            return;
        };
        if let Err(error) = parent.send_message(self.child_name.clone(), message) {
            zlog::warn!("Failed to send worker {} report: {error}", self.child_name);
        }
    }
}

impl Drop for CallerIdentity {
    fn drop(&mut self) {
        if let Ok(mut callers) = self.registry.callers.lock() {
            callers.remove(&self.token);
        }
    }
}

fn canonical_project(project: &Path) -> PathBuf {
    project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf())
}

fn new_worker_id() -> String {
    new_identity("worker")
}

fn new_identity(prefix: &str) -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{prefix}-{nanos}-{sequence}")
}

#[cfg(test)]
#[path = "caller_tests.rs"]
mod tests;
