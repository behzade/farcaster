mod catalog;
mod connection;
#[cfg(test)]
mod connection_tests;
mod contract;
mod wire;
mod worker;

pub(super) use catalog::{delete_session, discover, load_history, rename_session};
pub(crate) use worker::CodexWorkerFactory;
pub(super) use worker::{load_configuration, spawn_main};

const fn approvals_reviewer(mode: crate::agents::HarnessAccessMode) -> &'static str {
    match mode {
        crate::agents::HarnessAccessMode::Auto => "auto_review",
        crate::agents::HarnessAccessMode::Full | crate::agents::HarnessAccessMode::Sandboxed => {
            "user"
        }
    }
}

fn configure_permissions(
    command: &mut std::process::Command,
    mode: crate::agents::HarnessAccessMode,
) {
    use crate::agents::HarnessAccessMode;

    match mode {
        HarnessAccessMode::Full => {
            command.arg("--dangerously-bypass-approvals-and-sandbox");
        }
        HarnessAccessMode::Sandboxed => {
            command.args([
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "-c",
                "approvals_reviewer=\"user\"",
            ]);
        }
        HarnessAccessMode::Auto => {
            command.arg("--approve-for-me");
        }
    }
}

use super::super::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(crate) fn descriptor() -> AgentBackendDescriptor {
    use CapabilitySupport::{Available, Unsupported};

    AgentBackendDescriptor {
        id: AgentBackendId::new("codex-cli").expect("Codex backend id is valid"),
        name: "Codex".into(),
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
    fn native_process_uses_selected_harness_permissions() {
        let arguments = |mode| {
            let mut command = std::process::Command::new("codex");
            configure_permissions(&mut command, mode);
            command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        use crate::agents::HarnessAccessMode::{Auto, Full, Sandboxed};
        assert_eq!(
            arguments(Full),
            ["--dangerously-bypass-approvals-and-sandbox"]
        );
        assert_eq!(
            arguments(Sandboxed),
            [
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "-c",
                "approvals_reviewer=\"user\""
            ]
        );
        assert_eq!(arguments(Auto), ["--approve-for-me"]);
        assert_eq!(approvals_reviewer(Auto), "auto_review");
        assert_eq!(approvals_reviewer(Sandboxed), "user");
        assert_eq!(approvals_reviewer(Full), "user");
    }

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
