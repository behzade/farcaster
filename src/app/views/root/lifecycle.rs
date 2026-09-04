use gpui::{Context, Focusable as _, Window};

use super::super::FarcasterApp;
use crate::{app::AppSurface, protocol::ExtensionUiRequest};

impl FarcasterApp {
    pub(super) fn prepare_root_render(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.resolve_pending_submission(window, cx);
        let native_surface = matches!(self.surface, AppSurface::Editor | AppSurface::Terminal);

        if self.post_render_focus.is_some() {
            cx.defer_in(window, |this, window, cx| {
                this.apply_post_render_focus(window, cx);
            });
        }
        if self.pending_session_title_focus {
            self.pending_session_title_focus = false;
            let focus = self.session_title_input.read(cx).focus_handle(cx);
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        }
        if self.overlays.pending_setup {
            self.overlays.pending_setup = false;
            let focus = self.sheet_focus.clone();
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        }
        if self.pending_dialog_setup {
            if self.dialog_return_focus.is_none() {
                self.dialog_return_focus = window.focused(cx);
            }
            if native_surface {
                self.cover_native_workspace_surface(cx);
            }
            self.pending_dialog_setup = false;
            let dialog = self.extension.dialog.as_ref();
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
            let input = self.dialog_input.clone();
            let focus = if uses_textarea {
                input.read(cx).focus_handle(cx)
            } else {
                self.dialog_focus.clone()
            };
            cx.defer_in(window, move |_, window, cx| {
                if uses_textarea {
                    input.update(cx, |state, cx| {
                        state.set_value(prefill, window, cx);
                    });
                }
                focus.focus(window, cx);
            });
        }
        if self.native_surface_covered && !self.native_workspace_covered_by_overlay() {
            self.restore_active_native_workspace_surface(window, cx);
        }
        if let Some((generation, title)) = self.pending_title.take() {
            cx.defer_in(window, move |this, window, _| {
                if this.runtime_generation == generation {
                    window.set_window_title(&title);
                }
            });
        }
        if let Some((generation, text)) = self.pending_editor_text.take() {
            cx.defer_in(window, move |this, window, cx| {
                if this.runtime_generation == generation {
                    let snapshot =
                        crate::app::composer::sessions::ComposerSnapshot::new(text, 0, 0..0);
                    this.apply_composer_snapshot(snapshot.clone(), window, cx);
                    this.composer_sessions.capture_current(snapshot);
                }
            });
        }
        if let Some((target, snapshot)) = self.pending_composer_restore.take() {
            cx.defer_in(window, move |this, window, cx| {
                if this.composer_sessions.current_target() == target {
                    this.apply_composer_snapshot(snapshot.clone(), window, cx);
                    this.composer_sessions.capture_current(snapshot);
                }
            });
        }
    }
}
