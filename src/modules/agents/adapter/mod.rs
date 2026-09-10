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
mod live_tests;
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

fn external_acp_profile(harness: &str) -> Option<&'static acp::AcpProfile> {
    match harness {
        "antigravity-acp" => Some(&antigravity::PROFILE),
        _ => None,
    }
}

pub(crate) fn available_access_modes(
    harness: &str,
    model: Option<&crate::protocol::Model>,
) -> Vec<crate::agents::HarnessAccessMode> {
    let Some(descriptor) = known_backend_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.id.as_str() == harness)
    else {
        return vec![crate::agents::HarnessAccessMode::Full];
    };
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

pub(crate) fn supports_steering(harness: &str) -> bool {
    known_backend_descriptors().into_iter().any(|descriptor| {
        descriptor.id.as_str() == harness
            && descriptor.capabilities.turns.steer == super::contract::CapabilitySupport::Available
    })
}

pub(crate) fn supports_reasoning_effort(harness: &str) -> bool {
    known_backend_descriptors().into_iter().any(|descriptor| {
        descriptor.id.as_str() == harness
            && descriptor.capabilities.configuration.reasoning_effort
                == super::contract::CapabilitySupport::Available
    })
}

pub(crate) fn supports_session_fork(harness: &str) -> bool {
    known_backend_descriptors().into_iter().any(|descriptor| {
        descriptor.id.as_str() == harness
            && descriptor.capabilities.sessions.fork
                == super::contract::CapabilitySupport::Available
    })
}

pub(crate) fn validate_launch(
    config: &crate::agents::AgentLaunchConfig,
    harness: &str,
    project: &std::path::Path,
) -> Result<(), String> {
    launch_configuration(config, harness)?
        .command(project)
        .map(|_| ())
}

fn launch_configuration(
    config: &crate::agents::AgentLaunchConfig,
    harness: &str,
) -> Result<crate::agents::AgentLaunchConfig, String> {
    let mut config = config.clone();
    config.program = match harness {
        "pi" => return Ok(pi::launch_configuration(&config)),
        "codex-cli" => std::env::var_os("FARCASTER_CODEX_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "codex".into()),
        "cursor-cli" => cursor::PROFILE.program(),
        "claude" => claude::program(),
        "opencode2" => std::env::var_os("FARCASTER_OPENCODE_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "opencode2".into()),
        _ => external_acp_profile(harness)
            .ok_or_else(|| format!("unsupported session harness: {harness}"))?
            .program(),
    };
    Ok(config)
}

pub(crate) fn worker_factories(
    config: crate::agents::AgentLaunchConfig,
) -> (
    std::collections::BTreeMap<String, std::sync::Arc<dyn crate::agents::WorkerSessionFactory>>,
    String,
) {
    let mut codex_config = config.clone();
    codex_config.program = std::env::var_os("FARCASTER_CODEX_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "codex".into());
    let cursor_config = config.clone();
    let mut opencode_config = config.clone();
    opencode_config.access_mode = crate::agents::HarnessAccessMode::Sandboxed;
    opencode_config.program = std::env::var_os("FARCASTER_OPENCODE_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "opencode2".into());
    let [pi, codex, cursor, opencode, _, _] = known_backend_descriptors();
    let default_backend = pi.id.as_str().to_owned();
    let mut factories = std::collections::BTreeMap::from([
        (
            pi.id.as_str().to_owned(),
            std::sync::Arc::new(pi::PiWorkerFactory::new(config.clone())) as _,
        ),
        (
            codex.id.as_str().to_owned(),
            std::sync::Arc::new(codex::CodexWorkerFactory::new(codex_config)) as _,
        ),
        (
            cursor.id.as_str().to_owned(),
            std::sync::Arc::new(cursor::worker_factory(cursor_config)) as _,
        ),
        (
            opencode.id.as_str().to_owned(),
            std::sync::Arc::new(opencode::OpenCodeWorkerFactory::new(opencode_config)) as _,
        ),
    ]);
    let mut claude_config = config.clone();
    claude_config.program = claude::program();
    factories.insert(
        claude::BACKEND.into(),
        std::sync::Arc::new(claude::ClaudeWorkerFactory::new(claude_config)) as _,
    );
    let profile = &antigravity::PROFILE;
    let mut command = config;
    command.program = profile.program();
    factories.insert(
        profile.backend.into(),
        std::sync::Arc::new(acp::AcpWorkerFactory::new(command, profile.clone())) as _,
    );
    (factories, default_backend)
}

pub(crate) fn load_configuration_catalog(
    config: &crate::agents::AgentLaunchConfig,
    harness: &str,
    project: &std::path::Path,
) -> Result<crate::agents::ConfigurationCatalog, String> {
    match harness {
        "codex-cli" => {
            let command = launch_configuration(config, harness)?;
            codex::load_configuration(&command, project).and_then(configuration_catalog)
        }
        "cursor-cli" => cursor::load_configuration(project).and_then(configuration_catalog),
        "opencode2" => {
            let command = launch_configuration(config, harness)?;
            opencode::load_configuration(&command, project).and_then(configuration_catalog)
        }
        "claude" => {
            let command = launch_configuration(config, harness)?;
            claude::load_configuration(&command, project).and_then(configuration_catalog)
        }
        "pi" => load_pi_configuration(config, project),
        _ => {
            let profile = external_acp_profile(harness)
                .ok_or_else(|| format!("unsupported main-session harness: {harness}"))?;
            let (metadata, _) = acp::load_configuration(profile, project)?;
            configuration_catalog(metadata)
        }
    }
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
            Some(crate::agents::SessionEvent::Response(response)) if response.success => {
                match response.operation {
                    crate::agents::SessionOperation::ListModels => {
                        catalog.models = serde_json::from_value(
                            response.data.get("models").cloned().unwrap_or_default(),
                        )
                        .map_err(|error| format!("decode Pi model catalog: {error}"))?;
                        models_loaded = true;
                    }
                    crate::agents::SessionOperation::ListReasoningLevels => {
                        catalog.efforts = serde_json::from_value(
                            response.data.get("levels").cloned().unwrap_or_default(),
                        )
                        .map_err(|error| format!("decode Pi effort catalog: {error}"))?;
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
    let _ = process.close();
    if models_loaded && efforts_loaded {
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
    if launch.harness != "pi"
        && let crate::agents::SessionStart::Resume(path) | crate::agents::SessionStart::Fork(path) =
            &launch.start
    {
        session_storage::validate_session_locator(&launch.harness, path)?;
    }
    if launch.harness == "codex-cli" {
        let history = launch_history(&launch, codex::load_history)?;
        let command = launch_configuration(config, &launch.harness)?;
        let (worker, locator, metadata) = codex::spawn_main(&command, &launch)?;
        let locator_root = config
            .session_locator_root
            .as_deref()
            .ok_or_else(|| "agent session locator root is not configured".to_owned())?;
        return main_session::WorkerSessionTransport::new(
            locator_root,
            "codex-cli",
            locator,
            worker,
            metadata,
            history,
        )
        .map(|transport| Box::new(transport) as _);
    }
    if launch.harness == "cursor-cli" || external_acp_profile(&launch.harness).is_some() {
        if matches!(&launch.start, crate::agents::SessionStart::Fork(_)) {
            return Err(format!(
                "{} ACP session fork is not supported",
                launch.harness
            ));
        }
        let command = launch_configuration(config, &launch.harness)?;
        let (worker, locator, metadata, history) =
            if let Some(profile) = external_acp_profile(&launch.harness) {
                acp::spawn_main(&command, profile, &launch)?
            } else {
                cursor::spawn_main(&command, &launch)?
            };
        let locator_root = config
            .session_locator_root
            .as_deref()
            .ok_or_else(|| "agent session locator root is not configured".to_owned())?;
        return main_session::WorkerSessionTransport::new(
            locator_root,
            &launch.harness,
            locator,
            worker,
            metadata,
            history,
        )
        .map(|transport| Box::new(transport) as _);
    }
    if matches!(launch.harness.as_str(), "opencode2" | "claude") {
        let history = launch_history(
            &launch,
            if launch.harness == "claude" {
                claude::load_history
            } else {
                opencode::load_history
            },
        )?;
        let command = launch_configuration(config, &launch.harness)?;
        let (worker, locator, metadata) = if launch.harness == "claude" {
            claude::spawn_main(&command, &launch)?
        } else {
            opencode::spawn_main(&command, &launch)?
        };
        let locator_root = config
            .session_locator_root
            .as_deref()
            .ok_or_else(|| "agent session locator root is not configured".to_owned())?;
        return main_session::WorkerSessionTransport::new(
            locator_root,
            &launch.harness,
            locator,
            worker,
            metadata,
            history,
        )
        .map(|transport| Box::new(transport) as _);
    }
    if launch.harness != "pi" {
        return Err(format!(
            "unsupported main-session harness: {}",
            launch.harness
        ));
    }
    let process = match &launch.start {
        crate::agents::SessionStart::New => {
            pi::PiRpcProcess::spawn_with_optional_waker(config, &launch.project, None, launch.wake)
        }
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
    Ok(Box::new(process))
}

pub(crate) fn rename_session(
    config: &crate::agents::AgentLaunchConfig,
    harness: &str,
    project: &std::path::Path,
    session: &std::path::Path,
    session_id: &str,
    name: &str,
) -> Result<(), String> {
    session_storage::validate_session_target(&crate::sessions::SessionTarget {
        harness: harness.into(),
        id: session_id.into(),
        path: session.into(),
    })?;
    match harness {
        "pi" => pi::PiRpcProcess::rename_session(config, project, session, name),
        "codex-cli" => codex::rename_session(session_id, name),
        "cursor-cli" => cursor::rename_session(session_id, name),
        "opencode2" => opencode::rename_session(session_id, name),
        _ => Err(format!("unsupported session harness: {harness}")),
    }
}

pub(crate) fn external_session_identity(path: &std::path::Path) -> Option<(&'static str, String)> {
    for backend in [claude::BACKEND, antigravity::PROFILE.backend] {
        if let Some(locator) = main_session::external_session_locator(backend, path) {
            return Some((backend, locator));
        }
    }
    if let Some(locator) = main_session::external_session_locator("codex-cli", path) {
        return Some(("codex-cli", locator));
    }
    if let Some(locator) = main_session::external_session_locator("cursor-cli", path) {
        return Some(("cursor-cli", locator));
    }
    main_session::external_session_locator("opencode2", path).map(|locator| ("opencode2", locator))
}

#[cfg(test)]
pub(crate) fn delete_external_session(path: &std::path::Path) -> Option<Result<(), String>> {
    external_session_identity(path).map(|(harness, locator)| match harness {
        "codex-cli" => codex::delete_session(&locator),
        "cursor-cli" => cursor::delete_session(&locator),
        "opencode2" => opencode::delete_session(&locator),
        _ => Err(format!("Session deletion is not supported for {harness}")),
    })
}

pub(crate) fn discover_external_sessions_for(
    harness: &str,
    locator_root: Option<&std::path::Path>,
    query: &str,
) -> Result<Vec<crate::agents::DiscoveredSession>, String> {
    let Some(locator_root) = locator_root else {
        return Err("session locator root is unavailable".to_owned());
    };
    match harness {
        "codex-cli" => codex::discover(locator_root, query),
        "cursor-cli" => cursor::discover(locator_root, query),
        "opencode2" => opencode::discover(locator_root, query),
        "antigravity-acp" => Ok(Vec::new()),
        "claude" => claude::discover(locator_root, query),
        _ => Err(format!("unsupported session harness: {harness}")),
    }
}

pub(crate) fn annotate_history_message(harness: &str, message: &mut serde_json::Value) {
    if harness == "pi" {
        pi::annotate_history_message(message);
    }
}

#[cfg(test)]
pub(crate) fn load_external_history(
    path: &std::path::Path,
) -> Option<Result<crate::agents::DiscoveredHistory, String>> {
    external_session_identity(path).map(|(harness, _)| match harness {
        "codex-cli" => codex::load_history(path),
        "cursor-cli" => cursor::load_history(path),
        "opencode2" => opencode::load_history(path),
        "antigravity-acp" => {
            Err("Antigravity ACP does not expose history replay through this adapter".into())
        }
        "claude" => claude::load_history(path),
        _ => unreachable!("external session identity returned an unknown backend"),
    })
}

pub(crate) fn supports_startup_command(
    harness: &str,
    command: &crate::agents::SessionCommand,
) -> bool {
    use super::contract::CapabilitySupport::Available;
    use crate::agents::SessionCommand;

    let Some(configuration) = known_backend_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.id.as_str() == harness)
        .map(|descriptor| descriptor.capabilities.configuration)
    else {
        return true;
    };
    match command {
        SessionCommand::ListModels => configuration.models == Available,
        SessionCommand::ListReasoningLevels => configuration.reasoning_effort == Available,
        SessionCommand::ListModes => configuration.modes == Available,
        SessionCommand::ListCommands => configuration.commands == Available,
        _ => true,
    }
}

pub(crate) fn backend_display_name(harness: &str) -> String {
    known_backend_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.id.as_str() == harness)
        .map_or_else(|| harness.to_owned(), |descriptor| descriptor.name)
}

pub(crate) fn backend_statuses() -> Vec<super::contract::AgentBackendStatus> {
    let pi_program = pi::launch_configuration(&crate::agents::AgentLaunchConfig::default()).program;
    let codex_program = std::env::var_os("FARCASTER_CODEX_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "codex".into());
    let cursor_program = cursor::PROFILE.program();
    let opencode_program = std::env::var_os("FARCASTER_OPENCODE_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "opencode2".into());
    known_backend_descriptors()
        .into_iter()
        .zip([
            pi_program,
            codex_program,
            cursor_program,
            opencode_program,
            claude::program(),
            antigravity::PROFILE.program(),
        ])
        .map(
            |(descriptor, program)| super::contract::AgentBackendStatus {
                id: descriptor.id.as_str().to_owned(),
                name: descriptor.name,
                available: program_available(&program),
                program,
                capabilities: descriptor.capabilities,
            },
        )
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
    [
        pi::descriptor(),
        codex::descriptor(),
        cursor::descriptor(),
        opencode::descriptor(),
        claude::descriptor(),
        antigravity::descriptor(),
    ]
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
