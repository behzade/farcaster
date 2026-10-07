use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{AgentLaunchConfig, Backend};

pub enum SignInEvent {
    Url(String),
    Finished(Result<(), String>),
}

pub struct SignIn {
    pub events: async_channel::Receiver<SignInEvent>,
    cancelled: Arc<AtomicBool>,
}

impl SignIn {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl Drop for SignIn {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn supports_sign_in(backend: Backend) -> bool {
    super::for_backend(backend).supports_sign_in()
}

/// Checks local credentials without starting a process or making a network request.
pub fn sign_in_required(backend: Backend, config: &AgentLaunchConfig) -> bool {
    super::for_backend(backend).sign_in_required(config)
}

pub fn sign_in(
    config: AgentLaunchConfig,
    backend: Backend,
    project: PathBuf,
) -> Result<SignIn, String> {
    config.validate_profile_backend(backend)?;
    let (sender, events) = async_channel::unbounded();
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel = cancelled.clone();
    std::thread::Builder::new()
        .name("harness-sign-in".into())
        .spawn(move || {
            let result = super::for_backend(backend).sign_in(&config, &project, &cancel, &|url| {
                let _ = sender.try_send(SignInEvent::Url(url));
            });
            if result.is_ok() {
                super::catalog_cache::invalidate(&config, backend);
            }
            let _ = sender.try_send(SignInEvent::Finished(result));
        })
        .map_err(|error| format!("Start sign-in: {error}"))?;
    Ok(SignIn { events, cancelled })
}
