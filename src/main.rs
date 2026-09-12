mod app;
mod builtin_mcp;
#[cfg(target_os = "linux")]
mod linux_graphics;
mod modules;

use app::infrastructure::performance::StartupTiming;
pub(crate) use app::runtime;
pub(crate) use modules::agents::extensions as protocol;
pub(crate) use modules::sessions::activity as agent_activity;
pub(crate) use modules::{access, agents, projects, repository, sessions};

fn main() -> std::process::ExitCode {
    if let Err(error) = app::infrastructure::neovim_launch::run_if_requested() {
        return fail(error);
    }

    #[cfg(target_os = "linux")]
    if let Err(error) = linux_graphics::relaunch() {
        return fail(error);
    }

    let shell_import_ms = match app::shell_environment::import() {
        Ok(elapsed) => elapsed,
        Err(error) => return fail(format!("import app shell environment: {error}")),
    };

    zlog::init();
    zlog::init_output_stderr();
    if let Err(error) = init_log_file() {
        zlog::error!("Failed to initialize application log file: {error}");
    }
    if let Some(elapsed_ms) = shell_import_ms {
        zlog::info!("STARTUP operation=main.import_shell_environment elapsed_ms={elapsed_ms}");
    }
    let prepare_timing = StartupTiming::always("main.prepare");
    let project = match app::launch::resolve_project(std::env::args_os().nth(1).map(Into::into)) {
        Ok(project) => project,
        Err(error) => return fail(error),
    };
    let data_root = match app::paths::data_dir() {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    let state_store = app::persistence::StateStore::open().ok();
    let builtin_mcp_enabled = state_store
        .as_ref()
        .and_then(|store| store.load_builtin_mcp_enabled().ok())
        .unwrap_or(true);
    builtin_mcp::set_enabled(builtin_mcp_enabled);
    let worker_command = startup_worker_command(&data_root, state_store.as_ref());
    let worker_proxy = worker_command.app_proxy.clone();
    let (factories, default_backend) = agents::worker_factories(worker_command);
    let worker_pool = match agents::WorkerPool::new(factories, default_backend, project.clone(), 8)
    {
        Ok(pool) => {
            if let Err(error) = pool.set_app_proxy(worker_proxy) {
                return fail(format!("initialize worker proxy: {error}"));
            }
            pool
        }
        Err(error) => return fail(format!("initialize worker pool: {error}")),
    };
    let worker_updates = worker_pool.updates();
    let (workgraph_updates, workgraph_update_receiver) = async_channel::bounded(1);
    let _mcp_server = match app::persistence::state_path()
        .and_then(|database| app::mcp_server::start(database, worker_pool, workgraph_updates))
    {
        Ok(server) => server,
        Err(error) => return fail(format!("start MCP server: {error}")),
    };

    drop(prepare_timing);
    match app::launch::run(project, workgraph_update_receiver, worker_updates) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

fn startup_worker_command(
    data_root: &std::path::Path,
    state_store: Option<&app::persistence::StateStore>,
) -> agents::AgentLaunchConfig {
    let app_proxy = state_store.and_then(|store| crate::access::load_proxy(store).unwrap_or(None));
    agents::AgentLaunchConfig {
        app_proxy,
        session_locator_root: Some(data_root.join("session-locators")),
        ..agents::AgentLaunchConfig::default()
    }
}

fn init_log_file() -> Result<(), String> {
    static LOG_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    static OLD_LOG_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

    let directory = crate::app::paths::data_dir()?.join("logs");
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
#[path = "main_tests.rs"]
mod main_tests;
