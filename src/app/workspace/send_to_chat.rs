use gpui::{Context, Entity, FocusHandle, Focusable as _, Subscription, Window};
use gpui_component::input::TextareaState;

use super::neovim::CodeContext;
use crate::app::{AppSurface, FarcasterApp, ui::primitives::create_submit_textarea};
use crate::runtime::TaskSettings;

#[path = "code_destinations.rs"]
mod destinations;
pub(in crate::app) use destinations::CodeDestination;
use destinations::DestinationPicker;

impl CodeContext {
    pub fn location(&self) -> String {
        if self.mode == "n" {
            format!("{}:{}:{}", self.path, self.cursor_line, self.cursor_column)
        } else {
            let mut endpoints = [
                (self.anchor_line, self.anchor_column),
                (self.cursor_line, self.cursor_column),
            ];
            endpoints.sort();
            format!(
                "{}:{}:{}–{}:{}",
                self.path, endpoints[0].0, endpoints[0].1, endpoints[1].0, endpoints[1].1
            )
        }
    }

    pub fn prompt(&self, instruction: &str) -> String {
        // A selection may itself contain Markdown fences.
        let longest = self
            .text
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0);
        let fence = "`".repeat(longest.max(2) + 1);
        let kind = match self.mode.as_str() {
            "n" => "Current line",
            "V" => "Selected lines",
            "\u{16}" => "Block selection",
            _ => "Selected text",
        };
        format!(
            "{}\n\nCode context: {}\n{kind}{} (captured from the editor):\n{fence}\n{}\n{fence}",
            instruction.trim(),
            self.location(),
            if self.modified {
                "; buffer has unsaved edits"
            } else {
                ""
            },
            self.text
        )
    }
}

pub(in crate::app) struct SendToChat {
    pub focus: FocusHandle,
    pub input: Entity<TextareaState>,
    pub context: CodeContext,
    pub settings: TaskSettings,
    destination: Option<usize>,
    pub picker: Option<DestinationPicker>,
    pub error: Option<String>,
    destinations: Vec<CodeDestination>,
    target: String,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

impl SendToChat {
    pub(in crate::app) fn destination(&self) -> Option<&CodeDestination> {
        self.destination.map(|index| &self.destinations[index])
    }
}

impl FarcasterApp {
    pub(in crate::app) fn open_send_to_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_code_prompt(false, window, cx);
    }

    pub(in crate::app) fn start_task_from_code(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_code_prompt(true, window, cx);
    }

    fn capture_code_prompt(
        &mut self,
        start_task: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.surface != AppSurface::Editor
            || self.native_workspace_covered_by_overlay()
            || self.send_to_chat_capture.is_some()
        {
            return;
        }
        let Some(editor) = self.editor.clone().filter(|_| self.editor_ready) else {
            return;
        };
        let settings = TaskSettings {
            project: self.workspace_project(),
            harness: self.active_harness().to_owned(),
            model: self.snapshot.session_identity().model.cloned(),
            effort: self.snapshot.session_identity().effort.map(str::to_owned),
            access_mode: self.snapshot.access_mode,
        };
        let target = self.composer_sessions.current_target().to_owned();
        let generation = self.editor_request_generation;
        let return_focus = window.focused(cx);
        let capture = editor.update(cx, |editor, cx| editor.capture_code(cx));
        self.send_to_chat_capture = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = capture.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.send_to_chat_capture = None;
                // Never open a late capture over another session, editor, or modal.
                if this.composer_sessions.current_target() != target
                    || this.editor_request_generation != generation
                    || this.editor.as_ref() != Some(&editor)
                    || this.surface != AppSurface::Editor
                    || this.native_workspace_covered_by_overlay()
                    || window.focused(cx) != return_focus
                {
                    return;
                }
                let context = match result {
                    Ok(context) => context,
                    Err(error) => {
                        this.notify_workspace_error(
                            if start_task {
                                "Start task"
                            } else {
                                "Send to chat"
                            },
                            error,
                            cx,
                        );
                        return;
                    }
                };
                let (input, subscription) = create_submit_textarea(
                    window,
                    cx,
                    |input| {
                        input.auto_grow(1, 8).placeholder(if start_task {
                            "What should the agent do?"
                        } else {
                            "Message…"
                        })
                    },
                    FarcasterApp::confirm_send_to_chat,
                );
                this.cover_native_workspace_surface(cx);
                let input_focus = input.read(cx).focus_handle(cx);
                let current = CodeDestination {
                    target: target.clone(),
                    session: this.snapshot.session_target(),
                    label: "Current chat".into(),
                    harness: settings.harness.clone(),
                };
                let destinations =
                    destinations::choices(&settings.project, current, &this.all_sessions);
                this.send_to_chat = Some(SendToChat {
                    focus: cx.focus_handle(),
                    input,
                    context,
                    settings,
                    destination: (!start_task).then_some(0),
                    destinations,
                    picker: None,
                    error: None,
                    target,
                    return_focus,
                    _subscription: subscription,
                });
                input_focus.focus(window, cx);
                cx.notify();
            });
        }));
    }

    pub(in crate::app) fn close_send_to_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .send_to_chat
            .as_ref()
            .is_some_and(|dialog| dialog.picker.is_some())
        {
            self.close_code_destination_picker(window, cx);
            return;
        }
        let Some(dialog) = self.send_to_chat.take() else {
            return;
        };
        self.restore_overlay_focus(dialog.return_focus, &dialog.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn confirm_send_to_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.send_to_chat.as_mut() else {
            return;
        };
        dialog.error = None;
        if dialog.picker.is_some() {
            return;
        }
        if dialog.target != self.composer_sessions.current_target() {
            self.send_to_chat_error(
                "Return to the original session to use this code capture.".into(),
                cx,
            );
            return;
        }
        let instruction = dialog.input.read(cx).value();
        if instruction.trim().is_empty() {
            return;
        }
        let prompt = dialog.context.prompt(&instruction);
        if let Some(destination) = dialog.destination().cloned() {
            let project = dialog.settings.project.clone();
            self.submit_to_chat(destination, project, prompt, window, cx);
        } else {
            let settings = dialog.settings.clone();
            self.submit_code_task(settings, prompt, window, cx);
        }
    }

    pub(super) fn send_to_chat_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.send_to_chat.as_mut() {
            dialog.error = Some(message);
            cx.notify();
        }
    }
}

#[cfg(test)]
#[path = "send_to_chat_tests.rs"]
mod tests;
