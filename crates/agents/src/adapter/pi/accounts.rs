use super::super::process_command::environment;
use crate::{AgentLaunchConfig, Backend, accounts::*};
use std::path::{Path, PathBuf};

pub(super) fn read(config: &AgentLaunchConfig, project: &Path) -> Result<AccountDiscovery, String> {
    let launch = super::launch_configuration(config);
    let command = launch.command(project)?;
    let root = environment(&command, "PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| environment(&command, "HOME").map(|h| PathBuf::from(h).join(".pi/agent")))
        .ok_or("Pi credential directory unavailable")?;
    super::super::accounts::stored_connections(
        config,
        Backend::Pi,
        project,
        &root.join("auth.json"),
    )
}
