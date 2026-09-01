use std::{fmt, path::PathBuf, thread, time::SystemTime};

use serde::{Deserialize, Serialize};

pub(crate) mod extensions;
mod workers;

pub(crate) use workers::{StartWorker, WorkerMessageMode, WorkerSnapshot, WorkerStatus};

use crate::access::{GrantStore, SandboxRuntime};
use extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptImage, PromptMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiscoveredSession {
    pub(crate) id: String,
    pub(crate) harness: String,
    pub(crate) path: PathBuf,
    pub(crate) project: PathBuf,
    pub(crate) title: String,
    pub(crate) first_user_message: String,
    pub(crate) timestamp: String,
    pub(crate) parent_session: Option<String>,
    pub(crate) modified: SystemTime,
    pub(crate) message_count: usize,
    pub(crate) usage: DiscoveredUsage,
    pub(crate) archived: bool,
    pub(crate) is_running: bool,
    pub(crate) search: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DiscoveredUsage {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write: u64,
    pub(crate) total: u64,
    pub(crate) cost_micros: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct DiscoveredHistory {
    pub(crate) messages: Vec<serde_json::Value>,
    pub(crate) model: Option<(String, String)>,
    pub(crate) thinking_level: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ConfigurationCatalog {
    pub(crate) models: Vec<extensions::Model>,
    pub(crate) efforts: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QueuedPrompt {
    pub(crate) id: i64,
    pub(crate) target: String,
    pub(crate) harness: String,
    pub(crate) project: PathBuf,
    pub(crate) session: Option<PathBuf>,
    pub(crate) mode: PromptMode,
    pub(crate) message: String,
    pub(crate) images: Vec<PromptImage>,
}

#[derive(Clone)]
pub(crate) struct AgentLaunchConfig {
    pub(crate) program: PathBuf,
    pub(crate) prefix_args: Vec<String>,
    pub(crate) permission_level: PermissionLevel,
    pub(crate) sandbox: SandboxRuntime,
    pub(crate) grants: Option<GrantStore>,
    pub(crate) app_proxy: Option<String>,
    pub(crate) session_locator_root: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SessionEvent {
    Response(SessionResponse),
    Interaction(ExtensionUiRequest),
    Activity(serde_json::Value),
    Stderr(String),
    Failure(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionCommand {
    ConfigureSteering,
    LoadState,
    LoadHistory,
    LoadUsage,
    ListModels,
    ListReasoningLevels,
    ListModes,
    ListCommands,
    Prompt {
        mode: PromptMode,
        message: String,
        images: Vec<PromptImage>,
    },
    Abort,
    Compact {
        instructions: Option<String>,
    },
    ExportHtml {
        output_path: Option<String>,
    },
    Rename {
        name: String,
    },
    ForkAt {
        entry_id: String,
    },
    SelectModel {
        provider: String,
        model_id: String,
    },
    SelectReasoning {
        level: String,
    },
    SelectMode {
        mode: String,
    },
}

impl SessionCommand {
    pub(crate) const fn operation(&self) -> &'static str {
        match self {
            Self::ConfigureSteering => "configure steering",
            Self::LoadState => "load state",
            Self::LoadHistory => "load history",
            Self::LoadUsage => "load usage",
            Self::ListModels => "list models",
            Self::ListReasoningLevels => "list reasoning levels",
            Self::ListModes => "list modes",
            Self::ListCommands => "list commands",
            Self::Prompt { mode, .. } => match mode {
                PromptMode::Normal => "prompt",
                PromptMode::Steer => "steer",
                PromptMode::FollowUp => "follow up",
            },
            Self::Abort => "abort",
            Self::Compact { .. } => "compact",
            Self::ExportHtml { .. } => "export HTML",
            Self::Rename { .. } => "rename session",
            Self::ForkAt { .. } => "fork session",
            Self::SelectModel { .. } => "select model",
            Self::SelectReasoning { .. } => "select reasoning",
            Self::SelectMode { .. } => "select mode",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct SessionResponse {
    pub(crate) id: Option<String>,
    pub(crate) command: String,
    pub(crate) success: bool,
    #[serde(default)]
    pub(crate) data: serde_json::Value,
    pub(crate) error: Option<String>,
}

pub(crate) trait SessionTransport {
    fn send(&mut self, command: SessionCommand) -> Result<String, String>;
    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String>;
    fn poll(&mut self) -> Option<SessionEvent>;
    fn close(&mut self) -> Result<(), String>;
}

#[derive(Clone, Debug)]
pub(crate) enum SessionStart {
    New,
    Resume(PathBuf),
    Fork(PathBuf),
}

pub(crate) struct SessionLaunch {
    pub(crate) harness: String,
    pub(crate) session_id: Option<String>,
    pub(crate) project: PathBuf,
    pub(crate) start: SessionStart,
    pub(crate) wake: Option<thread::Thread>,
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentBackendStatus {
    pub id: String,
    pub name: String,
    pub available: bool,
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
