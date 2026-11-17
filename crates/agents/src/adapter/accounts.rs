use crate::{AgentLaunchConfig, Backend, accounts::*};
use std::path::Path;

pub fn account_capabilities(backend: Backend) -> AccountCapabilities {
    super::for_backend(backend).account_capabilities()
}

pub fn read_accounts(
    config: &AgentLaunchConfig,
    backend: Backend,
    project: &Path,
) -> Result<Vec<AccountObservation>, String> {
    config.validate_profile_backend(backend)?;
    super::for_backend(backend).read_accounts(config, project)
}

pub fn logout_account(
    config: &AgentLaunchConfig,
    source: &AccountSource,
    identity: &AccountIdentity,
    project: &Path,
) -> Result<(), String> {
    config.validate_profile_backend(source.backend)?;
    let result =
        super::for_backend(source.backend).logout_account(config, source, identity, project);
    crate::accounts::publish_account_event(AccountEvent::LoginChanged {
        backend: source.backend,
        profile_id: source.profile_id.clone(),
    });
    result
}

pub(super) fn source(
    config: &AgentLaunchConfig,
    backend: Backend,
    project: &Path,
    provider: &str,
    credential_source: String,
) -> AccountSource {
    AccountSource {
        backend,
        profile_id: config.profile_id.clone(),
        context: project.to_string_lossy().into_owned(),
        provider: provider.into(),
        credential_source,
        incarnation: None,
    }
}

pub(super) fn environment(
    command: &std::process::Command,
    name: &str,
) -> Option<std::ffi::OsString> {
    match command.get_envs().find(|(key, _)| *key == name) {
        Some((_, value)) => value.map(std::ffi::OsString::from),
        None => std::env::var_os(name),
    }
}

/// Reads only the provider roster into the neutral model. Secret fields never leave this function.
pub(super) fn stored_connections(
    config: &AgentLaunchConfig,
    backend: Backend,
    project: &Path,
    path: &Path,
) -> Result<Vec<AccountObservation>, String> {
    use std::io::Read as _;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("Account credential store could not be read".into()),
    };
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|_| "Account credential store could not be read")?;
    if bytes.len() > 1_048_576 {
        return Err("Account credential store is too large".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Account credential store format is unsupported")?;
    let values = value
        .as_object()
        .ok_or("Account credential store format is unsupported")?;
    Ok(values
        .iter()
        .filter(|(_, v)| matches!(v["type"].as_str(), Some("oauth" | "api_key" | "api")))
        .map(|(provider, _)| AccountObservation {
            source: source(
                config,
                backend,
                project,
                provider,
                format!("native:{}", path.display()),
            ),
            identity: None,
            auth: AccountAuthState::Configured,
            limits: Vec::new(),
            complete_limits: true,
            limits_failed: false,
            observed_at: account_now(),
            capabilities: AccountCapabilities {
                discover: true,
                ..Default::default()
            },
        })
        .collect())
}
