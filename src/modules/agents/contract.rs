use std::{fmt, path::PathBuf, thread, time::SystemTime};

use serde::{Deserialize, Serialize};

pub(crate) mod extensions;
mod workers;

pub(crate) use workers::{StartWorker, WorkerSnapshot, WorkerStatus};

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
    /// Canonical message objects, without backend persistence envelopes.
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
    pub(crate) display_message: Option<String>,
    pub(crate) invocation: Option<String>,
    pub(crate) images: Vec<PromptImage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PromptPresentation {
    pub(crate) resolved_message: String,
    pub(crate) display_message: String,
    pub(crate) invocation: String,
}

#[derive(Clone)]
pub(crate) struct AgentLaunchConfig {
    pub(crate) program: PathBuf,
    pub(crate) prefix_args: Vec<String>,
    pub(crate) access_mode: HarnessAccessMode,
    pub(crate) app_proxy: Option<String>,
    pub(crate) session_locator_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SessionActivityKind {
    AgentStarted,
    AgentEnded,
    AgentSettled,
    MessageStarted,
    MessageUpdated,
    MessageEnded,
    ToolStarted,
    ToolUpdated,
    ToolFinished,
    QueueUpdated,
    CompactionStarted,
    CompactionFinished,
    RetryStarted,
    TurnEnded,
    SessionChanged,
    Other(String),
}

impl SessionActivityKind {
    fn from_name(name: &str) -> Self {
        match name {
            "agent_start" => Self::AgentStarted,
            "agent_end" => Self::AgentEnded,
            "agent_settled" => Self::AgentSettled,
            "message_start" => Self::MessageStarted,
            "message_update" => Self::MessageUpdated,
            "message_end" => Self::MessageEnded,
            "tool_execution_start" => Self::ToolStarted,
            "tool_execution_update" => Self::ToolUpdated,
            "tool_execution_end" => Self::ToolFinished,
            "queue_update" => Self::QueueUpdated,
            "compaction_start" => Self::CompactionStarted,
            "compaction_end" => Self::CompactionFinished,
            "auto_retry_start"
            | "summarization_retry_scheduled"
            | "summarization_retry_attempt_start" => Self::RetryStarted,
            "turn_end" => Self::TurnEnded,
            "session_info_changed" => Self::SessionChanged,
            other => Self::Other(other.to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionActivity {
    kind: SessionActivityKind,
    value: serde_json::Value,
}

impl SessionActivity {
    pub(crate) fn kind(&self) -> &SessionActivityKind {
        &self.kind
    }

    pub(crate) fn value(&self) -> &serde_json::Value {
        &self.value
    }
}

impl From<serde_json::Value> for SessionActivity {
    fn from(value: serde_json::Value) -> Self {
        let kind = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .map(SessionActivityKind::from_name)
            .unwrap_or_else(|| SessionActivityKind::Other(String::new()));
        Self { kind, value }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SessionEvent {
    Response(SessionResponse),
    Interaction(ExtensionUiRequest),
    Activity(SessionActivity),
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
    pub(crate) const fn response_operation(&self) -> SessionOperation {
        match self {
            Self::ConfigureSteering => SessionOperation::ConfigureSteering,
            Self::LoadState => SessionOperation::LoadState,
            Self::LoadHistory => SessionOperation::LoadHistory,
            Self::LoadUsage => SessionOperation::LoadUsage,
            Self::ListModels => SessionOperation::ListModels,
            Self::ListReasoningLevels => SessionOperation::ListReasoningLevels,
            Self::ListModes => SessionOperation::ListModes,
            Self::ListCommands => SessionOperation::ListCommands,
            Self::Prompt { mode, .. } => SessionOperation::Prompt(*mode),
            Self::Abort => SessionOperation::Abort,
            Self::Compact { .. } => SessionOperation::Compact,
            Self::ExportHtml { .. } => SessionOperation::ExportHtml,
            Self::Rename { .. } => SessionOperation::Rename,
            Self::ForkAt { .. } => SessionOperation::ForkAt,
            Self::SelectModel { .. } => SessionOperation::SelectModel,
            Self::SelectReasoning { .. } => SessionOperation::SelectReasoning,
            Self::SelectMode { .. } => SessionOperation::SelectMode,
        }
    }

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionOperation {
    ConfigureSteering,
    LoadState,
    LoadHistory,
    LoadUsage,
    ListModels,
    ListReasoningLevels,
    ListModes,
    ListCommands,
    Prompt(PromptMode),
    Abort,
    Compact,
    ExportHtml,
    Rename,
    ForkAt,
    SelectModel,
    SelectReasoning,
    SelectMode,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionResponse {
    pub(crate) id: Option<String>,
    pub(crate) operation: SessionOperation,
    pub(crate) success: bool,
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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum HarnessAccessMode {
    Full,
    #[default]
    Sandboxed,
    Auto,
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
