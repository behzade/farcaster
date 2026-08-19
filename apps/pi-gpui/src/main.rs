mod agent_activity;
mod app;
mod assets;
mod composer_sessions;
#[cfg(test)]
mod composer_sessions_test;
mod conversation;
mod extension_ui;
mod framing;
mod keybindings;
mod launch;
mod layout;
mod performance;
mod primitives;
mod projects;
mod protocol;
mod rpc_process;
mod runtime;
mod session_changes;
mod session_transfer;
mod session_watcher;
mod sessions;
#[cfg(any(target_os = "macos", test))]
mod shell_environment;
mod state;
#[cfg(test)]
mod state_test;
mod syntax_highlight;
mod theme;
mod tool_changes;
mod transcript;
mod workgraph_rpc;

fn main() -> std::process::ExitCode {
    zlog::init();
    zlog::init_output_stderr();
    if let Err(error) = init_log_file() {
        zlog::error!("Failed to initialize application log file: {error}");
    }

    match launch::resolve_project(std::env::args_os().nth(1).map(Into::into)).and_then(launch::run)
    {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

fn init_log_file() -> Result<(), String> {
    static LOG_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    static OLD_LOG_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_owned())?;
    let directory = std::path::PathBuf::from(home).join("Library/Logs");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("create {}: {error}", directory.display()))?;
    let path = LOG_PATH.get_or_init(|| directory.join("pi-gpui.log"));
    let old_path = OLD_LOG_PATH.get_or_init(|| directory.join("pi-gpui.log.old"));
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
}
