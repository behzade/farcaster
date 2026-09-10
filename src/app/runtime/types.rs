use super::*;

#[derive(Clone, Debug)]
pub(crate) struct TaskSettings {
    pub project: PathBuf,
    pub harness: String,
    pub model: Option<Model>,
    pub effort: Option<String>,
    pub access_mode: HarnessAccessMode,
}

#[derive(Clone)]
#[allow(dead_code)]
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
    UpdateConfigurationCatalog {
        harness: String,
        project: PathBuf,
        catalog: crate::agents::ConfigurationCatalog,
    },
    Abort,
    ApplySteering,
    StopSessionFamily {
        path: PathBuf,
    },
    DeleteSessionFamily {
        path: PathBuf,
    },
    Reload,
    LoadConfiguration {
        harness: String,
        project: PathBuf,
    },
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
    StartTask {
        id: String,
        settings: TaskSettings,
        message: String,
    },
    SendToSession {
        target: String,
        session: Option<crate::sessions::SessionTarget>,
        project: PathBuf,
        message: String,
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
        harness: String,
    },
    SetModel(Model),
    SetThinking(String),
    SetServiceTier(String),
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
    UpdateSessionMetadata(agents::SessionMetadata),
    ScheduleSessionRefresh,
    PreviewImport {
        harness: String,
        generation: u64,
    },
    CommitImport {
        sessions: Vec<SessionSummary>,
    },
    Shutdown,
}

#[derive(Clone, Debug)]
pub(crate) enum RuntimeEvent {
    SessionTarget(crate::sessions::SessionTarget),
    SystemNotification {
        title: String,
        body: String,
        target: Option<(PathBuf, PathBuf)>,
    },
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
        target: crate::sessions::SessionTarget,
        target_project: PathBuf,
        paths: Arc<HashMap<PathBuf, PathBuf>>,
    },
    SessionDeleted {
        generation: u64,
        paths: Arc<HashSet<PathBuf>>,
    },
    RefreshCatalog,
    SessionMetadata(agents::SessionMetadata),
    SessionUpdated(SessionSummary),
    ExtensionUi {
        generation: u64,
        request: crate::protocol::ExtensionUiRequest,
        system_notification_target: Option<(PathBuf, PathBuf)>,
    },
    PromptResult {
        target: String,
        accepted: bool,
        session: Option<PathBuf>,
    },
    SessionStatus {
        target: String,
        session: Option<PathBuf>,
        status: String,
    },
    ImportPreview {
        generation: u64,
        harness: String,
        sessions: Vec<SessionSummary>,
    },
    ImportPreviewFailed {
        generation: u64,
        harness: String,
        message: String,
    },
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum ConfigurationStatus {
    #[default]
    Loading,
    Loaded,
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RuntimeSnapshot {
    pub connected: bool,
    pub status: String,
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
    pub configuration_status: ConfigurationStatus,
    pub modes: Vec<AgentMode>,
    pub selected_mode: Option<String>,
    pub session_goal: Option<crate::agents::SessionGoal>,
    pub stats: Value,
    pub commands: Vec<SlashCommand>,
    pub stderr: String,
    pub auto_retry: bool,
    pub access_mode: HarnessAccessMode,
    pub history_preview: bool,
    pub pending_question: Option<ExtensionUiRequest>,
    pub transcript_changed_from: Option<usize>,
}
