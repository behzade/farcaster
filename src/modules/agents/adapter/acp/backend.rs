//! Externally installed ACP agents. No runtime installation or credential storage.
use super::AcpProfile;
use crate::agents::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(in crate::modules::agents::adapter) fn descriptor(
    profile: &AcpProfile,
    replays_history: bool,
) -> AgentBackendDescriptor {
    use crate::agents::HarnessAccessMode::{Full, Sandboxed};
    use CapabilitySupport::{Available, Unsupported};
    let history = if replays_history {
        Available
    } else {
        Unsupported
    };
    AgentBackendDescriptor {
        id: AgentBackendId::new(profile.backend).expect("valid ACP backend id"),
        name: profile.name.into(),
        capabilities: AgentCapabilities {
            sessions: SessionCapabilities {
                list: history.clone(),
                history,
                resume: Available,
                fork: Unsupported,
                rename: Unsupported,
                move_project: Unsupported,
                close: Available,
                delete: Unsupported,
            },
            turns: TurnCapabilities {
                prompt: Available,
                images: Available,
                interrupt: Available,
                steer: Unsupported,
                follow_up: Available,
                compact: Unsupported,
                queue: Available,
            },
            configuration: ConfigurationCapabilities {
                access_modes: &[Sandboxed, Full],
                model_required_access_modes: &[],
                models: Available,
                select_model: Available,
                reasoning_effort: Available,
                modes: Available,
                commands: Available,
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

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
