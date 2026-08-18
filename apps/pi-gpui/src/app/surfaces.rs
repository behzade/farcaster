//! Focus restoration and dismissal policy for app-owned overlays.

use gpui::{Context, Window};

use super::PiApp;
use crate::{
    protocol::ExtensionUiRequest, runtime::RuntimeCommand, sessions::root_session_for_path,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppSheet {
    Sessions,
    Run,
    WorkGraph,
    Keybindings,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SheetFlags {
    sessions: bool,
    run: bool,
    workgraph: bool,
    keybindings: bool,
}

const fn sheet_flags(active: Option<AppSheet>) -> SheetFlags {
    SheetFlags {
        sessions: matches!(active, Some(AppSheet::Sessions)),
        run: matches!(active, Some(AppSheet::Run)),
        workgraph: matches!(active, Some(AppSheet::WorkGraph)),
        keybindings: matches!(active, Some(AppSheet::Keybindings)),
    }
}

impl SheetFlags {
    const fn any(self) -> bool {
        self.sessions || self.run || self.workgraph || self.keybindings
    }
}

const fn should_capture_return_focus(flags: SheetFlags) -> bool {
    !flags.any()
}

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
        self.open_sheet(AppSheet::Sessions, window, cx);
    }

    pub(super) fn open_run_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(AppSheet::Run, window, cx);
    }

    pub(super) fn open_workgraph_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.project.clone();
        let active_session =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| (session.id.clone(), session.path.display().to_string()));
        self.workgraph_view.update(cx, |view, cx| {
            view.refresh_for(project, active_session, cx);
        });
        self.open_sheet(AppSheet::WorkGraph, window, cx);
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
        self.open_sheet(AppSheet::Keybindings, window, cx);
    }

    fn open_sheet(&mut self, sheet: AppSheet, window: &mut Window, cx: &mut Context<Self>) {
        if should_capture_return_focus(self.current_sheet_flags()) {
            self.sheet_return_focus = window.focused(cx);
        }
        self.apply_sheet_flags(sheet_flags(Some(sheet)));
        self.pending_sheet_setup = true;
        cx.notify();
    }

    fn current_sheet_flags(&self) -> SheetFlags {
        SheetFlags {
            sessions: self.sessions_sheet,
            run: self.run_sheet,
            workgraph: self.workgraph_sheet,
            keybindings: self.keybindings_help,
        }
    }

    fn apply_sheet_flags(&mut self, flags: SheetFlags) {
        self.sessions_sheet = flags.sessions;
        self.run_sheet = flags.run;
        self.workgraph_sheet = flags.workgraph;
        self.keybindings_help = flags.keybindings;
    }

    pub(super) fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_sheet_flags(sheet_flags(None));
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
        } else if self.sessions_sheet
            || self.run_sheet
            || self.workgraph_sheet
            || self.keybindings_help
        {
            self.close_sheet(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activating_a_sheet_never_stacks_it_with_an_existing_sheet() {
        for sheet in [
            AppSheet::Sessions,
            AppSheet::Run,
            AppSheet::WorkGraph,
            AppSheet::Keybindings,
        ] {
            let flags = sheet_flags(Some(sheet));
            assert_eq!(
                [
                    flags.sessions,
                    flags.run,
                    flags.workgraph,
                    flags.keybindings
                ]
                .into_iter()
                .filter(|active| *active)
                .count(),
                1
            );
        }
        assert!(!sheet_flags(None).any());
    }

    #[test]
    fn an_existing_sheet_prevents_recapturing_the_return_focus() {
        assert!(should_capture_return_focus(sheet_flags(None)));
        assert!(!should_capture_return_focus(sheet_flags(Some(
            AppSheet::Sessions
        ))));
        assert!(!should_capture_return_focus(sheet_flags(Some(
            AppSheet::WorkGraph
        ))));
    }
}

impl Drop for PiApp {
    fn drop(&mut self) {
        let _ = self.runtime.send(RuntimeCommand::Shutdown);
    }
}
