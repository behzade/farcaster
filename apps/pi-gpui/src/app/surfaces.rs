//! Focus restoration and dismissal policy for app-owned overlays.

use gpui::{Context, Window};

use super::{AppSurface, PiApp};
use crate::{
    protocol::{ExtensionUiRequest, PromptMode},
    runtime::RuntimeCommand,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppSheet {
    Sessions,
    Run,
    Keybindings,
    ProjectTrust,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SheetFlags {
    sessions: bool,
    run: bool,
    keybindings: bool,
    project_trust: bool,
}

const fn sheet_flags(active: Option<AppSheet>) -> SheetFlags {
    SheetFlags {
        sessions: matches!(active, Some(AppSheet::Sessions)),
        run: matches!(active, Some(AppSheet::Run)),
        keybindings: matches!(active, Some(AppSheet::Keybindings)),
        project_trust: matches!(active, Some(AppSheet::ProjectTrust)),
    }
}

impl SheetFlags {
    const fn any(self) -> bool {
        self.sessions || self.run || self.keybindings || self.project_trust
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
        let value = if matches!(
            self.extension.dialog.as_ref(),
            Some(ExtensionUiRequest::Secret { .. })
        ) {
            self.dialog_secret_input.read(cx).value().to_string()
        } else {
            self.dialog_input.read(cx).value().to_string()
        };
        self.respond_dialog_value(id, value, window, cx);
    }

    pub(super) fn respond_dialog_value(
        &mut self,
        id: String,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.respond_to_restored_dialog(&id, value.clone(), window, cx) {
            return;
        }
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
        if self.respond_to_restored_dialog(
            &id,
            if confirmed { "Yes" } else { "No" }.to_owned(),
            window,
            cx,
        ) {
            return;
        }
        if let Some(response) = self.extension.respond_confirm(&id, confirmed) {
            self.send(RuntimeCommand::ExtensionResponse(response));
            self.advance_or_restore_dialog(window, cx);
        }
    }

    fn respond_to_restored_dialog(
        &mut self,
        id: &str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.restored_dialog_id.as_deref() != Some(id) {
            return false;
        }
        if !self.can_submit() {
            return true;
        }
        let _ = self.extension.cancel(id);
        self.restored_dialog_id = None;
        self.dismissed_restored_dialog_id = Some(id.to_owned());
        self.submit(value, PromptMode::Normal, window, cx);
        true
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
        if self.restored_dialog_id.as_deref() == Some(id.as_str()) {
            let _ = self.extension.cancel(&id);
            self.restored_dialog_id = None;
            self.dismissed_restored_dialog_id = Some(id);
            self.advance_or_restore_dialog(window, cx);
        } else if let Some(response) = self.extension.cancel(&id) {
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
            self.dialog_input.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
            self.dialog_secret_input.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
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

    pub(super) fn toggle_workgraph_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.surface.toggled() {
            AppSurface::Chat => self.show_chat_surface(window, cx),
            AppSurface::Work => self.open_workgraph_surface(window, cx),
        }
    }

    pub(super) fn show_chat_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.surface != AppSurface::Chat || self.workgraph_inspector_issue.is_some();
        self.surface = AppSurface::Chat;
        self.workgraph_inspector_issue = None;
        if changed {
            self.composer_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn open_workgraph_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.run_sheet {
            self.close_sheet(window, cx);
        }
        self.workgraph_inspector_issue = None;
        if self.surface != AppSurface::Work {
            self.refresh_workgraph_board(cx);
            self.surface = AppSurface::Work;
            cx.notify();
        }
        self.workgraph_view
            .update(cx, |view, cx| view.focus(window, cx));
    }

    pub(super) fn inspect_workgraph_issue(
        &mut self,
        number: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_workgraph_board(cx);
        self.workgraph_view
            .update(cx, |view, cx| view.select_issue(number, cx));
        self.workgraph_inspector_issue = Some(number);
        self.surface = AppSurface::Chat;
        if !self.run_sheet
            && crate::layout::shows_run_sheet_button(crate::layout::layout_mode(
                window.viewport_size().width,
            ))
        {
            self.open_run_sheet(window, cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn close_workgraph_inspector(&mut self, cx: &mut Context<Self>) {
        if self.workgraph_inspector_issue.take().is_some() {
            cx.notify();
        }
    }

    fn refresh_workgraph_board(&mut self, cx: &mut Context<Self>) {
        let project = self.project.clone();
        let active_session = self.active_workgraph_session();
        self.workgraph_view.update(cx, |view, cx| {
            view.refresh_for(project, active_session, cx);
        });
    }

    pub(super) fn refresh_workgraph_sidebar(&mut self, cx: &mut Context<Self>) {
        let project = self.project.clone();
        let session_id = self
            .active_workgraph_session()
            .map(|(session_id, _)| session_id);
        self.workgraph_sidebar_view.update(cx, |view, cx| {
            view.refresh_for(project, session_id, cx);
        });
    }

    pub(super) fn refresh_workgraph_data(&mut self, cx: &mut Context<Self>) {
        self.refresh_workgraph_sidebar(cx);
        if self.surface == AppSurface::Work {
            self.refresh_workgraph_board(cx);
        }
    }

    pub(super) fn active_workgraph_session(&self) -> Option<(String, String)> {
        let selected = self.snapshot.selected_session.as_deref()?;
        self.all_sessions
            .iter()
            .chain(&self.sessions)
            .find(|session| session.path == selected)
            .map(|session| (session.id.clone(), session.path.display().to_string()))
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

    pub(super) fn open_project_trust(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.project_trust_error = None;
        self.project_trust_project = Some(self.project.clone());
        self.pending_project_trust_command = None;
        self.open_sheet(AppSheet::ProjectTrust, window, cx);
    }

    fn open_sheet(&mut self, sheet: AppSheet, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            self.picker_return_focus = None;
        }
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
            keybindings: self.keybindings_help,
            project_trust: self.project_trust_sheet,
        }
    }

    fn apply_sheet_flags(&mut self, flags: SheetFlags) {
        self.sessions_sheet = flags.sessions;
        self.run_sheet = flags.run;
        self.keybindings_help = flags.keybindings;
        self.project_trust_sheet = flags.project_trust;
    }

    pub(super) fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.run_sheet {
            self.workgraph_inspector_issue = None;
        }
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
        if self.picker.is_some() {
            self.close_picker(window, cx);
        } else if self.changes.diff.is_some() {
            self.close_file_diff(window, cx);
        } else if self.extension.dialog.is_some() {
            self.cancel_dialog(window, cx);
        } else if self.project_trust_sheet {
            self.dismiss_project_trust(window, cx);
        } else if self.sessions_sheet || self.run_sheet || self.keybindings_help {
            self.close_sheet(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_work_shortcut_toggles_back_to_chat() {
        assert_eq!(AppSurface::Chat.toggled(), AppSurface::Work);
        assert_eq!(AppSurface::Work.toggled(), AppSurface::Chat);
    }

    #[test]
    fn activating_a_sheet_never_stacks_it_with_an_existing_sheet() {
        for sheet in [
            AppSheet::Sessions,
            AppSheet::Run,
            AppSheet::Keybindings,
            AppSheet::ProjectTrust,
        ] {
            let flags = sheet_flags(Some(sheet));
            assert_eq!(
                [
                    flags.sessions,
                    flags.run,
                    flags.keybindings,
                    flags.project_trust,
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
            AppSheet::Run
        ))));
    }
}

impl Drop for PiApp {
    fn drop(&mut self) {
        let _ = self.runtime.send(RuntimeCommand::Shutdown);
    }
}
