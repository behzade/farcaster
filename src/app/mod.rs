use crate::agents::Backend;
mod bootstrap;
mod change_detection;
mod composer;
mod event_projection;
pub(crate) mod extensions;
pub(crate) mod infrastructure;
#[cfg(test)]
mod live_e2e_tests;
#[cfg(test)]
mod mcp_server_live_children_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[allow(unused_imports)]
pub(crate) use infrastructure::{launch, paths, persistence, shell_environment};
pub(crate) mod mcp_server;
mod navigation;
mod project;
pub(crate) use farcaster_runtime::runtime;
mod runtime_host;
mod session;
mod session_folders;
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
use session::{archive, status::roots_waiting_for_descendants};
pub(crate) use views::OVERLAY_KEY_CONTEXT;
pub(crate) use views::transcript::list::TRANSCRIPT_SELECTION_KEY_CONTEXT;
pub(crate) use views::workgraph::{WORKGRAPH_KEY_CONTEXT, WORKGRAPH_NAV_KEY_CONTEXT};
use views::workgraph::{WorkGraphBoardView, WorkGraphSidebarView};
use views::{
    ComposerView, InactiveSessionRailView, RunPanelView, SessionRailKind, SessionRailView,
    TranscriptView, WorkGraphDetailView,
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
use workspace::Terminal;
use workspace::neovim::NvimEditor;
use workspace::terminal_editor::TerminalEditor;

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
    sessions,
    sessions::{
        SessionRootIndex, SessionSummary, SessionTarget, descendant_sessions_for_root,
        root_session_for_path,
    },
};
#[cfg(test)]
use crate::{app::views::transcript::transcript_splice, protocol::ExtensionUiRequest};

const SYSTEM_NOTIFICATION_TAG: &str = "farcaster-agent";
pub(crate) const COMPOSER_KEY_CONTEXT: &str = "FarcasterComposer";
pub(crate) const APP_SHORTCUT_CONTEXT: &str = "FarcasterApp && input == app";
pub(crate) const APP_INPUT_CONTEXT: &str = "FarcasterApp input=app";
pub(crate) const NATIVE_INPUT_CONTEXT: &str = "FarcasterApp input=native";
pub(crate) const CHAT_INPUT_CONTEXT: &str = "FarcasterApp input=app surface=chat";
pub(crate) const CHAT_SHORTCUT_CONTEXT: &str = "FarcasterApp && input == app && surface == chat";

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
        NextTranscriptSession,
        PreviousTranscriptSession,
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
    runtime: RuntimeHandle,
    pub(crate) snapshot: Arc<RuntimeSnapshot>,
    runtime_generation: u64,
    project: project::ProjectState,
    sessions: session::SessionState,
    activity: session::ActivityState,
    composer: composer::ComposerState,
    navigation: navigation::NavigationState,
    workspace: workspace::WorkspaceState,
    settings: workspace::SettingsState,
    extensions: extensions::ExtensionState,
    views: views::AppViews,
    overlays: views::AppOverlays,
    lifecycle: infrastructure::AppLifecycle,
    worker_notices: mcp_server::NoticeBoard,
}

#[cfg(test)]
mod tests;
