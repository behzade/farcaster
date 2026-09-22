mod app;
#[cfg(target_os = "linux")]
mod linux_graphics;

use app::infrastructure::performance::StartupTiming;
pub(crate) use app::runtime;
pub(crate) use farcaster_access as access;
pub(crate) use farcaster_agents as agents;
pub(crate) use farcaster_agents::builtin_mcp;
pub(crate) use farcaster_agents::extensions as protocol;
pub(crate) use farcaster_conversation as conversation;
pub(crate) use farcaster_projects as projects;
pub(crate) use farcaster_repository as repository;
pub(crate) use farcaster_reviews as reviews;
pub(crate) use farcaster_sessions as sessions;
pub(crate) use farcaster_sessions::activity as agent_activity;
pub(crate) use farcaster_utility as utility;

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
    let data_root = match app::paths::data_dir() {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    let state_store = match app::persistence::initialize() {
        Ok(store) => store,
        Err(error) => return fail(format!("initialize state database: {error}")),
    };
    let project = match app::launch::resolve_project(std::env::args_os().nth(1).map(Into::into)) {
        Ok(project) => project,
        Err(error) => return fail(error),
    };
    let (builtin_mcp_enabled, agent_launch, saved_worker_routes) = {
        let store = match state_store.lock() {
            Ok(store) => store,
            Err(error) => return fail(error),
        };
        let builtin_mcp_enabled = match store.load_builtin_mcp_enabled() {
            Ok(enabled) => enabled,
            Err(error) => return fail(format!("load MCP setting: {error}")),
        };
        let agent_launch = match load_agent_launch_config(&data_root, &store) {
            Ok(command) => command,
            Err(error) => return fail(format!("load agent launch settings: {error}")),
        };
        let saved_worker_routes = match store.load_worker_routes() {
            Ok(routes) => routes,
            Err(error) => return fail(format!("load saved worker routes: {error}")),
        };
        (builtin_mcp_enabled, agent_launch, saved_worker_routes)
    };
    builtin_mcp::set_enabled(builtin_mcp_enabled);
    let (factories, default_backend) = agents::worker_factories(agent_launch.clone());
    let worker_pool = match agents::WorkerPool::new(factories, default_backend, project.clone(), 8)
    {
        Ok(pool) => {
            if let Err(error) = pool.set_app_proxy(agent_launch.app_proxy.clone()) {
                return fail(format!("initialize worker proxy: {error}"));
            }
            if let Err(error) = pool.restore_families(saved_worker_routes) {
                return fail(format!("restore saved worker routes: {error}"));
            }
            pool
        }
        Err(error) => return fail(format!("initialize worker pool: {error}")),
    };
    let worker_updates = worker_pool.updates();
    let (workgraph_updates, workgraph_update_receiver) = async_channel::bounded(1);
    let notice_board = app::mcp_server::NoticeBoard::default();
    let _mcp_server = match app::mcp_server::start(
        state_store,
        worker_pool,
        workgraph_updates,
        notice_board.clone(),
    ) {
        Ok(server) => server,
        Err(error) => return fail(format!("start MCP server: {error}")),
    };

    drop(prepare_timing);
    match app::launch::run(
        project,
        agent_launch,
        workgraph_update_receiver,
        worker_updates,
        notice_board,
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

fn load_agent_launch_config(
    data_root: &std::path::Path,
    state_store: &app::persistence::StateStore,
) -> Result<agents::AgentLaunchConfig, String> {
    let app_proxy = crate::access::load_proxy(state_store)?;
    Ok(agents::AgentLaunchConfig {
        app_proxy,
        session_locator_root: Some(data_root.join("session-locators")),
        ..agents::AgentLaunchConfig::default()
    })
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
