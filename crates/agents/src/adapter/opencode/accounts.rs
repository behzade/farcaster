use super::super::process_command::environment;
use crate::{AgentLaunchConfig, Backend, accounts::*};
use std::path::{Path, PathBuf};

pub(super) fn read(config: &AgentLaunchConfig, project: &Path) -> Result<AccountDiscovery, String> {
    let launch = super::super::configuration_launch(config, Backend::OpenCode)?;
    let command = launch.command(project)?;
    let root = environment(&command, "XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| environment(&command, "HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("OpenCode credential directory unavailable")?;
    super::super::accounts::stored_connections(
        config,
        Backend::OpenCode,
        project,
        &root.join("opencode/auth.json"),
    )
}
