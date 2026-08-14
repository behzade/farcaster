//! Top-level GPUI composition for the active root session.

mod views;
pub(crate) use views::OVERLAY_KEY_CONTEXT;

use std::{collections::HashSet, path::PathBuf, time::Duration};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, ScrollHandle, Subscription,
    Task, Window, actions,
};
use gpui_component::input::{InputEvent, InputState};

use crate::{
    extension_ui::{ExtensionEffect, ExtensionUiState},
    protocol::{ExtensionUiRequest, Model, PromptMode},
    runtime::{RuntimeCommand, RuntimeEvent, RuntimeHandle, RuntimeSnapshot},
    sessions::SessionSummary,
};

const MAX_SESSION_ROWS: usize = 100;
const MAX_EXTENSION_ERRORS: usize = 16;
const INITIAL_TRANSCRIPT_ITEMS: usize = crate::transcript::DEFAULT_VISIBLE_ITEMS;

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingSubmission {
    generation: u64,
    text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubmissionResolution {
    ClearEditor,
    KeepEditor,
    Ignore,
}

actions!(pi_gpui, [DismissSurface]);

pub(crate) struct PiApp {
    project: PathBuf,
    runtime: RuntimeHandle,
    snapshot: RuntimeSnapshot,
    sessions: Vec<SessionSummary>,
    sessions_error: Option<String>,
    session_generation: u64,
    runtime_generation: u64,
    composer: Entity<InputState>,
    search: Entity<InputState>,
    dialog_input: Entity<InputState>,
    composer_focus: FocusHandle,
    dialog_focus: FocusHandle,
    dialog_return_focus: Option<FocusHandle>,
    sheet_focus: FocusHandle,
    sheet_return_focus: Option<FocusHandle>,
    pending_sheet_setup: bool,
    transcript_scroll: ScrollHandle,
    transcript_following: bool,
    transcript_unseen: usize,
    transcript_visible_items: usize,
    expanded_transcript_items: HashSet<usize>,
    last_transcript_count: usize,
    prompt_mode: PromptMode,
    extension: ExtensionUiState,
    pending_dialog_setup: bool,
    pending_title: Option<(u64, String)>,
    pending_editor_text: Option<(u64, String)>,
    pending_submission: Option<PendingSubmission>,
    pending_submission_result: Option<(u64, bool)>,
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
        let runtime = RuntimeHandle::spawn(project.clone());
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(2, 8)
                .submit_on_enter(true)
                .placeholder("Ask Pi")
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions"));
        let dialog_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(2, 12)
                .submit_on_enter(false)
        });
        let composer_focus = composer.read(cx).focus_handle(cx);
        let dialog_focus = cx.focus_handle();
        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, state, event: &InputEvent, _window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = event {
                    let value = state.read(cx).value().trim().to_owned();
                    if !value.is_empty() {
                        this.submit(value, cx);
                    }
                }
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
        Self {
            project,
            runtime,
            snapshot: RuntimeSnapshot {
                status: "Starting".into(),
                ..RuntimeSnapshot::default()
            },
            sessions: Vec::new(),
            sessions_error: None,
            session_generation: 0,
            runtime_generation: 0,
            composer,
            search,
            dialog_input,
            composer_focus,
            dialog_focus,
            dialog_return_focus: None,
            sheet_focus: cx.focus_handle(),
            sheet_return_focus: None,
            pending_sheet_setup: false,
            transcript_scroll: ScrollHandle::new(),
            transcript_following: true,
            transcript_unseen: 0,
            transcript_visible_items: INITIAL_TRANSCRIPT_ITEMS,
            expanded_transcript_items: HashSet::new(),
            last_transcript_count: 0,
            prompt_mode: PromptMode::Normal,
            extension: ExtensionUiState::default(),
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
            self.snapshot.conversation.push_transport_error(error);
        }
    }

    fn submit(&mut self, value: String, cx: &mut Context<Self>) {
        if !self.can_submit() {
            if self.prompt_mode == PromptMode::Normal && self.snapshot.conversation.running {
                self.snapshot.conversation.push_local_error(
                    "Prompt not sent",
                    "Pi is working. Choose Steer or Follow-up to queue this draft.".into(),
                );
                cx.notify();
            }
            return;
        }
        match self.runtime.send(RuntimeCommand::Prompt {
            mode: self.prompt_mode,
            message: value.clone(),
        }) {
            Ok(()) => {
                self.pending_submission = Some(PendingSubmission {
                    generation: self.runtime_generation,
                    text: value,
                });
                self.jump_to_latest(cx);
            }
            Err(error) => {
                self.snapshot.conversation.push_transport_error(error);
                cx.notify();
            }
        }
    }

    fn can_submit(&self) -> bool {
        self.pending_submission.is_none()
            && self.snapshot.connected
            && !(self.prompt_mode == PromptMode::Normal && self.snapshot.conversation.running)
    }

    fn drain_runtime(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.extension.prune_notifications();
        while let Ok(event) = self.runtime.try_recv() {
            changed = true;
            match event {
                RuntimeEvent::Snapshot {
                    generation,
                    snapshot,
                } if generation >= self.runtime_generation => {
                    if generation > self.runtime_generation {
                        self.reset_session_ui(generation);
                    }
                    let count = snapshot.conversation.items.len();
                    if count > self.last_transcript_count && !self.transcript_following {
                        self.transcript_unseen = self
                            .transcript_unseen
                            .saturating_add(count - self.last_transcript_count);
                    }
                    self.last_transcript_count = count;
                    self.snapshot = *snapshot;
                }
                RuntimeEvent::SessionReset { generation }
                    if generation >= self.runtime_generation =>
                {
                    self.reset_session_ui(generation);
                }
                RuntimeEvent::Sessions {
                    generation,
                    sessions,
                } if generation >= self.session_generation => {
                    self.session_generation = generation;
                    self.sessions = sessions;
                    self.sessions_error = None;
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
                } if generation == self.runtime_generation => match self.extension.apply(request) {
                    ExtensionEffect::DialogOpened => self.pending_dialog_setup = true,
                    ExtensionEffect::SetTitle(title) => {
                        self.pending_title = Some((generation, title))
                    }
                    ExtensionEffect::SetEditorText(text) => {
                        self.pending_editor_text = Some((generation, text))
                    }
                    ExtensionEffect::PersistError(message) => {
                        self.extension_errors.push(message);
                        if self.extension_errors.len() > MAX_EXTENSION_ERRORS {
                            self.extension_errors.remove(0);
                        }
                    }
                    ExtensionEffect::Diagnostic(message) => {
                        self.snapshot.conversation.diagnostics.push(message)
                    }
                    ExtensionEffect::None => {}
                },
                RuntimeEvent::PromptResult {
                    generation,
                    accepted,
                } if generation == self.runtime_generation => {
                    self.pending_submission_result = Some((generation, accepted));
                }
                RuntimeEvent::Stopped => self.snapshot.status = "Stopped".into(),
                RuntimeEvent::Snapshot { .. }
                | RuntimeEvent::SessionReset { .. }
                | RuntimeEvent::ExtensionUi { .. }
                | RuntimeEvent::PromptResult { .. }
                | RuntimeEvent::Sessions { .. }
                | RuntimeEvent::SessionsFailed { .. } => {}
            }
        }
        if changed {
            if self.transcript_following {
                self.transcript_scroll.scroll_to_bottom();
            }
            cx.notify();
        }
    }

    fn reset_session_ui(&mut self, generation: u64) {
        self.runtime_generation = generation;
        self.extension.reset();
        self.pending_dialog_setup = false;
        self.pending_title = Some((generation, "Pi".into()));
        self.pending_editor_text = None;
        self.pending_submission = None;
        self.pending_submission_result = None;
        self.pending_session_reset = true;
        self.dialog_return_focus = None;
        self.sessions_sheet = false;
        self.run_sheet = false;
        self.sheet_return_focus = None;
        self.pending_sheet_setup = false;
        self.extension_errors.clear();
        self.transcript_visible_items = INITIAL_TRANSCRIPT_ITEMS;
        self.expanded_transcript_items.clear();
        self.transcript_following = true;
        self.transcript_unseen = 0;
        self.last_transcript_count = 0;
    }

    fn set_prompt_mode(&mut self, mode: PromptMode, cx: &mut Context<Self>) {
        self.prompt_mode = mode;
        cx.notify();
    }
    fn resume(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::Resume(path));
        self.sessions_sheet = false;
        cx.notify();
    }
    fn new_session(&mut self, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::NewSession);
        self.sessions_sheet = false;
        cx.notify();
    }

    fn cycle_model(&mut self, cx: &mut Context<Self>) {
        let Some(next) = next_model(
            &self.snapshot.models,
            self.snapshot
                .session
                .as_ref()
                .and_then(|state| state.model.as_ref()),
        ) else {
            return;
        };
        self.send(RuntimeCommand::SetModel {
            provider: next.provider.clone(),
            model_id: next.id.clone(),
        });
        cx.notify();
    }
    fn cycle_thinking(&mut self, cx: &mut Context<Self>) {
        if self.snapshot.thinking_levels.is_empty() {
            return;
        }
        let current = self
            .snapshot
            .session
            .as_ref()
            .map(|state| state.thinking_level.as_str())
            .unwrap_or("off");
        let index = self
            .snapshot
            .thinking_levels
            .iter()
            .position(|level| level == current)
            .map_or(0, |index| (index + 1) % self.snapshot.thinking_levels.len());
        self.send(RuntimeCommand::SetThinking(
            self.snapshot.thinking_levels[index].clone(),
        ));
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

    pub(crate) fn pause_transcript_follow(&mut self, cx: &mut Context<Self>) {
        self.transcript_following = false;
        cx.notify();
    }

    pub(crate) fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.transcript_following = true;
        self.transcript_unseen = 0;
        self.transcript_scroll.scroll_to_bottom();
        cx.notify();
    }

    pub(crate) fn reveal_older_transcript(&mut self, total: usize, cx: &mut Context<Self>) {
        self.transcript_visible_items =
            crate::transcript::next_visible_limit(self.transcript_visible_items, total);
        cx.notify();
    }

    pub(crate) fn toggle_transcript_item(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.expanded_transcript_items.remove(&index) {
            self.expanded_transcript_items.insert(index);
        }
        cx.notify();
    }

    fn resolve_pending_submission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((generation, accepted)) = self.pending_submission_result.take() else {
            return;
        };
        let current = self.composer.read(cx).value().to_string();
        match submission_resolution(
            self.pending_submission.as_ref(),
            generation,
            accepted,
            &current,
        ) {
            SubmissionResolution::ClearEditor => {
                self.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.pending_submission = None;
            }
            SubmissionResolution::KeepEditor => self.pending_submission = None,
            SubmissionResolution::Ignore => {}
        }
    }
}

fn submission_resolution(
    pending: Option<&PendingSubmission>,
    generation: u64,
    accepted: bool,
    current_text: &str,
) -> SubmissionResolution {
    let Some(pending) = pending.filter(|pending| pending.generation == generation) else {
        return SubmissionResolution::Ignore;
    };
    if accepted && current_text == pending.text {
        SubmissionResolution::ClearEditor
    } else {
        SubmissionResolution::KeepEditor
    }
}

fn next_model<'a>(models: &'a [Model], current: Option<&Model>) -> Option<&'a Model> {
    if models.is_empty() {
        return None;
    }
    let index = current
        .and_then(|current| {
            models
                .iter()
                .position(|model| model.provider == current.provider && model.id == current.id)
        })
        .map_or(0, |index| (index + 1) % models.len());
    models.get(index)
}

impl Drop for PiApp {
    fn drop(&mut self) {
        let _ = self.runtime.send(RuntimeCommand::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending() -> PendingSubmission {
        PendingSubmission {
            generation: 7,
            text: "submitted".into(),
        }
    }

    #[test]
    fn accepted_submission_clears_only_the_unchanged_editor() {
        let pending = pending();
        assert_eq!(
            submission_resolution(Some(&pending), 7, true, "submitted"),
            SubmissionResolution::ClearEditor
        );
        assert_eq!(
            submission_resolution(Some(&pending), 7, true, "newer edit"),
            SubmissionResolution::KeepEditor
        );
    }

    #[test]
    fn rejected_or_stale_submission_never_clears_the_editor() {
        let pending = pending();
        assert_eq!(
            submission_resolution(Some(&pending), 7, false, "submitted"),
            SubmissionResolution::KeepEditor
        );
        assert_eq!(
            submission_resolution(Some(&pending), 8, true, "submitted"),
            SubmissionResolution::Ignore
        );
    }
}
