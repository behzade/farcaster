//! Top-level GPUI composition for the active root session.

mod archive;
mod changes;
mod composer_completion;
mod composer_images;
mod drafts;
mod expiries;
mod file_mentions;
mod picker;
mod region_state;
mod session_titles;
mod slash_commands;
mod submissions;
mod surfaces;
mod transcript_ui;
mod trust;
mod views;
mod workgraph;
pub(crate) use composer_images::ComposerImage;
pub(crate) use picker::{PICKER_KEY_CONTEXT, PickerScope, ProjectPickerIntent};
use submissions::PendingSubmission;
pub(crate) use views::OVERLAY_KEY_CONTEXT;
use views::{
    ComposerView, InactiveSessionRailView, RunPanelView, SessionRailKind, SessionRailView,
    TranscriptView, WorkGraphDetailView, roots_waiting_for_descendants,
};
pub(crate) use workgraph::adapter::{WORKGRAPH_KEY_CONTEXT, WORKGRAPH_NAV_KEY_CONTEXT};
use workgraph::{adapter::WorkGraphBoardView, sidebar::WorkGraphSidebarView};

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, FollowMode, ListAlignment,
    ListState, PathPromptOptions, ScrollHandle, Subscription, SystemNotification, Task, Window,
    actions, point, px,
};
use gpui_component::input::{InputEvent, InputState, TextareaState};
use gpui_fps::FpsMonitor;

#[cfg(test)]
use crate::transcript::transcript_splice;
use crate::{
    agent_activity::AgentActivity,
    composer_sessions::{
        ComposerSessions, ComposerSnapshot, HistoryNavigation, draft_target, project_target,
        session_target,
    },
    extension_ui::{ExtensionEffect, ExtensionUiState},
    projects,
    protocol::{ExtensionUiRequest, Model},
    runtime::{RuntimeCommand, RuntimeEvent, RuntimeHandle, RuntimeSnapshot},
    sessions::{SessionRootIndex, SessionSummary, descendant_sessions, root_session_for_path},
};

const SYSTEM_NOTIFICATION_TAG: &str = "pi-agent";
pub(crate) const COMPOSER_KEY_CONTEXT: &str = "PiComposer";

fn transcript_list_state() -> ListState {
    ListState::new(
        0,
        ListAlignment::Top,
        crate::theme::THEME.layout.transcript_overdraw,
    )
    .with_uniform_item_height(crate::transcript::TRANSCRIPT_ROW_HEIGHT_HINT)
}

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
    pi_gpui,
    [
        DismissSurface,
        QuitApplication,
        SubmitFollowUp,
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
        PreviousSession,
        NextSession,
        ToggleArchivedSessions,
        SubmitPrompt,
        AbortRun,
        CloseCurrent,
        ComposerHistoryPrevious,
        ComposerHistoryNext,
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
#[action(namespace = pi_gpui, no_json)]
pub(crate) struct RemoveProject {
    pub(crate) path: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum AppSurface {
    #[default]
    Chat,
    Work,
}

impl AppSurface {
    const fn toggled(self) -> Self {
        match self {
            Self::Chat => Self::Work,
            Self::Work => Self::Chat,
        }
    }
}

#[derive(Clone)]
struct SessionTitleEdit {
    path: PathBuf,
    project: PathBuf,
    original: String,
}

pub(crate) struct PiApp {
    project: PathBuf,
    runtime: RuntimeHandle,
    pub(crate) snapshot: Arc<RuntimeSnapshot>,
    sessions: Vec<SessionSummary>,
    all_sessions: Vec<SessionSummary>,
    agent_activities: HashMap<String, AgentActivity>,
    agent_row_focus: HashMap<String, FocusHandle>,
    changes: changes::ChangesState,
    session_order: Vec<i64>,
    session_drop_target: Option<(i64, crate::primitives::ReorderPosition)>,
    run_statuses: HashMap<String, String>,
    recent_completions: HashMap<String, Instant>,
    recent_completion_expiries: HashMap<String, (Instant, Task<()>)>,
    system_notification_target: Option<(PathBuf, PathBuf)>,
    projects: Vec<PathBuf>,
    drafts: Vec<projects::DraftSession>,
    draft_session_ids: HashMap<String, i64>,
    selected_draft: Option<String>,
    submitted_drafts: HashMap<String, Option<PathBuf>>,
    sessions_error: Option<String>,
    session_project_filter: Option<PathBuf>,
    picker: Option<picker::PickerState>,
    pending_extension_picker: Option<PickerScope>,
    picker_return_focus: Option<FocusHandle>,
    session_list: ListState,
    session_list_rows: RefCell<Vec<String>>,
    review_session_list: ListState,
    review_session_list_rows: RefCell<Vec<String>>,
    archived_session_list: ListState,
    archived_session_list_rows: RefCell<Vec<String>>,
    session_generation: u64,
    runtime_generation: u64,
    composer: Entity<TextareaState>,
    composer_project_files: Vec<String>,
    composer_project_files_project: Option<PathBuf>,
    composer_project_files_loading: Option<PathBuf>,
    composer_suggestion_selection: usize,
    session_rail_view: Entity<SessionRailView>,
    review_session_rail_view: Entity<InactiveSessionRailView>,
    archived_session_rail_view: Entity<InactiveSessionRailView>,
    transcript_view: Entity<TranscriptView>,
    composer_view: Entity<ComposerView>,
    run_panel_view: Entity<RunPanelView>,
    workgraph_view: Entity<WorkGraphBoardView>,
    workgraph_detail_view: Entity<WorkGraphDetailView>,
    workgraph_sidebar_view: Entity<WorkGraphSidebarView>,
    surface: AppSurface,
    workgraph_inspector_issue: Option<u64>,
    run_panel_scroll: ScrollHandle,
    composer_footer_scroll: ScrollHandle,
    composer_sessions: ComposerSessions,
    composer_history_marker: Option<(String, usize, String)>,
    composer_images: HashMap<String, Vec<ComposerImage>>,
    search: Entity<InputState>,
    search_focus: FocusHandle,
    session_title_input: Entity<InputState>,
    editing_session_title: Option<SessionTitleEdit>,
    pending_session_title_focus: bool,
    dialog_input: Entity<TextareaState>,
    dialog_secret_input: Entity<InputState>,
    composer_focus: FocusHandle,
    dialog_focus: FocusHandle,
    dialog_return_focus: Option<FocusHandle>,
    sheet_focus: FocusHandle,
    sheet_return_focus: Option<FocusHandle>,
    pending_sheet_setup: bool,
    transcript_list: ListState,
    transcript_rows: Arc<crate::persistent_vec::PersistentVec<crate::transcript::TranscriptRow>>,
    transcript_following: bool,
    transcript_unseen: usize,
    pub(crate) transcript_disclosure_states: HashMap<usize, bool>,
    last_transcript_count: usize,
    fps_monitor: Option<Entity<FpsMonitor>>,
    performance_monitor: Option<crate::performance::PerformanceMonitor>,
    _performance_task: Option<Task<()>>,
    pending_session_switch: Option<(PathBuf, crate::performance::Timing)>,
    extension: ExtensionUiState,
    parked_extension: Option<ExtensionUiState>,
    restored_dialog_id: Option<String>,
    dismissed_restored_dialog_id: Option<String>,
    notification_expiries: HashMap<(String, Instant), Task<()>>,
    pending_dialog_setup: bool,
    pending_title: Option<(u64, String)>,
    pending_editor_text: Option<(u64, String)>,
    pending_submissions: HashMap<String, PendingSubmission>,
    pending_session_reset: bool,
    sessions_sheet: bool,
    review_sessions_expanded: bool,
    pending_archive: Option<archive::PendingArchive>,
    archived_sessions_expanded: bool,
    run_sheet: bool,
    keybindings_help: bool,
    project_trust_sheet: bool,
    project_trust_error: Option<String>,
    project_trust_project: Option<PathBuf>,
    pending_project_trust_command: Option<RuntimeCommand>,
    completed_agents_expanded: bool,
    limited_agents_expanded: bool,
    session_shortcuts_visible: bool,
    _composer_subscription: Subscription,
    _search_subscription: Subscription,
    _session_title_subscription: Subscription,
    _window_activation_subscription: Subscription,
    _window_placement_subscription: Subscription,
    _event_task: Task<()>,
}

fn session_shortcuts_visible_for_window(current: bool, window_active: bool) -> bool {
    current && window_active
}

impl PiApp {
    pub(crate) fn new(project: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (mut registry, mut project_registry_error) = match projects::load() {
            Ok(registry) => (registry, None),
            Err(error) => (projects::Registry::default(), Some(error)),
        };
        projects::select(&mut registry.projects, project.clone());
        let session_order = match projects::load_app_session_order() {
            Ok(order) => order,
            Err(error) => {
                if project_registry_error.is_none() {
                    project_registry_error = Some(error);
                }
                Vec::new()
            }
        };
        let initial_draft = match projects::new_draft(project.clone()) {
            Ok(draft) => draft,
            Err(error) => {
                if project_registry_error.is_none() {
                    project_registry_error = Some(error);
                }
                projects::DraftSession::with_id(
                    format!("untracked-draft-{}", std::process::id()),
                    project.clone(),
                )
            }
        };
        let selected_draft = initial_draft.id.clone();
        let mut draft_session_ids = registry
            .drafts
            .iter()
            .map(|draft| (draft.id.clone(), draft.app_session_id))
            .collect::<HashMap<_, _>>();
        draft_session_ids.insert(initial_draft.id, initial_draft.app_session_id);
        if project_registry_error.is_none()
            && let Err(error) = projects::save(&registry)
        {
            project_registry_error = Some(error);
        }
        let (composer_sessions, composer_error) =
            ComposerSessions::load(draft_target(&selected_draft));
        if project_registry_error.is_none() {
            project_registry_error = composer_error;
        }
        let submitted_drafts = drafts::submitted_draft_associations(&registry.drafts);
        let runtime = RuntimeHandle::spawn(project.clone(), selected_draft.clone(), None);
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .submit_on_enter(true)
                .placeholder("Ask Pi")
        });
        let initial_composer = composer_sessions.current();
        composer.update(cx, |input, cx| {
            input.set_value(initial_composer.text.clone(), window, cx);
            input.set_selected_range(initial_composer.restore_range(), cx);
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions"));
        let search_focus = search.read(cx).focus_handle(cx);
        let session_title_input = cx.new(|cx| InputState::new(window, cx));
        let dialog_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 12)
                .submit_on_enter(false)
        });
        let dialog_secret_input = cx.new(|cx| InputState::new(window, cx).masked(true));
        let composer_focus = composer.read(cx).focus_handle(cx);
        let dialog_focus = cx.focus_handle();
        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, state, event: &InputEvent, _window, cx| match event {
                InputEvent::Change => {
                    this.composer_suggestion_selection = 0;
                    this.composer_sessions.exit_history();
                    let snapshot = input_snapshot(state.read(cx));
                    let has_mention =
                        file_mentions::query_at_cursor(&snapshot.text, snapshot.cursor).is_some();
                    this.composer_sessions.capture_current(snapshot);
                    if has_mention {
                        this.request_composer_project_files(cx);
                    }
                    this.notify_composer(cx);
                }
                InputEvent::Blur => {
                    this.composer_sessions
                        .capture_current(input_snapshot(state.read(cx)));
                }
                InputEvent::PressEnter { shift: false, .. } => {
                    let input = state.read(cx);
                    let value = input.value();
                    if let Some(completion) = composer_completion::resolve(
                        &value,
                        input.cursor(),
                        &this.composer_project_files,
                        this.composer_suggestion_selection,
                        &this.snapshot.commands,
                    ) {
                        let submitted_value = completion
                            .submit
                            .then(|| completion.snapshot.text.trim().to_owned());
                        this.apply_composer_snapshot(completion.snapshot, _window, cx);
                        if let Some(value) = submitted_value {
                            this.submit(value, this.enter_mode(), _window, cx);
                        } else {
                            this.composer_focus.focus(_window, cx);
                        }
                    } else {
                        let value = value.trim().to_owned();
                        if !value.is_empty() || this.has_composer_images() {
                            this.submit(value, this.enter_mode(), _window, cx);
                        }
                    }
                }
                InputEvent::PressEnter { .. } | InputEvent::Focus => {}
            },
        );
        let search_subscription =
            cx.subscribe_in(&search, window, |this, state, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = state.read(cx).value().trim().to_owned();
                    this.send(RuntimeCommand::LoadSessions(query));
                }
            });
        let session_title_subscription = cx.subscribe_in(
            &session_title_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.commit_session_title_edit(window, cx);
                }
                InputEvent::Change | InputEvent::Focus => {}
            },
        );
        let window_activation_subscription =
            cx.observe_window_activation(window, |this, window, cx| {
                let visible = session_shortcuts_visible_for_window(
                    this.session_shortcuts_visible,
                    window.is_window_active(),
                );
                if this.session_shortcuts_visible != visible {
                    this.session_shortcuts_visible = visible;
                    this.notify_session_rail(cx);
                }
            });
        let window_placement_subscription = crate::launch::observe_window_placement(window, cx);
        let runtime_wake = runtime.wake_receiver();
        let event_task = cx.spawn(async move |weak, cx| {
            while runtime_wake.recv().await.is_ok() {
                if weak.update(cx, |this, cx| this.drain_runtime(cx)).is_err() {
                    break;
                }
            }
        });
        let transcript_list = transcript_list_state();
        transcript_list.set_follow_mode(FollowMode::Tail);
        let debug = std::env::var("DEBUG").ok().as_deref() == Some("true");
        let fps_monitor = debug.then(|| {
            cx.new(|cx| {
                FpsMonitor::new(window, cx)
                    .continuous(false)
                    .show_resources(false)
            })
        });
        let performance_monitor = debug.then(|| {
            crate::performance::PerformanceMonitor::new(window.window_handle().window_id())
        });
        let performance_task =
            debug.then(|| {
                cx.spawn(async move |weak, cx| {
                    loop {
                        cx.background_executor()
                            .timer(crate::performance::sample_interval())
                            .await;
                        if weak
                            .update(cx, |this, cx| {
                                if this.performance_monitor.as_mut().is_some_and(
                                    crate::performance::PerformanceMonitor::sample_if_due,
                                ) {
                                    this.notify_run_panel(cx);
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
            });
        let app = cx.entity().downgrade();
        let session_rail_view = cx.new(|_| SessionRailView::new(app.clone()));
        let review_session_rail_view =
            cx.new(|_| InactiveSessionRailView::new(app.clone(), SessionRailKind::Review));
        let archived_session_rail_view =
            cx.new(|_| InactiveSessionRailView::new(app.clone(), SessionRailKind::Archived));
        let transcript_view = cx.new(|_| TranscriptView::new(app.clone()));
        let composer_view = cx.new(|_| ComposerView::new(app.clone()));
        let run_panel_view = cx.new(|_| RunPanelView::new(app.clone()));
        let workgraph_view =
            cx.new(|cx| WorkGraphBoardView::new(crate::state::state_path(), project.clone(), cx));
        let workgraph_detail_view =
            cx.new(|cx| WorkGraphDetailView::new(app.clone(), workgraph_view.clone(), cx));
        let workgraph_sidebar_view = cx.new(|cx| {
            WorkGraphSidebarView::new(app.clone(), crate::state::state_path(), project.clone(), cx)
        });
        transcript_list.set_scroll_handler(move |event, _, cx| {
            let following = event.is_following_tail;
            let needs_update = app.upgrade().is_some_and(|app| {
                let app = app.read(cx);
                transcript_follow_state_needs_update(
                    app.transcript_following,
                    app.transcript_unseen,
                    following,
                )
            });
            if !needs_update {
                return;
            }
            let app = app.clone();
            cx.defer(move |cx| {
                let _ = app.update(cx, |this, cx| {
                    if update_transcript_follow_state(
                        &mut this.transcript_following,
                        &mut this.transcript_unseen,
                        following,
                    ) {
                        this.notify_transcript(cx);
                    }
                });
            });
        });
        Self {
            project: project.clone(),
            runtime,
            snapshot: Arc::new(RuntimeSnapshot {
                status: "Starting".into(),
                project: project.clone(),
                ..RuntimeSnapshot::default()
            }),
            sessions: Vec::new(),
            all_sessions: Vec::new(),
            agent_activities: HashMap::new(),
            agent_row_focus: HashMap::new(),
            changes: changes::ChangesState::new(cx),
            session_order,
            session_drop_target: None,
            run_statuses: HashMap::new(),
            recent_completions: HashMap::new(),
            recent_completion_expiries: HashMap::new(),
            system_notification_target: None,
            projects: registry.projects,
            drafts: registry.drafts,
            draft_session_ids,
            selected_draft: Some(selected_draft),
            submitted_drafts,
            sessions_error: project_registry_error,
            session_project_filter: None,
            picker: None,
            pending_extension_picker: None,
            picker_return_focus: None,
            session_list: ListState::new(
                0,
                ListAlignment::Top,
                crate::theme::THEME.layout.transcript_overdraw,
            ),
            session_list_rows: RefCell::new(Vec::new()),
            review_session_list: ListState::new(
                0,
                ListAlignment::Top,
                crate::theme::THEME.layout.transcript_overdraw,
            ),
            review_session_list_rows: RefCell::new(Vec::new()),
            archived_session_list: ListState::new(
                0,
                ListAlignment::Top,
                crate::theme::THEME.layout.transcript_overdraw,
            ),
            archived_session_list_rows: RefCell::new(Vec::new()),
            session_generation: 0,
            runtime_generation: 0,
            composer,
            composer_project_files: Vec::new(),
            composer_project_files_project: None,
            composer_project_files_loading: None,
            composer_suggestion_selection: 0,
            session_rail_view,
            review_session_rail_view,
            archived_session_rail_view,
            transcript_view,
            composer_view,
            run_panel_view,
            workgraph_view,
            workgraph_detail_view,
            workgraph_sidebar_view,
            surface: AppSurface::Chat,
            workgraph_inspector_issue: None,
            run_panel_scroll: ScrollHandle::new(),
            composer_footer_scroll: ScrollHandle::new(),
            composer_sessions,
            composer_history_marker: None,
            composer_images: HashMap::new(),
            search,
            search_focus,
            session_title_input,
            editing_session_title: None,
            pending_session_title_focus: false,
            dialog_input,
            dialog_secret_input,
            composer_focus,
            dialog_focus,
            dialog_return_focus: None,
            sheet_focus: cx.focus_handle(),
            sheet_return_focus: None,
            pending_sheet_setup: false,
            transcript_list,
            transcript_rows: Arc::new(crate::persistent_vec::PersistentVec::default()),
            transcript_following: true,
            transcript_unseen: 0,
            transcript_disclosure_states: HashMap::new(),
            last_transcript_count: 0,
            fps_monitor,
            performance_monitor,
            _performance_task: performance_task,
            pending_session_switch: None,
            extension: ExtensionUiState::default(),
            parked_extension: None,
            restored_dialog_id: None,
            dismissed_restored_dialog_id: None,
            notification_expiries: HashMap::new(),
            pending_dialog_setup: false,
            pending_title: None,
            pending_editor_text: None,
            pending_submissions: HashMap::new(),
            pending_session_reset: false,
            sessions_sheet: false,
            review_sessions_expanded: false,
            pending_archive: None,
            archived_sessions_expanded: false,
            run_sheet: false,
            keybindings_help: false,
            project_trust_sheet: false,
            project_trust_error: None,
            project_trust_project: None,
            pending_project_trust_command: None,
            completed_agents_expanded: false,
            limited_agents_expanded: false,
            session_shortcuts_visible: false,
            _composer_subscription: composer_subscription,
            _search_subscription: search_subscription,
            _session_title_subscription: session_title_subscription,
            _window_activation_subscription: window_activation_subscription,
            _window_placement_subscription: window_placement_subscription,
            _event_task: event_task,
        }
    }

    fn drain_runtime(&mut self, cx: &mut Context<Self>) {
        let mut operation = crate::performance::OperationTiming::new(
            crate::performance::OperationKind::RuntimeDrain,
            0,
        );
        let _timing = crate::performance::Timing::new("runtime.drain_events");
        let mut root_dirty = false;
        let performance_changed = self
            .performance_monitor
            .as_mut()
            .is_some_and(crate::performance::PerformanceMonitor::sample_if_due);
        let mut rail_dirty = false;
        let mut review_rail_dirty = false;
        let mut archived_rail_dirty = false;
        let mut transcript_dirty = false;
        let mut composer_dirty = false;
        let mut run_dirty = performance_changed;
        let mut workgraph_session_dirty = false;
        let mut workgraph_data_dirty = false;
        while let Ok(event) = self.runtime.try_recv() {
            operation.increment_work();
            match &event {
                RuntimeEvent::Snapshot { snapshot, .. } => {
                    let roots = SessionRootIndex::new(&self.sessions);
                    rail_dirty |= session_rail_snapshot_changed(&roots, &self.snapshot, snapshot);
                    review_rail_dirty |= inactive_session_rail_snapshot_changed(
                        SessionRailKind::Review,
                        &roots,
                        &self.snapshot,
                        snapshot,
                    );
                    archived_rail_dirty |= inactive_session_rail_snapshot_changed(
                        SessionRailKind::Archived,
                        &roots,
                        &self.snapshot,
                        snapshot,
                    );
                    composer_dirty |= composer_snapshot_changed(&self.snapshot, snapshot);
                    root_dirty |= self.snapshot.pending_question != snapshot.pending_question;
                    run_dirty |= run_panel_snapshot_changed(&self.snapshot, snapshot);
                    workgraph_session_dirty |=
                        self.snapshot.selected_session != snapshot.selected_session;
                }
                RuntimeEvent::Sessions { .. }
                | RuntimeEvent::SessionsFailed { .. }
                | RuntimeEvent::SessionFilesModified { .. } => {}
                RuntimeEvent::SessionMoved { .. } => {
                    root_dirty = true;
                    rail_dirty = true;
                    review_rail_dirty = true;
                    archived_rail_dirty = true;
                    transcript_dirty = true;
                    composer_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::SessionStatus {
                    target, session, ..
                } => {
                    rail_dirty |= session_event_affects_active_rail(
                        &self.drafts,
                        &self.submitted_drafts,
                        &self.sessions,
                        &target,
                        session.as_deref(),
                    );
                    archived_rail_dirty |= archive::session_event_affects_archived_rail(
                        &self.sessions,
                        target,
                        session.as_deref(),
                    );
                }
                RuntimeEvent::HistoryReset { .. } => transcript_dirty = true,
                RuntimeEvent::SessionReset { .. } => {
                    root_dirty = true;
                    transcript_dirty = true;
                    composer_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::ExtensionUi { .. } => {}
                RuntimeEvent::PromptResult {
                    target, session, ..
                } => {
                    root_dirty = true;
                    rail_dirty |= session_event_affects_active_rail(
                        &self.drafts,
                        &self.submitted_drafts,
                        &self.sessions,
                        target,
                        session.as_deref(),
                    );
                    archived_rail_dirty |= archive::session_event_affects_archived_rail(
                        &self.sessions,
                        target,
                        session.as_deref(),
                    );
                    composer_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::WorkGraphChanged { project, .. } => {
                    workgraph_data_dirty |= project == &self.project;
                }
                RuntimeEvent::RefreshCatalog | RuntimeEvent::Stopped => run_dirty = true,
            }
            match event {
                RuntimeEvent::Snapshot {
                    generation,
                    snapshot,
                } if generation >= self.runtime_generation => {
                    if self
                        .pending_session_switch
                        .as_ref()
                        .is_some_and(|(path, _)| {
                            snapshot.selected_session.as_deref() == Some(path.as_path())
                        })
                    {
                        drop(self.pending_session_switch.take());
                    }
                    let session_changed = generation > self.runtime_generation;
                    let transcript_preselected = session_changed
                        && self.snapshot.selected_session == snapshot.selected_session;
                    if session_changed {
                        self.reset_session_ui(generation, transcript_preselected);
                        root_dirty = true;
                    }
                    let row_update = if transcript_preselected {
                        self.project_transcript_rows(&snapshot)
                    } else if session_changed {
                        let _timing = crate::performance::OperationTiming::new(
                            crate::performance::OperationKind::FullProjection,
                            snapshot.conversation.items.len(),
                        );
                        crate::transcript::TranscriptRowUpdate::replace(
                            crate::transcript::project_rows(&snapshot.conversation.items),
                        )
                    } else {
                        self.project_transcript_rows(&snapshot)
                    };
                    let count = row_update.row_count(self.transcript_rows.len());
                    if count > self.last_transcript_count && !self.transcript_following {
                        self.transcript_unseen = self
                            .transcript_unseen
                            .saturating_add(count - self.last_transcript_count);
                    }
                    if snapshot.history_preview && !self.snapshot.history_preview {
                        root_dirty = true;
                        park_extension_surface(&mut self.extension, &mut self.parked_extension);
                        self.pending_dialog_setup = false;
                        self.dialog_return_focus = None;
                    } else if !snapshot.history_preview && self.snapshot.history_preview {
                        root_dirty = true;
                        self.clear_restored_dialog();
                        restore_extension_surface(&mut self.extension, &mut self.parked_extension);
                        self.pending_dialog_setup = self.extension.dialog.is_some();
                        self.dialog_return_focus = None;
                    }
                    self.snapshot = snapshot;
                    transcript_dirty |= self.apply_transcript_rows(row_update);
                    self.last_transcript_count = count;
                    self.sync_restored_dialog();
                    self.sync_composer_history();
                    rail_dirty |= self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::SessionReset {
                    generation,
                    preserve_submission,
                } if generation >= self.runtime_generation => {
                    self.reset_session_ui(generation, preserve_submission);
                }
                RuntimeEvent::HistoryReset { generation }
                    if generation == self.runtime_generation =>
                {
                    self.reset_transcript_ui();
                }
                RuntimeEvent::Sessions {
                    generation,
                    sessions,
                    all_sessions,
                    activities,
                    ..
                } if generation >= self.session_generation => {
                    self.session_generation = generation;
                    let catalog_changed = session_catalog_changed(
                        &self.sessions,
                        &self.all_sessions,
                        self.sessions_error.as_deref(),
                        &sessions,
                        &all_sessions,
                    );
                    let review_catalog_changed = inactive_session_catalog_changed(
                        SessionRailKind::Review,
                        &self.sessions,
                        &self.all_sessions,
                        &sessions,
                        &all_sessions,
                    );
                    let archived_catalog_changed = inactive_session_catalog_changed(
                        SessionRailKind::Archived,
                        &self.sessions,
                        &self.all_sessions,
                        &sessions,
                        &all_sessions,
                    );
                    let run_catalog_changed = run_panel_sessions_changed(
                        &self.all_sessions,
                        &all_sessions,
                        self.snapshot.selected_session.as_deref(),
                    );
                    let composer_usage_changed = composer_usage_sessions_changed(
                        &self.all_sessions,
                        &all_sessions,
                        self.snapshot.selected_session.as_deref(),
                    );
                    let previous_workgraph_session = self.active_workgraph_session();
                    let visible_activities_changed = run_panel_activities_changed(
                        &self.agent_activities,
                        activities.as_ref(),
                        &self.all_sessions,
                        self.snapshot.selected_session.as_deref(),
                    );
                    for session in &all_sessions {
                        projects::add_unique(&mut self.projects, session.project.clone());
                    }
                    self.sessions_error = None;
                    self.sessions = sessions;
                    self.all_sessions = all_sessions;
                    if let Some((activities, exhaustive)) = activities {
                        if exhaustive {
                            self.agent_activities = activities;
                        } else {
                            self.agent_activities.extend(activities);
                        }
                    }
                    self.agent_row_focus
                        .retain(|id, _| self.agent_activities.contains_key(id));
                    for id in self.agent_activities.keys() {
                        self.agent_row_focus
                            .entry(id.clone())
                            .or_insert_with(|| cx.focus_handle());
                    }
                    rail_dirty |= catalog_changed;
                    review_rail_dirty |= review_catalog_changed;
                    archived_rail_dirty |= archived_catalog_changed;
                    composer_dirty |= composer_usage_changed;
                    run_dirty |= run_catalog_changed || visible_activities_changed;
                    workgraph_session_dirty |=
                        previous_workgraph_session != self.active_workgraph_session();
                    rail_dirty |= self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::SessionMoved {
                    target_root,
                    target_project,
                    paths,
                } => {
                    for (source, target) in paths.iter() {
                        let source_target = session_target(source);
                        let target_target = session_target(target);
                        self.composer_sessions
                            .promote(&source_target, target_target.clone());
                        if let Some(images) = self.composer_images.remove(&source_target) {
                            self.composer_images.insert(target_target.clone(), images);
                        }
                        if let Some(status) = self.run_statuses.remove(&source_target) {
                            self.run_statuses.insert(target_target.clone(), status);
                        }
                        if let Some(completion) = self.recent_completions.remove(&source_target) {
                            self.recent_completions
                                .insert(target_target.clone(), completion);
                        }
                        if let Some(expiry) = self.recent_completion_expiries.remove(&source_target)
                        {
                            self.recent_completion_expiries
                                .insert(target_target.clone(), expiry);
                        }
                        for draft in &mut self.drafts {
                            if draft.session_path.as_deref() == Some(source.as_path()) {
                                draft.session_path = Some(target.clone());
                                draft.project = target_project.clone();
                            }
                        }
                        for session_path in self.submitted_drafts.values_mut().flatten() {
                            if session_path == source {
                                *session_path = target.clone();
                            }
                        }
                    }
                    if let Some((session, project)) = self.system_notification_target.as_mut()
                        && let Some(target) = paths.get(session)
                    {
                        *session = target.clone();
                        *project = target_project.clone();
                    }
                    let selected_was_moved = self
                        .snapshot
                        .selected_session
                        .as_ref()
                        .or(self.snapshot.live_session.as_ref())
                        .is_some_and(|path| paths.contains_key(path));
                    if selected_was_moved {
                        self.select_project(target_project.clone());
                        self.send(RuntimeCommand::SelectSession {
                            path: target_root,
                            project: target_project,
                        });
                    }
                    self.save_project_registry();
                }
                RuntimeEvent::SessionsFailed {
                    generation,
                    message,
                } if generation >= self.session_generation => {
                    self.session_generation = generation;
                    let changed = self.sessions_error.as_deref() != Some(message.as_str());
                    self.sessions_error = Some(message);
                    rail_dirty |= changed;
                    run_dirty |= changed;
                }
                RuntimeEvent::ExtensionUi {
                    generation,
                    request,
                    system_notification_target,
                } if generation == self.runtime_generation => {
                    if let Some((title, body)) = request.gpui_system_notification() {
                        self.system_notification_target = system_notification_target;
                        cx.show_system_notification(SystemNotification {
                            tag: SYSTEM_NOTIFICATION_TAG.into(),
                            title: title.into(),
                            body: body.into(),
                            actions: Vec::new(),
                        });
                    } else if let Some(extension) = self.parked_extension.as_mut() {
                        let _ = extension.apply(request);
                    } else {
                        self.apply_extension_request(request, generation, cx);
                        root_dirty = true;
                        composer_dirty = true;
                    }
                }
                RuntimeEvent::PromptResult {
                    generation,
                    target,
                    accepted,
                    session,
                } if generation == self.runtime_generation => {
                    self.record_draft_submission(&target, accepted, session.clone());
                    if !accepted {
                        self.run_statuses.insert(target.clone(), "Failed".into());
                    }
                    if let Some(pending) = self.pending_submissions.get_mut(&target) {
                        pending.result = Some((accepted, session));
                    }
                    rail_dirty |= self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::SessionStatus {
                    target,
                    session,
                    status,
                } => {
                    self.record_session_status(target, session, status);
                    rail_dirty |= self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::Stopped => {
                    Arc::make_mut(&mut self.snapshot).status = "Stopped".into()
                }
                RuntimeEvent::WorkGraphChanged { project, .. } => {
                    workgraph_data_dirty |= project == self.project;
                }
                RuntimeEvent::Snapshot { .. }
                | RuntimeEvent::RefreshCatalog
                | RuntimeEvent::SessionReset { .. }
                | RuntimeEvent::HistoryReset { .. }
                | RuntimeEvent::ExtensionUi { .. }
                | RuntimeEvent::PromptResult { .. }
                | RuntimeEvent::Sessions { .. }
                | RuntimeEvent::SessionsFailed { .. }
                | RuntimeEvent::SessionFilesModified { .. } => {}
            }
        }
        if workgraph_data_dirty {
            self.refresh_workgraph_data(cx);
        } else if workgraph_session_dirty {
            self.refresh_workgraph_sidebar(cx);
        }
        self.sync_notification_expiries(cx);
        self.sync_recent_completion_expiries(cx);
        if rail_dirty {
            self.notify_session_rail_shell(cx);
        }
        if review_rail_dirty {
            self.notify_review_session_rail(cx);
        }
        if archived_rail_dirty {
            self.notify_archived_session_rail(cx);
        }
        if transcript_dirty {
            self.notify_transcript(cx);
        }
        if composer_dirty {
            self.notify_composer(cx);
        }
        if run_dirty {
            self.notify_run_panel(cx);
            self.request_changes_refresh(cx);
        }
        if root_dirty {
            cx.notify();
        }
    }

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
        self.pending_extension_picker = None;
        self.restored_dialog_id = None;
        self.dismissed_restored_dialog_id = None;
        self.notification_expiries.clear();
        self.pending_dialog_setup = false;
        self.pending_title = Some((generation, "Pi".into()));
        self.pending_editor_text = None;
        self.pending_session_reset = true;
        self.dialog_return_focus = None;
        self.sessions_sheet = false;
        self.run_sheet = false;
        self.sheet_return_focus = None;
        self.pending_sheet_setup = false;
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
        cx: &mut Context<Self>,
    ) {
        if let Some(scope) = picker::provider_login_scope(&request) {
            self.pending_extension_picker = Some(scope);
            return;
        }
        match self.extension.apply(request) {
            ExtensionEffect::DialogOpened => self.pending_dialog_setup = true,
            ExtensionEffect::SetTitle(title) => self.pending_title = Some((generation, title)),
            ExtensionEffect::SetEditorText(text) => {
                self.pending_editor_text = Some((generation, text))
            }
            ExtensionEffect::OpenUrl(url) => cx.open_url(&url),
            ExtensionEffect::PersistError(_) | ExtensionEffect::None => {}
            ExtensionEffect::Diagnostic(message) => {
                Arc::make_mut(&mut Arc::make_mut(&mut self.snapshot).conversation)
                    .diagnostics
                    .push(message)
            }
        }
    }

    fn reset_transcript_ui(&mut self) {
        self.transcript_list.reset(0);
        self.transcript_rows = Arc::new(crate::persistent_vec::PersistentVec::default());
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        self.transcript_disclosure_states.clear();
        self.transcript_following = true;
        self.transcript_unseen = 0;
        self.last_transcript_count = 0;
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
        let _timing = crate::performance::Timing::new("switch.session_request");
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
        self.select_project(project.clone());
        if let Some((_, timing)) = self.pending_session_switch.take() {
            timing.cancel();
        }
        self.pending_session_switch = Some((
            path.clone(),
            crate::performance::Timing::new("switch.session_total"),
        ));
        self.send_project_command(
            &project,
            RuntimeCommand::SelectSession {
                path,
                project: project.clone(),
            },
            window,
            cx,
        );
        self.close_sessions_sheet_after_selection(window, cx);
        if previous_root != next_root {
            self.run_panel_scroll.set_offset(point(px(0.0), px(0.0)));
            self.notify_session_rail(cx);
        }
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    fn new_session(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        self.run_panel_scroll.set_offset(point(px(0.0), px(0.0)));
        let draft = match projects::new_draft(project.clone()) {
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
                project: project.clone(),
            },
            window,
            cx,
        );
        self.select_project(project);
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
        self.run_panel_scroll.set_offset(point(px(0.0), px(0.0)));
        self.switch_composer_target(draft_target(&id), window, cx);
        self.selected_draft = Some(id.clone());
        self.select_project(project.clone());
        let command = if let Some(Some(path)) = self.submitted_drafts.get(&id).cloned() {
            RuntimeCommand::SelectSession {
                path,
                project: project.clone(),
            }
        } else {
            RuntimeCommand::ResumeDraft {
                id,
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
        if projects::add_unique(&mut self.projects, project.clone()) {
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
        if !projects::remove(&mut self.projects, project) {
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

    fn select_project(&mut self, project: PathBuf) {
        if self.project != project {
            self.composer_project_files.clear();
            self.composer_project_files_project = None;
            self.composer_project_files_loading = None;
        }
        self.project = project.clone();
        if projects::select(&mut self.projects, project) {
            self.save_project_registry();
        }
    }

    fn request_composer_project_files(&mut self, cx: &mut Context<Self>) {
        let project = self.project.clone();
        if self.composer_project_files_project.as_ref() == Some(&project)
            || self.composer_project_files_loading.as_ref() == Some(&project)
        {
            return;
        }
        self.composer_project_files_loading = Some(project.clone());
        let task = cx.background_spawn(async move {
            let files = file_mentions::project_files(&project);
            (project, files)
        });
        cx.spawn(async move |weak, cx| {
            let (project, files) = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.composer_project_files_loading.as_ref() == Some(&project) {
                    this.composer_project_files_loading = None;
                }
                if this.project == project {
                    this.composer_project_files = files;
                    this.composer_project_files_project = Some(project);
                    this.notify_composer(cx);
                }
            });
        })
        .detach();
    }

    fn save_project_registry(&mut self) {
        if let Err(error) = projects::save(&projects::Registry {
            projects: self.projects.clone(),
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
        self.drafts.retain(|draft| draft.id != id);
        self.draft_session_ids.remove(id);
        self.submitted_drafts.remove(id);
        self.run_statuses.remove(&target);
        self.recent_completions.remove(&target);
        self.recent_completion_expiries.remove(&target);
        if was_selected {
            self.selected_draft = None;
            if let Some(session) = self.sessions.first().cloned() {
                self.select_project(session.project.clone());
                let snapshot = self
                    .composer_sessions
                    .discard_and_switch(&target, session_target(&session.path));
                self.apply_composer_snapshot(snapshot, window, cx);
                self.send_project_command(
                    &session.project,
                    RuntimeCommand::SelectSession {
                        path: session.path,
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
        self.set_session_category(path, false, false, cx);
    }

    fn set_session_review(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.set_session_category(path, true, false, cx);
    }

    fn set_session_archived(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.set_session_category(path, false, true, cx);
    }

    fn set_session_category(
        &mut self,
        path: PathBuf,
        in_review: bool,
        archived: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.path == path)
        {
            session.in_review = in_review;
            session.archived = archived;
        }
        if !self.sessions.iter().any(|session| session.in_review) {
            self.review_sessions_expanded = false;
        }
        if !self.sessions.iter().any(|session| session.archived) {
            self.archived_sessions_expanded = false;
        }
        self.send(RuntimeCommand::SetSessionCategory {
            path,
            in_review,
            archived,
        });
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
            self.composer_sessions
                .discard_and_switch(&current_target, target)
        } else {
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

    fn select_provider(&mut self, provider: &str, cx: &mut Context<Self>) {
        if let Some(model) = self
            .snapshot
            .models
            .iter()
            .find(|model| model.provider == provider)
            .cloned()
        {
            self.select_model(&model, cx);
        }
    }

    fn select_model(&mut self, model: &Model, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetModel {
            provider: model.provider.clone(),
            model_id: model.id.clone(),
        });
        cx.notify();
    }

    fn add_provider(&mut self) {
        self.send(RuntimeCommand::Login(None));
    }

    fn set_thinking_level(&mut self, level: String, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetThinking(level));
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

fn session_has_rail_kind(session: &SessionSummary, kind: SessionRailKind) -> bool {
    session.parent_session.is_none()
        && match kind {
            SessionRailKind::Project => !session.in_review && !session.archived,
            SessionRailKind::Review => session.in_review && !session.archived,
            SessionRailKind::Archived => session.archived,
        }
}

fn inactive_session_catalog_changed(
    kind: SessionRailKind,
    current: &[SessionSummary],
    current_all: &[SessionSummary],
    next: &[SessionSummary],
    next_all: &[SessionSummary],
) -> bool {
    let rows = |sessions: &[SessionSummary]| {
        sessions
            .iter()
            .filter(|session| session_has_rail_kind(session, kind))
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
    if kind != SessionRailKind::Archived {
        return false;
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
        .is_some_and(|root| !root.in_review && !root.archived)
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
    kind: SessionRailKind,
    roots: &SessionRootIndex<'_>,
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    let root_id = |path| {
        roots
            .root_for_path(path)
            .filter(|session| session_has_rail_kind(session, kind))
            .map(|session| session.id.as_str())
    };
    if root_id(previous.selected_session.as_deref()) != root_id(next.selected_session.as_deref()) {
        return true;
    }
    if kind != SessionRailKind::Archived {
        return false;
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
#[path = "app_tests.rs"]
mod tests;
