mod backend;
pub(super) use backend::CodexAdapter;
mod catalog;
mod connection;
#[cfg(test)]
mod connection_tests;
mod contract;
mod notification;
mod skills;
mod subagents;
mod tool;
mod transfer;
mod wire;
mod worker;

pub(super) use catalog::{delete_session, discover, load_history};
pub(super) use transfer::move_family;
pub use worker::CodexWorkerFactory;
pub(super) use worker::{load_configuration, spawn_main};

const fn approvals_reviewer(mode: crate::HarnessAccessMode) -> &'static str {
    match mode {
        crate::HarnessAccessMode::Auto => "auto_review",
        crate::HarnessAccessMode::Full | crate::HarnessAccessMode::Sandboxed => "user",
    }
}

fn configure_permissions(command: &mut std::process::Command, mode: crate::HarnessAccessMode) {
    use crate::HarnessAccessMode;

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
    AgentBackendDescriptor, AgentCapabilities, Backend, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub fn descriptor() -> AgentBackendDescriptor {
    use crate::HarnessAccessMode::{Auto, Full, Sandboxed};
    use CapabilitySupport::Available;

    AgentBackendDescriptor {
        id: Backend::Codex,
        name: "Codex".into(),
        capabilities: AgentCapabilities {
            sessions: SessionCapabilities {
                list: Available,
                history: Available,
                resume: Available,
                fork: Available,
                rename: Available,
                move_project: Available,
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
                access_modes: &[Sandboxed, Auto, Full],
                model_required_access_modes: &[],
                models: Available,
                select_model: Available,
                reasoning_effort: Available,
                effort_label: "Effort",
                reset_reasoning_effort: CapabilitySupport::Unsupported,
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
#[path = "mod_tests.rs"]
mod tests;
