//! Top-level GPUI composition for the active root session.

mod changes;
mod composer_images;
mod drafts;
mod file_mentions;
mod region_state;
mod session_titles;
mod slash_commands;
mod submissions;
mod surfaces;
mod transcript_ui;
mod views;
pub(crate) use composer_images::ComposerImage;
use submissions::PendingSubmission;
pub(crate) use views::OVERLAY_KEY_CONTEXT;
use views::{ComposerView, RunPanelView, SessionRailView, TranscriptView, session_move_allowed};

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, FollowMode, ListAlignment,
    ListState, PathPromptOptions, ScrollHandle, Subscription, Task, Window, actions, point, px,
};
use gpui_component::input::{InputEvent, InputState, TextareaState};
use gpui_fps::FpsMonitor;

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
    sessions::{SessionSummary, root_session_for_path},
};

const MAX_EXTENSION_ERRORS: usize = 16;
const RECENT_COMPLETION_LIFETIME: Duration = Duration::from_secs(10 * 60);
pub(crate) const COMPOSER_KEY_CONTEXT: &str = "PiComposer";
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
        FocusSessionSearch,
        FocusComposer,
        PreviousSession,
        NextSession,
        ToggleArchivedSessions,
        SubmitPrompt,
        AbortRun,
        ShowKeybindings
    ]
);

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
    session_order: Vec<String>,
    run_statuses: HashMap<String, String>,
    recent_completions: HashMap<String, Instant>,
    projects: Vec<PathBuf>,
    drafts: Vec<projects::DraftSession>,
    selected_draft: Option<String>,
    submitted_drafts: HashMap<String, Option<PathBuf>>,
    sessions_error: Option<String>,
    session_project_filter: Option<PathBuf>,
    collapsed_projects: HashSet<PathBuf>,
    session_list: ListState,
    session_list_rows: RefCell<Vec<String>>,
    archived_session_list: ListState,
    archived_session_list_rows: RefCell<Vec<String>>,
    session_generation: u64,
    runtime_generation: u64,
    composer: Entity<TextareaState>,
    composer_project_files: Vec<String>,
    composer_mention_selection: usize,
    session_rail_view: Entity<SessionRailView>,
    transcript_view: Entity<TranscriptView>,
    composer_view: Entity<ComposerView>,
    run_panel_view: Entity<RunPanelView>,
    run_panel_scroll: ScrollHandle,
    composer_sessions: ComposerSessions,
    composer_history_marker: Option<(String, usize, String)>,
    composer_images: HashMap<String, Vec<ComposerImage>>,
    search: Entity<InputState>,
    search_focus: FocusHandle,
    session_title_input: Entity<InputState>,
    editing_session_title: Option<SessionTitleEdit>,
    pending_session_title_focus: bool,
    dialog_input: Entity<TextareaState>,
    composer_focus: FocusHandle,
    dialog_focus: FocusHandle,
    dialog_return_focus: Option<FocusHandle>,
    sheet_focus: FocusHandle,
    sheet_return_focus: Option<FocusHandle>,
    pending_sheet_setup: bool,
    transcript_list: ListState,
    transcript_rows: Arc<Vec<crate::transcript::TranscriptRow>>,
    transcript_following: bool,
    transcript_unseen: usize,
    pub(crate) transcript_disclosure_states: HashMap<usize, bool>,
    last_transcript_count: usize,
    fps_monitor: Option<Entity<FpsMonitor>>,
    extension: ExtensionUiState,
    parked_extension: Option<ExtensionUiState>,
    pending_dialog_setup: bool,
    pending_title: Option<(u64, String)>,
    pending_editor_text: Option<(u64, String)>,
    pending_submission: Option<PendingSubmission>,
    pending_submission_result: Option<(String, bool, Option<PathBuf>)>,
    pending_session_reset: bool,
    extension_errors: Vec<String>,
    sessions_sheet: bool,
    archived_sessions_expanded: bool,
    run_sheet: bool,
    keybindings_help: bool,
    context_details_expanded: bool,
    completed_agents_expanded: bool,
    limited_agents_expanded: bool,
    session_shortcuts_visible: bool,
    _composer_subscription: Subscription,
    _search_subscription: Subscription,
    _session_title_subscription: Subscription,
    _event_task: Task<()>,
}

impl PiApp {
    pub(crate) fn new(project: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (mut registry, mut project_registry_error) = match projects::load() {
            Ok(registry) => (registry, None),
            Err(error) => (projects::Registry::default(), Some(error)),
        };
        projects::select(&mut registry.projects, project.clone());
        let session_order = match projects::load_session_order() {
            Ok(order) => order,
            Err(error) => {
                if project_registry_error.is_none() {
                    project_registry_error = Some(error);
                }
                Vec::new()
            }
        };
        let selected_draft = projects::DraftSession::new(project.clone()).id;
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
                .auto_grow(2, 8)
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
        let composer_focus = composer.read(cx).focus_handle(cx);
        let dialog_focus = cx.focus_handle();
        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, state, event: &InputEvent, _window, cx| match event {
                InputEvent::Change => {
                    this.composer_mention_selection = 0;
                    this.composer_sessions.exit_history();
                    this.composer_sessions
                        .capture_current(input_snapshot(state.read(cx)));
                    this.notify_composer(cx);
                }
                InputEvent::Blur => {
                    this.composer_sessions
                        .capture_current(input_snapshot(state.read(cx)));
                }
                InputEvent::PressEnter { shift: false, .. } => {
                    let value = state.read(cx).value().trim().to_owned();
                    if !value.is_empty() || this.has_composer_images() {
                        this.submit(value, this.enter_mode(), _window, cx);
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
        let event_task = cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if weak.update(cx, |this, cx| this.drain_runtime(cx)).is_err() {
                    break;
                }
            }
        });
        let transcript_list = ListState::new(
            0,
            ListAlignment::Top,
            crate::theme::THEME.layout.transcript_overdraw,
        );
        transcript_list.set_follow_mode(FollowMode::Tail);
        let fps_monitor = (std::env::var("DEBUG").ok().as_deref() == Some("true")).then(|| {
            cx.new(|cx| {
                FpsMonitor::new(window, cx)
                    .continuous(true)
                    .show_resources(false)
            })
        });
        let app = cx.entity().downgrade();
        let session_rail_view = cx.new(|_| SessionRailView::new(app.clone()));
        let transcript_view = cx.new(|_| TranscriptView::new(app.clone()));
        let composer_view = cx.new(|_| ComposerView::new(app.clone()));
        let run_panel_view = cx.new(|_| RunPanelView::new(app.clone()));
        transcript_list.set_scroll_handler(move |event, _, cx| {
            let following = event.is_following_tail;
            let app = app.clone();
            cx.defer(move |cx| {
                let _ = app.update(cx, |this, cx| {
                    this.transcript_following = following;
                    if following {
                        this.transcript_unseen = 0;
                    }
                    this.notify_transcript(cx);
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
            run_statuses: HashMap::new(),
            recent_completions: HashMap::new(),
            projects: registry.projects,
            drafts: registry.drafts,
            selected_draft: Some(selected_draft),
            submitted_drafts,
            sessions_error: project_registry_error,
            session_project_filter: None,
            collapsed_projects: HashSet::new(),
            session_list: ListState::new(
                0,
                ListAlignment::Top,
                crate::theme::THEME.layout.transcript_overdraw,
            ),
            session_list_rows: RefCell::new(Vec::new()),
            archived_session_list: ListState::new(
                0,
                ListAlignment::Top,
                crate::theme::THEME.layout.transcript_overdraw,
            ),
            archived_session_list_rows: RefCell::new(Vec::new()),
            session_generation: 0,
            runtime_generation: 0,
            composer,
            composer_project_files: file_mentions::project_files(&project),
            composer_mention_selection: 0,
            session_rail_view,
            transcript_view,
            composer_view,
            run_panel_view,
            run_panel_scroll: ScrollHandle::new(),
            composer_sessions,
            composer_history_marker: None,
            composer_images: HashMap::new(),
            search,
            search_focus,
            session_title_input,
            editing_session_title: None,
            pending_session_title_focus: false,
            dialog_input,
            composer_focus,
            dialog_focus,
            dialog_return_focus: None,
            sheet_focus: cx.focus_handle(),
            sheet_return_focus: None,
            pending_sheet_setup: false,
            transcript_list,
            transcript_rows: Arc::new(Vec::new()),
            transcript_following: true,
            transcript_unseen: 0,
            transcript_disclosure_states: HashMap::new(),
            last_transcript_count: 0,
            fps_monitor,
            extension: ExtensionUiState::default(),
            parked_extension: None,
            pending_dialog_setup: false,
            pending_title: None,
            pending_editor_text: None,
            pending_submission: None,
            pending_submission_result: None,
            pending_session_reset: false,
            extension_errors: Vec::new(),
            sessions_sheet: false,
            archived_sessions_expanded: false,
            run_sheet: false,
            keybindings_help: false,
            context_details_expanded: false,
            completed_agents_expanded: false,
            limited_agents_expanded: false,
            session_shortcuts_visible: false,
            _composer_subscription: composer_subscription,
            _search_subscription: search_subscription,
            _session_title_subscription: session_title_subscription,
            _event_task: event_task,
        }
    }

    fn drain_runtime(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.extension.prune_notifications();
        let completions_changed = self.prune_recent_completions();
        changed |= completions_changed;
        let mut rail_dirty = completions_changed;
        let mut transcript_dirty = false;
        let mut composer_dirty = false;
        let mut run_dirty = completions_changed;
        while let Ok(event) = self.runtime.try_recv() {
            changed = true;
            match &event {
                RuntimeEvent::Snapshot { snapshot, .. } => {
                    rail_dirty |=
                        session_rail_snapshot_changed(&self.sessions, &self.snapshot, snapshot);
                    transcript_dirty = true;
                    composer_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::Sessions { .. } | RuntimeEvent::SessionsFailed { .. } => {
                    rail_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::SessionStatus { .. } => {
                    rail_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::HistoryReset { .. } => transcript_dirty = true,
                RuntimeEvent::SessionReset { .. } => {
                    transcript_dirty = true;
                    composer_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::ExtensionUi { .. } => composer_dirty = true,
                RuntimeEvent::PromptResult { .. } => {
                    rail_dirty = true;
                    composer_dirty = true;
                    run_dirty = true;
                }
                RuntimeEvent::RefreshCatalog | RuntimeEvent::Stopped => run_dirty = true,
            }
            match event {
                RuntimeEvent::Snapshot {
                    generation,
                    snapshot,
                } if generation >= self.runtime_generation => {
                    if generation > self.runtime_generation {
                        self.reset_session_ui(generation, false);
                    }
                    let next_rows = self.project_transcript_rows(&snapshot);
                    let count = next_rows.len();
                    if count > self.last_transcript_count && !self.transcript_following {
                        self.transcript_unseen = self
                            .transcript_unseen
                            .saturating_add(count - self.last_transcript_count);
                    }
                    if snapshot.history_preview && !self.snapshot.history_preview {
                        park_extension_surface(&mut self.extension, &mut self.parked_extension);
                        self.pending_dialog_setup = false;
                        self.dialog_return_focus = None;
                    } else if !snapshot.history_preview && self.snapshot.history_preview {
                        restore_extension_surface(&mut self.extension, &mut self.parked_extension);
                        self.pending_dialog_setup = self.extension.dialog.is_some();
                        self.dialog_return_focus = None;
                    }
                    self.sync_transcript_rows(next_rows);
                    self.last_transcript_count = count;
                    self.snapshot = snapshot;
                    self.sync_composer_history();
                    self.reconcile_submitted_drafts(cx);
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
                    for session in &all_sessions {
                        projects::add_unique(&mut self.projects, session.project.clone());
                    }
                    self.sessions_error = None;
                    if add_new_sessions_to_order(&mut self.session_order, &all_sessions) {
                        self.save_session_order();
                    }
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
                    self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::SessionsFailed {
                    generation,
                    message,
                } if generation >= self.session_generation => {
                    self.session_generation = generation;
                    self.sessions_error = Some(message);
                }
                RuntimeEvent::ExtensionUi {
                    generation,
                    request,
                } if generation == self.runtime_generation => {
                    if let Some(extension) = self.parked_extension.as_mut() {
                        let _ = extension.apply(request);
                    } else {
                        self.apply_extension_request(request, generation);
                    }
                }
                RuntimeEvent::PromptResult {
                    generation,
                    target,
                    accepted,
                    session,
                } if generation == self.runtime_generation => {
                    self.record_draft_submission(&target, accepted, session.clone());
                    if self
                        .pending_submission
                        .as_ref()
                        .is_some_and(|pending| pending.target == target)
                    {
                        self.pending_submission_result = Some((target, accepted, session));
                    }
                    self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::SessionStatus {
                    target,
                    session,
                    status,
                } => {
                    self.record_session_status(target, session, status);
                    self.reconcile_submitted_drafts(cx);
                }
                RuntimeEvent::Stopped => {
                    Arc::make_mut(&mut self.snapshot).status = "Stopped".into()
                }
                RuntimeEvent::Snapshot { .. }
                | RuntimeEvent::RefreshCatalog
                | RuntimeEvent::SessionReset { .. }
                | RuntimeEvent::HistoryReset { .. }
                | RuntimeEvent::ExtensionUi { .. }
                | RuntimeEvent::PromptResult { .. }
                | RuntimeEvent::Sessions { .. }
                | RuntimeEvent::SessionsFailed { .. } => {}
            }
        }
        if rail_dirty {
            self.notify_session_rail(cx);
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
        if changed {
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

    fn prune_recent_completions(&mut self) -> bool {
        let before = self.recent_completions.len();
        self.recent_completions
            .retain(|_, completed| completed.elapsed() < RECENT_COMPLETION_LIFETIME);
        self.run_statuses.retain(|target, status| {
            status != "Done" || self.recent_completions.contains_key(target)
        });
        self.recent_completions.len() != before
    }

    fn reset_session_ui(&mut self, generation: u64, preserve_submission: bool) {
        self.runtime_generation = generation;
        self.extension.reset();
        self.parked_extension = None;
        self.pending_dialog_setup = false;
        self.pending_title = Some((generation, "Pi".into()));
        self.pending_editor_text = None;
        self.pending_session_reset = true;
        self.dialog_return_focus = None;
        self.sessions_sheet = false;
        self.run_sheet = false;
        self.sheet_return_focus = None;
        self.pending_sheet_setup = false;
        self.extension_errors.clear();
        if !preserve_submission {
            self.reset_transcript_ui();
        }
    }

    fn apply_extension_request(&mut self, request: ExtensionUiRequest, generation: u64) {
        match self.extension.apply(request) {
            ExtensionEffect::DialogOpened => self.pending_dialog_setup = true,
            ExtensionEffect::SetTitle(title) => self.pending_title = Some((generation, title)),
            ExtensionEffect::SetEditorText(text) => {
                self.pending_editor_text = Some((generation, text))
            }
            ExtensionEffect::PersistError(message) => {
                self.extension_errors.push(message);
                if self.extension_errors.len() > MAX_EXTENSION_ERRORS {
                    self.extension_errors.remove(0);
                }
            }
            ExtensionEffect::Diagnostic(message) => Arc::make_mut(&mut self.snapshot)
                .conversation
                .diagnostics
                .push(message),
            ExtensionEffect::None => {}
        }
    }

    fn reset_transcript_ui(&mut self) {
        self.transcript_list.reset(0);
        self.transcript_rows = Arc::new(Vec::new());
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        self.transcript_disclosure_states.clear();
        self.transcript_following = true;
        self.transcript_unseen = 0;
        self.last_transcript_count = 0;
    }

    fn resume(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let next_root =
            root_session_for_path(&self.sessions, Some(&path)).map(|session| session.id.clone());
        self.switch_composer_target(session_target(&path), window, cx);
        self.selected_draft = None;
        self.select_project(project.clone());
        self.send(RuntimeCommand::Resume { path, project });
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
        self.run_panel_scroll.set_offset(point(px(0.0), px(0.0)));
        let draft = projects::DraftSession::new(project.clone());
        let draft_key = draft_target(&draft.id);
        self.switch_composer_target(draft_key, window, cx);
        self.selected_draft = Some(draft.id.clone());
        self.drafts.push(draft.clone());
        self.save_project_registry();
        self.send(RuntimeCommand::NewSession {
            id: draft.id,
            project: project.clone(),
        });
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
        if self.selected_draft.as_deref() == Some(id.as_str()) && !self.snapshot.history_preview {
            self.close_sessions_sheet_after_selection(window, cx);
            return;
        }
        self.run_panel_scroll.set_offset(point(px(0.0), px(0.0)));
        self.switch_composer_target(draft_target(&id), window, cx);
        self.selected_draft = Some(id.clone());
        self.select_project(project.clone());
        if let Some(Some(path)) = self.submitted_drafts.get(&id).cloned() {
            self.send(RuntimeCommand::Resume { path, project });
        } else {
            self.send(RuntimeCommand::ResumeDraft { id, project });
        }
        self.close_sessions_sheet_after_selection(window, cx);
        self.notify_session_rail(cx);
        self.notify_transcript(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    fn choose_project_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let _ = this.update(cx, |this, cx| this.add_project(project, cx));
        })
        .detach();
    }

    fn add_project(&mut self, project: PathBuf, cx: &mut Context<Self>) {
        let project = match project.canonicalize() {
            Ok(project) if project.is_dir() => project,
            Ok(project) => {
                self.sessions_error = Some(format!(
                    "Project path is not a folder: {}",
                    project.display()
                ));
                self.notify_session_rail(cx);
                cx.notify();
                return;
            }
            Err(error) => {
                self.sessions_error = Some(format!("Open {}: {error}", project.display()));
                self.notify_session_rail(cx);
                cx.notify();
                return;
            }
        };
        if projects::add_unique(&mut self.projects, project) {
            self.save_project_registry();
        }
        self.notify_session_rail(cx);
        cx.notify();
    }

    pub(crate) fn move_session_to(&mut self, source: &str, target: &str, cx: &mut Context<Self>) {
        if session_move_allowed(&self.sessions, source, target)
            && move_to(&mut self.session_order, source, target)
        {
            self.save_session_order();
            self.notify_session_rail(cx);
        }
    }

    fn save_session_order(&mut self) {
        if let Err(error) = projects::save_session_order(&self.session_order) {
            self.sessions_error = Some(error);
        }
    }

    fn select_project(&mut self, project: PathBuf) {
        self.composer_project_files = file_mentions::project_files(&project);
        self.project = project.clone();
        if projects::select(&mut self.projects, project) {
            self.save_project_registry();
        }
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
        let was_selected = self.selected_draft.as_deref() == Some(id);
        let target = draft_target(id);
        self.composer_images.remove(&target);
        self.drafts.retain(|draft| draft.id != id);
        self.submitted_drafts.remove(id);
        self.run_statuses.remove(&target);
        self.recent_completions.remove(&target);
        if was_selected {
            self.selected_draft = None;
            if let Some(session) = self.sessions.first().cloned() {
                self.select_project(session.project.clone());
                let snapshot = self
                    .composer_sessions
                    .discard_and_switch(&target, session_target(&session.path));
                self.apply_composer_snapshot(snapshot, window, cx);
                self.send(RuntimeCommand::Resume {
                    path: session.path,
                    project: session.project,
                });
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

    fn set_session_settled(&mut self, path: PathBuf, settled: bool, cx: &mut Context<Self>) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.path == path)
        {
            session.settled = settled;
        }
        if !self.sessions.iter().any(|session| session.settled) {
            self.archived_sessions_expanded = false;
        }
        self.send(RuntimeCommand::SetSettled { path, settled });
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

    fn set_thinking_level(&mut self, level: String, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetThinking(level));
        cx.notify();
    }
}

fn session_rail_snapshot_changed(
    sessions: &[SessionSummary],
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    let root_id = |path| root_session_for_path(sessions, path).map(|session| session.id.as_str());
    root_id(previous.selected_session.as_deref()) != root_id(next.selected_session.as_deref())
        || root_id(previous.live_session.as_deref()) != root_id(next.live_session.as_deref())
        || previous.live_status != next.live_status
}

fn input_snapshot(input: &TextareaState) -> ComposerSnapshot {
    ComposerSnapshot::new(
        input.value().to_string(),
        input.cursor(),
        input.selected_range(),
    )
}

fn transcript_splice<T: PartialEq>(current: &[T], next: &[T]) -> Option<(Range<usize>, usize)> {
    let prefix = current
        .iter()
        .zip(next)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = current[prefix..]
        .iter()
        .rev()
        .zip(next[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let old_end = current.len().saturating_sub(suffix);
    let new_count = next.len().saturating_sub(prefix + suffix);
    (prefix != old_end || new_count != 0).then_some((prefix..old_end, new_count))
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

fn add_new_sessions_to_order(order: &mut Vec<String>, sessions: &[SessionSummary]) -> bool {
    let known = order.iter().cloned().collect::<HashSet<_>>();
    let mut added = sessions
        .iter()
        .filter(|session| session.parent_session.is_none() && !known.contains(&session.id))
        .collect::<Vec<_>>();
    added.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));
    if added.is_empty() {
        return false;
    }
    order.splice(0..0, added.into_iter().map(|session| session.id.clone()));
    true
}

fn move_to(order: &mut Vec<String>, source: &str, target: &str) -> bool {
    if source == target {
        return false;
    }
    let Some(source_index) = order.iter().position(|id| id == source) else {
        return false;
    };
    let Some(target_index) = order.iter().position(|id| id == target) else {
        return false;
    };
    let source = order.remove(source_index);
    let target_index = target_index.saturating_sub(usize::from(source_index < target_index));
    let insertion_index = target_index + usize::from(source_index < target_index);
    order.insert(insertion_index, source);
    true
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
