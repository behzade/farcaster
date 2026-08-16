//! Top-level GPUI composition for the active root session.

mod composer_images;
mod drafts;
mod slash_commands;
mod submissions;
mod transcript_ui;
mod views;
pub(crate) use composer_images::ComposerImage;
use submissions::PendingSubmission;
pub(crate) use views::OVERLAY_KEY_CONTEXT;

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, FollowMode, ListAlignment,
    ListState, PathPromptOptions, Subscription, Task, Window, actions,
};
use gpui_component::input::{InputEvent, InputState, TextareaState};
use gpui_fps::FpsMonitor;

use crate::{
    composer_sessions::{
        ComposerSessions, ComposerSnapshot, HistoryNavigation, draft_target, project_target,
        session_target,
    },
    extension_ui::{ExtensionEffect, ExtensionUiState},
    projects,
    protocol::{ExtensionUiRequest, Model},
    runtime::{RuntimeCommand, RuntimeEvent, RuntimeHandle, RuntimeSnapshot},
    sessions::SessionSummary,
};

const MAX_EXTENSION_ERRORS: usize = 16;
const RECENT_COMPLETION_LIFETIME: Duration = Duration::from_secs(10 * 60);
pub(crate) const COMPOSER_KEY_CONTEXT: &str = "PiComposer";
actions!(pi_gpui, [DismissSurface, QuitApplication, SubmitFollowUp]);

pub(crate) struct PiApp {
    project: PathBuf,
    runtime: RuntimeHandle,
    pub(crate) snapshot: Arc<RuntimeSnapshot>,
    sessions: Vec<SessionSummary>,
    run_statuses: HashMap<String, String>,
    recent_completions: HashMap<String, Instant>,
    projects: Vec<PathBuf>,
    drafts: Vec<projects::DraftSession>,
    selected_draft: Option<String>,
    submitted_drafts: HashMap<String, Option<PathBuf>>,
    sessions_error: Option<String>,
    session_generation: u64,
    runtime_generation: u64,
    composer: Entity<TextareaState>,
    composer_sessions: ComposerSessions,
    composer_history_marker: Option<(String, usize, String)>,
    composer_images: HashMap<String, Vec<ComposerImage>>,
    search: Entity<InputState>,
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
    pub(crate) transcript_disclosure_overrides: HashSet<usize>,
    last_transcript_count: usize,
    fps_monitor: Option<Entity<FpsMonitor>>,
    extension: ExtensionUiState,
    parked_extension: Option<ExtensionUiState>,
    pending_dialog_setup: bool,
    pending_title: Option<(u64, String)>,
    pending_editor_text: Option<(u64, String)>,
    pending_submission: Option<PendingSubmission>,
    pending_submission_result: Option<(String, bool)>,
    pending_session_reset: bool,
    extension_errors: Vec<String>,
    sessions_sheet: bool,
    run_sheet: bool,
    _composer_subscription: Subscription,
    _search_subscription: Subscription,
    _event_task: Task<()>,
}

impl PiApp {
    pub(crate) fn new(project: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (mut registry, mut project_registry_error) = match projects::load() {
            Ok(registry) => (registry, None),
            Err(error) => (projects::Registry::default(), Some(error)),
        };
        projects::add_unique(&mut registry.projects, project.clone());
        let selected_draft = registry
            .drafts
            .iter()
            .find(|draft| draft.project == project)
            .map(|draft| draft.id.clone())
            .unwrap_or_else(|| {
                let draft = projects::DraftSession::new(project.clone());
                let id = draft.id.clone();
                registry.drafts.insert(0, draft);
                id
            });
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
        let runtime = RuntimeHandle::spawn(project.clone(), selected_draft.clone());
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
                    this.composer_sessions.exit_history();
                    this.composer_sessions
                        .capture_current(input_snapshot(state.read(cx)));
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
        let fps_monitor = debug_enabled().then(|| {
            cx.new(|cx| {
                FpsMonitor::new(window, cx)
                    .continuous(true)
                    .show_resources(false)
            })
        });
        let app = cx.entity().downgrade();
        transcript_list.set_scroll_handler(move |event, _, cx| {
            let following = event.is_following_tail;
            let app = app.clone();
            cx.defer(move |cx| {
                let _ = app.update(cx, |this, cx| {
                    this.transcript_following = following;
                    if following {
                        this.transcript_unseen = 0;
                    }
                    cx.notify();
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
            run_statuses: HashMap::new(),
            recent_completions: HashMap::new(),
            projects: registry.projects,
            drafts: registry.drafts,
            selected_draft: Some(selected_draft),
            submitted_drafts,
            sessions_error: project_registry_error,
            session_generation: 0,
            runtime_generation: 0,
            composer,
            composer_sessions,
            composer_history_marker: None,
            composer_images: HashMap::new(),
            search,
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
            transcript_disclosure_overrides: HashSet::new(),
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
            run_sheet: false,
            _composer_subscription: composer_subscription,
            _search_subscription: search_subscription,
            _event_task: event_task,
        }
    }

    fn send(&mut self, command: RuntimeCommand) {
        if let Err(error) = self.runtime.send(command) {
            let snapshot = Arc::make_mut(&mut self.snapshot);
            let index = snapshot.conversation.items.len();
            snapshot.conversation.push_transport_error(error);
            self.mark_transcript_changed(index, index == 0);
        }
    }

    fn drain_runtime(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.extension.prune_notifications();
        changed |= self.prune_recent_completions();
        while let Ok(event) = self.runtime.try_recv() {
            changed = true;
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
                    ..
                } if generation >= self.session_generation => {
                    self.session_generation = generation;
                    for session in &sessions {
                        projects::add_unique(&mut self.projects, session.project.clone());
                    }
                    self.sessions = sessions;
                    self.sessions_error = None;
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
                    self.record_draft_submission(&target, accepted, session);
                    if self
                        .pending_submission
                        .as_ref()
                        .is_some_and(|pending| pending.target == target)
                    {
                        self.pending_submission_result = Some((target, accepted));
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
        self.transcript_disclosure_overrides.clear();
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
        self.selected_draft = None;
        self.project = project.clone();
        self.switch_composer_target(session_target(&path), window, cx);
        self.send(RuntimeCommand::Resume { path, project });
        self.sessions_sheet = false;
        cx.notify();
    }

    fn new_session(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let draft = projects::DraftSession::new(project.clone());
        let draft_key = draft_target(&draft.id);
        self.selected_draft = Some(draft.id.clone());
        self.drafts.insert(0, draft);
        self.save_project_registry();
        self.send(RuntimeCommand::NewSession {
            id: self.selected_draft.clone().unwrap_or_default(),
            project: project.clone(),
        });
        self.project = project;
        self.switch_composer_target(draft_key, window, cx);
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.sessions_sheet = false;
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
            return;
        }
        self.selected_draft = Some(id.clone());
        self.project = project.clone();
        self.switch_composer_target(draft_target(&id), window, cx);
        if let Some(Some(path)) = self.submitted_drafts.get(&id).cloned() {
            self.send(RuntimeCommand::Resume { path, project });
        } else {
            self.send(RuntimeCommand::ResumeDraft { id, project });
        }
        self.sessions_sheet = false;
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
                cx.notify();
                return;
            }
            Err(error) => {
                self.sessions_error = Some(format!("Open {}: {error}", project.display()));
                cx.notify();
                return;
            }
        };
        if projects::add_unique(&mut self.projects, project) {
            self.save_project_registry();
        }
        cx.notify();
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
                self.project = session.project.clone();
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
        self.send(RuntimeCommand::SetSettled { path, settled });
        cx.notify();
    }

    fn switch_composer_target(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = input_snapshot(self.composer.read(cx));
        let snapshot = self.composer_sessions.switch_to(target, current);
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

    fn respond_value(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.dialog_input.read(cx).value().to_string();
        if let Some(response) = self.extension.respond_value(&id, value) {
            self.send(RuntimeCommand::ExtensionResponse(response));
            self.advance_or_restore_dialog(window, cx);
        }
    }
    fn respond_confirm(
        &mut self,
        id: String,
        confirmed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(response) = self.extension.respond_confirm(&id, confirmed) {
            self.send(RuntimeCommand::ExtensionResponse(response));
            self.advance_or_restore_dialog(window, cx);
        }
    }
    fn cancel_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self
            .extension
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id)
            .map(str::to_owned)
        else {
            return;
        };
        if let Some(response) = self.extension.cancel(&id) {
            self.send(RuntimeCommand::ExtensionResponse(response));
            self.advance_or_restore_dialog(window, cx);
        }
    }
    fn advance_or_restore_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.extension.dialog.is_some() {
            self.pending_dialog_setup = true;
            cx.notify();
        } else {
            self.restore_dialog_focus(window, cx);
        }
    }
    fn restore_dialog_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .dialog_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone());
        focus.focus(window, cx);
        cx.notify();
    }

    fn open_sessions_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_return_focus = window.focused(cx);
        self.sessions_sheet = true;
        self.pending_sheet_setup = true;
        cx.notify();
    }

    fn open_run_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_return_focus = window.focused(cx);
        self.run_sheet = true;
        self.pending_sheet_setup = true;
        cx.notify();
    }

    fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sessions_sheet = false;
        self.run_sheet = false;
        self.pending_sheet_setup = false;
        let focus = self
            .sheet_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone());
        focus.focus(window, cx);
        cx.notify();
    }

    fn dismiss_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.extension.dialog.is_some() {
            self.cancel_dialog(window, cx);
        } else if self.sessions_sheet || self.run_sheet {
            self.close_sheet(window, cx);
        }
    }
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

fn debug_enabled() -> bool {
    debug_value_enabled(std::env::var("DEBUG").ok().as_deref())
}

fn debug_value_enabled(value: Option<&str>) -> bool {
    value == Some("true")
}

impl Drop for PiApp {
    fn drop(&mut self) {
        let _ = self.runtime.send(RuntimeCommand::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{TranscriptItem, TranscriptKind};

    fn item(text: &str) -> TranscriptItem {
        TranscriptItem {
            kind: TranscriptKind::Assistant,
            label: "Pi".into(),
            text: text.into(),
            streaming: false,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
        }
    }

    #[test]
    fn done_is_recent_only_after_an_active_status_transition() {
        assert!(!starts_recent_completion(None, "Done", false));
        assert!(!starts_recent_completion(Some("Done"), "Done", false));
        assert!(starts_recent_completion(Some("Working"), "Done", false));
        assert!(starts_recent_completion(None, "Done", true));
        assert!(!starts_recent_completion(Some("Working"), "Failed", false));
    }

    #[test]
    fn fps_debug_flag_accepts_only_literal_true() {
        assert!(super::debug_value_enabled(Some("true")));
        assert!(!super::debug_value_enabled(Some("TRUE")));
        assert!(!super::debug_value_enabled(Some("1")));
        assert!(!super::debug_value_enabled(None));
    }

    #[test]
    fn transcript_splice_keeps_unchanged_rows_out_of_the_render_path() {
        let current = vec![item("one"), item("two"), item("three")];
        assert_eq!(transcript_splice(&current, &current), None);

        let mut updated = current.clone();
        updated[1] = item("changed");
        assert_eq!(transcript_splice(&current, &updated), Some((1..2, 1)));

        let mut appended = current.clone();
        appended.push(item("four"));
        assert_eq!(transcript_splice(&current, &appended), Some((3..3, 1)));
    }

    #[test]
    fn extension_dialog_is_parked_and_restored_with_its_session() {
        let mut visible = ExtensionUiState::default();
        visible.apply(ExtensionUiRequest::Confirm {
            id: "approval".into(),
            title: "Permission".into(),
            message: "Allow it?".into(),
            timeout: None,
        });
        let mut parked = None;

        park_extension_surface(&mut visible, &mut parked);
        assert!(visible.dialog.is_none());
        assert_eq!(
            parked
                .as_ref()
                .and_then(|session| session.dialog.as_ref())
                .and_then(ExtensionUiRequest::dialog_id),
            Some("approval")
        );
        parked
            .as_mut()
            .expect("live session surface should be parked")
            .apply(ExtensionUiRequest::Input {
                id: "follow-up".into(),
                title: "Need a note".into(),
                placeholder: None,
                timeout: None,
            });

        restore_extension_surface(&mut visible, &mut parked);
        assert!(parked.is_none());
        assert_eq!(
            visible
                .dialog
                .as_ref()
                .and_then(ExtensionUiRequest::dialog_id),
            Some("approval")
        );
        assert!(visible.respond_confirm("approval", true).is_some());
        assert_eq!(
            visible
                .dialog
                .as_ref()
                .and_then(ExtensionUiRequest::dialog_id),
            Some("follow-up")
        );
    }
}
