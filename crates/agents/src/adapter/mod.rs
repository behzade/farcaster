use crate::Backend;
mod acp;
mod antigravity;
mod auxiliary;
mod backend;
mod child_stderr;
mod claude;
#[allow(dead_code)]
mod codex;
mod cursor;
mod farcaster_mcp;
mod handler;
#[cfg(test)]
mod live_basic_tests;
#[cfg(test)]
mod live_input_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod live_tests;
mod main_session;
#[allow(dead_code)]
mod opencode;
mod pi;
mod process_command;
mod prompt_boundary;
mod queued_session;
mod session_storage;
mod shell_environment;
use backend::for_backend;
pub fn profile_data_environment_key(backend: Backend) -> Option<&'static str> {
    for_backend(backend).profile_data_environment_key()
}
pub use session_storage::{
    delete_session_family, delete_session_family_with_config, discover_sessions_for,
    discover_sessions_for_profile, load_session_history, load_session_history_for_profile,
    move_session_family, move_session_family_with_config, supports_session_move,
    validate_session_move,
};
mod trust;
pub use trust::{
    apply_project_trust, project_trust, project_trust_description, saved_project_trust,
};

pub use auxiliary::{generate_session_title, supports_auto_title_generation};
#[cfg(feature = "test-support")]
pub use shell_environment::set_test_project_environment;
pub use shell_environment::{
    app_shell_environment, default_login_shell, project_shell_environment,
};

pub fn available_access_modes(
    harness: impl Into<Option<Backend>>,
    model: Option<&crate::extensions::Model>,
    sandbox_adapter: Option<&str>,
) -> Vec<crate::HarnessAccessMode> {
    let Some(harness) = harness.into() else {
        return Vec::new();
    };
    for_backend(harness).access_modes(model, sandbox_adapter)
}

pub fn supports_sandbox_discovery(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    for_backend(harness).supports_sandbox_discovery()
}

pub fn supports_steering(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    for_backend(harness).descriptor().capabilities.turns.steer
        == super::contract::CapabilitySupport::Available
}

pub fn supports_individual_queue_cancellation(harness: impl Into<Option<Backend>>) -> bool {
    harness.into().is_some()
}

pub fn supports_reasoning_effort(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    for_backend(harness)
        .descriptor()
        .capabilities
        .configuration
        .reasoning_effort
        == super::contract::CapabilitySupport::Available
}

pub fn supports_reasoning_reset(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    for_backend(harness)
        .descriptor()
        .capabilities
        .configuration
        .reset_reasoning_effort
        == super::contract::CapabilitySupport::Available
}

pub fn effort_label(harness: impl Into<Option<Backend>>) -> &'static str {
    let Some(harness) = harness.into() else {
        return "Effort";
    };
    for_backend(harness)
        .descriptor()
        .capabilities
        .configuration
        .effort_label
}

pub fn supports_session_fork(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    for_backend(harness).descriptor().capabilities.sessions.fork
        == super::contract::CapabilitySupport::Available
}

pub fn validate_launch(
    config: &crate::AgentLaunchConfig,
    harness: impl Into<Option<Backend>>,
    project: &std::path::Path,
) -> Result<(), String> {
    let Some(harness) = harness.into() else {
        return Err("Choose a backend before launching a session.".into());
    };
    launch_configuration(config, harness)?
        .command(project)
        .map(|_| ())
}

fn launch_configuration(
    config: &crate::AgentLaunchConfig,
    harness: Backend,
) -> Result<crate::AgentLaunchConfig, String> {
    config.validate_profile_backend(harness)?;
    Ok(for_backend(harness).launch_configuration(config))
}

pub fn worker_factories(
    config: crate::AgentLaunchConfig,
) -> (
    std::collections::BTreeMap<Backend, std::sync::Arc<dyn crate::WorkerSessionFactory>>,
    Backend,
) {
    let factories = Backend::ALL
        .into_iter()
        .map(|backend| (backend, for_backend(backend).worker_factory(config.clone())))
        .collect();
    (factories, Backend::Pi)
}

pub fn load_configuration_catalog(
    config: &crate::AgentLaunchConfig,
    harness: Backend,
    project: &std::path::Path,
) -> Result<crate::ConfigurationCatalog, String> {
    for_backend(harness).configuration_catalog(config, project)
}

fn configuration_launch(
    config: &crate::AgentLaunchConfig,
    harness: Backend,
) -> Result<crate::AgentLaunchConfig, String> {
    config.validate_profile_backend(harness)?;
    let mut command = for_backend(harness).launch_configuration(config);
    command.access_mode = configuration_access_mode(harness, config.access_mode)?;
    Ok(command)
}

fn configuration_access_mode(
    harness: Backend,
    requested: crate::HarnessAccessMode,
) -> Result<crate::HarnessAccessMode, String> {
    use crate::HarnessAccessMode::{Auto, Sandboxed};

    if supports_sandbox_discovery(harness) {
        return Ok(requested);
    }
    let descriptor = for_backend(harness).descriptor();
    let supported = descriptor.capabilities.configuration.access_modes;
    if supported.contains(&requested) {
        return Ok(requested);
    }
    if requested == Auto && supported.contains(&Sandboxed) {
        return Ok(Sandboxed);
    }
    Err(format!(
        "{harness} does not support the requested {requested:?} access mode"
    ))
}

fn configuration_catalog(
    metadata: main_session::MainSessionMetadata,
) -> Result<crate::ConfigurationCatalog, String> {
    let models = metadata
        .models
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("decode model catalog: {error}"))?;
    Ok(crate::ConfigurationCatalog {
        models,
        efforts: metadata.efforts,
        sandbox_adapter: None,
    })
}

pub fn spawn_session(
    config: &crate::AgentLaunchConfig,
    launch: crate::SessionLaunch,
) -> Result<Box<dyn crate::SessionTransport>, String> {
    use queued_session::SteeringBoundary;
    let policy = for_backend(launch.harness).steering_boundary();
    let hook = matches!(
        policy,
        SteeringBoundary::Held | SteeringBoundary::StopAfterBatch
    )
    .then(|| prompt_boundary::PromptBoundary::new(launch.wake.clone()))
    .transpose()?;
    let mut config = config.clone();
    config.prompt_boundary_url = hook.as_ref().map(|hook| hook.url.clone());
    let inner = spawn_native_session(&config, launch)?;
    Ok(Box::new(queued_session::QueuedSession::new(
        inner, policy, hook,
    )))
}

fn spawn_native_session(
    config: &crate::AgentLaunchConfig,
    launch: crate::SessionLaunch,
) -> Result<Box<dyn crate::SessionTransport>, String> {
    config.validate_profile_backend(launch.harness)?;
    let adapter = for_backend(launch.harness);
    adapter.validate_launch_locator(&launch)?;
    adapter.spawn(config, launch)
}

pub fn rename_session(
    config: &crate::AgentLaunchConfig,
    harness: Backend,
    project: &std::path::Path,
    session: &std::path::Path,
    session_id: &str,
    name: &str,
) -> Result<(), String> {
    session_storage::validate_session_target(&farcaster_sessions::SessionTarget {
        harness,
        id: session_id.into(),
        path: session.into(),
    })?;
    let mut config = config.clone();
    config.profile_id = crate::profile_id_from_locator(session);
    config.validate_profile_backend(harness)?;
    for_backend(harness).rename_session(&config, project, session, session_id, name)
}

pub fn external_session_identity(path: &std::path::Path) -> Option<(Backend, String)> {
    for backend in [
        claude::BACKEND,
        antigravity::PROFILE.backend,
        Backend::Codex,
        Backend::Cursor,
        Backend::OpenCode,
    ] {
        if let Some(locator) = main_session::external_session_locator(backend, path) {
            return Some((backend, locator));
        }
    }
    None
}

#[cfg(any(test, feature = "test-support"))]
pub fn delete_external_session(path: &std::path::Path) -> Option<Result<(), String>> {
    external_session_identity(path).map(|(harness, locator)| {
        for_backend(harness)
            .delete_session(&locator, path)
            .map(|_| ())
    })
}

pub fn annotate_history_message(harness: Backend, message: &mut serde_json::Value) {
    for_backend(harness).annotate_history_message(message);
}

#[cfg(any(test, feature = "test-support"))]
pub fn load_external_history(
    path: &std::path::Path,
    project: &std::path::Path,
) -> Option<Result<crate::DiscoveredHistory, String>> {
    external_session_identity(path)
        .map(|(harness, _)| for_backend(harness).external_history(path, project))
}

pub fn supports_startup_command(
    harness: impl Into<Option<Backend>>,
    command: &crate::SessionCommand,
) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    use super::contract::CapabilitySupport::Available;
    use crate::SessionCommand;

    let configuration = for_backend(harness).descriptor().capabilities.configuration;
    match command {
        SessionCommand::ListModels => configuration.models == Available,
        SessionCommand::ListReasoningLevels => configuration.reasoning_effort == Available,
        SessionCommand::ListModes => configuration.modes == Available,
        SessionCommand::ListCommands => configuration.commands == Available,
        _ => true,
    }
}

pub fn backend_display_name(harness: impl Into<Option<Backend>>) -> String {
    let Some(harness) = harness.into() else {
        return "Choose a backend".into();
    };
    for_backend(harness).descriptor().name
}

pub fn backend_statuses() -> Vec<super::contract::AgentBackendStatus> {
    known_backend_descriptors()
        .into_iter()
        .map(|descriptor| {
            let program = for_backend(descriptor.id)
                .launch_configuration(&crate::AgentLaunchConfig::default())
                .program;
            super::contract::AgentBackendStatus {
                id: descriptor.id,
                name: descriptor.name,
                available: program_available(&program),
                program,
                capabilities: descriptor.capabilities,
            }
        })
        .collect()
}

fn program_available(program: &std::path::Path) -> bool {
    if program.is_absolute()
        || program
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        return program.is_file();
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(program))
            .any(|candidate| candidate.is_file())
    })
}

pub(super) fn known_backend_descriptors() -> [super::contract::AgentBackendDescriptor; 6] {
    Backend::ALL.map(|backend| for_backend(backend).descriptor())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
