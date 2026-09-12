pub(super) mod deletion;
mod framing;
mod mcp_config;
mod process;
mod protocol;
mod response;
pub(super) mod session_files;
mod tool;
pub(super) mod transfer;
pub(super) mod trust;
mod wire;
mod worker;

pub(super) use process::{PiRpcProcess, launch_configuration};
pub(super) use tool::annotate_pi_message as annotate_history_message;
pub(super) use worker::PiWorkerFactory;

use super::super::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(crate) fn descriptor() -> AgentBackendDescriptor {
    use crate::agents::HarnessAccessMode::{Full, Sandboxed};
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
#[path = "mod_tests.rs"]
mod tests;
