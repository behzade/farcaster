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

pub fn terminal_sign_in(
    config: &AgentLaunchConfig,
    backend: Backend,
    project: &std::path::Path,
    method: &str,
) -> Result<Option<std::process::Command>, String> {
    config.validate_profile_backend(backend)?;
    super::for_backend(backend).terminal_sign_in(config, project, method)
}

pub fn sign_in_methods(backend: Backend) -> &'static [(&'static str, &'static str)] {
    super::for_backend(backend).sign_in_methods()
}

pub fn refresh_login(config: &AgentLaunchConfig, backend: Backend) {
    if backend == Backend::Antigravity {
        super::acp::catalog::invalidate_processes(backend);
    }
    super::catalog_cache::invalidate(config, backend);
    crate::accounts::publish_account_event(crate::accounts::AccountEvent::LoginChanged {
        backend,
        profile_id: config.profile_id.clone(),
    });
}

pub fn supports_sign_in(backend: Backend) -> bool {
    super::for_backend(backend).supports_sign_in()
}

pub fn sign_in_required(backend: Backend, config: &AgentLaunchConfig) -> bool {
    super::for_backend(backend).sign_in_required(config)
}

pub fn sign_in(
    config: AgentLaunchConfig,
    backend: Backend,
    project: PathBuf,
) -> Result<SignIn, String> {
    let method = sign_in_methods(backend)[0].0;
    sign_in_with_method(config, backend, project, method)
}

pub fn sign_in_with_method(
    config: AgentLaunchConfig,
    backend: Backend,
    project: PathBuf,
    method: &str,
) -> Result<SignIn, String> {
    config.validate_profile_backend(backend)?;
    let method = sign_in_methods(backend)
        .iter()
        .find(|(id, _)| *id == method)
        .ok_or("Unknown sign-in method")?
        .0;
    let (sender, events) = async_channel::unbounded();
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel = cancelled.clone();
    std::thread::Builder::new()
        .name("harness-sign-in".into())
        .spawn(move || {
            let result =
                super::for_backend(backend).sign_in(&config, &project, method, &cancel, &|url| {
                    let _ = sender.try_send(SignInEvent::Url(url));
                });
            refresh_login(&config, backend);
            let _ = sender.try_send(SignInEvent::Finished(result));
        })
        .map_err(|error| format!("Start sign-in: {error}"))?;
    Ok(SignIn { events, cancelled })
}
