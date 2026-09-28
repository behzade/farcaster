use std::path::Path;

use super::{AgentLaunchConfig, SharedStateStore};

pub(super) enum LaunchTarget<'a> {
    Draft(&'a str),
    Session(&'a Path),
}

impl<'a> LaunchTarget<'a> {
    pub(super) fn for_command(command: &'a super::RuntimeCommand) -> Option<Self> {
        use super::RuntimeCommand;
        match command {
            RuntimeCommand::NewSession { id, .. } | RuntimeCommand::ResumeDraft { id, .. } => {
                Some(Self::Draft(id))
            }
            RuntimeCommand::SelectSession { path, .. }
            | RuntimeCommand::RestartSession { path, .. }
            | RuntimeCommand::ForkSession { path, .. } => Some(Self::Session(path)),
            _ => None,
        }
    }
}

/// Select the persisted harness configuration before reading defaults, loading a
/// catalog, or starting an adapter. Native session paths need not encode it.
pub(super) fn configuration_for_target(
    base: &AgentLaunchConfig,
    state: Option<&SharedStateStore>,
    target: LaunchTarget<'_>,
) -> Result<AgentLaunchConfig, String> {
    let mut config = base.clone();
    config.profile_id = match (state, target) {
        (Some(state), LaunchTarget::Draft(id)) => state.with(|store| store.draft_profile_id(id))?,
        (Some(state), LaunchTarget::Session(path)) => {
            state.with(|store| store.session_profile_id(path))?
        }
        (None, LaunchTarget::Draft(_)) => None,
        (None, LaunchTarget::Session(path)) => crate::sessions::profile_id_from_locator(path),
    };
    Ok(config)
}

#[cfg(test)]
#[path = "launch_context_tests.rs"]
mod tests;
