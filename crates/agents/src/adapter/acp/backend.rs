//! Externally installed ACP agents. No runtime installation or credential storage.
use super::AcpProfile;
use crate::contract::{
    AgentBackendDescriptor, AgentCapabilities, CapabilitySupport, ConfigurationCapabilities,
    InteractionCapabilities, ObservationCapabilities, SessionCapabilities, TurnCapabilities,
};

pub fn descriptor(profile: &AcpProfile, replays_history: bool) -> AgentBackendDescriptor {
    use crate::HarnessAccessMode::{Full, Sandboxed};
    use CapabilitySupport::{Available, Unsupported};
    let history = if replays_history {
        Available
    } else {
        Unsupported
    };
    AgentBackendDescriptor {
        id: profile.backend,
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
                effort_label: "Effort",
                reset_reasoning_effort: Unsupported,
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
