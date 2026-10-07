mod backend;
pub(super) use backend::CursorAdapter;

// Retain the legacy catalog for the existing ACP rename hook. SDK sessions use history.rs.
#[allow(dead_code)]
mod catalog;
pub(super) use catalog::rename as rename_session;

mod auth;
mod bridge;
mod client;
mod configuration;
mod events;
mod history;
mod timing;
mod worker;

use super::super::contract::{
    AgentBackendDescriptor, AgentCapabilities, Backend, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};
use std::path::PathBuf;

pub(super) const PROFILE: super::acp::AcpProfile = super::acp::AcpProfile {
    backend: Backend::Cursor,
    name: "Cursor",
    command: "agent",
    path_environment: "FARCASTER_CURSOR_PATH",
    arguments: &["acp"],
    auth_method: None,
    force_argument: Some("--force"),
    resume_method: "session/load",
    permission_modes: None,
};

pub(super) fn program() -> PathBuf {
    if let Some(path) = std::env::var_os("FARCASTER_CURSOR_PATH").filter(|p| !p.is_empty()) {
        return path.into();
    }
    if let Some(root) = installation_dir() {
        let installed = root.join("bin/cursor-sdk-bridge");
        if installed.is_file() {
            return installed;
        }
    }
    "cursor-sdk-bridge".into()
}

fn installation_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/share/farcaster/cursor-sdk/1.0.35"))
}

pub fn descriptor() -> AgentBackendDescriptor {
    use crate::HarnessAccessMode::{Auto, Full, Sandboxed};
    use CapabilitySupport::{Available, Unsupported};

    AgentBackendDescriptor {
        id: Backend::Cursor,
        name: "Cursor".into(),
        capabilities: AgentCapabilities {
            sessions: SessionCapabilities {
                list: Available,
                history: Available,
                resume: Available,
                fork: Unsupported,
                rename: Unsupported,
                move_project: Unsupported,
                close: Available,
                delete: Available,
            },
            turns: TurnCapabilities {
                prompt: Available,
                images: Available,
                interrupt: Available,
                steer: Available,
                follow_up: Available,
                compact: Unsupported,
                queue: Available,
            },
            configuration: ConfigurationCapabilities {
                access_modes: &[Sandboxed, Auto, Full],
                model_required_access_modes: &[],
                models: Available,
                select_model: Available,
                service_tier: crate::ServiceTierPolicy::default(),
                reasoning_effort: Available,
                effort_label: "Effort",
                reset_reasoning_effort: CapabilitySupport::Unsupported,
                modes: Available,
                commands: Unsupported,
                mcp_servers: Available,
            },
            interactions: InteractionCapabilities {
                approvals: Unsupported,
                questions: Unsupported,
                notifications: Unsupported,
            },
            observation: ObservationCapabilities {
                streamed_text: Available,
                reasoning: Available,
                tool_activity: Available,
                usage: Available,
                child_agents: Unsupported,
                file_changes: Available,
            },
        },
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
