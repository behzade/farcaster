use gpui::{AppContext as _, Context, Entity, FocusHandle, Focusable as _, Subscription, Window};
use gpui_component::input::{InputEvent, TextareaState};

use super::neovim::CodeContext;
use crate::app::{AppSurface, FarcasterApp};

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

    pub fn prompt(&self, comment: &str) -> String {
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
            comment.trim(),
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

pub(in crate::app) struct CodeComment {
    pub focus: FocusHandle,
    pub input: Entity<TextareaState>,
    pub context: CodeContext,
    target: String,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

impl FarcasterApp {
    pub(in crate::app) fn comment_on_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.surface != AppSurface::Editor
            || self.native_workspace_covered_by_overlay()
            || self.code_comment_capture.is_some()
        {
            return;
        }
        let Some(editor) = self.editor.clone().filter(|_| self.editor_ready) else {
            return;
        };
        let target = self.composer_sessions.current_target().to_owned();
        let generation = self.editor_request_generation;
        let return_focus = window.focused(cx);
        let capture = editor.update(cx, |editor, cx| editor.capture_code(cx));
        self.code_comment_capture = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = capture.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.code_comment_capture = None;
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
                        this.notify_workspace_error("Comment on code", error, cx);
                        return;
                    }
                };
                let input = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .auto_grow(1, 8)
                        .submit_on_enter(true)
                        .placeholder("Comment…")
                });
                let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                        this.add_code_comment(window, cx);
                    }
                    cx.notify();
                });
                this.cover_native_workspace_surface(cx);
                let input_focus = input.read(cx).focus_handle(cx);
                this.code_comment = Some(CodeComment {
                    focus: cx.focus_handle(),
                    input,
                    context,
                    target,
                    return_focus,
                    _subscription: subscription,
                });
                input_focus.focus(window, cx);
                cx.notify();
            });
        }));
    }

    pub(in crate::app) fn close_code_comment(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(comment) = self.code_comment.take() else {
            return;
        };
        self.restore_overlay_focus(comment.return_focus, &comment.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn add_code_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(comment) = self.code_comment.as_ref() else {
            return;
        };
        if comment.target != self.composer_sessions.current_target() {
            self.notify_workspace_error(
                "Comment on code",
                "Return to the original session to add this comment.".into(),
                cx,
            );
            return;
        }
        let instruction = comment.input.read(cx).value();
        if instruction.trim().is_empty() {
            return;
        }
        let prompt = comment.context.prompt(&instruction);
        let existing = self.composer.read(cx).value();
        let combined = if existing.is_empty() {
            prompt
        } else {
            format!("{existing}\n\n{prompt}")
        };
        self.close_code_comment(window, cx);
        self.composer.update(cx, |input, cx| {
            input.set_value(combined.clone(), window, cx);
            input.set_selected_range(combined.len()..combined.len(), cx);
        });
        self.capture_composer_session(cx);
        self.return_to_chat_composer(window, cx);
    }
}

#[cfg(test)]
#[path = "code_comment_tests.rs"]
mod tests;
