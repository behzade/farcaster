//! Pass the agents' captured project environment through Ghostty's command-only API.
//! The private launch file avoids exposing environment values in process arguments.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

pub(crate) const ARGUMENT: &str = "--internal-neovim-launch";

#[derive(Serialize, Deserialize)]
struct Launch {
    program: PathBuf,
    arguments: Vec<OsString>,
    project: PathBuf,
    environment: Vec<(OsString, OsString)>,
}

pub(crate) fn prepare(
    path: &Path,
    program: PathBuf,
    arguments: Vec<OsString>,
    project: PathBuf,
) -> Result<(), String> {
    let environment = crate::agents::project_shell_environment(&project)?
        .unwrap_or_else(|| std::env::vars_os().collect());
    write_launch(
        path,
        &Launch {
            program,
            arguments,
            project,
            environment,
        },
    )
}

fn write_launch(path: &Path, launch: &Launch) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("create Neovim launch file: {error}"))?;
    serde_json::to_writer(file, launch)
        .map_err(|error| format!("write Neovim launch file: {error}"))
}

fn take_command(path: &Path) -> Result<Command, String> {
    let file =
        std::fs::File::open(path).map_err(|error| format!("open Neovim launch file: {error}"))?;
    let launch: Launch = serde_json::from_reader(file)
        .map_err(|error| format!("read Neovim launch file: {error}"))?;
    std::fs::remove_file(path).map_err(|error| format!("remove Neovim launch file: {error}"))?;
    let mut command = Command::new(launch.program);
    command
        .args(launch.arguments)
        .current_dir(launch.project)
        .env_clear()
        .envs(launch.environment);
    Ok(command)
}

/// Handle the internal child launch before app environment imports or GUI startup.
/// Ordinary app invocations return; a successful child launch replaces this process.
pub(crate) fn run_if_requested() -> Result<(), String> {
    use std::os::unix::process::CommandExt as _;
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new(ARGUMENT)) {
        return Ok(());
    }
    let path = arguments.next().ok_or("missing Neovim launch file")?;
    let error = take_command(Path::new(&path))?.exec();
    Err(format!("launch Neovim: {error}"))
}

#[cfg(test)]
#[path = "neovim_launch_tests.rs"]
mod tests;
