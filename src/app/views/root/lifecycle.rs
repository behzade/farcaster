use gpui::{Context, Focusable as _, Window};

use super::super::FarcasterApp;
use crate::{app::AppSurface, protocol::ExtensionUiRequest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DialogLifecycleAction {
    None,
    Setup,
    RestoreFocus,
}

fn dialog_lifecycle_action(pending: bool, has_dialog: bool) -> DialogLifecycleAction {
    match (pending, has_dialog) {
        (false, _) => DialogLifecycleAction::None,
        (true, true) => DialogLifecycleAction::Setup,
        (true, false) => DialogLifecycleAction::RestoreFocus,
    }
}

impl FarcasterApp {
    pub(super) fn prepare_root_render(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.resolve_pending_submission(window, cx);
        self.close_stale_worker_model_picker(window, cx);
        let native_surface = matches!(
            self.workspace.surface,
            AppSurface::Editor | AppSurface::Terminal
        );

        if self.overlays.post_render_focus.is_some() {
            cx.defer_in(window, |this, window, cx| {
                this.apply_post_render_focus(window, cx);
            });
        }
        if self.sessions.pending_title_focus {
            self.sessions.pending_title_focus = false;
            let focus = self.sessions.title_input.read(cx).focus_handle(cx);
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        }
        if self.overlays.view.pending_setup {
            self.overlays.view.pending_setup = false;
            let focus = self.overlays.sheet_focus.clone();
            cx.defer_in(window, move |this, window, cx| {
                if this.keyboard_overlay_focus(window, cx).as_ref() == Some(&focus) {
                    focus.focus(window, cx);
                }
            });
        }
        let dialog_lifecycle = dialog_lifecycle_action(
            self.extensions.pending_dialog_setup,
            self.extensions.active.dialog.is_some(),
        );
        if dialog_lifecycle == DialogLifecycleAction::RestoreFocus {
            self.extensions.pending_dialog_setup = false;
            cx.defer_in(window, |this, window, cx| {
                if this.extensions.active.dialog.is_none() {
                    this.advance_or_restore_dialog(window, cx);
                }
            });
        }
        if dialog_lifecycle == DialogLifecycleAction::Setup {
            if self.extensions.dialog_return_focus.is_none() {
                self.extensions.dialog_return_focus = window.focused(cx);
            }
            if native_surface && self.native_workspace_covered_by_overlay() {
                self.cover_native_workspace_surface(cx);
            }
            self.extensions.pending_dialog_setup = false;
            let dialog = self.extensions.active.dialog.as_ref();
            let prefill = match dialog {
                Some(ExtensionUiRequest::Editor { prefill, .. }) => {
                    prefill.clone().unwrap_or_default()
                }
                _ => String::new(),
            };
            let uses_textarea = matches!(
                dialog,
                Some(ExtensionUiRequest::Input { .. } | ExtensionUiRequest::Editor { .. })
            );
            let dialog_id = dialog
                .and_then(ExtensionUiRequest::dialog_id)
                .map(str::to_owned);
            let input = self.extensions.dialog_input.clone();
            let focus = if uses_textarea {
                input.read(cx).focus_handle(cx)
            } else {
                self.extensions.dialog_focus.clone()
            };
            let composer_slot_owns = self.composer.focus.is_focused(window)
                || self.extensions.dialog_focus.contains_focused(window, cx)
                || self
                    .extensions
                    .dialog_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window);
            cx.defer_in(window, move |this, window, cx| {
                if dialog_id.is_none()
                    || this
                        .extensions
                        .active
                        .dialog
                        .as_ref()
                        .and_then(ExtensionUiRequest::dialog_id)
                        != dialog_id.as_deref()
                {
                    return;
                }
                if uses_textarea {
                    input.update(cx, |state, cx| {
                        state.set_value(prefill, window, cx);
                    });
                }
                if composer_slot_owns && this.keyboard_overlay_focus(window, cx).is_none() {
                    focus.focus(window, cx);
                }
            });
        }
        if self.workspace.native_surface_covered && !self.native_workspace_covered_by_overlay() {
            self.restore_active_native_workspace_surface(window, cx);
        }
        if let Some((generation, title)) = self.extensions.pending_title.take() {
            cx.defer_in(window, move |this, window, _| {
                if this.runtime_generation == generation {
                    window.set_window_title(&title);
                }
            });
        }
        if let Some((generation, text)) = self.extensions.pending_editor_text.take() {
            cx.defer_in(window, move |this, window, cx| {
                if this.runtime_generation == generation {
                    let snapshot =
                        crate::app::composer::sessions::ComposerSnapshot::new(text, 0, 0..0);
                    this.apply_composer_snapshot(snapshot.clone(), window, cx);
                    this.composer.sessions.capture_current(snapshot);
                }
            });
        }
        if let Some((target, snapshot)) = self.composer.pending_restore.take() {
            cx.defer_in(window, move |this, window, cx| {
                if this.composer.sessions.current_target() == target {
                    this.apply_composer_snapshot(snapshot.clone(), window, cx);
                    this.composer.sessions.capture_current(snapshot);
                }
            });
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
