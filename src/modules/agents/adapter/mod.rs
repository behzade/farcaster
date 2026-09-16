use crate::agents::Backend;
mod acp;
mod antigravity;
mod auxiliary;
mod child_stderr;
mod claude;
#[allow(dead_code)]
mod codex;
mod cursor;
mod farcaster_mcp;
#[cfg(test)]
mod live_basic_tests;
#[cfg(test)]
mod live_input_tests;
#[cfg(test)]
pub(crate) mod live_tests;
mod main_session;
#[allow(dead_code)]
mod opencode;
mod pi;
mod process_command;
mod session_storage;
mod shell_environment;
pub(crate) use session_storage::{
    delete_session_family, discover_sessions_for, load_session_history, move_session_family,
    supports_session_move, validate_session_move,
};
mod trust;
pub(crate) use trust::{
    apply_project_trust, project_trust, project_trust_description, saved_project_trust,
};

pub(crate) use auxiliary::{generate_session_title, supports_auto_title_generation};
pub(crate) use shell_environment::{
    app_shell_environment, default_login_shell, project_shell_environment,
};

fn external_acp_profile(harness: Backend) -> Option<&'static acp::AcpProfile> {
    match harness {
        Backend::Antigravity => Some(&antigravity::PROFILE),
        Backend::Pi | Backend::Codex | Backend::Cursor | Backend::OpenCode | Backend::Claude => {
            None
        }
    }
}

pub(crate) fn available_access_modes(
    harness: impl Into<Option<Backend>>,
    model: Option<&crate::protocol::Model>,
    sandbox_adapter: Option<&str>,
) -> Vec<crate::agents::HarnessAccessMode> {
    let Some(harness) = harness.into() else {
        return Vec::new();
    };
    if harness == Backend::Pi {
        return pi::sandbox::access_modes(sandbox_adapter).to_vec();
    }
    let descriptor = harness.descriptor();
    let capabilities = descriptor.capabilities.configuration;
    let declared = model.and_then(|model| model.access_modes.as_deref());
    capabilities
        .access_modes
        .iter()
        .copied()
        .filter(|mode| {
            declared.map_or(
                !capabilities.model_required_access_modes.contains(mode),
                |modes| modes.contains(mode),
            )
        })
        .collect()
}

pub(crate) fn supports_sandbox_discovery(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    harness == Backend::Pi
}

pub(crate) fn supports_steering(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    harness.descriptor().capabilities.turns.steer == super::contract::CapabilitySupport::Available
}

pub(crate) fn supports_reasoning_effort(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    harness
        .descriptor()
        .capabilities
        .configuration
        .reasoning_effort
        == super::contract::CapabilitySupport::Available
}

pub(crate) fn supports_reasoning_reset(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    harness
        .descriptor()
        .capabilities
        .configuration
        .reset_reasoning_effort
        == super::contract::CapabilitySupport::Available
}

pub(crate) fn effort_label(harness: impl Into<Option<Backend>>) -> &'static str {
    let Some(harness) = harness.into() else {
        return "Effort";
    };
    harness.descriptor().capabilities.configuration.effort_label
}

pub(crate) fn supports_session_fork(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    harness.descriptor().capabilities.sessions.fork == super::contract::CapabilitySupport::Available
}

pub(crate) fn validate_launch(
    config: &crate::agents::AgentLaunchConfig,
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
    config: &crate::agents::AgentLaunchConfig,
    harness: Backend,
) -> Result<crate::agents::AgentLaunchConfig, String> {
    let mut config = config.clone();
    config.program = match harness {
        Backend::Pi => return Ok(pi::launch_configuration(&config)),
        Backend::Codex => std::env::var_os("FARCASTER_CODEX_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "codex".into()),
        Backend::Cursor => cursor::PROFILE.program(),
        Backend::Claude => claude::program(),
        Backend::OpenCode => opencode::program(),
        Backend::Antigravity => antigravity::PROFILE.program(),
    };
    Ok(config)
}

pub(crate) fn worker_factories(
    config: crate::agents::AgentLaunchConfig,
) -> (
    std::collections::BTreeMap<Backend, std::sync::Arc<dyn crate::agents::WorkerSessionFactory>>,
    Backend,
) {
    use std::sync::Arc;
    let factories = Backend::ALL
        .into_iter()
        .map(|backend| {
            let mut command = config.clone();
            let factory: Arc<dyn crate::agents::WorkerSessionFactory> = match backend {
                Backend::Pi => Arc::new(pi::PiWorkerFactory::new(command)),
                Backend::Codex => {
                    command.program = std::env::var_os("FARCASTER_CODEX_PATH")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| "codex".into());
                    Arc::new(codex::CodexWorkerFactory::new(command))
                }
                Backend::Cursor => Arc::new(cursor::worker_factory(command)),
                Backend::OpenCode => {
                    command.access_mode = crate::agents::HarnessAccessMode::Sandboxed;
                    command.program = opencode::program();
                    Arc::new(opencode::OpenCodeWorkerFactory::new(command))
                }
                Backend::Claude => {
                    command.program = claude::program();
                    Arc::new(claude::ClaudeWorkerFactory::new(command))
                }
                Backend::Antigravity => {
                    command.program = antigravity::PROFILE.program();
                    Arc::new(acp::AcpWorkerFactory::new(
                        command,
                        antigravity::PROFILE.clone(),
                    ))
                }
            };
            (backend, factory)
        })
        .collect();
    (factories, Backend::Pi)
}

pub(crate) fn load_configuration_catalog(
    config: &crate::agents::AgentLaunchConfig,
    harness: Backend,
    project: &std::path::Path,
) -> Result<crate::agents::ConfigurationCatalog, String> {
    match harness {
        Backend::Codex => {
            let command = configuration_launch(config, harness)?;
            codex::load_configuration(&command, project).and_then(configuration_catalog)
        }
        Backend::Cursor => cursor::load_configuration(project).and_then(configuration_catalog),
        Backend::OpenCode => {
            let command = configuration_launch(config, harness)?;
            opencode::load_configuration(&command, project).and_then(configuration_catalog)
        }
        Backend::Claude => {
            let command = configuration_launch(config, harness)?;
            claude::load_configuration(&command, project).and_then(configuration_catalog)
        }
        Backend::Pi => load_pi_configuration(config, project),
        Backend::Antigravity => {
            let profile = &antigravity::PROFILE;
            let (metadata, _) = acp::load_configuration(profile, project)?;
            configuration_catalog(metadata)
        }
    }
}

fn configuration_launch(
    config: &crate::agents::AgentLaunchConfig,
    harness: Backend,
) -> Result<crate::agents::AgentLaunchConfig, String> {
    let mut command = launch_configuration(config, harness)?;
    command.access_mode = configuration_access_mode(harness, config.access_mode)?;
    Ok(command)
}

fn configuration_access_mode(
    harness: Backend,
    requested: crate::agents::HarnessAccessMode,
) -> Result<crate::agents::HarnessAccessMode, String> {
    use crate::agents::HarnessAccessMode::{Auto, Full, Sandboxed};

    if supports_sandbox_discovery(harness) {
        return Ok(if requested == Auto { Full } else { requested });
    }
    let descriptor = harness.descriptor();
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
) -> Result<crate::agents::ConfigurationCatalog, String> {
    let models = metadata
        .models
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("decode model catalog: {error}"))?;
    Ok(crate::agents::ConfigurationCatalog {
        models,
        efforts: metadata.efforts,
        sandbox_adapter: None,
    })
}

fn load_pi_configuration(
    config: &crate::agents::AgentLaunchConfig,
    project: &std::path::Path,
) -> Result<crate::agents::ConfigurationCatalog, String> {
    use crate::agents::SessionTransport as _;

    let mut process = pi::PiRpcProcess::spawn_catalog(config, project)?;
    process.send(crate::agents::SessionCommand::ListModels)?;
    process.send(crate::agents::SessionCommand::ListReasoningLevels)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut catalog = crate::agents::ConfigurationCatalog::default();
    let mut models_loaded = false;
    let mut efforts_loaded = false;
    while std::time::Instant::now() < deadline && !(models_loaded && efforts_loaded) {
        match process.poll() {
            Some(crate::agents::SessionEvent::Response(response)) => {
                match response.result.map_err(|error| error.to_string())? {
                    crate::agents::SessionResponsePayload::ListModels(models) => {
                        catalog.models = models;
                        models_loaded = true;
                    }
                    crate::agents::SessionResponsePayload::ListReasoningLevels(levels) => {
                        catalog.efforts = levels;
                        efforts_loaded = true;
                    }
                    _ => {}
                }
            }
            Some(crate::agents::SessionEvent::Failure(error)) => return Err(error),
            Some(_) => {}
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    let sandbox_adapter = process.sandbox_adapter().map(str::to_owned);
    let _ = process.close();
    if models_loaded && efforts_loaded {
        catalog.sandbox_adapter = sandbox_adapter;
        Ok(catalog)
    } else {
        Err("timed out loading Pi configuration catalog".into())
    }
}

fn launch_history(
    launch: &crate::agents::SessionLaunch,
    load: impl FnOnce(&std::path::Path) -> Result<crate::agents::DiscoveredHistory, String>,
) -> Result<Option<crate::agents::DiscoveredHistory>, String> {
    match &launch.start {
        crate::agents::SessionStart::New => Ok(None),
        crate::agents::SessionStart::Resume(path) | crate::agents::SessionStart::Fork(path) => {
            load(path).map(Some)
        }
    }
}

pub(crate) fn spawn_session(
    config: &crate::agents::AgentLaunchConfig,
    launch: crate::agents::SessionLaunch,
) -> Result<Box<dyn crate::agents::SessionTransport>, String> {
    if launch.harness != Backend::Pi
        && let crate::agents::SessionStart::Resume(path) | crate::agents::SessionStart::Fork(path) =
            &launch.start
    {
        session_storage::validate_session_locator(launch.harness, path)?;
    }
    match launch.harness {
        Backend::Codex => {
            let history = launch_history(&launch, codex::load_history)?;
            let command = launch_configuration(config, launch.harness)?;
            let (worker, locator, metadata) = codex::spawn_main(&command, &launch)?;
            let locator_root = config
                .session_locator_root
                .as_deref()
                .ok_or_else(|| "agent session locator root is not configured".to_owned())?;
            main_session::WorkerSessionTransport::new(
                locator_root,
                Backend::Codex,
                locator,
                worker,
                metadata,
                history,
            )
            .map(|transport| Box::new(transport) as _)
        }
        Backend::Cursor | Backend::Antigravity => {
            if matches!(&launch.start, crate::agents::SessionStart::Fork(_)) {
                return Err(format!(
                    "{} ACP session fork is not supported",
                    launch.harness
                ));
            }
            let command = launch_configuration(config, launch.harness)?;
            let (worker, locator, metadata, history) =
                if let Some(profile) = external_acp_profile(launch.harness) {
                    acp::spawn_main(&command, profile, &launch)?
                } else {
                    cursor::spawn_main(&command, &launch)?
                };
            let locator_root = config
                .session_locator_root
                .as_deref()
                .ok_or_else(|| "agent session locator root is not configured".to_owned())?;
            main_session::WorkerSessionTransport::new(
                locator_root,
                launch.harness,
                locator,
                worker,
                metadata,
                history,
            )
            .map(|transport| Box::new(transport) as _)
        }
        Backend::OpenCode | Backend::Claude => {
            let history = launch_history(
                &launch,
                if launch.harness == Backend::Claude {
                    claude::load_history
                } else {
                    opencode::load_history
                },
            )?;
            let command = launch_configuration(config, launch.harness)?;
            let (worker, locator, metadata) = if launch.harness == Backend::Claude {
                claude::spawn_main(&command, &launch)?
            } else {
                opencode::spawn_main(&command, &launch)?
            };
            let locator_root = config
                .session_locator_root
                .as_deref()
                .ok_or_else(|| "agent session locator root is not configured".to_owned())?;
            main_session::WorkerSessionTransport::new(
                locator_root,
                launch.harness,
                locator,
                worker,
                metadata,
                history,
            )
            .map(|transport| Box::new(transport) as _)
        }
        Backend::Pi => {
            let process = match &launch.start {
                crate::agents::SessionStart::New => pi::PiRpcProcess::spawn_with_optional_waker(
                    config,
                    &launch.project,
                    None,
                    launch.wake,
                ),
                crate::agents::SessionStart::Resume(session) => {
                    pi::PiRpcProcess::spawn_with_optional_waker(
                        config,
                        &launch.project,
                        Some(session),
                        launch.wake,
                    )
                }
                crate::agents::SessionStart::Fork(source) => {
                    pi::PiRpcProcess::spawn_fork_with_optional_waker(
                        config,
                        &launch.project,
                        source,
                        launch.wake,
                    )
                }
            }?;
            Ok(Box::new(process) as _)
        }
    }
}

pub(crate) fn rename_session(
    config: &crate::agents::AgentLaunchConfig,
    harness: Backend,
    project: &std::path::Path,
    session: &std::path::Path,
    session_id: &str,
    name: &str,
) -> Result<(), String> {
    session_storage::validate_session_target(&crate::sessions::SessionTarget {
        harness,
        id: session_id.into(),
        path: session.into(),
    })?;
    match harness {
        Backend::Pi => pi::PiRpcProcess::rename_session(config, project, session, name),
        Backend::Codex => codex::rename_session(session_id, name),
        Backend::Cursor => cursor::rename_session(session_id, name),
        Backend::OpenCode => opencode::rename_session(session_id, name),
        Backend::Claude | Backend::Antigravity => {
            Err(format!("unsupported session harness: {harness}"))
        }
    }
}

pub(crate) fn external_session_identity(path: &std::path::Path) -> Option<(Backend, String)> {
    for backend in [claude::BACKEND, antigravity::PROFILE.backend] {
        if let Some(locator) = main_session::external_session_locator(backend, path) {
            return Some((backend, locator));
        }
    }
    if let Some(locator) = main_session::external_session_locator(Backend::Codex, path) {
        return Some((Backend::Codex, locator));
    }
    if let Some(locator) = main_session::external_session_locator(Backend::Cursor, path) {
        return Some((Backend::Cursor, locator));
    }
    main_session::external_session_locator(Backend::OpenCode, path)
        .map(|locator| (Backend::OpenCode, locator))
}

#[cfg(test)]
pub(crate) fn delete_external_session(path: &std::path::Path) -> Option<Result<(), String>> {
    external_session_identity(path).map(|(harness, locator)| match harness {
        Backend::Codex => codex::delete_session(&locator),
        Backend::Cursor => cursor::delete_session(&locator),
        Backend::OpenCode => opencode::delete_session(&locator),
        Backend::Pi | Backend::Claude | Backend::Antigravity => {
            Err(format!("Session deletion is not supported for {harness}"))
        }
    })
}

pub(crate) fn discover_external_sessions_for(
    harness: Backend,
    locator_root: Option<&std::path::Path>,
    query: &str,
) -> Result<Vec<crate::agents::DiscoveredSession>, String> {
    let Some(locator_root) = locator_root else {
        return Err("session locator root is unavailable".to_owned());
    };
    match harness {
        Backend::Codex => codex::discover(locator_root, query),
        Backend::Cursor => cursor::discover(locator_root, query),
        Backend::OpenCode => opencode::discover(locator_root, query),
        Backend::Antigravity => Ok(Vec::new()),
        Backend::Claude => claude::discover(locator_root, query),
        Backend::Pi => Err(format!("unsupported session harness: {harness}")),
    }
}

pub(crate) fn annotate_history_message(harness: Backend, message: &mut serde_json::Value) {
    if harness == Backend::Pi {
        pi::annotate_history_message(message);
    }
}

#[cfg(test)]
pub(crate) fn load_external_history(
    path: &std::path::Path,
    project: &std::path::Path,
) -> Option<Result<crate::agents::DiscoveredHistory, String>> {
    external_session_identity(path).map(|(harness, _)| match harness {
        Backend::Codex => codex::load_history(path),
        Backend::Cursor => cursor::load_history(path),
        Backend::OpenCode => opencode::load_history(path),
        Backend::Antigravity => antigravity::load_history(path, project),
        Backend::Claude => claude::load_history(path),
        Backend::Pi => unreachable!("Pi does not have an external session locator"),
    })
}

pub(crate) fn supports_startup_command(
    harness: impl Into<Option<Backend>>,
    command: &crate::agents::SessionCommand,
) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    use super::contract::CapabilitySupport::Available;
    use crate::agents::SessionCommand;

    let configuration = harness.descriptor().capabilities.configuration;
    match command {
        SessionCommand::ListModels => configuration.models == Available,
        SessionCommand::ListReasoningLevels => configuration.reasoning_effort == Available,
        SessionCommand::ListModes => configuration.modes == Available,
        SessionCommand::ListCommands => configuration.commands == Available,
        _ => true,
    }
}

pub(crate) fn backend_display_name(harness: impl Into<Option<Backend>>) -> String {
    let Some(harness) = harness.into() else {
        return "Choose a backend".into();
    };
    harness.descriptor().name
}

pub(crate) fn backend_statuses() -> Vec<super::contract::AgentBackendStatus> {
    known_backend_descriptors()
        .into_iter()
        .map(|descriptor| {
            let program = match descriptor.id {
                Backend::Pi => {
                    pi::launch_configuration(&crate::agents::AgentLaunchConfig::default()).program
                }
                Backend::Codex => std::env::var_os("FARCASTER_CODEX_PATH")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| "codex".into()),
                Backend::Cursor => cursor::PROFILE.program(),
                Backend::OpenCode => opencode::program(),
                Backend::Claude => claude::program(),
                Backend::Antigravity => antigravity::PROFILE.program(),
            };
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

impl Backend {
    pub(super) fn descriptor(self) -> super::contract::AgentBackendDescriptor {
        match self {
            Self::Pi => pi::descriptor(),
            Self::Codex => codex::descriptor(),
            Self::Cursor => cursor::descriptor(),
            Self::OpenCode => opencode::descriptor(),
            Self::Claude => claude::descriptor(),
            Self::Antigravity => antigravity::descriptor(),
        }
    }
}

pub(super) fn known_backend_descriptors() -> [super::contract::AgentBackendDescriptor; 6] {
    Backend::ALL.map(Backend::descriptor)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
