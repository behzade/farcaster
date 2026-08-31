use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
pub(crate) struct CallerRegistry {
    callers: Arc<Mutex<HashMap<String, RegisteredCaller>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CallerContext {
    pub(crate) project: PathBuf,
    pub(crate) session: String,
}

#[derive(Clone)]
struct RegisteredCaller {
    project: PathBuf,
    session: Option<String>,
}

pub(crate) struct CallerIdentity {
    token: String,
    registry: CallerRegistry,
}

impl CallerRegistry {
    pub(crate) fn shared() -> &'static Self {
        static REGISTRY: OnceLock<CallerRegistry> = OnceLock::new();
        REGISTRY.get_or_init(Self::default)
    }

    pub(crate) fn issue(&self, project: &Path) -> CallerIdentity {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let token = format!("{}-{nanos}-{sequence}", std::process::id());
        if let Ok(mut callers) = self.callers.lock() {
            callers.insert(
                token.clone(),
                RegisteredCaller {
                    project: project.to_owned(),
                    session: None,
                },
            );
        }
        CallerIdentity {
            token,
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
                        project: caller.project.clone(),
                        session: caller.session.clone()?,
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
}

impl Drop for CallerIdentity {
    fn drop(&mut self) {
        if let Ok(mut callers) = self.registry.callers.lock() {
            callers.remove(&self.token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_session_with_the_project_that_launched_it() {
        let registry = CallerRegistry::default();
        let identity = registry.issue(Path::new("/project/two"));
        identity.bind("session-2");

        assert_eq!(
            registry.resolve(identity.token()),
            Ok(CallerContext {
                project: PathBuf::from("/project/two"),
                session: "session-2".into(),
            })
        );
    }
}
