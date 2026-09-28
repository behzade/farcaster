use crate::Backend;
use farcaster_sessions::{AppSessionId, NativeSessionIdentity, SessionKey};
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
    pub child_key: Option<SessionKey>,
    #[serde(default)]
    pub parent_key: Option<SessionKey>,
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
    bindings: Arc<Mutex<Vec<std::sync::Weak<Mutex<Option<CallerSession>>>>>>,
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
    /// The indexed Farcaster path, when known.
    pub session_locator: Option<PathBuf>,
    /// Explicit launch profile; native session paths need not encode it.
    pub harness_profile_id: Option<String>,
    pub app_session_id: Option<AppSessionId>,
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
    session_locator: Option<PathBuf>,
    harness_profile_id: Option<String>,
    backend: Backend,
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    access_mode: crate::HarnessAccessMode,
    parent_worker_id: Option<String>,
    parent_session: Option<CallerSession>,
    binding: SessionBinding,
    assignment: Option<super::WorkerAssignment>,
    activity: WorkerActivityState,
    inbox: mpsc::Sender<PeerMessage>,
    wake: Option<thread::Thread>,
}

pub(super) type SessionBinding = Arc<Mutex<Option<CallerSession>>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CallerSession {
    pub(super) native: NativeSessionIdentity,
    pub(super) key: Option<SessionKey>,
}

impl CallerSession {
    pub(super) fn from_context(context: &CallerContext) -> Self {
        let key = context.app_session_id.map(SessionKey::App).or_else(|| {
            context
                .session_locator
                .as_deref()
                .or_else(|| {
                    Path::new(&context.session)
                        .is_absolute()
                        .then(|| Path::new(&context.session))
                })
                .map(|path| SessionKey::Locator {
                    harness: context.backend,
                    profile_id: context.harness_profile_id.clone(),
                    path: farcaster_sessions::normalize_session_path(path),
                })
        });
        Self {
            native: NativeSessionIdentity {
                project: context.project.clone(),
                harness: context.backend,
                profile_id: context.harness_profile_id.clone(),
                id: context.session.clone(),
            },
            key,
        }
    }

    pub(super) fn same_session(&self, other: &Self) -> bool {
        if self.native.project != other.native.project
            || self.native.harness != other.native.harness
        {
            return false;
        }
        match (&self.key, &other.key) {
            (Some(left), Some(right)) => left == right,
            (None, None) => self.native == other.native,
            _ => false,
        }
    }
}

pub struct CallerIdentity {
    token: String,
    inbox: mpsc::Receiver<PeerMessage>,
    registry: CallerRegistry,
    slot: Option<super::WorkerSlot>,
    pending_message: RefCell<Option<PeerMessage>>,
}

impl CallerContext {
    pub fn session_key(&self) -> Option<SessionKey> {
        CallerSession::from_context(self).key
    }
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
        if let Err(error) = self.try_bind_record(token) {
            zlog::error!("Register caller session: {error}");
        }
    }

    fn try_bind_record(&self, token: &str) -> Result<(), String> {
        let context = {
            let callers = self
                .callers
                .lock()
                .map_err(|_| "caller registry unavailable")?;
            callers
                .get(token)
                .filter(|caller| caller.persist_session)
                .and_then(RegisteredCaller::context)
        };
        let sink = self
            .session_sink
            .lock()
            .map_err(|_| "caller session sink unavailable")?
            .clone();
        let (Some(context), Some(sink)) = (context, sink) else {
            return Ok(());
        };
        let record = sink(&context)?;
        if let Some(caller) = self
            .callers
            .lock()
            .map_err(|_| "caller registry unavailable")?
            .get_mut(token)
            && caller.session.as_deref() == Some(context.session.as_str())
            && caller.session_locator == context.session_locator
            && caller.harness_profile_id == context.harness_profile_id
        {
            caller.session_record = Some(record);
        }
        self.refresh_family(token);
        Ok(())
    }

    // Call before taking the pool lock: the sink may acquire storage locks.
    pub(super) fn refresh_session_bindings(&self, project: &Path) -> Result<(), String> {
        let tokens = self
            .callers
            .lock()
            .map_err(|_| "caller registry unavailable")?
            .iter()
            .filter(|(_, caller)| caller.project == project)
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in tokens {
            self.try_bind_record(&token)?;
        }
        Ok(())
    }

    pub(super) fn worker_bindings(&self, project: &Path) -> Vec<(String, SessionBinding)> {
        self.callers
            .lock()
            .map(|callers| {
                callers
                    .values()
                    .filter(|caller| caller.project == project)
                    .map(|caller| (caller.worker_id.clone(), caller.binding.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn worker_binding(&self, project: &Path, id: &str) -> Option<SessionBinding> {
        self.callers
            .lock()
            .ok()?
            .values()
            .find(|caller| caller.project == project && caller.worker_id == id)
            .map(|caller| caller.binding.clone())
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

    // Replace snapshots only after an exact live binding or storage resolution.
    // Pool/report handles share this binding, so App ID merges update them too.
    fn refresh_family(&self, token: &str) {
        let Some((worker_id, session, old_ids, live_ids, children)) = (|| {
            let mut callers = self.callers.lock().ok()?;
            let caller = callers.get_mut(token)?;
            let worker_id = caller.worker_id.clone();
            let session = caller.session_key()?;
            let previous = caller.binding.lock().ok()?.clone();
            let mut bindings = self.bindings.lock().ok()?;
            bindings.retain(|binding| binding.strong_count() > 0);
            // Retained handles from an earlier process follow only an explicit
            // binding transition from the same exact identity.
            let previous = previous.as_ref().unwrap_or(&session);
            for binding in bindings.iter().filter_map(std::sync::Weak::upgrade) {
                if let Ok(mut value) = binding.lock()
                    && previous.native == session.native
                    && value
                        .as_ref()
                        .is_some_and(|value| value.same_session(previous))
                {
                    *value = Some(session.clone());
                }
            }
            *caller.binding.lock().ok()? = Some(session.clone());
            drop(bindings);
            self.track_binding(&caller.binding);
            let live_ids = callers
                .values()
                .map(|caller| caller.worker_id.clone())
                .collect::<std::collections::HashSet<_>>();
            let mut old_ids = Vec::new();
            let mut children = Vec::new();
            for (token, child) in callers.iter_mut() {
                let exact = child.parent_worker_id.as_deref() == Some(&worker_id);
                let restart = child
                    .parent_worker_id
                    .as_ref()
                    .is_some_and(|id| !live_ids.contains(id))
                    && child
                        .parent_session
                        .as_ref()
                        .is_some_and(|parent| parent.same_session(&session));
                if exact || restart {
                    if restart
                        && let Some(old_id) = child.parent_worker_id.replace(worker_id.clone())
                    {
                        old_ids.push(old_id);
                    }
                    let changed = child.parent_session.as_ref() != Some(&session);
                    child.parent_session = Some(session.clone());
                    if changed || restart {
                        children.push(token.clone());
                    }
                }
            }
            Some((worker_id, session, old_ids, live_ids, children))
        })() else {
            return;
        };
        if let Ok(mut inputs) = self.inputs.lock() {
            for input in inputs.iter_mut().filter(|input| {
                input.parent_id == worker_id
                    || old_ids.contains(&input.parent_id)
                    || (!live_ids.contains(&input.parent_id)
                        && input.parent_session.same_session(&session))
            }) {
                input.parent_id.clone_from(&worker_id);
                input.parent_session = session.clone();
            }
        }
        if let Ok(mut expired) = self.expired_inputs.lock() {
            for input in expired.iter_mut().filter(|input| {
                input.parent_id == worker_id
                    || old_ids.contains(&input.parent_id)
                    || (!live_ids.contains(&input.parent_id) && input.parent.same_session(&session))
            }) {
                input.parent_id.clone_from(&worker_id);
                input.parent = session.clone();
            }
        }
        for child in children {
            self.persist_family(&child);
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
                parent_backend: parent.native.harness,
                parent_session: parent.native.id.clone(),
                parent_key: parent.key.clone(),
                child_key: child.session_key().and_then(|session| session.key),
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
                    session_locator: None,
                    harness_profile_id: None,
                    backend: profile.backend,
                    provider: profile.provider,
                    model: profile.model,
                    effort: profile.effort,
                    access_mode,
                    parent_worker_id: None,
                    parent_session: None,
                    binding: Arc::default(),
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
                session_locator: None,
                harness_profile_id: None,
                backend: profile.backend,
                provider: profile.provider,
                model: profile.model,
                effort: profile.effort,
                access_mode,
                parent_worker_id,
                parent_session,
                binding: Arc::default(),
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
        self.bind_record(token);
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

    pub(super) fn track_binding(&self, binding: &SessionBinding) {
        if let Ok(mut bindings) = self.bindings.lock() {
            bindings.retain(|binding| binding.strong_count() > 0);
            let weak = Arc::downgrade(binding);
            if !bindings.iter().any(|binding| binding.ptr_eq(&weak)) {
                bindings.push(weak);
            }
        }
    }

    pub(super) fn has_worker(&self, id: &str) -> bool {
        self.callers
            .lock()
            .ok()
            .is_some_and(|callers| callers.values().any(|caller| caller.worker_id == id))
    }

    pub fn session_caller(&self, key: &SessionKey) -> Option<(String, CallerProfile)> {
        let callers = self.callers.lock().ok()?;
        let caller = unique_caller(callers.values().filter(|caller| {
            caller
                .session_key()
                .and_then(|session| session.key)
                .as_ref()
                == Some(key)
        }))?;
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

    pub fn session_worker_profile(&self, key: &SessionKey) -> Option<String> {
        let callers = self.callers.lock().ok()?;
        unique_caller(callers.values().filter(|caller| {
            caller
                .session_key()
                .and_then(|session| session.key)
                .as_ref()
                == Some(key)
        }))?
        .assignment
        .as_ref()
        .map(|assignment| assignment.profile.clone())
    }

    pub fn session_parent(&self, backend: Backend, session: &str) -> Option<String> {
        let callers = self.callers.lock().ok()?;
        let child = unique_caller(callers.values().filter(|caller| {
            caller.backend == backend
                && caller.harness_profile_id.is_none()
                && caller.session.as_deref() == Some(session)
        }))?;
        let parent = child.parent_session.as_ref()?;
        (parent.native.harness == child.backend).then(|| parent.native.id.clone())
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
        let Some(parent) = callers
            .values()
            .find(|caller| caller.worker_id == parent.worker_id)
        else {
            return Ok(None);
        };
        Ok(unique_caller(callers.values().filter(|child| {
            child.belongs_to(parent, &callers) && child.worker_name.eq_ignore_ascii_case(name)
        }))
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
                    candidate.worker_id == parent_id && candidate.project == caller.project
                })
                .or_else(|| {
                    caller.parent_session.as_ref().and_then(|parent| {
                        unique_caller(callers.values().filter(|candidate| {
                            candidate
                                .session_key()
                                .is_some_and(|key| key.same_session(parent))
                        }))
                    })
                }),
            None => callers.values().find(|candidate| {
                candidate.belongs_to(caller, &callers)
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
            session_locator: self.session_locator.clone(),
            harness_profile_id: self.harness_profile_id.clone(),
            app_session_id: self.session_record.and_then(AppSessionId::new),
            backend: self.backend,
            provider: self.provider.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            access_mode: self.access_mode,
            parent_worker_id: self.parent_worker_id.clone(),
        })
    }

    fn session_key(&self) -> Option<CallerSession> {
        self.context()
            .map(|context| CallerSession::from_context(&context))
    }

    fn belongs_to(
        &self,
        parent: &RegisteredCaller,
        callers: &HashMap<String, RegisteredCaller>,
    ) -> bool {
        if let Some(parent_id) = &self.parent_worker_id {
            if callers
                .values()
                .any(|caller| &caller.worker_id == parent_id)
            {
                return parent_id == &parent.worker_id;
            }
        }
        self.parent_session
            .as_ref()
            .zip(parent.session_key().as_ref())
            .is_some_and(|(child_parent, candidate)| child_parent.same_session(candidate))
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
        self.registry.refresh_family(&self.token);
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
        self.registry.refresh_family(&self.token);
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
        self.bind_with_locator(session_locator, None);
    }

    /// Set the resolved launch profile before binding a native session.
    pub fn set_harness_profile_id(&self, profile_id: Option<String>) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(caller) = callers.get_mut(&self.token)
            && caller.harness_profile_id != profile_id
        {
            caller.harness_profile_id = profile_id;
            caller.session_record = None;
            caller.execution = None;
        }
    }

    pub fn bind_with_locator(&self, session: impl Into<String>, locator: Option<PathBuf>) {
        let session = session.into();
        let before = self.registry.callers.lock().ok().and_then(|callers| {
            callers
                .get(&self.token)
                .and_then(RegisteredCaller::session_key)
        });
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            if context.session.as_deref() != Some(&session) || context.session_locator != locator {
                context.session_record = None;
                context.execution = None;
            }
            context.session = Some(session);
            context.session_locator = locator;
            context.activity = WorkerActivityState::Idle;
        }
        self.registry.bind_record(&self.token);
        self.registry.refresh_family(&self.token);
        let after = self.registry.callers.lock().ok().and_then(|callers| {
            callers
                .get(&self.token)
                .and_then(RegisteredCaller::session_key)
        });
        if before != after {
            self.registry.persist_family(&self.token);
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

#[derive(Clone)]
pub(super) struct WorkerParent {
    pub(super) id: String,
    pub(super) project: PathBuf,
    pub(super) child_name: String,
    pub(super) binding: SessionBinding,
}

impl WorkerParent {
    pub(super) fn new(id: String, project: PathBuf, child_name: String, _session: String) -> Self {
        let binding = CallerRegistry::shared()
            .worker_binding(&project, &id)
            .unwrap_or_default();
        Self {
            id,
            project,
            child_name,
            binding,
        }
    }

    fn find<'a>(
        &self,
        callers: &'a HashMap<String, RegisteredCaller>,
    ) -> Option<&'a RegisteredCaller> {
        if let Some(exact) = callers
            .values()
            .find(|caller| caller.worker_id == self.id && caller.project == self.project)
        {
            return Some(exact);
        }
        let session = self.binding.lock().ok()?.clone()?;
        unique_caller(callers.values().filter(|caller| {
            caller
                .session_key()
                .is_some_and(|candidate| candidate.same_session(&session))
        }))
    }

    pub(super) fn report(&self, message: String) {
        let registry = CallerRegistry::shared();
        let Ok(callers) = registry.callers.lock() else {
            return;
        };
        let Some(parent) = self.find(&callers) else {
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

fn unique_caller<'a>(
    mut callers: impl Iterator<Item = &'a RegisteredCaller>,
) -> Option<&'a RegisteredCaller> {
    let caller = callers.next()?;
    callers.next().is_none().then_some(caller)
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
