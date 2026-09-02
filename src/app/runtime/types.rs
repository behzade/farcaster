//! Commands and snapshots crossing the app/runtime boundary.

use super::*;

#[derive(Clone)]
pub(crate) enum RuntimeCommand {
    Prompt {
        target: String,
        mode: PromptMode,
        message: String,
        display_message: Option<String>,
        invocation: Option<String>,
        images: Vec<PromptImage>,
        allow_while_running: bool,
    },
    Abort,
    StopSessionFamily {
        path: PathBuf,
    },
    DeleteSessionFamily {
        path: PathBuf,
    },
    Reload,
    Compact {
        custom_instructions: Option<String>,
    },
    ExportHtml {
        output_path: Option<String>,
    },
    SetSessionName(String),
    RenameSession {
        path: PathBuf,
        harness: String,
        session_id: String,
        project: PathBuf,
        name: String,
    },
    MoveSession {
        path: PathBuf,
        target_project: PathBuf,
    },
    NewSession {
        id: String,
        harness: String,
        project: PathBuf,
    },
    ForkSession {
        path: PathBuf,
        harness: String,
        session_id: String,
        project: PathBuf,
    },
    ResumeDraft {
        id: String,
        harness: String,
        project: PathBuf,
    },
    SelectSession {
        path: PathBuf,
        harness: String,
        session_id: String,
        project: PathBuf,
    },
    RestartSession {
        path: PathBuf,
        harness: String,
        session_id: String,
        project: PathBuf,
    },
    RefreshSessionDocument {
        path: PathBuf,
        project: PathBuf,
    },
    SetModel(Model),
    SetThinking(String),
    SetMode(String),
    SetAccessMode(HarnessAccessMode),
    SetAppProxy(Option<String>),
    ExtensionResponse(ExtensionUiResponse),
    DeliverQueued(crate::agents::QueuedPrompt),
    SetSessionArchived {
        path: PathBuf,
        archived: bool,
    },
    LoadSessions(String),
    RefreshSessions,
    Shutdown,
}

#[derive(Clone, Debug)]
pub(crate) enum RuntimeEvent {
    Snapshot {
        generation: u64,
        snapshot: Arc<RuntimeSnapshot>,
    },
    SessionReset {
        generation: u64,
        preserve_submission: bool,
    },
    HistoryReset {
        generation: u64,
    },
    Sessions {
        generation: u64,
        sessions: Vec<SessionSummary>,
        all_sessions: Vec<SessionSummary>,
        activities: Option<(HashMap<String, AgentActivity>, bool)>,
    },
    SessionsFailed {
        generation: u64,
        message: String,
    },
    SessionMoved {
        target_root: PathBuf,
        target_project: PathBuf,
        paths: Arc<HashMap<PathBuf, PathBuf>>,
    },
    SessionDeleted {
        generation: u64,
        paths: Arc<HashSet<PathBuf>>,
    },
    RefreshCatalog,
    ExtensionUi {
        generation: u64,
        request: crate::protocol::ExtensionUiRequest,
        system_notification_target: Option<(PathBuf, PathBuf)>,
    },
    PromptResult {
        generation: u64,
        target: String,
        accepted: bool,
        session: Option<PathBuf>,
    },
    SessionStatus {
        target: String,
        session: Option<PathBuf>,
        status: String,
    },
    SessionFilesModified {
        paths: Vec<PathBuf>,
    },
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RuntimeSnapshot {
    pub connected: bool,
    pub status: String,
    /// Selected backend, including while an unsubmitted draft is cold.
    pub harness: String,
    pub project: PathBuf,
    pub live_session: Option<PathBuf>,
    pub live_status: String,
    pub session: Option<SessionState>,
    pub prefill_model: Option<Model>,
    pub prefill_thinking_level: Option<String>,
    pub selected_session: Option<PathBuf>,
    pub conversation: Arc<ConversationState>,
    pub models: Vec<Model>,
    pub thinking_levels: Vec<String>,
    pub configuration_loaded: bool,
    pub configuration_error: Option<String>,
    pub modes: Vec<AgentMode>,
    pub selected_mode: Option<String>,
    pub stats: Value,
    pub commands: Vec<SlashCommand>,
    pub stderr: String,
    pub auto_retry: bool,
    pub access_mode: HarnessAccessMode,
    pub history_preview: bool,
    pub pending_question: Option<ExtensionUiRequest>,
    pub transcript_changed_from: Option<usize>,
}
