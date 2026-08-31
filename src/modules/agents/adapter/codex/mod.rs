mod connection;
#[cfg(test)]
mod connection_tests;
mod contract;
mod wire;
mod worker;

pub(crate) use worker::CodexWorkerFactory;

use super::super::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(crate) fn descriptor() -> AgentBackendDescriptor {
    use CapabilitySupport::{Available, Unsupported};

    AgentBackendDescriptor {
        id: AgentBackendId::new("codex-cli").expect("Codex CLI backend id is valid"),
        name: "Codex CLI".into(),
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
                modes: Available,
                commands: Unsupported,
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
    fn descriptor_keeps_codex_specific_features_independent() {
        let capabilities = descriptor().capabilities;
        assert_eq!(capabilities.turns.queue, CapabilitySupport::Available);
        assert_eq!(capabilities.turns.follow_up, CapabilitySupport::Available);
        assert_eq!(
            capabilities.observation.child_agents,
            CapabilitySupport::Available
        );
    }
}
