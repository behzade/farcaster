mod backend;
pub(super) use backend::OpenCodeAdapter;
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

pub(super) fn model_override() -> Option<contract::OpenCodeModelSelection> {
    let value = std::env::var("FARCASTER_OPENCODE_MODEL").ok()?;
    let selection = parse_model_override(&value);
    if selection.is_none() {
        zlog::warn!("Ignoring FARCASTER_OPENCODE_MODEL={value}: expected provider/model");
    }
    selection
}

fn parse_model_override(value: &str) -> Option<contract::OpenCodeModelSelection> {
    let (model, variant) = match value.split_once('#') {
        Some((model, variant)) => (model, Some(variant)),
        None => (value, None),
    };
    let (provider_id, id) = model.split_once('/')?;
    if provider_id.is_empty() || id.is_empty() {
        return None;
    }
    Some(contract::OpenCodeModelSelection {
        id: id.to_owned(),
        provider_id: provider_id.to_owned(),
        variant: variant
            .filter(|variant| !variant.is_empty() && *variant != "default")
            .map(str::to_owned),
    })
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
