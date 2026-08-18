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
mod sessions;
#[cfg(any(target_os = "macos", test))]
mod shell_environment;
mod state;
#[cfg(test)]
mod state_test;
mod syntax_highlight;
mod theme;
mod title_generation;
mod tool_changes;
mod transcript;
mod workgraph_cli;

fn main() -> std::process::ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new("workgraph")) {
        return match workgraph_cli::run(
            arguments,
            &match state::state_path() {
                Ok(path) => path,
                Err(error) => return fail(error),
            },
        ) {
            Ok(output) => write_success(std::io::stdout(), &output),
            Err(error) => fail(error),
        };
    }
    match launch::resolve_project(std::env::args_os().nth(1).map(Into::into)).and_then(launch::run)
    {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
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

fn write_success(mut destination: impl std::io::Write, value: &str) -> std::process::ExitCode {
    if destination.write_all(value.as_bytes()).is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::from(1)
    }
}

#[cfg(test)]
mod main_tests {
    use super::*;

    #[test]
    fn reported_errors_return_failure_even_when_stderr_write_succeeds() {
        assert_eq!(fail_to(Vec::new(), "failed"), std::process::ExitCode::from(1));
    }
}
