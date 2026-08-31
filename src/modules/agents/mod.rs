mod adapter;
mod contract;
mod core;

pub(crate) use adapter::{app_shell_environment, default_login_shell};
pub(crate) use contract::extensions;
pub(crate) use contract::{
    AgentLaunchConfig, FileAccessMode, NetworkAccessMode, PermissionLevel, SessionCommand,
    SessionEvent, SessionLaunch, SessionResponse, SessionStart, SessionTransport, WorkerContext,
    WorkerInput, WorkerInputResponse,
};
pub(crate) use core::{
    CallerRegistry, WorkerEvent, WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory,
};

pub(crate) fn validate_launch(
    config: &AgentLaunchConfig,
    project: &std::path::Path,
) -> Result<(), String> {
    adapter::validate_launch(config, project)
}

pub(crate) fn worker_factories(
    config: AgentLaunchConfig,
) -> (
    std::collections::BTreeMap<String, std::sync::Arc<dyn WorkerSessionFactory>>,
    String,
) {
    adapter::worker_factories(config)
}

pub(crate) fn spawn_session(
    config: &AgentLaunchConfig,
    launch: SessionLaunch,
) -> Result<Box<dyn SessionTransport>, String> {
    adapter::spawn_session(config, launch)
}

pub(crate) fn rename_session(
    config: &AgentLaunchConfig,
    project: &std::path::Path,
    session: &std::path::Path,
    name: &str,
) -> Result<(), String> {
    adapter::rename_session(config, project, session, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_catalog_preserves_backend_specific_capabilities() {
        use contract::CapabilitySupport;

        let [pi, codex, opencode] = adapter::known_backend_descriptors();
        assert_eq!(pi.id.as_str(), "pi");
        assert_eq!(codex.id.as_str(), "codex-cli");
        assert_eq!(opencode.id.as_str(), "opencode2");
        assert_eq!(
            pi.capabilities.turns.follow_up,
            CapabilitySupport::Available
        );
        assert_eq!(
            codex.capabilities.turns.follow_up,
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            opencode.capabilities.configuration.commands,
            CapabilitySupport::Available
        );
    }
}
