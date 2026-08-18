mod agent_activity;
mod app;
mod assets;
mod composer_sessions;
#[cfg(test)]
mod composer_sessions_test;
mod conversation;
mod extension_support;
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

fn main() -> Result<(), launch::LaunchError> {
    let project = launch::resolve_project(std::env::args_os().nth(1).map(Into::into))?;
    launch::run(project)
}
