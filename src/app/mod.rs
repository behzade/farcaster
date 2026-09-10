mod bootstrap;
mod change_detection;
mod composer;
mod event_projection;
pub(crate) mod extensions;
pub(crate) mod infrastructure;
#[allow(unused_imports)]
pub(crate) use infrastructure::{launch, paths, persistence, shell_environment};
pub(crate) mod mcp_server;
mod navigation;
mod project;
pub(crate) mod runtime;
mod session;
pub(crate) mod ui;
pub(crate) mod views;
mod workspace;
use change_detection::*;
pub(crate) use composer::ComposerImage;
pub(crate) use composer::ComposerPaste;
use composer::submissions::PendingSubmission;
use composer::{completion as composer_completion, file_mentions};
pub(crate) use navigation::{PICKER_KEY_CONTEXT, PickerScope, ProjectPickerIntent};
use project::{registry as project_registry, repository};
use session::{archive, drafts};
pub(crate) use views::OVERLAY_KEY_CONTEXT;
pub(crate) use views::transcript::list::TRANSCRIPT_SELECTION_KEY_CONTEXT;
pub(crate) use views::workgraph::{WORKGRAPH_KEY_CONTEXT, WORKGRAPH_NAV_KEY_CONTEXT};
use views::workgraph::{WorkGraphBoardView, WorkGraphSidebarView};
use views::{
    ComposerView, InactiveSessionRailView, RunPanelView, SessionRailKind, SessionRailView,
    TranscriptView, WorkGraphDetailView, roots_waiting_for_descendants,
};

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, Image, PathPromptOptions,
    RenderImage, Subscription, SystemNotification, Task, Window, actions,
};
use gpui_component::input::{InputEvent, InputState, TextareaState};
use gpui_libghostty::Terminal;
use workspace::neovim::NvimEditor;

use crate::{
    agent_activity::AgentActivity,
    app::composer::sessions::{
        ComposerSessions, ComposerSnapshot, HistoryNavigation, draft_target, project_target,
        session_target,
    },
    app::extensions::ExtensionUiState,
    app::views::transcript::list::TranscriptListState,
    projects,
    protocol::{BackgroundJob, Model},
    runtime::{RuntimeCommand, RuntimeEvent, RuntimeHandle, RuntimeSnapshot},
    sessions::{
        SessionRootIndex, SessionSummary, SessionTarget, descendant_sessions, root_session_for_path,
    },
};
#[cfg(test)]
use crate::{app::views::transcript::transcript_splice, protocol::ExtensionUiRequest};

const SYSTEM_NOTIFICATION_TAG: &str = "farcaster-agent";
pub(crate) const COMPOSER_KEY_CONTEXT: &str = "FarcasterComposer";
pub(crate) const APP_SHORTCUT_CONTEXT: &str = "FarcasterApp && input == app";
pub(crate) const APP_INPUT_CONTEXT: &str = "FarcasterApp input=app";
pub(crate) const NATIVE_INPUT_CONTEXT: &str = "FarcasterApp input=native";

#[derive(Debug, Eq, PartialEq)]
enum CurrentCloseTarget {
    Draft(String),
    Session(PathBuf),
    None,
}

actions!(
    farcaster,
    [
        DismissSurface,
        QuitApplication,
        SubmitFollowUp,
        SwitchSession0,
        SwitchSession1,
        SwitchSession2,
        SwitchSession3,
        SwitchSession4,
        SwitchSession5,
        SwitchSession6,
        SwitchSession7,
        SwitchSession8,
        SwitchSession9,
        NewSession,
        AddProject,
        SetSandbox,
        SetRuntime,
        SetHarness,
        RestoreSession,
        ShowActionPicker,
        PickerBack,
        PickerNavigateBack,
        FocusSessionSearch,
        FocusComposer,
        ShowEditor,
        OpenTranscriptScratch,
        ShowTerminal,
        PreviousSession,
        NextSession,
        ToggleArchivedSessions,
        SubmitPrompt,
        AbortRun,
        ComposerEscape,
        CloseCurrent,
        ComposerHistoryPrevious,
        ComposerHistoryNext,
        ComposerCompletionPrevious,
        ComposerCompletionNext,
        ShowKeybindings,
        IncreaseTranscriptFontSize,
        DecreaseTranscriptFontSize,
        ShowWorkGraph,
        WorkPreviousIssue,
        WorkNextIssue,
        WorkFocusSearch,
        WorkCreateIssue,
        WorkDismiss,
        WorkBack
    ]
);

#[derive(Clone, Debug, Eq, PartialEq, gpui::Action)]
#[action(namespace = farcaster, no_json)]
pub(crate) struct RemoveProject {
    pub(crate) path: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum AppSurface {
    #[default]
    Chat,
    Editor,
    Terminal,
    Work,
}

enum PostRenderFocus {
    ActiveSurface(Option<FocusHandle>),
    ImagePreview,
}

#[derive(Clone)]
struct SessionTitleEdit {
    path: PathBuf,
    project: PathBuf,
    original: String,
}

#[derive(Clone)]
pub(crate) struct ImagePreview {
    pub(crate) image: Arc<Image>,
    pub(crate) index: usize,
    pub(crate) total: usize,
}

pub(crate) struct FarcasterApp {
    project: PathBuf,
    runtime: RuntimeHandle,
    pub(crate) snapshot: Arc<RuntimeSnapshot>,
    sessions: Vec<SessionSummary>,
    all_sessions: Vec<SessionSummary>,
    agent_activities: HashMap<String, AgentActivity>,
    agent_row_focus: HashMap<String, FocusHandle>,
    background_jobs: Vec<BackgroundJob>,
    repository: repository::RepositoryState,
    session_order: Vec<i64>,
    session_drop_target: Option<(i64, crate::app::ui::primitives::ReorderPosition)>,
    run_statuses: HashMap<String, String>,
    recent_completions: HashMap<String, Instant>,
    recent_completion_expiries: HashMap<String, (Instant, Task<()>)>,
    system_notification_targets: HashMap<String, (PathBuf, PathBuf)>,
    projects: Vec<PathBuf>,
    excluded_projects: Vec<PathBuf>,
    drafts: Vec<projects::DraftSession>,
    draft_session_ids: HashMap<String, i64>,
    selected_draft: Option<String>,
    preferred_harness: String,
    submitted_drafts: HashMap<String, Option<PathBuf>>,
    sessions_error: Option<String>,
    session_project_filter: Option<PathBuf>,
    picker: Option<navigation::PickerState>,
    picker_return_focus: Option<FocusHandle>,
    session_generation: u64,
    runtime_generation: u64,
    composer: Entity<TextareaState>,
    composer_project_files: Vec<String>,
    composer_project_files_project: Option<PathBuf>,
    composer_project_files_loading: Option<PathBuf>,
    session_rail_view: Entity<SessionRailView>,
    archived_session_rail_view: Entity<InactiveSessionRailView>,
    transcript_view: Entity<TranscriptView>,
    composer_view: Entity<ComposerView>,
    run_panel_view: Entity<RunPanelView>,
    workgraph_view: Entity<WorkGraphBoardView>,
    workgraph_detail_view: Entity<WorkGraphDetailView>,
    workgraph_sidebar_view: Entity<WorkGraphSidebarView>,
    editor: Option<Entity<NvimEditor>>,
    project_editors: HashMap<(PathBuf, u64), Entity<NvimEditor>>,
    session_editor_tabs: HashMap<String, u64>,
    editor_ready: bool,
    editor_request_generation: u64,
    editor_return_focus: Option<FocusHandle>,
    terminal: Option<Entity<Terminal>>,
    terminal_project: Option<PathBuf>,
    project_terminals: HashMap<PathBuf, Entity<Terminal>>,
    native_surface_snapshot: Option<Arc<RenderImage>>,
    native_surface_covered: bool,
    surface: AppSurface,
    workgraph_inspector_issue: Option<u64>,
    composer_sessions: ComposerSessions,
    session_surfaces: HashMap<String, AppSurface>,
    composer_history_marker: Option<(String, usize, String)>,
    composer_escape_armed: Option<(String, Instant)>,
    composer_images: HashMap<String, Vec<ComposerImage>>,
    composer_pastes: HashMap<String, Vec<ComposerPaste>>,
    search: Entity<InputState>,
    search_focus: FocusHandle,
    session_title_input: Entity<InputState>,
    worker_profile_editor: workspace::worker_tasks::WorkerProfileEditor,
    runtime_picker: workspace::runtime_picker::RuntimePickerState,
    network_proxy_input: Entity<InputState>,
    network_proxy_error: Option<String>,
    settings_proxy_save: Option<Task<()>>,
    settings_mcp_error: Option<String>,
    expand_transcript_folders: bool,
    settings_transcript_error: Option<String>,
    editing_session_title: Option<SessionTitleEdit>,
    pending_session_titles: HashMap<PathBuf, String>,
    pending_session_title_focus: bool,
    dialog_input: Entity<TextareaState>,
    composer_focus: FocusHandle,
    chat_navigation: ui::navigation::ChatNavigation,
    dialog_focus: FocusHandle,
    dialog_return_focus: Option<FocusHandle>,
    image_preview: Option<ImagePreview>,
    code_comment: Option<workspace::code_comment::CodeComment>,
    code_comment_capture: Option<Task<()>>,
    code_tasks: workspace::code_tasks::CodeTasks,
    image_preview_focus: FocusHandle,
    image_preview_return_focus: Option<FocusHandle>,
    sheet_focus: FocusHandle,
    sheet_return_focus: Option<FocusHandle>,
    overlays: views::overlay_state::OverlayViewState,
    performance_monitor: Option<crate::app::infrastructure::performance::PerformanceMonitor>,
    _performance_task: Option<Task<()>>,
    pending_session_switch: Option<(PathBuf, crate::app::infrastructure::performance::Timing)>,
    extension: ExtensionUiState,
    parked_extension: Option<ExtensionUiState>,
    restored_dialog_id: Option<String>,
    dismissed_restored_dialog_id: Option<String>,
    notification_expiries: HashMap<(String, Instant), Task<()>>,
    pending_dialog_setup: bool,
    pending_title: Option<(u64, String)>,
    pending_editor_text: Option<(u64, String)>,
    pending_composer_restore: Option<(String, ComposerSnapshot)>,
    pending_submissions: HashMap<String, PendingSubmission>,
    post_render_focus: Option<PostRenderFocus>,
    pending_archive: Option<session::archive::PendingArchive>,
    pending_delete: Option<session::deletion::PendingDelete>,
    session_import: Option<session::import::SessionImportDialog>,
    session_import_generation: u64,
    archived_sessions_expanded: bool,
    project_trust_error: Option<String>,
    project_trust_project: Option<PathBuf>,
    project_trust_backend: Option<String>,
    pending_project_trust_command: Option<RuntimeCommand>,
    _composer_subscription: Subscription,
    _search_subscription: Subscription,
    _session_title_subscription: Subscription,
    _network_proxy_subscription: Subscription,
    _window_placement_subscription: Subscription,
    _event_task: Task<()>,
    _workgraph_update_task: Task<()>,
    _worker_update_task: Task<()>,
}

#[cfg(test)]
mod tests;
