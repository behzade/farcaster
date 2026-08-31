mod client;
#[cfg(test)]
mod client_tests;
mod contract;
mod event;
mod server;
mod transport;
mod worker;

pub(crate) use worker::OpenCodeWorkerFactory;

use super::super::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(crate) fn descriptor() -> AgentBackendDescriptor {
    use CapabilitySupport::{Available, Unsupported};

    AgentBackendDescriptor {
        id: AgentBackendId::new("opencode2").expect("OpenCode backend id is valid"),
        name: "OpenCode 2".into(),
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
                follow_up: Unsupported,
                compact: Available,
                queue: Available,
            },
            configuration: ConfigurationCapabilities {
                models: Available,
                select_model: Available,
                reasoning_effort: Available,
                modes: Available,
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
                child_agents: Available,
                file_changes: Available,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_keeps_opencode_specific_features_independent() {
        let capabilities = descriptor().capabilities;
        assert_eq!(capabilities.turns.queue, CapabilitySupport::Available);
        assert_eq!(capabilities.turns.follow_up, CapabilitySupport::Unsupported);
        assert_eq!(
            capabilities.configuration.commands,
            CapabilitySupport::Available
        );
    }
}
