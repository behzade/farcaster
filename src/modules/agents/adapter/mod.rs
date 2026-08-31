#[allow(dead_code)]
mod codex;
mod farcaster_mcp;
mod main_session;
#[allow(dead_code)]
mod opencode;
mod pi;
mod process_command;
mod shell_environment;

pub(crate) use shell_environment::{app_shell_environment, default_login_shell};

pub(crate) fn validate_launch(
    config: &crate::agents::AgentLaunchConfig,
    project: &std::path::Path,
) -> Result<(), String> {
    config.command(project).map(|_| ())
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
    let mut opencode_config = config.clone();
    opencode_config.program = std::env::var_os("FARCASTER_OPENCODE_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "opencode2".into());
    let [pi, codex, opencode] = known_backend_descriptors();
    let default_backend = pi.id.as_str().to_owned();
    let factories = std::collections::BTreeMap::from([
        (
            pi.id.as_str().to_owned(),
            std::sync::Arc::new(pi::PiWorkerFactory::new(config)) as _,
        ),
        (
            codex.id.as_str().to_owned(),
            std::sync::Arc::new(codex::CodexWorkerFactory::new(codex_config)) as _,
        ),
        (
            opencode.id.as_str().to_owned(),
            std::sync::Arc::new(opencode::OpenCodeWorkerFactory::new(opencode_config)) as _,
        ),
    ]);
    (factories, default_backend)
}

pub(crate) fn spawn_session(
    config: &crate::agents::AgentLaunchConfig,
    launch: crate::agents::SessionLaunch,
) -> Result<Box<dyn crate::agents::SessionTransport>, String> {
    if launch.harness == "codex-cli" {
        let mut command = config.clone();
        command.program = std::env::var_os("FARCASTER_CODEX_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "codex".into());
        let (worker, locator, metadata) = codex::spawn_main(&command, &launch)?;
        return main_session::WorkerSessionTransport::new(
            "codex-cli",
            locator,
            worker,
            metadata,
        )
            .map(|transport| Box::new(transport) as _);
    }
    if launch.harness == "opencode2" {
        let mut command = config.clone();
        command.program = std::env::var_os("FARCASTER_OPENCODE_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "opencode2".into());
        let (worker, locator, metadata) = opencode::spawn_main(&command, &launch)?;
        return main_session::WorkerSessionTransport::new(
            "opencode2",
            locator,
            worker,
            metadata,
        )
            .map(|transport| Box::new(transport) as _);
    }
    if launch.harness != "pi" {
        return Err(format!("unsupported main-session harness: {}", launch.harness));
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
    match harness {
        "pi" => pi::PiRpcProcess::rename_session(config, project, session, name),
        "codex-cli" => codex::rename_session(session_id, name),
        "opencode2" => opencode::rename_session(session_id, name),
        _ => Err(format!("unsupported session harness: {harness}")),
    }
}

pub(crate) fn is_external_session(path: &std::path::Path) -> bool {
    main_session::external_session_locator("codex-cli", path).is_some()
        || main_session::external_session_locator("opencode2", path).is_some()
}

pub(crate) fn delete_external_session(path: &std::path::Path) -> Option<Result<(), String>> {
    if let Some(locator) = main_session::external_session_locator("codex-cli", path) {
        return Some(codex::delete_session(&locator));
    }
    if let Some(locator) = main_session::external_session_locator("opencode2", path) {
        return Some(opencode::delete_session(&locator));
    }
    None
}

pub(crate) fn discover_external_sessions(
    query: &str,
) -> (Vec<crate::sessions::SessionSummary>, bool) {
    let statuses = backend_statuses();
    let mut sessions = Vec::new();
    let mut exhaustive = true;
    if statuses
        .iter()
        .any(|backend| backend.id == "codex-cli" && backend.available)
    {
        match codex::discover(query) {
            Ok(mut discovered) => sessions.append(&mut discovered),
            Err(_) => exhaustive = false,
        }
    }
    if statuses
        .iter()
        .any(|backend| backend.id == "opencode2" && backend.available)
    {
        match opencode::discover(query) {
            Ok(mut discovered) => sessions.append(&mut discovered),
            Err(_) => exhaustive = false,
        }
    }
    (sessions, exhaustive)
}

pub(crate) fn load_external_history(
    path: &std::path::Path,
) -> Option<Result<crate::sessions::LoadedHistory, String>> {
    if main_session::external_session_locator("codex-cli", path).is_some() {
        return Some(codex::load_history(path));
    }
    if main_session::external_session_locator("opencode2", path).is_some() {
        return Some(opencode::load_history(path));
    }
    None
}

pub(crate) fn backend_statuses() -> Vec<super::contract::AgentBackendStatus> {
    let pi_program = crate::agents::AgentLaunchConfig::default().program;
    let codex_program = std::env::var_os("FARCASTER_CODEX_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "codex".into());
    let opencode_program = std::env::var_os("FARCASTER_OPENCODE_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "opencode2".into());
    known_backend_descriptors()
        .into_iter()
        .zip([pi_program, codex_program, opencode_program])
        .map(|(descriptor, program)| super::contract::AgentBackendStatus {
            id: descriptor.id.as_str().to_owned(),
            name: descriptor.name,
            available: program_available(&program),
            capabilities: descriptor.capabilities,
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

pub(super) fn known_backend_descriptors() -> [super::contract::AgentBackendDescriptor; 3] {
    [
        pi::descriptor(),
        codex::descriptor(),
        opencode::descriptor(),
    ]
}
