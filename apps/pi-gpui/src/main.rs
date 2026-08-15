mod app;
mod assets;
mod conversation;
mod extension_ui;
mod framing;
mod launch;
mod layout;
mod primitives;
mod protocol;
mod rpc_process;
mod runtime;
mod sessions;
mod theme;
mod transcript;

fn main() -> Result<(), launch::LaunchError> {
    let project = launch::resolve_project(std::env::args_os().nth(1).map(Into::into))?;
    launch::run(project)
}
