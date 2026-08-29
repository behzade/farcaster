//! Focus restoration and dismissal policy for app-owned overlays.

use std::{path::PathBuf, sync::Arc};

use gpui::{Context, FocusHandle, Image, Window};

use super::{AppSurface, FarcasterApp, ImagePreview, PostRenderFocus};
use crate::{
    protocol::{ExtensionUiRequest, PromptMode},
    runtime::RuntimeCommand,
    sessions::root_session_for_path,
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

impl FarcasterApp {
    pub(super) fn set_surface(&mut self, surface: AppSurface, cx: &mut Context<Self>) -> bool {
        let changed = self.surface != surface;
        self.surface = surface;
        if changed {
            self.notify_session_rail_shell(cx);
            cx.notify();
        }
        changed
    }

    pub(super) fn hide_native_workspace_surfaces(&self, cx: &mut Context<Self>) {
        self.hide_editor(cx);
        self.hide_terminal(cx);
    }

    pub(super) fn cover_native_workspace_surface(&mut self, cx: &mut Context<Self>) {
        if !self.native_surface_covered {
            self.native_surface_covered = match self.surface {
                AppSurface::Editor => self.editor_error.is_none() && self.editor.is_some(),
                AppSurface::Terminal => self.terminal_error.is_none() && self.terminal.is_some(),
                AppSurface::Chat | AppSurface::Work => false,
            };
            if self.native_surface_covered {
                self.native_surface_snapshot = match self.surface {
                    AppSurface::Editor => self.editor.as_ref().and_then(|editor| {
                        editor.update(cx, |editor, cx| editor.snapshot(cx)).ok()
                    }),
                    AppSurface::Terminal => self.terminal.as_ref().and_then(|terminal| {
                        terminal.update(cx, |terminal, _| terminal.snapshot()).ok()
                    }),
                    AppSurface::Chat | AppSurface::Work => None,
                };
            }
        }
        self.hide_native_workspace_surfaces(cx);
    }

    pub(super) fn restore_active_native_workspace_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.native_workspace_modal_active() {
            self.native_surface_covered = false;
            if let Some(snapshot) = self.native_surface_snapshot.take() {
                let _ = window.drop_image(snapshot);
            }
        }
        if self.native_workspace_covered_by_overlay() {
            self.hide_native_workspace_surfaces(cx);
            return;
        }
        self.restore_editor_visibility(cx);
        self.restore_terminal_visibility(cx);
    }

    pub(super) fn workspace_project(&self) -> PathBuf {
        root_session_for_path(
            &self.all_sessions,
            self.snapshot.selected_session.as_deref(),
        )
        .map(|root| root.project.clone())
        .or_else(|| {
            let selected = self.selected_draft.as_deref()?;
            self.drafts
                .iter()
                .find(|draft| draft.id == selected)
                .map(|draft| draft.project.clone())
        })
        .unwrap_or_else(|| self.project.clone())
    }

    pub(super) fn capture_center_surface(&mut self) {
        let target = self.composer_sessions.current_target().to_owned();
        match self.surface {
            AppSurface::Editor | AppSurface::Terminal => {
                self.session_surfaces.insert(target, self.surface);
            }
            AppSurface::Chat | AppSurface::Work => {
                self.session_surfaces.remove(&target);
            }
        }
    }

    pub(super) fn restore_center_surface(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sessions_sheet {
            self.apply_sheet_flags(sheet_flags(None));
            self.pending_sheet_setup = false;
            self.sheet_return_focus = None;
        }
        if self.surface == AppSurface::Work {
            return;
        }
        match self
            .session_surfaces
            .get(self.composer_sessions.current_target())
            .copied()
            .unwrap_or(AppSurface::Chat)
        {
            AppSurface::Editor => self.activate_editor_for_project(project, window, cx),
            AppSurface::Terminal => self.activate_terminal_for_project(project, window, cx),
            AppSurface::Chat | AppSurface::Work => self.activate_chat_center(cx),
        }
    }

    pub(super) fn promote_center_surface(&mut self, from: &str, to: &str) {
        if let Some(surface) = self.session_surfaces.remove(from) {
            self.session_surfaces.insert(to.to_owned(), surface);
        }
    }

    pub(super) fn activate_chat_center(&mut self, cx: &mut Context<Self>) {
        if self.native_workspace_covered_by_overlay() {
            if self.surface != AppSurface::Chat {
                self.hide_native_workspace_surfaces(cx);
                self.set_surface(AppSurface::Chat, cx);
            }
            return;
        }
        let _ = self.enter_chat_surface(self.composer_focus.clone(), cx);
    }

    pub(super) fn reveal_native_center_surface(
        &mut self,
        surface: AppSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_surface(surface, cx);
        if self.native_workspace_covered_by_overlay() {
            self.cover_native_workspace_surface(cx);
        } else {
            self.restore_active_native_workspace_surface(window, cx);
            self.request_active_surface_focus(None);
        }
        cx.notify();
    }

    fn request_active_surface_focus(&mut self, chat: Option<FocusHandle>) {
        self.post_render_focus = Some(PostRenderFocus::ActiveSurface(chat));
    }

    pub(super) fn apply_post_render_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.post_render_focus.take() else {
            return;
        };
        match request {
            PostRenderFocus::ImagePreview => {
                if self.image_preview.is_some() {
                    self.image_preview_focus.focus(window, cx);
                }
            }
            PostRenderFocus::ActiveSurface(chat) => {
                if self.native_workspace_covered_by_overlay() {
                    return;
                }
                match self.surface {
                    AppSurface::Chat => chat
                        .unwrap_or_else(|| self.composer_focus.clone())
                        .focus(window, cx),
                    AppSurface::Editor => {
                        if let Some(editor) = self.editor.as_ref() {
                            editor.update(cx, |editor, cx| editor.focus(window, cx));
                        }
                    }
                    AppSurface::Terminal => {
                        if let Some(terminal) = self.terminal.as_ref() {
                            terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
                        }
                    }
                    AppSurface::Work => {}
                }
            }
        }
    }

    pub(super) fn native_workspace_modal_active(&self) -> bool {
        self.picker.is_some()
            || self.sessions_sheet
            || self.run_sheet
            || self.keybindings_help
            || self.project_trust_sheet
            || self.pending_archive.is_some()
            || self.pending_delete.is_some()
            || self.image_preview.is_some()
            || self.repository.pending_jj_init.is_some()
    }

    fn native_workspace_covered_by_overlay(&self) -> bool {
        self.native_workspace_modal_active()
            || self.extension.dialog.is_some()
            || self.extension.provider_auth.is_some()
    }

    pub(super) fn center_surface_switch_blocked(&self) -> bool {
        self.native_workspace_covered_by_overlay()
    }

    pub(super) fn workspace_switch_blocked(&self) -> bool {
        self.center_surface_switch_blocked() || self.surface == AppSurface::Work
    }

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
        if self.surface == AppSurface::Work {
            self.show_chat_surface(window, cx);
        } else {
            self.open_workgraph_surface(window, cx);
        }
    }

    pub(super) fn enter_chat_surface(
        &mut self,
        focus: FocusHandle,
        cx: &mut Context<Self>,
    ) -> bool {
        self.hide_native_workspace_surfaces(cx);
        let changed = self.set_surface(AppSurface::Chat, cx);
        self.request_active_surface_focus(Some(focus));
        cx.notify();
        changed
    }

    pub(super) fn show_chat_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.enter_chat_surface(self.composer_focus.clone(), cx) {
            self.workgraph_view
                .update(cx, |view, cx| view.prepare_open(window, cx));
        }
    }

    pub(super) fn open_workgraph_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_native_workspace_surfaces(cx);
        if self.run_sheet {
            self.close_sheet(window, cx);
        }
        if self.surface != AppSurface::Work {
            self.refresh_workgraph_board(cx);
            self.set_surface(AppSurface::Work, cx);
        }
        self.workgraph_view
            .update(cx, |view, cx| view.prepare_open(window, cx));
    }

    pub(super) fn open_workgraph_node(
        &mut self,
        number: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_workgraph_surface(window, cx);
        self.workgraph_view
            .update(cx, |view, cx| view.select_node(number, cx));
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
        self.cover_native_workspace_surface(cx);
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

    pub(crate) fn open_image_preview(
        &mut self,
        image: Arc<Image>,
        index: usize,
        total: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.image_preview.is_none() {
            self.image_preview_return_focus = window.focused(cx);
        }
        self.cover_native_workspace_surface(cx);
        self.image_preview = Some(ImagePreview {
            image,
            index,
            total,
        });
        self.post_render_focus = Some(PostRenderFocus::ImagePreview);
        cx.notify();
    }

    pub(super) fn close_image_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.image_preview.take().is_none() {
            return;
        }
        self.image_preview_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone())
            .focus(window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(super) fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_sheet_flags(sheet_flags(None));
        self.pending_sheet_setup = false;
        let focus = self
            .sheet_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone());
        focus.focus(window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(super) fn dismiss_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.image_preview.is_some() {
            self.close_image_preview(window, cx);
        } else if self.repository.pending_jj_init.is_some() {
            self.close_jj_init_confirmation(window, cx);
        } else if self.picker.is_some() {
            self.close_picker(window, cx);
        } else if self.extension.dialog.is_some() {
            self.cancel_dialog(window, cx);
        } else if self.project_trust_sheet {
            self.dismiss_project_trust(window, cx);
        } else if self.sessions_sheet || self.run_sheet || self.keybindings_help {
            self.close_sheet(window, cx);
        }
    }
}

impl Drop for FarcasterApp {
    fn drop(&mut self) {
        let _ = self.runtime.send(RuntimeCommand::Shutdown);
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
