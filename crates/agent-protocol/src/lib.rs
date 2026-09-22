use std::{path::PathBuf, thread, time::SystemTime};

pub use farcaster_contracts::{Backend, WorkerInput, WorkerSnapshot, WorkerStatus};
use serde::{Deserialize, Serialize};

mod effort;
pub mod extensions;
pub use effort::{effort_rank, model_efforts};
mod tool;
mod workers;

pub use tool::{CommonTool, ToolCategory, ToolMetadata};
pub use workers::{PeerMessage, StartWorker, valid_worker_name, validate_child_access};

use extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptImage, PromptMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredSession {
    pub id: String,
    pub harness: Backend,
    pub path: PathBuf,
    pub project: PathBuf,
    pub title: String,
    pub first_user_message: String,
    pub timestamp: String,
    pub parent_session: Option<String>,
    pub modified: SystemTime,
    pub message_count: usize,
    pub usage: DiscoveredUsage,
    pub archived: bool,
    pub is_running: bool,
    pub model: Option<(String, String)>,
    pub thinking_level: Option<String>,
    pub search: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
    pub cost_micros: u64,
}

/// Metadata supplied by a live session, never by a global history scan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub harness: Backend,
    pub id: String,
    pub path: PathBuf,
    pub project: PathBuf,
    pub title: Option<String>,
    pub first_user_message: Option<String>,
    pub parent_session: Option<String>,
    pub message_count: Option<usize>,
    pub model: Option<(String, String)>,
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default)]
    pub access_mode: Option<HarnessAccessMode>,
    pub usage: Option<DiscoveredUsage>,
    pub is_running: bool,
}

#[derive(Clone, Debug)]
pub struct DiscoveredHistory {
    pub messages: Vec<serde_json::Value>,
    pub model: Option<(String, String)>,
    pub thinking_level: Option<String>,
    pub prompt_deliveries: Option<farcaster_sessions::PromptDeliveryReconciliation>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigurationCatalog {
    pub models: Vec<extensions::Model>,
    pub efforts: Vec<String>,
    #[serde(default)]
    pub sandbox_adapter: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionGoal {
    pub objective: String,
    pub status: String,
    #[serde(default)]
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub time_used_seconds: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    #[serde(default)]
    pub weekly: Option<AccountUsageWindow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsageWindow {
    pub remaining_percent: f64,
    #[serde(default)]
    pub resets_at: Option<i64>,
}

impl AccountUsageWindow {
    pub fn from_used_percent(used_percent: f64, resets_at: Option<i64>) -> Option<Self> {
        used_percent.is_finite().then(|| Self {
            remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
            resets_at,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedPrompt {
    pub id: i64,
    /// Identifies the exact UI submission while this process remains alive.
    /// Recovered rows predate the UI process and have no pending composer entry.
    pub submission_id: Option<String>,
    pub target: String,
    pub harness: Backend,
    pub project: PathBuf,
    pub session: Option<PathBuf>,
    pub mode: PromptMode,
    pub message: String,
    pub display_message: Option<String>,
    pub invocation: Option<String>,
    pub images: Vec<PromptImage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptPresentation {
    pub resolved_message: String,
    pub display_message: String,
    pub invocation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionActivityKind {
    AgentStarted,
    AgentEnded,
    AgentSettled,
    MessageStarted,
    MessageUpdated,
    MessageEnded,
    PeerMessage,
    ToolStarted,
    ToolUpdated,
    ToolMetadataChanged,
    ToolFinished,
    QueueUpdated,
    CompactionStarted,
    CompactionFinished,
    RetryStarted,
    TurnEnded,
    SessionChanged,
    ServiceStatusChanged,
    AccountUsageChanged,
    SessionGoalChanged,
    ChildSessionsChanged,
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
            "peer_message" => Self::PeerMessage,
            "tool_execution_start" => Self::ToolStarted,
            "tool_execution_update" => Self::ToolUpdated,
            "tool_metadata_changed" => Self::ToolMetadataChanged,
            "tool_execution_end" => Self::ToolFinished,
            "queue_update" => Self::QueueUpdated,
            "compaction_start" => Self::CompactionStarted,
            "compaction_end" => Self::CompactionFinished,
            "auto_retry_start"
            | "summarization_retry_scheduled"
            | "summarization_retry_attempt_start" => Self::RetryStarted,
            "turn_end" => Self::TurnEnded,
            "session_info_changed" => Self::SessionChanged,
            "service_status_changed" => Self::ServiceStatusChanged,
            "account_usage_changed" => Self::AccountUsageChanged,
            "session_goal_changed" => Self::SessionGoalChanged,
            "child_sessions_changed" => Self::ChildSessionsChanged,
            other => Self::Other(other.to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionActivity {
    kind: SessionActivityKind,
    value: serde_json::Value,
}

impl SessionActivity {
    pub fn kind(&self) -> &SessionActivityKind {
        &self.kind
    }

    pub fn value(&self) -> &serde_json::Value {
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
pub enum SessionEvent {
    Response(SessionResponse),
    Interaction(ExtensionUiRequest),
    Activity(SessionActivity),
    Stderr(String),
    Failure(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionCommand {
    ConfigureSteering,
    ApplySteering,
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
    ResetReasoning,
    SelectServiceTier {
        tier: String,
    },
    #[allow(
        dead_code,
        reason = "Native mode selection remains part of the adapter contract."
    )]
    SelectMode {
        mode: String,
    },
}

impl SessionCommand {
    pub const fn response_operation(&self) -> SessionOperation {
        match self {
            Self::ConfigureSteering => SessionOperation::ConfigureSteering,
            Self::ApplySteering => SessionOperation::ApplySteering,
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
            Self::SelectReasoning { .. } | Self::ResetReasoning => {
                SessionOperation::SelectReasoning
            }
            Self::SelectServiceTier { .. } => SessionOperation::SelectServiceTier,
            Self::SelectMode { .. } => SessionOperation::SelectMode,
        }
    }

    pub const fn operation(&self) -> &'static str {
        match self {
            Self::ConfigureSteering => "configure steering",
            Self::ApplySteering => "apply steering",
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
            Self::ResetReasoning => "reset reasoning",
            Self::SelectServiceTier { .. } => "select service tier",
            Self::SelectMode { .. } => "select mode",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionOperation {
    ConfigureSteering,
    ApplySteering,
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
    SelectServiceTier,
    SelectMode,
    Other,
}

mod response;
pub use response::{
    PromptOutcome, SessionContextUsage, SessionHistory, SessionResponse, SessionResponseErrorKind,
    SessionResponsePayload, SessionUsage, SessionUsageTokens,
};

pub trait SessionTransport {
    fn clear_queue(&mut self) -> Result<(), String> {
        Err("This harness cannot clear its queue".into())
    }
    /// Requests cancellation of exactly one pending input. Completion arrives as
    /// a prompt_delivery activity with status cancelled, or a request error.
    /// Queue owners may ignore stale requests for inputs they no longer own;
    /// Ok alone is not evidence of cancellation. Only the receipt confirms it.
    fn cancel_prompt(&mut self, _id: &str) -> Result<(), String> {
        Err("This harness cannot cancel individual queued messages".into())
    }
    fn sandbox_adapter(&self) -> Option<&str> {
        None
    }
    fn sandbox_mode(&self) -> Option<HarnessAccessMode> {
        None
    }
    fn tracks_prompt_delivery(&self, _mode: PromptMode) -> bool {
        false
    }
    fn send(&mut self, command: SessionCommand) -> Result<String, String>;
    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String>;
    fn poll(&mut self) -> Option<SessionEvent>;
    fn close(&mut self) -> Result<(), String>;
}

#[derive(Clone, Debug)]
pub enum SessionStart {
    New,
    Resume(PathBuf),
    Fork(PathBuf),
}

pub struct SessionLaunch {
    pub harness: Backend,
    pub session_id: Option<String>,
    pub project: PathBuf,
    pub start: SessionStart,
    pub wake: Option<thread::Thread>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilitySupport {
    Available,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionCapabilities {
    pub list: CapabilitySupport,
    pub history: CapabilitySupport,
    pub resume: CapabilitySupport,
    pub fork: CapabilitySupport,
    pub rename: CapabilitySupport,
    pub move_project: CapabilitySupport,
    pub close: CapabilitySupport,
    pub delete: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnCapabilities {
    pub prompt: CapabilitySupport,
    pub images: CapabilitySupport,
    pub interrupt: CapabilitySupport,
    pub steer: CapabilitySupport,
    pub follow_up: CapabilitySupport,
    pub compact: CapabilitySupport,
    pub queue: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationCapabilities {
    /// Modes implemented by the adapter.
    pub access_modes: &'static [HarnessAccessMode],
    /// Modes that also require an explicit declaration from the selected model.
    pub model_required_access_modes: &'static [HarnessAccessMode],
    pub models: CapabilitySupport,
    pub select_model: CapabilitySupport,
    pub reasoning_effort: CapabilitySupport,
    pub effort_label: &'static str,
    pub reset_reasoning_effort: CapabilitySupport,
    pub modes: CapabilitySupport,
    pub commands: CapabilitySupport,
    pub mcp_servers: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractionCapabilities {
    pub approvals: CapabilitySupport,
    pub questions: CapabilitySupport,
    pub notifications: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationCapabilities {
    pub streamed_text: CapabilitySupport,
    pub reasoning: CapabilitySupport,
    pub tool_activity: CapabilitySupport,
    pub usage: CapabilitySupport,
    pub child_agents: CapabilitySupport,
    pub file_changes: CapabilitySupport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentCapabilities {
    pub sessions: SessionCapabilities,
    pub turns: TurnCapabilities,
    pub configuration: ConfigurationCapabilities,
    pub interactions: InteractionCapabilities,
    pub observation: ObservationCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentBackendDescriptor {
    pub id: Backend,
    pub name: String,
    pub capabilities: AgentCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentBackendStatus {
    pub id: Backend,
    pub name: String,
    pub program: std::path::PathBuf,
    pub available: bool,
    pub capabilities: AgentCapabilities,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WorkerContext {
    #[default]
    Fresh,
    Session {
        session_locator: String,
    },
    Resume {
        session_locator: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerInputResponse {
    pub id: String,
    pub value: Option<String>,
    pub cancel: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessAccessMode {
    Full,
    Sandboxed,
    #[default]
    Auto,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SandboxState {
    #[default]
    Unmanaged,
    // The previous mode remains effective until the queued change can start.
    Pending(HarnessAccessMode),
    Checking,
    Active(HarnessAccessMode),
    Failed,
}
