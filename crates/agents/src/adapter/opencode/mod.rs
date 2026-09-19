mod catalog;
mod client;
#[cfg(test)]
mod client_tests;
mod commands;
mod contract;
mod event;
mod server;
mod tool;
mod transfer;
mod transport;
mod worker;

pub(super) use catalog::{delete_session, discover, load_history, rename_session};
pub(super) use transfer::move_family;
pub use worker::OpenCodeWorkerFactory;
pub(super) use worker::{load_configuration, spawn_main};

use super::super::contract::{
    AgentBackendDescriptor, AgentCapabilities, Backend, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(super) fn program() -> std::path::PathBuf {
    std::env::var_os("FARCASTER_OPENCODE_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "opencode".into())
}

pub fn descriptor() -> AgentBackendDescriptor {
    use crate::HarnessAccessMode::{Full, Sandboxed};
    use CapabilitySupport::Available;

    AgentBackendDescriptor {
        id: Backend::OpenCode,
        name: "OpenCode".into(),
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
                access_modes: &[Sandboxed, Full],
                model_required_access_modes: &[],
                models: Available,
                select_model: Available,
                reasoning_effort: Available,
                effort_label: "Variant",
                reset_reasoning_effort: Available,
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
