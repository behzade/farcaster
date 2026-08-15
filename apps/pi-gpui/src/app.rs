//! Top-level GPUI composition for the active root session.

mod views;
pub(crate) use views::OVERLAY_KEY_CONTEXT;

use std::{collections::HashSet, ops::Range, path::PathBuf, time::Duration};

use gpui::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable as _, FollowMode,
    ListAlignment, ListState, Pixels, Point, Subscription, Task, Window, actions,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_fps::FpsMonitor;

use crate::{
    extension_ui::{ExtensionEffect, ExtensionUiState},
    protocol::{ExtensionUiRequest, Model, PromptMode},
    runtime::{RuntimeCommand, RuntimeEvent, RuntimeHandle, RuntimeSnapshot},
    sessions::SessionSummary,
};

const MAX_EXTENSION_ERRORS: usize = 16;
pub(crate) const COMPOSER_KEY_CONTEXT: &str = "PiComposer";
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

actions!(pi_gpui, [DismissSurface, QuitApplication, SubmitFollowUp]);

pub(crate) struct PiApp {
    project: PathBuf,
    runtime: RuntimeHandle,
    pub(crate) snapshot: RuntimeSnapshot,
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
    transcript_list: ListState,
    transcript_following: bool,
    transcript_unseen: usize,
    pub(crate) expanded_transcript_items: HashSet<usize>,
    last_transcript_count: usize,
    pub(crate) transcript_layout: crate::transcript::TranscriptLayoutCache,
    pub(crate) transcript_bounds: Option<Bounds<Pixels>>,
    pub(crate) transcript_width: Pixels,
    fps_monitor: Option<Entity<FpsMonitor>>,
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
                        this.submit(value, this.enter_mode(), cx);
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
            snapshot: RuntimeSnapshot {
                status: "Starting".into(),
                project: project.clone(),
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
            transcript_list,
            transcript_following: true,
            transcript_unseen: 0,
            expanded_transcript_items: HashSet::new(),
            last_transcript_count: 0,
            transcript_layout: crate::transcript::TranscriptLayoutCache::default(),
            transcript_bounds: None,
            transcript_width: crate::theme::THEME.layout.transcript_max,
            fps_monitor,
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
            let index = self.snapshot.conversation.items.len();
            self.snapshot.conversation.push_transport_error(error);
            self.mark_transcript_changed(index, index == 0);
        }
    }

    fn submit(&mut self, value: String, mode: PromptMode, cx: &mut Context<Self>) {
        if !self.can_submit() {
            return;
        }
        match self.runtime.send(RuntimeCommand::Prompt {
            mode,
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
                let index = self.snapshot.conversation.items.len();
                self.snapshot.conversation.push_transport_error(error);
                self.transcript_list.splice(index..index, 1);
                cx.notify();
            }
        }
    }

    fn can_submit(&self) -> bool {
        self.pending_submission.is_none() && self.snapshot.connected
    }

    fn enter_mode(&self) -> PromptMode {
        prompt_mode_for_enter(self.snapshot.conversation.running)
    }

    fn submit_follow_up(&mut self, cx: &mut Context<Self>) {
        let value = self.composer.read(cx).value().trim().to_owned();
        if !value.is_empty() {
            self.submit(value, PromptMode::FollowUp, cx);
        }
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
                        self.reset_session_ui(generation, false);
                    }
                    let count = snapshot.conversation.items.len();
                    if count > self.last_transcript_count && !self.transcript_following {
                        self.transcript_unseen = self
                            .transcript_unseen
                            .saturating_add(count - self.last_transcript_count);
                    }
                    self.sync_transcript_list(&snapshot.conversation.items);
                    self.last_transcript_count = count;
                    self.snapshot = *snapshot;
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

    fn reset_session_ui(&mut self, generation: u64, preserve_submission: bool) {
        self.runtime_generation = generation;
        self.extension.reset();
        self.pending_dialog_setup = false;
        self.pending_title = Some((generation, "Pi".into()));
        self.pending_editor_text = None;
        if preserve_submission {
            if let Some(pending) = self.pending_submission.as_mut() {
                pending.generation = generation;
            }
        } else {
            self.pending_submission = None;
        }
        self.pending_submission_result = None;
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

    fn reset_transcript_ui(&mut self) {
        self.snapshot.conversation.items.clear();
        self.transcript_list.reset(0);
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        self.expanded_transcript_items.clear();
        self.transcript_layout.clear();
        self.transcript_bounds = None;
        self.transcript_following = true;
        self.transcript_unseen = 0;
        self.last_transcript_count = 0;
    }

    fn resume(&mut self, path: PathBuf, project: PathBuf, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::Resume { path, project });
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

    pub(crate) fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.transcript_following = true;
        self.transcript_unseen = 0;
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        self.transcript_list.scroll_to_end();
        cx.notify();
    }

    pub(crate) fn toggle_transcript_item(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.expanded_transcript_items.remove(&index) {
            self.expanded_transcript_items.insert(index);
        }
        self.transcript_layout.mark_dirty(index);
        self.transcript_list.remeasure_items(0..1);
        cx.notify();
    }

    pub(crate) fn toggle_transcript_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.transcript_bounds else {
            return;
        };
        if let Some(index) = self.transcript_layout.thinking_item_at(bounds, position) {
            self.toggle_transcript_item(index, cx);
        }
    }

    fn sync_transcript_list(&mut self, next: &[crate::conversation::TranscriptItem]) {
        if let Some((old_range, _new_count)) =
            transcript_splice(&self.snapshot.conversation.items, next)
        {
            self.transcript_layout.mark_dirty(old_range.start);
            match (self.snapshot.conversation.items.is_empty(), next.is_empty()) {
                (true, false) => self.transcript_list.splice(0..0, 1),
                (false, true) => self.transcript_list.splice(0..1, 0),
                (false, false) => self.transcript_list.remeasure_items(0..1),
                (true, true) => {}
            }
        }
    }

    fn mark_transcript_changed(&mut self, index: usize, was_empty: bool) {
        self.transcript_layout.mark_dirty(index);
        if was_empty {
            self.transcript_list.splice(0..0, 1);
        } else {
            self.transcript_list.remeasure_items(0..1);
        }
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

fn transcript_splice(
    current: &[crate::conversation::TranscriptItem],
    next: &[crate::conversation::TranscriptItem],
) -> Option<(Range<usize>, usize)> {
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

fn prompt_mode_for_enter(running: bool) -> PromptMode {
    if running {
        PromptMode::Steer
    } else {
        PromptMode::Normal
    }
}

fn debug_enabled() -> bool {
    debug_value_enabled(std::env::var("DEBUG").ok().as_deref())
}

fn debug_value_enabled(value: Option<&str>) -> bool {
    value == Some("true")
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
    use crate::conversation::{TranscriptItem, TranscriptKind};

    fn pending() -> PendingSubmission {
        PendingSubmission {
            generation: 7,
            text: "submitted".into(),
        }
    }

    fn item(text: &str) -> TranscriptItem {
        TranscriptItem {
            kind: TranscriptKind::Assistant,
            label: "Pi".into(),
            text: text.into(),
            streaming: false,
            is_error: false,
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

    #[test]
    fn enter_prompts_when_idle_and_steers_while_running() {
        assert_eq!(prompt_mode_for_enter(false), PromptMode::Normal);
        assert_eq!(prompt_mode_for_enter(true), PromptMode::Steer);
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
}
