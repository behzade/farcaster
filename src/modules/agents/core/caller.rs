use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use super::worker::WorkerActivityState;

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
    pub(crate) project: PathBuf,
    pub(crate) session: String,
    pub(crate) backend: String,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) parent_worker_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerMessage {
    pub(crate) from: String,
    pub(crate) message: String,
}

impl PeerMessage {
    pub(crate) fn prompt(&self) -> String {
        format!(
            "Message from Farcaster peer {}:\n\n{}",
            self.from, self.message
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkerPeer {
    pub(crate) id: String,
    pub(crate) backend: String,
    pub(crate) status: WorkerActivityState,
}

struct RegisteredCaller {
    worker_id: String,
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
        let worker_id = new_worker_id();
        self.issue_as(project, profile, wake, worker_id, None)
    }

    pub(crate) fn issue_as(
        &self,
        project: &Path,
        profile: CallerProfile,
        wake: Option<thread::Thread>,
        worker_id: String,
        parent_worker_id: Option<String>,
    ) -> CallerIdentity {
        let token = new_identity("caller");
        let project = canonical_project(project);
        let (inbox, receiver) = mpsc::channel();
        if let Ok(mut callers) = self.callers.lock() {
            callers.insert(
                token.clone(),
                RegisteredCaller {
                    worker_id: worker_id.clone(),
                    project: project.to_owned(),
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
        }
        CallerIdentity {
            token,
            inbox: receiver,
            registry: self.clone(),
        }
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

    pub(crate) fn list(&self, token: &str) -> Result<(String, Vec<WorkerPeer>), String> {
        let callers = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable".to_owned())?;
        let caller = callers
            .get(token)
            .ok_or_else(|| "unknown Farcaster caller".to_owned())?;
        let mut workers = callers
            .values()
            .filter(|peer| visible_to(caller, peer))
            .map(|peer| WorkerPeer {
                id: peer.worker_id.clone(),
                backend: peer.backend.clone(),
                status: peer.activity,
            })
            .collect::<Vec<_>>();
        workers.sort_by(|left, right| left.id.cmp(&right.id));
        Ok((caller.worker_id.clone(), workers))
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

    pub(crate) fn send(&self, token: &str, to: &str, message: String) -> Result<String, String> {
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
        if caller.worker_id == to {
            return Err("cannot send a worker message to yourself".into());
        }
        let recipient = callers
            .values()
            .find(|peer| {
                peer.worker_id == to && peer.project == caller.project && peer.session.is_some()
            })
            .ok_or_else(|| format!("unknown worker in this project: {to}"))?;
        allow_send(caller, recipient)?;
        let worker_id = recipient.worker_id.clone();
        recipient
            .inbox
            .send(PeerMessage {
                from: caller.worker_id.clone(),
                message,
            })
            .map_err(|_| format!("worker {worker_id} is unavailable"))?;
        if let Some(wake) = &recipient.wake {
            wake.unpark();
        }
        Ok(worker_id)
    }

    pub(crate) fn send_from_worker(
        &self,
        from: &str,
        to: &str,
        message: String,
    ) -> Result<String, String> {
        let token = self
            .callers
            .lock()
            .map_err(|_| "worker caller registry is unavailable".to_owned())?
            .iter()
            .find(|(_, caller)| caller.worker_id == from && caller.session.is_some())
            .map(|(token, _)| token.clone())
            .ok_or_else(|| format!("unknown Farcaster worker: {from}"))?;
        self.send(&token, to, message)
    }
}

impl CallerIdentity {
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
        self.inbox.try_recv().ok()
    }
}

impl Drop for CallerIdentity {
    fn drop(&mut self) {
        if let Ok(mut callers) = self.registry.callers.lock() {
            callers.remove(&self.token);
        }
    }
}

fn visible_to(caller: &RegisteredCaller, peer: &RegisteredCaller) -> bool {
    if peer.worker_id == caller.worker_id
        || peer.project != caller.project
        || peer.session.is_none()
    {
        return false;
    }
    match caller.parent_worker_id.as_deref() {
        Some(parent) => peer.worker_id == parent,
        None => peer.parent_worker_id.is_none(),
    }
}

fn allow_send(caller: &RegisteredCaller, recipient: &RegisteredCaller) -> Result<(), String> {
    match (
        caller.parent_worker_id.as_deref(),
        recipient.parent_worker_id.as_deref(),
    ) {
        (Some(parent), _) if parent == recipient.worker_id => Ok(()),
        (Some(_), _) => Err("child workers can only message their parent".into()),
        (_, Some(parent)) if parent == caller.worker_id => Ok(()),
        (_, Some(_)) => Err("only the parent can message a child worker".into()),
        (None, None) => Ok(()),
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

    fn worker_id(registry: &CallerRegistry, identity: &CallerIdentity) -> String {
        registry
            .resolve(identity.token())
            .expect("registered caller")
            .worker_id
    }

    #[test]
    fn resolves_session_with_the_project_and_profile_that_launched_it() {
        let registry = CallerRegistry::default();
        let identity = identity(&registry, Path::new("/project/two"), "pi");
        identity.bind("session-2");
        identity.select_model("anthropic", "sonnet");
        identity.select_effort("high");

        assert_eq!(
            registry.resolve(identity.token()),
            Ok(CallerContext {
                worker_id: worker_id(&registry, &identity),
                project: PathBuf::from("/project/two"),
                session: "session-2".into(),
                backend: "pi".into(),
                provider: Some("anthropic".into()),
                model: Some("sonnet".into()),
                effort: Some("high".into()),
                parent_worker_id: None,
            })
        );
    }

    #[test]
    fn peers_can_list_and_message_each_other_within_a_project() -> Result<(), String> {
        let registry = CallerRegistry::default();
        let first = identity(&registry, Path::new("/project"), "pi");
        let second = identity(&registry, Path::new("/project"), "codex-cli");
        let outsider = identity(&registry, Path::new("/other"), "pi");
        first.bind("session-1");
        second.bind("session-2");
        second.set_activity(WorkerActivityState::Working);
        outsider.bind("session-3");

        let first_id = worker_id(&registry, &first);
        let second_id = worker_id(&registry, &second);
        let outsider_id = worker_id(&registry, &outsider);
        let (self_id, peers) = registry.list(first.token())?;
        assert_eq!(self_id, first_id);
        assert_eq!(peers.len(), 1);
        assert!(!peers.iter().any(|peer| peer.id == self_id));
        assert_eq!(
            peers
                .iter()
                .find(|peer| peer.id == second_id)
                .map(|peer| peer.status),
            Some(WorkerActivityState::Working)
        );
        assert!(!peers.iter().any(|peer| peer.id == outsider_id));

        registry.send(first.token(), &second_id, "check this".into())?;
        assert_eq!(
            second.try_recv(),
            Some(PeerMessage {
                from: first_id,
                message: "check this".into(),
            })
        );
        assert!(
            registry
                .send(first.token(), &outsider_id, "no".into())
                .is_err()
        );

        let starting = registry.issue_as(
            Path::new("/project"),
            CallerProfile {
                backend: "pi".into(),
                provider: None,
                model: None,
                effort: None,
            },
            None,
            "starting-worker".into(),
            None,
        );
        assert!(
            registry
                .send(first.token(), "starting-worker", "no".into())
                .is_err()
        );
        drop(starting);
        Ok(())
    }

    #[test]
    fn canonical_project_paths_share_one_peer_scope() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        std::fs::create_dir(&project)?;
        let aliased = project.join("..").join("project");
        let registry = CallerRegistry::default();
        let first = identity(&registry, &project, "pi");
        let second = identity(&registry, &aliased, "pi");
        first.bind("session-1");
        second.bind("session-2");

        assert_eq!(registry.list(first.token())?.1.len(), 1);
        Ok(())
    }

    #[test]
    fn children_only_list_and_message_their_parent() -> Result<(), String> {
        let registry = CallerRegistry::default();
        let parent = identity(&registry, Path::new("/project"), "pi");
        let peer = identity(&registry, Path::new("/project"), "codex-cli");
        parent.bind("parent-session");
        peer.bind("peer-session");
        let parent_id = worker_id(&registry, &parent);
        let peer_id = worker_id(&registry, &peer);

        let child = registry.issue_as(
            Path::new("/project"),
            CallerProfile {
                backend: "pi".into(),
                provider: None,
                model: None,
                effort: None,
            },
            None,
            "child-1".into(),
            Some(parent_id.clone()),
        );
        child.bind("child-session");

        let (self_id, listed) = registry.list(child.token())?;
        assert_eq!(self_id, "child-1");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, parent_id);

        let (_, parent_list) = registry.list(parent.token())?;
        assert!(parent_list.iter().any(|peer| peer.id == peer_id));
        assert!(!parent_list.iter().any(|peer| peer.id == "child-1"));

        registry.send(child.token(), &parent_id, "review done".into())?;
        assert_eq!(
            parent.try_recv(),
            Some(PeerMessage {
                from: "child-1".into(),
                message: "review done".into(),
            })
        );
        assert!(registry.send(child.token(), &peer_id, "no".into()).is_err());
        assert!(registry.send(peer.token(), "child-1", "no".into()).is_err());

        assert_eq!(
            registry.session_parent("pi", "child-session").as_deref(),
            Some("parent-session")
        );

        registry.send(parent.token(), "child-1", "look at this".into())?;
        assert_eq!(
            child.try_recv(),
            Some(PeerMessage {
                from: parent_id,
                message: "look at this".into(),
            })
        );
        Ok(())
    }
}
