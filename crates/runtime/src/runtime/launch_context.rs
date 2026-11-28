use std::path::Path;

use super::{AgentLaunchConfig, SharedStateStore};

pub(super) enum LaunchTarget<'a> {
    Draft(&'a str),
    NewDraft(Option<&'a str>),
    Session(&'a Path, Option<crate::agents::Backend>),
}

impl<'a> LaunchTarget<'a> {
    pub(super) fn for_command(command: &'a super::RuntimeCommand) -> Option<Self> {
        use super::RuntimeCommand;
        match command {
            RuntimeCommand::NewSession { profile_id, .. } => {
                Some(Self::NewDraft(profile_id.as_deref()))
            }
            RuntimeCommand::ResumeDraft { id, .. } => Some(Self::Draft(id)),
            RuntimeCommand::SelectSession { path, harness, .. }
            | RuntimeCommand::RestartSession { path, harness, .. }
            | RuntimeCommand::ForkSession { path, harness, .. } => {
                Some(Self::Session(path, Some(*harness)))
            }
            _ => None,
        }
    }
}

pub(super) fn configuration_for_target(
    base: &AgentLaunchConfig,
    state: Option<&SharedStateStore>,
    target: LaunchTarget<'_>,
) -> Result<AgentLaunchConfig, String> {
    let mut config = base.clone();
    config.profile_id = match (state, target) {
        (_, LaunchTarget::NewDraft(profile_id)) => profile_id.map(str::to_owned),
        (Some(state), LaunchTarget::Draft(id)) => state.with(|store| store.draft_profile_id(id))?,
        (Some(state), LaunchTarget::Session(path, harness)) => {
            state.with(|store| store.session_profile_for_backend(path, harness))?
        }
        (None, LaunchTarget::Draft(_)) => None,
        (None, LaunchTarget::Session(path, _)) => crate::sessions::profile_id_from_locator(path),
    };
    Ok(config)
}

#[cfg(test)]
#[path = "launch_context_tests.rs"]
mod tests;
