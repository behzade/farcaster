use std::path::Path;

use super::super::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};
use super::acp::{AcpProfile, AcpWorkerFactory};

pub(super) const PROFILE: AcpProfile = AcpProfile {
    backend: "cursor-cli",
    name: "Cursor",
    command: "agent",
    path_environment: "FARCASTER_CURSOR_PATH",
    arguments: &["acp"],
    auth_method: Some("cursor_login"),
    force_argument: Some("--force"),
};

pub(crate) fn descriptor() -> AgentBackendDescriptor {
    use CapabilitySupport::{Available, Unsupported};

    AgentBackendDescriptor {
        id: AgentBackendId::new(PROFILE.backend).expect("Cursor backend id is valid"),
        name: PROFILE.name.into(),
        capabilities: AgentCapabilities {
            sessions: SessionCapabilities {
                list: Available,
                history: Available,
                resume: Available,
                fork: Unsupported,
                rename: Unsupported,
                close: Available,
                delete: Unsupported,
            },
            turns: TurnCapabilities {
                prompt: Available,
                images: Available,
                interrupt: Available,
                steer: Unsupported,
                follow_up: Unsupported,
                compact: Unsupported,
                queue: Unsupported,
            },
            configuration: ConfigurationCapabilities {
                models: Available,
                select_model: Available,
                reasoning_effort: Unsupported,
                modes: Available,
                commands: Unsupported,
                mcp_servers: Available,
            },
            interactions: InteractionCapabilities {
                approvals: Available,
                questions: Unsupported,
                notifications: Available,
            },
            observation: ObservationCapabilities {
                streamed_text: Available,
                reasoning: Available,
                tool_activity: Available,
                usage: Unsupported,
                child_agents: Unsupported,
                file_changes: Available,
            },
        },
    }
}

pub(super) fn worker_factory(mut command: crate::agents::AgentLaunchConfig) -> AcpWorkerFactory {
    command.program = PROFILE.program();
    AcpWorkerFactory::new(command, PROFILE)
}

pub(super) fn spawn_main(
    command: &crate::agents::AgentLaunchConfig,
    launch: &crate::agents::SessionLaunch,
) -> Result<
    (
        Box<dyn crate::agents::WorkerSession>,
        String,
        super::main_session::MainSessionMetadata,
    ),
    String,
> {
    super::acp::spawn_main(command, &PROFILE, launch)
}

pub(super) fn discover(
    locator_root: &Path,
    query: &str,
) -> Result<Vec<crate::agents::DiscoveredSession>, String> {
    super::acp::discover(&PROFILE, locator_root, query)
}

pub(super) fn load_history(path: &Path) -> Result<crate::agents::DiscoveredHistory, String> {
    super::acp::load_history(&PROFILE, path, None)
}

pub(super) fn load_history_at(
    path: &Path,
    project: &Path,
) -> Result<crate::agents::DiscoveredHistory, String> {
    super::acp::load_history(&PROFILE, path, Some(project))
}
