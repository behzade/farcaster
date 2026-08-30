use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
pub(crate) struct CallerRegistry {
    sessions: Arc<Mutex<HashMap<String, String>>>,
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

    pub(crate) fn issue(&self) -> CallerIdentity {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        CallerIdentity {
            token: format!("{}-{nanos}-{sequence}", std::process::id()),
            registry: self.clone(),
        }
    }

    pub(crate) fn resolve(&self, token: &str) -> Result<String, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(session) = self
                .sessions
                .lock()
                .map_err(|_| "worker caller registry is unavailable".to_owned())?
                .get(token)
                .cloned()
            {
                return Ok(session);
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
        if let Ok(mut sessions) = self.registry.sessions.lock() {
            sessions.insert(self.token.clone(), session_locator.into());
        }
    }
}

impl Drop for CallerIdentity {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.registry.sessions.lock() {
            sessions.remove(&self.token);
        }
    }
}
