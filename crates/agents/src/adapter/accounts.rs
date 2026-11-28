use crate::{AgentLaunchConfig, Backend, accounts::*};
use std::path::Path;

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;

pub fn account_capabilities(backend: Backend) -> AccountCapabilities {
    super::for_backend(backend).account_capabilities()
}

pub fn read_accounts(
    config: &AgentLaunchConfig,
    backend: Backend,
    project: &Path,
) -> Result<AccountDiscovery, String> {
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
    super::sign_in::refresh_login(config, source.backend);
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

pub(super) fn environment_source(
    command: &std::process::Command,
    name: &str,
    project: &Path,
) -> Option<String> {
    let value = super::process_command::environment(command, name)?;
    let origin = (std::env::var_os(name).as_ref() != Some(&value)).then(|| {
        project
            .canonicalize()
            .unwrap_or_else(|_| project.to_owned())
            .to_string_lossy()
            .into_owned()
    });
    Some(format!(
        "environment:{}",
        serde_json::to_string(&(name, origin)).expect("environment origin serializes")
    ))
}

/// Reads only the provider roster into the neutral model. Secret fields never leave this function.
pub(super) fn stored_connections(
    config: &AgentLaunchConfig,
    backend: Backend,
    project: &Path,
    path: &Path,
) -> Result<AccountDiscovery, String> {
    use std::io::Read as _;
    let source = source(
        config,
        backend,
        project,
        "",
        format!("native:{}", path.display()),
    );
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AccountDiscovery {
                source,
                observations: Vec::new(),
            });
        }
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
    let observations = values
        .iter()
        .filter(|(_, v)| matches!(v["type"].as_str(), Some("oauth" | "api_key" | "api")))
        .map(|(provider, _)| AccountObservation {
            source: AccountSource {
                provider: provider.clone(),
                ..source.clone()
            },
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
        .collect();
    Ok(AccountDiscovery {
        source,
        observations,
    })
}
