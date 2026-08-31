use std::fmt;

use serde::{Deserialize, Serialize};

pub(crate) mod extensions;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct AgentBackendId(String);

impl AgentBackendId {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(format!(
                "agent backend id must contain only lowercase ASCII letters, digits, or hyphens: {value}"
            ));
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentBackendId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CapabilitySupport {
    Available,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionCapabilities {
    pub list: CapabilitySupport,
    pub history: CapabilitySupport,
    pub resume: CapabilitySupport,
    pub fork: CapabilitySupport,
    pub rename: CapabilitySupport,
    pub close: CapabilitySupport,
    pub delete: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TurnCapabilities {
    pub prompt: CapabilitySupport,
    pub images: CapabilitySupport,
    pub interrupt: CapabilitySupport,
    pub steer: CapabilitySupport,
    pub follow_up: CapabilitySupport,
    pub compact: CapabilitySupport,
    pub queue: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConfigurationCapabilities {
    pub models: CapabilitySupport,
    pub select_model: CapabilitySupport,
    pub reasoning_effort: CapabilitySupport,
    pub modes: CapabilitySupport,
    pub commands: CapabilitySupport,
    pub mcp_servers: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InteractionCapabilities {
    pub approvals: CapabilitySupport,
    pub questions: CapabilitySupport,
    pub notifications: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObservationCapabilities {
    pub streamed_text: CapabilitySupport,
    pub reasoning: CapabilitySupport,
    pub tool_activity: CapabilitySupport,
    pub usage: CapabilitySupport,
    pub child_agents: CapabilitySupport,
    pub file_changes: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentCapabilities {
    pub sessions: SessionCapabilities,
    pub turns: TurnCapabilities,
    pub configuration: ConfigurationCapabilities,
    pub interactions: InteractionCapabilities,
    pub observation: ObservationCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentBackendDescriptor {
    pub id: AgentBackendId,
    pub name: String,
    pub capabilities: AgentCapabilities,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum WorkerContext {
    #[default]
    Fresh,
    Session {
        session_locator: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkerInput {
    pub(crate) id: String,
    pub(crate) prompt: String,
    pub(crate) options: Vec<String>,
    pub(crate) secret: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkerInputResponse {
    pub(crate) id: String,
    pub(crate) value: Option<String>,
    pub(crate) cancel: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum FileAccessMode {
    ReadOnly,
    #[default]
    Sandboxed,
    Full,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum NetworkAccessMode {
    #[default]
    Sandboxed,
    Full,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PermissionLevel {
    pub(crate) files: FileAccessMode,
    pub(crate) network: NetworkAccessMode,
}

impl PermissionLevel {
    pub(crate) fn with_files(self, files: FileAccessMode) -> Self {
        Self { files, ..self }
    }

    pub(crate) fn with_network(self, network: NetworkAccessMode) -> Self {
        Self { network, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_ids_are_stable_machine_keys() -> Result<(), String> {
        assert_eq!(AgentBackendId::new("codex-cli")?.as_str(), "codex-cli");
        assert!(AgentBackendId::new("Codex CLI").is_err());
        assert!(AgentBackendId::new("").is_err());
        Ok::<(), String>(())
    }
}
