mod catalog;
mod client;
#[cfg(test)]
mod client_tests;
mod contract;
mod event;
mod server;
mod transport;
mod worker;

pub(crate) use worker::OpenCodeWorkerFactory;
pub(super) use catalog::{delete_session, discover, load_history, rename_session};
pub(super) use worker::{load_configuration, spawn_main};

fn configure_permissions(command: &mut std::process::Command) {
    // Farcaster's nono profile is the process boundary. OpenCode should not
    // prompt for or deny work that the outer policy already allows.
    command.env("OPENCODE_PERMISSION", r#"{"*":"allow"}"#);
}

use super::super::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(crate) fn descriptor() -> AgentBackendDescriptor {
    use CapabilitySupport::Available;

    AgentBackendDescriptor {
        id: AgentBackendId::new("opencode2").expect("OpenCode backend id is valid"),
        name: "OpenCode".into(),
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
    fn native_process_skips_inner_permissions() {
        let mut command = std::process::Command::new("opencode");
        command.args(["serve", "--stdio", "--print-logs"]);
        configure_permissions(&mut command);
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            ["serve", "--stdio", "--print-logs"]
        );
        let permission = command
            .get_envs()
            .find(|(name, _)| *name == "OPENCODE_PERMISSION")
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned());
        assert_eq!(permission.as_deref(), Some(r#"{"*":"allow"}"#));
    }

    #[test]
    fn descriptor_keeps_opencode_specific_features_independent() {
        let capabilities = descriptor().capabilities;
        assert_eq!(capabilities.turns.queue, CapabilitySupport::Available);
        assert_eq!(capabilities.turns.follow_up, CapabilitySupport::Available);
        assert_eq!(
            capabilities.configuration.commands,
            CapabilitySupport::Available
        );
    }
}
