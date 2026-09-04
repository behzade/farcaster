mod framing;
mod mcp_config;
mod process;
mod protocol;
pub(super) mod trust;
mod wire;
mod worker;

pub(super) use process::PiRpcProcess;
pub(super) use worker::PiWorkerFactory;

use super::super::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(crate) fn descriptor() -> AgentBackendDescriptor {
    use CapabilitySupport::{Available, Unsupported};

    AgentBackendDescriptor {
        id: AgentBackendId::new("pi").expect("Pi backend id is valid"),
        name: "Pi".into(),
        capabilities: AgentCapabilities {
            sessions: SessionCapabilities {
                list: Available,
                history: Available,
                resume: Available,
                fork: Available,
                rename: Available,
                close: Available,
                delete: Available,
            },
            turns: TurnCapabilities {
                prompt: Available,
                images: Available,
                interrupt: Available,
                steer: Available,
                follow_up: Available,
                compact: Available,
                queue: Available,
            },
            configuration: ConfigurationCapabilities {
                models: Available,
                select_model: Available,
                reasoning_effort: Available,
                modes: Unsupported,
                commands: Available,
                mcp_servers: Available,
            },
            interactions: InteractionCapabilities {
                approvals: Available,
                questions: Available,
                notifications: Available,
            },
            observation: ObservationCapabilities {
                streamed_text: Available,
                reasoning: Available,
                tool_activity: Available,
                usage: Available,
                child_agents: Unsupported,
                file_changes: Unsupported,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_does_not_collapse_independent_features() {
        let descriptor = descriptor();
        assert_eq!(
            descriptor.capabilities.turns.steer,
            CapabilitySupport::Available
        );
        assert_eq!(
            descriptor.capabilities.turns.follow_up,
            CapabilitySupport::Available
        );
        assert_eq!(
            descriptor.capabilities.configuration.modes,
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            descriptor.capabilities.observation.child_agents,
            CapabilitySupport::Unsupported
        );
    }
}
