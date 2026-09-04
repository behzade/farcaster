use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{super::contract::PeerMessage, names, worker::WorkerActivityState};

#[derive(Clone, Default)]
pub(crate) struct CallerRegistry {
    callers: Arc<Mutex<HashMap<String, RegisteredCaller>>>,
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
        recipient
            .inbox
            .send(PeerMessage {
                from: caller.worker_name.clone(),
                message,
            })
            .map_err(|_| format!("worker {recipient_name} is unavailable"))?;
        if let Some(wake) = &recipient.wake {
            wake.unpark();
        }
        Ok(Some(recipient_name))
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

    pub(crate) fn bind(&self, session_locator: impl Into<String>) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            context.session = Some(session_locator.into());
            context.activity = WorkerActivityState::Idle;
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
    }

    pub(crate) fn select_effort(&self, effort: &str) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Some(context) = callers.get_mut(&self.token)
        {
            context.effort = Some(effort.to_owned());
        }
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
    pub(super) fn report_failure(&self, error: &str) {
        let registry = CallerRegistry::shared();
        let Ok(callers) = registry.callers.lock() else {
            return;
        };
        let Some(parent) = callers
            .values()
            .find(|caller| caller.worker_id == self.id && caller.project == self.project)
        else {
            return;
        };
        if parent
            .inbox
            .send(PeerMessage {
                from: self.child_name.clone(),
                message: format!("Worker failed: {error}"),
            })
            .is_err()
        {
            zlog::warn!(
                "Failed to notify parent of worker {} failure",
                self.child_name
            );
        } else if let Some(wake) = &parent.wake {
            wake.unpark();
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
mod tests {
    use super::*;

    fn identity(registry: &CallerRegistry, project: &Path, backend: &str) -> CallerIdentity {
        registry.issue(
            project,
            CallerProfile {
                backend: backend.into(),
                provider: None,
                model: None,
                effort: None,
            },
            None,
        )
    }

    fn context(registry: &CallerRegistry, identity: &CallerIdentity) -> CallerContext {
        registry
            .resolve(identity.token())
            .expect("registered caller")
    }

    fn child(
        registry: &CallerRegistry,
        parent: &CallerContext,
        name: &str,
    ) -> Result<CallerIdentity, String> {
        registry.issue_as(
            &parent.project,
            CallerProfile {
                backend: parent.backend.clone(),
                provider: None,
                model: None,
                effort: None,
            },
            None,
            new_worker_id(),
            name.into(),
            Some(parent.worker_id.clone()),
        )
    }

    #[test]
    fn top_level_workers_receive_distinct_human_names() {
        let registry = CallerRegistry::default();
        let first = identity(&registry, Path::new("/project"), "pi");
        let second = identity(&registry, Path::new("/project"), "pi");
        first.bind("session-1");
        second.bind("session-2");

        let first = context(&registry, &first);
        let second = context(&registry, &second);
        assert_ne!(first.worker_name, second.worker_name);
        assert!(crate::agents::valid_worker_name(&first.worker_name));
        assert!(!first.worker_name.starts_with("worker-"));
    }

    #[test]
    fn resolves_session_with_the_project_and_profile_that_launched_it() {
        let registry = CallerRegistry::default();
        let identity = identity(&registry, Path::new("/project/two"), "pi");
        identity.bind("session-2");
        identity.select_model("anthropic", "sonnet");
        identity.select_effort("high");

        let resolved = registry.resolve(identity.token()).expect("context");
        assert_eq!(resolved.project, PathBuf::from("/project/two"));
        assert_eq!(resolved.session, "session-2");
        assert_eq!(resolved.backend, "pi");
        assert_eq!(resolved.provider.as_deref(), Some("anthropic"));
        assert_eq!(resolved.model.as_deref(), Some("sonnet"));
        assert_eq!(resolved.effort.as_deref(), Some("high"));
        assert_eq!(resolved.parent_worker_id, None);
    }

    #[test]
    fn top_level_workers_only_message_their_named_children() -> Result<(), String> {
        let registry = CallerRegistry::default();
        let parent = identity(&registry, Path::new("/project"), "pi");
        let unrelated = identity(&registry, Path::new("/project"), "codex-cli");
        parent.bind("parent-session");
        unrelated.bind("unrelated-session");
        let parent_context = context(&registry, &parent);
        let child = child(&registry, &parent_context, "diff-review")?;
        child.bind("child-session");
        child.set_activity(WorkerActivityState::Working);
        assert!(!registry.is_child(parent.token())?);
        assert!(registry.is_child(child.token())?);

        assert_eq!(
            registry.send(parent.token(), "missing", "work".into())?,
            None
        );
        assert_eq!(
            registry.send(parent.token(), "DIFF-review", "check this".into())?,
            Some("diff-review".into())
        );
        assert_eq!(
            child.try_recv(),
            Some(PeerMessage {
                from: parent_context.worker_name,
                message: "check this".into(),
            })
        );
        assert!(unrelated.try_recv().is_none());
        Ok(())
    }

    #[test]
    fn children_only_report_to_their_parent() -> Result<(), String> {
        let registry = CallerRegistry::default();
        let parent = identity(&registry, Path::new("/project"), "pi");
        parent.bind("parent-session");
        let parent_context = context(&registry, &parent);
        let child = child(&registry, &parent_context, "review")?;
        child.bind("child-session");

        assert_eq!(
            registry.send(child.token(), "ignored", "review done".into())?,
            Some(parent_context.worker_name.clone())
        );
        assert_eq!(
            parent.try_recv(),
            Some(PeerMessage {
                from: "review".into(),
                message: "review done".into(),
            })
        );
        assert_eq!(
            registry.session_parent("pi", "child-session").as_deref(),
            Some("parent-session")
        );
        Ok(())
    }

    #[test]
    fn child_names_are_valid_and_unique_within_the_parent() -> Result<(), String> {
        let registry = CallerRegistry::default();
        let first_parent = identity(&registry, Path::new("/project"), "pi");
        let second_parent = identity(&registry, Path::new("/project"), "pi");
        first_parent.bind("first-parent");
        second_parent.bind("second-parent");
        let first = context(&registry, &first_parent);
        let second = context(&registry, &second_parent);

        let _first_child = child(&registry, &first, "review")?;
        assert!(child(&registry, &first, "REVIEW").is_err());
        assert!(child(&registry, &first, "bad name").is_err());
        assert!(child(&registry, &second, "review").is_ok());
        Ok(())
    }

    #[test]
    fn queued_child_message_waits_for_capacity_without_being_lost() -> Result<(), String> {
        let registry = CallerRegistry::default();
        let parent = identity(&registry, Path::new("/project"), "pi");
        parent.bind("parent");
        let concurrency = super::super::concurrency::WorkerConcurrency::new(1);
        let slot = concurrency.reserve()?;
        let child =
            child(&registry, &context(&registry, &parent), "review")?.with_slot(Some(slot.clone()));
        child.bind("child");
        slot.release();
        let other = concurrency.reserve()?;
        registry.send(parent.token(), "review", "first".into())?;
        registry.send(parent.token(), "review", "second".into())?;
        assert!(child.try_recv().is_none());
        assert!(child.try_recv().is_none());
        drop(other);
        assert_eq!(child.try_recv().expect("first message").message, "first");
        assert!(concurrency.reserve().is_err(), "delivery reserves capacity");
        assert_eq!(child.try_recv().expect("second message").message, "second");
        assert!(child.try_recv().is_none());
        Ok(())
    }
}
