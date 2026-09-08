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
pub(crate) use inputs::is_child_input_id;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct WorkerFamilyLink {
    pub(crate) project: PathBuf,
    pub(crate) child_backend: String,
    pub(crate) child_session: String,
    pub(crate) parent_backend: String,
    pub(crate) parent_session: String,
    #[serde(default)]
    pub(crate) execution: Option<super::WorkerExecution>,
}

pub(crate) type WorkerFamilySink =
    Arc<dyn Fn(&WorkerFamilyLink) -> Result<(), String> + Send + Sync>;

#[derive(Clone, Default)]
pub(crate) struct CallerRegistry {
    callers: Arc<Mutex<HashMap<String, RegisteredCaller>>>,
    family_sink: Arc<Mutex<Option<WorkerFamilySink>>>,
    inputs: Arc<Mutex<Vec<inputs::PendingInput>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CallerProfile {
    pub(crate) backend: String,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CallerContext {
    pub(crate) worker_id: String,
    pub(crate) worker_name: String,
    pub(crate) project: PathBuf,
    pub(crate) session: String,
    pub(crate) backend: String,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) parent_worker_id: Option<String>,
}

struct RegisteredCaller {
    worker_id: String,
    worker_name: String,
    project: PathBuf,
    session: Option<String>,
    backend: String,
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    parent_worker_id: Option<String>,
    assignment: Option<super::WorkerAssignment>,
    activity: WorkerActivityState,
    inbox: mpsc::Sender<PeerMessage>,
    wake: Option<thread::Thread>,
}

pub(crate) struct CallerIdentity {
    token: String,
    inbox: mpsc::Receiver<PeerMessage>,
    registry: CallerRegistry,
    slot: Option<super::WorkerSlot>,
    pending_message: RefCell<Option<PeerMessage>>,
}

impl CallerRegistry {
    pub(crate) fn shared() -> &'static Self {
        static REGISTRY: OnceLock<CallerRegistry> = OnceLock::new();
        REGISTRY.get_or_init(Self::default)
    }

    pub(crate) fn set_family_sink(&self, sink: Option<WorkerFamilySink>) {
        if let Ok(mut current) = self.family_sink.lock() {
            *current = sink;
        }
    }

    fn persist_family(&self, token: &str) {
        let link = (|| {
            let callers = self.callers.lock().ok()?;
            let child = callers.get(token)?;
            let parent_id = child.parent_worker_id.as_deref()?;
            let parent = callers
                .values()
                .find(|parent| parent.worker_id == parent_id && parent.project == child.project)?;
            Some(WorkerFamilyLink {
                project: child.project.clone(),
                child_backend: child.backend.clone(),
                child_session: child.session.clone()?,
                parent_backend: parent.backend.clone(),
                parent_session: parent.session.clone()?,
                execution: child.provider.as_ref().zip(child.model.as_ref()).map(
                    |(provider, model)| super::WorkerExecution {
                        harness: child.backend.clone(),
                        provider: provider.clone(),
                        model: model.clone(),
                        effort: child.effort.clone(),
                    },
                ),
            })
        })();
        let sink = self.family_sink.lock().ok().and_then(|sink| sink.clone());
        if let (Some(link), Some(sink)) = (link, sink) {
            if let Err(error) = sink(&link) {
                zlog::warn!("Persist worker family: {error}");
            }
        }
    }

    pub(crate) fn issue(
        &self,
        project: &Path,
        profile: CallerProfile,
        wake: Option<thread::Thread>,
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
                    worker_id,
                    worker_name,
                    project,
                    session: None,
                    backend: profile.backend,
                    provider: profile.provider,
                    model: profile.model,
                    effort: profile.effort,
                    parent_worker_id: None,
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

    pub(crate) fn issue_as(
        &self,
        project: &Path,
        profile: CallerProfile,
        wake: Option<thread::Thread>,
        worker_id: String,
        worker_name: String,
        parent_worker_id: Option<String>,
    ) -> Result<CallerIdentity, String> {
        if !crate::agents::valid_worker_name(&worker_name) {
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
        callers.insert(
            token.clone(),
            RegisteredCaller {
                worker_id,
                worker_name,
                project,
                session: None,
                backend: profile.backend,
                provider: profile.provider,
                model: profile.model,
                effort: profile.effort,
                parent_worker_id,
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

    pub(crate) fn resolve(&self, token: &str) -> Result<CallerContext, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(context) = self
                .callers
                .lock()
                .map_err(|_| "worker caller registry is unavailable".to_owned())?
                .get(token)
                .and_then(|caller| {
                    Some(CallerContext {
                        worker_id: caller.worker_id.clone(),
                        worker_name: caller.worker_name.clone(),
                        project: caller.project.clone(),
                        session: caller.session.clone()?,
                        backend: caller.backend.clone(),
                        provider: caller.provider.clone(),
                        model: caller.model.clone(),
                        effort: caller.effort.clone(),
                        parent_worker_id: caller.parent_worker_id.clone(),
                    })
                })
            {
                return Ok(context);
            }
            if std::time::Instant::now() >= deadline {
                return Err("worker caller has not established a persistent session".to_owned());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub(crate) fn is_child(&self, token: &str) -> Result<bool, String> {
        self.callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable".to_owned())?
            .get(token)
            .map(|caller| caller.parent_worker_id.is_some())
            .ok_or_else(|| "unknown Farcaster caller".to_owned())
    }

    pub(crate) fn session_profile(
        &self,
        project: &Path,
        backend: &str,
        session: &str,
    ) -> Option<CallerProfile> {
        let callers = self.callers.lock().ok()?;
        let caller = callers.values().find(|caller| {
            caller.project == project
                && caller.backend == backend
                && caller.session.as_deref() == Some(session)
        })?;
        Some(CallerProfile {
            backend: caller.backend.clone(),
            provider: caller.provider.clone(),
            model: caller.model.clone(),
            effort: caller.effort.clone(),
        })
    }

    pub(crate) fn session_parent(&self, backend: &str, session: &str) -> Option<String> {
        let callers = self.callers.lock().ok()?;
        let child = callers.values().find(|caller| {
            caller.backend == backend && caller.session.as_deref() == Some(session)
        })?;
        let parent_id = child.parent_worker_id.as_deref()?;
        callers
            .values()
            .find(|parent| {
                parent.worker_id == parent_id
                    && parent.backend == child.backend
                    && parent.project == child.project
            })?
            .session
            .clone()
    }

    pub(crate) fn set_assignment(
        &self,
        worker_id: &str,
        assignment: super::WorkerAssignment,
    ) -> Result<(), String> {
        let mut callers = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable")?;
        let caller = callers
            .values_mut()
            .find(|caller| caller.worker_id == worker_id)
            .ok_or("worker is not registered")?;
        caller.assignment = Some(assignment);
        Ok(())
    }

    pub(crate) fn child_assignment(
        &self,
        parent: &CallerContext,
        name: &str,
    ) -> Result<Option<super::WorkerAssignment>, String> {
        let callers = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable")?;
        Ok(callers
            .values()
            .find(|child| {
                child.parent_worker_id.as_deref() == Some(parent.worker_id.as_str())
                    && child.project == parent.project
                    && child.worker_name.eq_ignore_ascii_case(name)
            })
            .and_then(|child| child.assignment.clone()))
    }

    pub(crate) fn native_parent_session(&self, worker_id: &str, backend: &str) -> Option<String> {
        self.callers
            .lock()
            .ok()?
            .values()
            .find(|caller| caller.worker_id == worker_id && caller.backend == backend)?
            .session
            .clone()
    }

    pub(crate) fn send(
        &self,
        token: &str,
        to: &str,
        message: String,
    ) -> Result<Option<String>, String> {
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
            Some(parent_id) => callers.values().find(|candidate| {
                candidate.worker_id == parent_id
                    && candidate.project == caller.project
                    && candidate.session.is_some()
            }),
            None => callers.values().find(|candidate| {
                candidate.parent_worker_id.as_deref() == Some(caller.worker_id.as_str())
                    && candidate.project == caller.project
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
    pub(crate) fn with_slot(mut self, slot: Option<super::WorkerSlot>) -> Self {
        self.slot = slot;
        self
    }

    pub(crate) fn set_slot(&mut self, slot: Option<super::WorkerSlot>) {
        self.slot = slot;
    }

    pub(crate) fn try_activate(&self) -> bool {
        self.slot
            .as_ref()
            .is_none_or(super::WorkerSlot::try_activate)
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    pub(crate) fn worker_identity(&self) -> Option<(String, String)> {
        let callers = self.registry.callers.lock().ok()?;
        let caller = callers.get(&self.token)?;
        Some((caller.worker_id.clone(), caller.worker_name.clone()))
    }

    pub(crate) fn bind(&self, session_locator: impl Into<String>) {
        let session_locator = session_locator.into();
        let mut changed = false;
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            changed = context.session.as_deref() != Some(session_locator.as_str());
            context.session = Some(session_locator);
            context.activity = WorkerActivityState::Idle;
        }
        if changed {
            self.registry.persist_family(&self.token);
        }
    }

    pub(crate) fn set_activity(&self, activity: WorkerActivityState) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            context.activity = activity;
        }
    }

    pub(crate) fn select_model(&self, provider: &str, model: &str) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            context.provider = Some(provider.to_owned());
            context.model = Some(model.to_owned());
        }
        self.registry.persist_family(&self.token);
    }

    pub(crate) fn select_effort(&self, effort: &str) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            context.effort = Some(effort.to_owned());
        }
        self.registry.persist_family(&self.token);
    }

    pub(crate) fn try_recv(&self) -> Option<PeerMessage> {
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
}

pub(super) struct WorkerParent {
    pub(super) id: String,
    pub(super) project: PathBuf,
    pub(super) child_name: String,
}

impl WorkerParent {
    pub(super) fn report(&self, message: String) {
        let registry = CallerRegistry::shared();
        let Ok(callers) = registry.callers.lock() else {
            return;
        };
        let Some(parent) = callers
            .values()
            .find(|caller| caller.worker_id == self.id && caller.project == self.project)
        else {
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
