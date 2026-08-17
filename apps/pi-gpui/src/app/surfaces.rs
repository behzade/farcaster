//! Focus restoration and dismissal policy for app-owned overlays.

use gpui::{Context, Window};

use super::PiApp;
use crate::{protocol::ExtensionUiRequest, runtime::RuntimeCommand};

impl PiApp {
    pub(super) fn respond_value(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.dialog_input.read(cx).value().to_string();
        if let Some(response) = self.extension.respond_value(&id, value) {
            self.send(RuntimeCommand::ExtensionResponse(response));
            self.advance_or_restore_dialog(window, cx);
        }
    }

    pub(super) fn respond_confirm(
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

    pub(super) fn cancel_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn advance_or_restore_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn open_sessions_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_return_focus = window.focused(cx);
        self.sessions_sheet = true;
        self.pending_sheet_setup = true;
        cx.notify();
    }

    pub(super) fn open_run_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_return_focus = window.focused(cx);
        self.run_sheet = true;
        self.pending_sheet_setup = true;
        cx.notify();
    }

    pub(super) fn close_sessions_sheet_after_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sessions_sheet {
            self.close_sheet(window, cx);
        }
    }

    pub(super) fn open_keybindings_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_return_focus = window.focused(cx);
        self.keybindings_help = true;
        self.pending_sheet_setup = true;
        cx.notify();
    }

    pub(super) fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sessions_sheet = false;
        self.run_sheet = false;
        self.keybindings_help = false;
        self.pending_sheet_setup = false;
        let focus = self
            .sheet_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone());
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn dismiss_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.changes.diff.is_some() {
            self.close_file_diff(window, cx);
        } else if self.extension.dialog.is_some() {
            self.cancel_dialog(window, cx);
        } else if self.sessions_sheet || self.run_sheet || self.keybindings_help {
            self.close_sheet(window, cx);
        }
    }
}

impl Drop for PiApp {
    fn drop(&mut self) {
        let _ = self.runtime.send(RuntimeCommand::Shutdown);
    }
}
