mod catalog;
mod events;
mod process;
mod worker;

#[cfg(test)]
mod live_tests;

pub(super) use catalog::{discover, load_history};
pub(super) use worker::{ClaudeWorkerFactory, load_configuration, spawn_main};

pub(super) const BACKEND: &str = "claude";

pub(super) fn program() -> std::path::PathBuf {
    std::env::var_os("FARCASTER_CLAUDE_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "claude".into())
}

pub(super) fn descriptor() -> crate::agents::contract::AgentBackendDescriptor {
    use crate::agents::contract::*;
    use CapabilitySupport::{Available, Unsupported};
    AgentBackendDescriptor {
        id: AgentBackendId::new(BACKEND).expect("valid Claude backend id"),
        name: "Claude Code".into(),
        capabilities: AgentCapabilities {
            sessions: SessionCapabilities {
                list: Available,
                history: Available,
                resume: Available,
                fork: Unsupported,
                rename: Unsupported,
                move_project: Unsupported,
                close: Available,
                delete: Unsupported,
            },
            turns: TurnCapabilities {
                prompt: Available,
                images: Available,
                interrupt: Available,
                steer: Unsupported,
                follow_up: Available,
                compact: Unsupported,
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
