//! Top-level GPUI composition for the active root session.

mod bootstrap;
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
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, Image, ListAlignment, ListState,
    PathPromptOptions, RenderImage, Subscription, SystemNotification, Task, Window, actions, point,
    px,
};
use gpui_component::input::{InputEvent, InputState, TextareaState};
use gpui_libghostty::Terminal;
use gpui_neovim::NvimEditor;

#[cfg(test)]
use crate::app::views::transcript::transcript_splice;
use crate::{
    agent_activity::AgentActivity,
    app::composer::sessions::{
        ComposerSessions, ComposerSnapshot, HistoryNavigation, draft_target, project_target,
        session_target,
    },
    app::extensions::{ExtensionEffect, ExtensionUiState},
    app::views::transcript::list::TranscriptListState,
    projects,
    protocol::{BackgroundJob, ExtensionUiRequest, Model},
    runtime::{RuntimeCommand, RuntimeEvent, RuntimeHandle, RuntimeSnapshot},
    sessions::{
        SessionRootIndex, SessionSummary, SessionTarget, descendant_sessions, root_session_for_path,
    },
};

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

fn current_close_target(
    selected_draft: Option<&str>,
    selected_session: Option<&std::path::Path>,
) -> CurrentCloseTarget {
    if let Some(id) = selected_draft {
        CurrentCloseTarget::Draft(id.to_owned())
    } else if let Some(path) = selected_session {
        CurrentCloseTarget::Session(path.to_owned())
    } else {
        CurrentCloseTarget::None
    }
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
        ShowActionPicker,
        PickerBack,
        FocusSessionSearch,
        FocusComposer,
        ShowEditor,
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
        ShowWorkGraph,
        WorkPreviousIssue,
        WorkNextIssue,
        WorkFocusSearch,
        WorkCreateIssue,
        WorkDismiss
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
    system_notification_target: Option<(PathBuf, PathBuf)>,
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
    session_list: ListState,
    session_list_rows: RefCell<Vec<String>>,
    archived_session_list: ListState,
    archived_session_list_rows: RefCell<Vec<String>>,
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
    editor_error: Option<String>,
    editor_return_focus: Option<FocusHandle>,
    terminal: Option<Entity<Terminal>>,
    terminal_project: Option<PathBuf>,
    terminal_error: Option<String>,
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
    network_proxy_input: Entity<InputState>,
    network_proxy_error: Option<String>,
    settings_application_modifier: ui::keybindings::ApplicationModifier,
    settings_builtin_mcp_enabled: bool,
    editing_session_title: Option<SessionTitleEdit>,
    pending_session_titles: HashMap<PathBuf, String>,
    pending_session_title_focus: bool,
    dialog_input: Entity<TextareaState>,
    composer_focus: FocusHandle,
    dialog_focus: FocusHandle,
    dialog_return_focus: Option<FocusHandle>,
    image_preview: Option<ImagePreview>,
    image_preview_focus: FocusHandle,
    image_preview_return_focus: Option<FocusHandle>,
    sheet_focus: FocusHandle,
    sheet_return_focus: Option<FocusHandle>,
    view: views::state::ViewState,
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
    archived_sessions_expanded: bool,
    project_trust_error: Option<String>,
    project_trust_project: Option<PathBuf>,
    pending_project_trust_command: Option<RuntimeCommand>,
    _composer_subscription: Subscription,
    _search_subscription: Subscription,
    _session_title_subscription: Subscription,
    _window_activation_subscription: Subscription,
    _window_placement_subscription: Subscription,
    _event_task: Task<()>,
    _workgraph_update_task: Task<()>,
    _worker_update_task: Task<()>,
}

fn session_shortcuts_visible_for_window(current: bool, window_active: bool) -> bool {
    current && window_active
}

impl FarcasterApp {


    fn record_run_status(&mut self, target: String, status: String, force_recent: bool) -> bool {
        if status == "Done" {
            if starts_recent_completion(
                self.run_statuses.get(&target).map(String::as_str),
                &status,
                force_recent,
            ) {
                self.run_statuses.insert(target.clone(), status);
                self.recent_completions.insert(target, Instant::now());
                return true;
            }
            if self.recent_completions.contains_key(&target) {
                self.run_statuses.insert(target, status);
                return true;
            }
            self.run_statuses.remove(&target);
            self.recent_completions.remove(&target);
            return false;
        }
        self.recent_completions.remove(&target);
        self.run_statuses.insert(target, status);
        false
    }

    fn reset_session_ui(&mut self, generation: u64, preserve_submission: bool) {
        self.runtime_generation = generation;
        self.extension.reset();
        self.parked_extension = None;
        self.background_jobs.clear();
        self.restored_dialog_id = None;
        self.dismissed_restored_dialog_id = None;
        self.notification_expiries.clear();
        self.pending_dialog_setup = false;
        self.pending_title = Some((generation, "Pi".into()));
        self.pending_editor_text = None;
        self.post_render_focus = Some(PostRenderFocus::ActiveSurface(Some(
            self.composer_focus.clone(),
        )));
        self.dialog_return_focus = None;
        self.view.overlays.sessions = false;
        self.view.overlays.run = false;
        self.sheet_return_focus = None;
        self.view.overlays.pending_setup = false;
        if !preserve_submission {
            self.reset_transcript_ui();
        }
    }

    fn sync_restored_dialog(&mut self) {
        let Some(request) = self.snapshot.pending_question.clone() else {
            self.clear_restored_dialog();
            return;
        };
        let Some(id) = request.dialog_id().map(str::to_owned) else {
            return;
        };
        if self.restored_dialog_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.clear_restored_dialog();
        if self.dismissed_restored_dialog_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.dismissed_restored_dialog_id = None;
        if self.extension.dialog.is_some() {
            return;
        }
        if matches!(self.extension.apply(request), ExtensionEffect::DialogOpened) {
            self.restored_dialog_id = Some(id);
            self.pending_dialog_setup = true;
        }
    }

    fn clear_restored_dialog(&mut self) {
        if let Some(id) = self.restored_dialog_id.take() {
            let _ = self.extension.cancel(&id);
        }
    }

    fn apply_extension_request(
        &mut self,
        request: ExtensionUiRequest,
        generation: u64,
        _cx: &mut Context<Self>,
    ) {
        match self.extension.apply(request) {
            ExtensionEffect::DialogOpened => self.pending_dialog_setup = true,
            ExtensionEffect::SetTitle(title) => self.pending_title = Some((generation, title)),
            ExtensionEffect::SetEditorText(text) => {
                self.pending_editor_text = Some((generation, text))
            }
            ExtensionEffect::PersistError(_) | ExtensionEffect::None => {}
            ExtensionEffect::Diagnostic(message) => {
                Arc::make_mut(&mut Arc::make_mut(&mut self.snapshot).conversation)
                    .diagnostics
                    .push(message)
            }
        }
    }

    fn reset_transcript_ui(&mut self) {
        self.view.transcript.list.reset();
        self.view.transcript.list.scroll_to_end();
        self.view.transcript.rows =
            Arc::new(crate::app::ui::persistent_vec::PersistentVec::default());
        self.view.transcript.disclosure_states.clear();
        self.view.transcript.following = true;
        self.view.transcript.unseen = 0;
        self.view.transcript.last_count = 0;
    }

    pub(crate) fn activate_system_notification(
        &mut self,
        tag: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if tag != SYSTEM_NOTIFICATION_TAG {
            return;
        }
        if let Some((path, project)) = self.system_notification_target.clone() {
            self.select_session(path, project, window, cx);
        }
    }

    fn backend_target_for_path(&self, path: &Path) -> SessionTarget {
        self.all_sessions
            .iter()
            .find(|session| session.path == path)
            .map(SessionSummary::target)
            .unwrap_or_else(|| SessionTarget::pi(path.to_path_buf()))
    }

    fn select_session(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        let _timing =
            crate::app::infrastructure::performance::Timing::new("switch.session_request");
        if self.snapshot.selected_session.as_deref() == Some(path.as_path())
            && self.selected_draft.is_none()
        {
            self.close_sessions_sheet_after_selection(window, cx);
            return;
        }
        let previous_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let next_root =
            root_session_for_path(&self.sessions, Some(&path)).map(|session| session.id.clone());
        self.switch_composer_target(session_target(&path), window, cx);
        self.selected_draft = None;
        self.select_project(project.clone(), cx);
        self.restore_center_surface(project.clone(), window, cx);
        if let Some((_, timing)) = self.pending_session_switch.take() {
            timing.cancel();
        }
        self.pending_session_switch = Some((
            path.clone(),
            crate::app::infrastructure::performance::Timing::new("switch.session_total"),
        ));
        let target = self.backend_target_for_path(&path);
        self.send_project_command(
            &project,
            RuntimeCommand::SelectSession {
                path,
                harness: target.harness,
                session_id: target.id,
                project: project.clone(),
            },
            window,
            cx,
        );
        self.close_sessions_sheet_after_selection(window, cx);
        if previous_root != next_root {
            self.view
                .run_panel
                .scroll
                .set_offset(point(px(0.0), px(0.0)));
            self.notify_session_rail(cx);
        }
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    fn fork_session(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_project_trust_command.is_some() || self.workspace_switch_blocked() {
            return;
        }
        self.view
            .run_panel
            .scroll
            .set_offset(point(px(0.0), px(0.0)));
        self.selected_draft = None;
        self.select_project(project.clone(), cx);
        self.restore_center_surface(project.clone(), window, cx);
        let target = self.backend_target_for_path(&path);
        self.send_project_command(
            &project,
            RuntimeCommand::ForkSession {
                path,
                harness: target.harness,
                session_id: target.id,
                project: project.clone(),
            },
            window,
            cx,
        );
        self.close_sessions_sheet_after_selection(window, cx);
        self.notify_session_rail(cx);
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    fn new_session(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        self.view
            .run_panel
            .scroll
            .set_offset(point(px(0.0), px(0.0)));
        let draft = match project_registry::new_draft(project.clone(), &self.preferred_harness) {
            Ok(draft) => draft,
            Err(error) => {
                self.sessions_error = Some(error);
                self.notify_session_rail(cx);
                cx.notify();
                return;
            }
        };
        let draft_key = draft_target(&draft.id);
        self.switch_composer_target(draft_key, window, cx);
        self.selected_draft = Some(draft.id.clone());
        self.draft_session_ids
            .insert(draft.id.clone(), draft.app_session_id);
        self.drafts.push(draft.clone());
        self.save_project_registry();
        self.send_project_command(
            &project,
            RuntimeCommand::NewSession {
                id: draft.id,
                harness: draft.harness,
                project: project.clone(),
            },
            window,
            cx,
        );
        self.select_project(project.clone(), cx);
        self.restore_center_surface(project, window, cx);
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.close_sessions_sheet_after_selection(window, cx);
        self.notify_session_rail(cx);
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    fn resume_draft(
        &mut self,
        id: String,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        if self.selected_draft.as_deref() == Some(id.as_str()) && !self.snapshot.history_preview {
            self.close_sessions_sheet_after_selection(window, cx);
            return;
        }
        self.view
            .run_panel
            .scroll
            .set_offset(point(px(0.0), px(0.0)));
        self.switch_composer_target(draft_target(&id), window, cx);
        self.selected_draft = Some(id.clone());
        self.select_project(project.clone(), cx);
        self.restore_center_surface(project.clone(), window, cx);
        let draft_harness = self
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .map(|draft| draft.harness.clone())
            .unwrap_or_else(|| "pi".into());
        let command = if let Some(Some(path)) = self.submitted_drafts.get(&id).cloned() {
            let target = self.backend_target_for_path(&path);
            RuntimeCommand::SelectSession {
                path,
                harness: target.harness,
                session_id: target.id,
                project: project.clone(),
            }
        } else {
            RuntimeCommand::ResumeDraft {
                id,
                harness: draft_harness,
                project: project.clone(),
            }
        };
        self.send_project_command(&project, command, window, cx);
        self.close_sessions_sheet_after_selection(window, cx);
        self.notify_session_rail(cx);
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    fn choose_project_folder(
        &mut self,
        intent: Option<ProjectPickerIntent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = selected.await else {
                return;
            };
            let Some(project) = paths.pop() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(project) = this.add_project(project, cx) else {
                    return;
                };
                match intent {
                    Some(ProjectPickerIntent::NewSession) => {
                        this.new_session(project, window, cx);
                    }
                    Some(ProjectPickerIntent::ChangeDraft) => {
                        this.change_draft_project(project, window, cx);
                        this.composer_focus.focus(window, cx);
                    }
                    Some(ProjectPickerIntent::MoveSession { path, .. }) => {
                        this.move_session(path, project, window, cx);
                    }
                    None => {}
                }
            });
        })
        .detach();
    }

    fn add_project(&mut self, project: PathBuf, cx: &mut Context<Self>) -> Option<PathBuf> {
        let project = match project.canonicalize() {
            Ok(project) if project.is_dir() => project,
            Ok(project) => {
                self.sessions_error = Some(format!(
                    "Project path is not a folder: {}",
                    project.display()
                ));
                self.notify_session_rail(cx);
                cx.notify();
                return None;
            }
            Err(error) => {
                self.sessions_error = Some(format!("Open {}: {error}", project.display()));
                self.notify_session_rail(cx);
                cx.notify();
                return None;
            }
        };
        let restored = projects::restore(&mut self.excluded_projects, &project);
        if projects::add_unique(&mut self.projects, project.clone()) || restored {
            self.save_project_registry();
        }
        self.notify_session_rail(cx);
        cx.notify();
        Some(project)
    }

    fn remove_project_from_picker(
        &mut self,
        project: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !projects::remove(&mut self.projects, &mut self.excluded_projects, project) {
            return;
        }
        self.save_project_registry();
        let scope = self
            .picker
            .as_ref()
            .map(|picker| picker.scope.clone())
            .unwrap_or(PickerScope::Projects(ProjectPickerIntent::NewSession));
        self.open_picker(scope, window, cx);
    }

    fn select_project(&mut self, project: PathBuf, cx: &mut Context<Self>) {
        if self.project != project {
            self.composer_project_files.clear();
            self.composer_project_files_project = None;
            self.composer_project_files_loading = None;
        }
        self.project = project.clone();
        self.select_repository_project(project.clone(), cx);
        if projects::select(&mut self.projects, &self.excluded_projects, project) {
            self.save_project_registry();
        }
    }

    fn request_composer_project_files(&mut self, cx: &mut Context<Self>) {
        let project = self.project.clone();
        if !self.repository.execution_allowed {
            self.composer_project_files.clear();
            self.composer_project_files_project = Some(project);
            self.composer_project_files_loading = None;
            self.notify_composer(cx);
            return;
        }
        if self.composer_project_files_project.as_ref() == Some(&project)
            || self.composer_project_files_loading.as_ref() == Some(&project)
        {
            return;
        }
        self.composer_project_files_loading = Some(project.clone());
        let preference = self.repository.preference;
        let task = cx.background_spawn(async move {
            let files = file_mentions::project_files(&project, preference);
            (project, preference, files)
        });
        cx.spawn(async move |weak, cx| {
            let (project, preference, files) = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.composer_project_files_loading.as_ref() == Some(&project) {
                    this.composer_project_files_loading = None;
                }
                if this.project == project && this.repository.preference == preference {
                    this.composer_project_files = files;
                    this.composer_project_files_project = Some(project);
                    this.notify_composer(cx);
                }
            });
        })
        .detach();
    }

    fn save_project_registry(&mut self) {
        if let Err(error) = project_registry::save(&projects::Registry {
            projects: self.projects.clone(),
            excluded_projects: self.excluded_projects.clone(),
            drafts: self.drafts.clone(),
        }) {
            self.sessions_error = Some(error);
        }
    }

    fn discard_draft(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        let was_selected = self.selected_draft.as_deref() == Some(id);
        let target = draft_target(id);
        self.composer_images.remove(&target);
        self.composer_pastes.remove(&target);
        self.session_surfaces.remove(&target);
        self.drafts.retain(|draft| draft.id != id);
        self.draft_session_ids.remove(id);
        self.submitted_drafts.remove(id);
        self.run_statuses.remove(&target);
        self.recent_completions.remove(&target);
        self.recent_completion_expiries.remove(&target);
        if was_selected {
            self.selected_draft = None;
            if let Some(session) = self.sessions.first().cloned() {
                self.select_project(session.project.clone(), cx);
                let snapshot = self
                    .composer_sessions
                    .discard_and_switch(&target, session_target(&session.path));
                self.apply_composer_snapshot(snapshot, window, cx);
                self.restore_center_surface(session.project.clone(), window, cx);
                self.send_project_command(
                    &session.project,
                    RuntimeCommand::SelectSession {
                        path: session.path,
                        harness: session.harness,
                        session_id: session.id,
                        project: session.project.clone(),
                    },
                    window,
                    cx,
                );
            } else {
                let snapshot = self
                    .composer_sessions
                    .discard_and_switch(&target, project_target(&self.project));
                self.apply_composer_snapshot(snapshot, window, cx);
                self.restore_center_surface(self.project.clone(), window, cx);
            }
        } else {
            let current = self.composer_sessions.current_target().to_owned();
            let _ = self.composer_sessions.discard_and_switch(&target, current);
        }
        self.save_project_registry();
        self.notify_session_rail(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    fn move_session(
        &mut self,
        path: PathBuf,
        target_project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        let Some(session) = self
            .all_sessions
            .iter()
            .find(|session| session.path == path)
        else {
            self.sessions_error = Some("The session is no longer available to move".to_owned());
            self.notify_session_rail(cx);
            return;
        };
        if session.project == target_project {
            return;
        }
        if session.harness != "pi" {
            self.sessions_error = Some(format!(
                "Moving {} sessions between projects is not supported",
                session.harness
            ));
            self.notify_session_rail(cx);
            return;
        }
        if session.is_running {
            self.sessions_error = Some(
                "Wait for the session to finish before moving it to another project".to_owned(),
            );
            self.notify_session_rail(cx);
            return;
        }
        self.send_project_command(
            &target_project,
            RuntimeCommand::MoveSession {
                path,
                target_project: target_project.clone(),
            },
            window,
            cx,
        );
    }

    fn set_session_active(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.set_session_archived(path, false, cx);
    }

    fn set_session_archived(&mut self, path: PathBuf, archived: bool, cx: &mut Context<Self>) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.path == path)
        {
            session.archived = archived;
        }
        if !self.sessions.iter().any(|session| session.archived) {
            self.archived_sessions_expanded = false;
        }
        self.send(RuntimeCommand::SetSessionArchived { path, archived });
        self.notify_session_rail(cx);
        self.notify_run_panel(cx);
    }

    fn switch_composer_target(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = input_snapshot(self.composer.read(cx));
        let current_target = self.composer_sessions.current_target().to_owned();
        let discard = self.sync_current_draft(&current, &current_target);
        let snapshot = if discard {
            self.session_surfaces.remove(&current_target);
            self.composer_sessions
                .discard_and_switch(&current_target, target)
        } else {
            self.capture_center_surface();
            self.composer_sessions.switch_to(target, current)
        };
        self.apply_composer_snapshot(snapshot, window, cx);
    }

    fn capture_composer_session(&mut self, cx: &mut Context<Self>) {
        self.composer_sessions
            .capture_current(input_snapshot(self.composer.read(cx)));
    }

    fn apply_composer_snapshot(
        &self,
        snapshot: ComposerSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = snapshot.restore_range();
        let text = snapshot.text;
        self.composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_selected_range(range, cx);
        });
    }

    pub(super) fn handle_composer_history_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = input_snapshot(self.composer.read(cx));
        match self.composer_sessions.navigate_history(key, current) {
            HistoryNavigation::PassThrough => false,
            HistoryNavigation::Handled(snapshot) => {
                if let Some(snapshot) = snapshot {
                    self.apply_composer_snapshot(snapshot, window, cx);
                }
                true
            }
        }
    }

    fn select_model(&mut self, model: &Model, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetModel(model.clone()));
        cx.notify();
    }

    fn set_thinking_level(&mut self, level: String, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetThinking(level));
        cx.notify();
    }

    fn set_agent_mode(&mut self, mode: String, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetMode(mode));
        cx.notify();
    }

    fn set_access_mode(
        &mut self,
        level: crate::runtime::HarnessAccessMode,
        cx: &mut Context<Self>,
    ) {
        self.send(RuntimeCommand::SetAccessMode(level));
        cx.notify();
    }
}

fn transcript_follow_state_needs_update(current: bool, unseen: usize, following: bool) -> bool {
    current != following || (following && unseen != 0)
}

fn update_transcript_follow_state(current: &mut bool, unseen: &mut usize, following: bool) -> bool {
    let changed = transcript_follow_state_needs_update(*current, *unseen, following);
    *current = following;
    if following {
        *unseen = 0;
    }
    changed
}

fn session_catalog_changed(
    current: &[SessionSummary],
    current_all: &[SessionSummary],
    current_error: Option<&str>,
    next: &[SessionSummary],
    next_all: &[SessionSummary],
) -> bool {
    current != next || current_all != next_all || current_error.is_some()
}

fn inactive_session_catalog_changed(
    current: &[SessionSummary],
    current_all: &[SessionSummary],
    next: &[SessionSummary],
    next_all: &[SessionSummary],
) -> bool {
    let rows = |sessions: &[SessionSummary]| {
        sessions
            .iter()
            .filter(|session| session.parent_session.is_none() && session.archived)
            .map(|session| {
                (
                    session.id.clone(),
                    session.app_session_id,
                    session.path.clone(),
                    session.project.clone(),
                    session.title.clone(),
                    session.modified,
                    session.is_running,
                )
            })
            .collect::<Vec<_>>()
    };
    let current_rows = rows(current);
    if current_rows != rows(next) {
        return true;
    }
    let ids = current_rows
        .iter()
        .map(|(id, ..)| id.as_str())
        .collect::<HashSet<_>>();
    let waiting = |sessions| {
        roots_waiting_for_descendants(sessions)
            .into_iter()
            .filter(|id| ids.contains(id.as_str()))
            .collect::<HashSet<_>>()
    };
    waiting(current_all) != waiting(next_all)
}

fn session_event_affects_active_rail(
    drafts: &[projects::DraftSession],
    submitted_drafts: &HashMap<String, Option<PathBuf>>,
    sessions: &[SessionSummary],
    target: &str,
    session_path: Option<&Path>,
) -> bool {
    if target
        .strip_prefix("draft:")
        .is_some_and(|id| drafts.iter().any(|draft| draft.id == id))
        || submitted_drafts
            .values()
            .flatten()
            .any(|path| session_path == Some(path.as_path()) || session_target(path) == target)
    {
        return true;
    }
    let session = session_path
        .and_then(|path| sessions.iter().find(|session| session.path == path))
        .or_else(|| {
            sessions
                .iter()
                .find(|session| session_target(&session.path) == target)
        });
    session
        .and_then(|session| root_session_for_path(sessions, Some(&session.path)))
        .is_some_and(|root| !root.archived)
}

fn run_panel_sessions_changed(
    current: &[SessionSummary],
    next: &[SessionSummary],
    selected: Option<&Path>,
) -> bool {
    visible_sessions_changed(current, next, selected, |left, right| {
        left.id == right.id
            && left.path == right.path
            && left.project == right.project
            && left.timestamp == right.timestamp
            && left.parent_session == right.parent_session
            && left.is_running == right.is_running
    })
}

fn composer_usage_sessions_changed(
    current: &[SessionSummary],
    next: &[SessionSummary],
    selected: Option<&Path>,
) -> bool {
    visible_sessions_changed(current, next, selected, |left, right| {
        left.id == right.id
            && left.path == right.path
            && left.parent_session == right.parent_session
            && left.usage == right.usage
    })
}

fn visible_sessions_changed(
    current: &[SessionSummary],
    next: &[SessionSummary],
    selected: Option<&Path>,
    equal: impl Fn(&SessionSummary, &SessionSummary) -> bool,
) -> bool {
    fn visible<'a>(
        sessions: &'a [SessionSummary],
        selected: Option<&Path>,
    ) -> Vec<(&'a SessionSummary, usize)> {
        let Some(root) = root_session_for_path(sessions, selected) else {
            return Vec::new();
        };
        let mut result = vec![(root, 0)];
        result.extend(descendant_sessions(sessions, &root.id));
        result
    }

    let current = visible(current, selected);
    let next = visible(next, selected);
    current.len() != next.len()
        || current
            .iter()
            .zip(next)
            .any(|((left, left_depth), (right, right_depth))| {
                left_depth != &right_depth || !equal(left, right)
            })
}

fn run_panel_activities_changed(
    current: &HashMap<String, crate::agent_activity::AgentActivity>,
    next: Option<&(HashMap<String, crate::agent_activity::AgentActivity>, bool)>,
    sessions: &[SessionSummary],
    selected: Option<&Path>,
) -> bool {
    let Some((activities, exhaustive)) = next else {
        return false;
    };
    let Some(root) = root_session_for_path(sessions, selected) else {
        return false;
    };
    let visible_ids = std::iter::once(root.id.as_str())
        .chain(
            descendant_sessions(sessions, &root.id)
                .into_iter()
                .map(|(session, _)| session.id.as_str()),
        )
        .collect::<Vec<_>>();
    visible_ids.into_iter().any(|id| {
        activities
            .get(id)
            .is_some_and(|activity| current.get(id) != Some(activity))
            || (*exhaustive && current.contains_key(id) && !activities.contains_key(id))
    })
}

fn session_rail_snapshot_changed(
    roots: &SessionRootIndex<'_>,
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    let root_id = |path| roots.root_for_path(path).map(|session| session.id.as_str());
    root_id(previous.selected_session.as_deref()) != root_id(next.selected_session.as_deref())
        || root_id(previous.live_session.as_deref()) != root_id(next.live_session.as_deref())
        || previous.live_status != next.live_status
}

fn inactive_session_rail_snapshot_changed(
    roots: &SessionRootIndex<'_>,
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    let root_id = |path| {
        roots
            .root_for_path(path)
            .filter(|session| session.archived)
            .map(|session| session.id.as_str())
    };
    if root_id(previous.selected_session.as_deref()) != root_id(next.selected_session.as_deref()) {
        return true;
    }
    let previous_live = root_id(previous.live_session.as_deref());
    let next_live = root_id(next.live_session.as_deref());
    previous_live != next_live || (previous.live_status != next.live_status && next_live.is_some())
}

fn composer_snapshot_changed(previous: &RuntimeSnapshot, next: &RuntimeSnapshot) -> bool {
    previous.conversation.items.is_empty() != next.conversation.items.is_empty()
        || previous.selected_session != next.selected_session
        || previous.commands != next.commands
        || previous.conversation.running != next.conversation.running
        || previous.conversation.queue != next.conversation.queue
        || previous.conversation.average_cache_hit_rate != next.conversation.average_cache_hit_rate
        || previous.stats != next.stats
        || previous.pending_question != next.pending_question
        || previous.session_identity() != next.session_identity()
        || previous.models != next.models
        || previous.thinking_levels != next.thinking_levels
        || previous.configuration_status != next.configuration_status
        || previous.access_mode != next.access_mode
}

fn run_panel_snapshot_changed(previous: &RuntimeSnapshot, next: &RuntimeSnapshot) -> bool {
    previous.selected_session != next.selected_session
}

fn input_snapshot(input: &TextareaState) -> ComposerSnapshot {
    ComposerSnapshot::new(
        input.value().to_string(),
        input.cursor(),
        input.selected_range(),
    )
}

fn park_extension_surface(visible: &mut ExtensionUiState, parked: &mut Option<ExtensionUiState>) {
    if parked.is_none() {
        *parked = Some(std::mem::take(visible));
    }
}

fn restore_extension_surface(
    visible: &mut ExtensionUiState,
    parked: &mut Option<ExtensionUiState>,
) {
    if let Some(session) = parked.take() {
        *visible = session;
    }
}

fn starts_recent_completion(previous: Option<&str>, next: &str, force: bool) -> bool {
    next == "Done" && (force || previous.is_some_and(|status| status != "Done"))
}

#[cfg(test)]
mod tests;
