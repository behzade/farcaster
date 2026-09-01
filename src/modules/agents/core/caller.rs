use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

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
    pub(crate) status: &'static str,
}

struct RegisteredCaller {
    worker_id: String,
    project: PathBuf,
    session: Option<String>,
    backend: String,
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
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
        self.issue_as(project, profile, wake, worker_id)
    }

    pub(crate) fn issue_as(
        &self,
        project: &Path,
        profile: CallerProfile,
        wake: Option<thread::Thread>,
        worker_id: String,
    ) -> CallerIdentity {
        let token = new_identity("caller");
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
            .filter(|peer| peer.project == caller.project && peer.session.is_some())
            .map(|peer| WorkerPeer {
                id: peer.worker_id.clone(),
                backend: peer.backend.clone(),
                status: "active",
            })
            .collect::<Vec<_>>();
        workers.sort_by(|left, right| left.id.cmp(&right.id));
        Ok((caller.worker_id.clone(), workers))
    }

    pub(crate) fn send(&self, token: &str, to: &str, message: String) -> Result<(), String> {
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
            .find(|peer| peer.worker_id == to && peer.project == caller.project)
            .ok_or_else(|| format!("unknown worker in this project: {to}"))?;
        recipient
            .inbox
            .send(PeerMessage {
                from: caller.worker_id.clone(),
                message,
            })
            .map_err(|_| format!("worker {to} is unavailable"))?;
        if let Some(wake) = &recipient.wake {
            wake.unpark();
        }
        Ok(())
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
        outsider.bind("session-3");

        let first_id = worker_id(&registry, &first);
        let second_id = worker_id(&registry, &second);
        let outsider_id = worker_id(&registry, &outsider);
        let (self_id, peers) = registry.list(first.token())?;
        assert_eq!(self_id, first_id);
        assert_eq!(peers.len(), 2);
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
        Ok(())
    }
}
