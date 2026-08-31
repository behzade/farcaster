mod app;
mod app_paths;
mod assets;
mod composer_sessions;
#[cfg(test)]
mod composer_sessions_test;
mod extension_ui;
mod keybindings;
mod keyboard;
mod layout;
mod modules;
mod performance;
mod persistent_vec;
mod primitives;
mod project_trust_view;
mod theme;
mod tool_changes;
mod user_invocations;

pub(crate) use app::runtime;
pub(crate) use app::views::transcript::conversation;
pub(crate) use modules::agents::extensions as protocol;
pub(crate) use modules::sessions::activity as agent_activity;
pub(crate) use modules::{access, agents, projects, repository, sessions};

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    if let Err(error) = relaunch_with_linux_vulkan_driver_policy() {
        return fail(error);
    }

    if let Err(error) = app::shell_environment::import() {
        return fail(format!("import app shell environment: {error}"));
    }

    zlog::init();
    zlog::init_output_stderr();
    if let Err(error) = init_log_file() {
        zlog::error!("Failed to initialize application log file: {error}");
    }
    let project = match app::launch::resolve_project(std::env::args_os().nth(1).map(Into::into)) {
        Ok(project) => project,
        Err(error) => return fail(error),
    };
    let home = match std::env::var_os("HOME") {
        Some(home) => std::path::PathBuf::from(home),
        None => return fail("HOME is required to initialize sandbox approvals"),
    };
    let data_root = match app_paths::data_dir() {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    let agent_state = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".pi/agent"));
    let temporary = std::env::var_os("TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let nono = access::configured_sandbox_runtime(std::env::var_os("FARCASTER_NONO_PATH"));
    let (approvals, approval_ui) = match access::approval::channel(
        &project,
        &home,
        &data_root,
        &agent_state,
        &temporary,
        nono,
    ) {
        Ok(channel) => channel,
        Err(error) => return fail(format!("initialize sandbox approvals: {error}")),
    };
    let worker_command = agents::AgentLaunchConfig {
        grants: Some(approval_ui.grants()),
        ..agents::AgentLaunchConfig::default()
    };
    let (factories, default_backend) = agents::worker_factories(worker_command);
    let worker_pool = match agents::WorkerPool::new(factories, default_backend, project.clone(), 8)
    {
        Ok(pool) => pool,
        Err(error) => return fail(format!("initialize worker pool: {error}")),
    };
    let (workgraph_updates, workgraph_update_receiver) = async_channel::bounded(1);
    let _mcp_server = match app::persistence::state_path().and_then(|database| {
        app::mcp_server::start(database, approvals, worker_pool, workgraph_updates)
    }) {
        Ok(server) => server,
        Err(error) => return fail(format!("start MCP server: {error}")),
    };

    match app::launch::run(project, approval_ui, workgraph_update_receiver) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

#[cfg(target_os = "linux")]
const VULKAN_DRIVER_CONFIGURATION: [&str; 5] = [
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_ADD_DRIVER_FILES",
    "VK_LOADER_DRIVERS_SELECT",
    "VK_LOADER_DRIVERS_DISABLE",
];

#[cfg(target_os = "linux")]
fn relaunch_with_linux_vulkan_driver_policy() -> Result<(), String> {
    use std::os::unix::process::CommandExt as _;

    let is_wsl = std::env::var_os("WSL_INTEROP").is_some()
        || std::env::var_os("WSL_DISTRO_NAME").is_some()
        || ["/proc/sys/kernel/osrelease", "/proc/version"]
            .into_iter()
            .any(|path| {
                std::fs::read_to_string(path)
                    .is_ok_and(|version| kernel_version_reports_wsl(&version))
            });
    if !should_disable_dzn(is_wsl, |name| std::env::var_os(name).is_some()) {
        return Ok(());
    }

    let executable = std::env::current_exe()
        .map_err(|error| format!("resolve farcaster executable for Vulkan setup: {error}"))?;
    let error = std::process::Command::new(executable)
        .args(std::env::args_os().skip(1))
        .env("VK_LOADER_DRIVERS_DISABLE", "*dzn*")
        .exec();
    Err(format!(
        "relaunch farcaster with the Mesa DZN Vulkan ICD disabled: {error}"
    ))
}

#[cfg(target_os = "linux")]
fn should_disable_dzn(is_wsl: bool, mut environment_is_set: impl FnMut(&str) -> bool) -> bool {
    !is_wsl
        && !VULKAN_DRIVER_CONFIGURATION
            .into_iter()
            .any(&mut environment_is_set)
}

#[cfg(target_os = "linux")]
fn kernel_version_reports_wsl(version: &str) -> bool {
    version.to_ascii_lowercase().contains("microsoft")
}

fn init_log_file() -> Result<(), String> {
    static LOG_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    static OLD_LOG_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

    let directory = crate::app_paths::data_dir()?.join("logs");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("create {}: {error}", directory.display()))?;
    let path = LOG_PATH.get_or_init(|| directory.join("farcaster.log"));
    let old_path = OLD_LOG_PATH.get_or_init(|| directory.join("farcaster.log.old"));
    zlog::init_output_file(path, Some(old_path))
        .map_err(|error| format!("open {}: {error}", path.display()))
}

fn fail(error: impl std::fmt::Display) -> std::process::ExitCode {
    fail_to(std::io::stderr(), error)
}

fn fail_to(
    mut destination: impl std::io::Write,
    error: impl std::fmt::Display,
) -> std::process::ExitCode {
    let _written = destination.write_all(format!("{error}\n").as_bytes());
    std::process::ExitCode::from(1)
}

#[cfg(test)]
mod main_tests {
    use super::*;

    #[test]
    fn reported_errors_return_failure_even_when_stderr_write_succeeds() {
        assert_eq!(
            fail_to(Vec::new(), "failed"),
            std::process::ExitCode::from(1)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_linux_disables_dzn_without_overriding_driver_configuration() {
        assert!(should_disable_dzn(false, |_| false));
        assert!(!should_disable_dzn(true, |_| false));
        for configured_name in VULKAN_DRIVER_CONFIGURATION {
            assert!(!should_disable_dzn(false, |name| name == configured_name));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recognizes_wsl_kernel_versions() {
        assert!(kernel_version_reports_wsl(
            "5.15.90.1-microsoft-standard-WSL2"
        ));
        assert!(kernel_version_reports_wsl("4.4.0-Microsoft"));
        assert!(!kernel_version_reports_wsl("7.1.4"));
    }
}
