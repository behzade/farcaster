use std::{path::PathBuf, sync::Arc};

use gpui::{Context, FocusHandle, Focusable as _, Image, Window, actions};

use super::{AppSurface, FarcasterApp, ImagePreview, PostRenderFocus};
actions!(farcaster, [CycleWorkspaceForward, CycleWorkspaceBackward]);

use crate::{
    protocol::{ExtensionUiRequest, PromptMode},
    runtime::RuntimeCommand,
    sessions::root_session_for_path,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppSheet {
    Sessions,
    Run,
    WorkerNotices,
    Keybindings,
    Settings,
    ProjectTrust,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SheetFlags {
    sessions: bool,
    run: bool,
    worker_notices: bool,
    keybindings: bool,
    settings: bool,
    project_trust: bool,
}

const fn sheet_flags(active: Option<AppSheet>) -> SheetFlags {
    SheetFlags {
        sessions: matches!(active, Some(AppSheet::Sessions)),
        run: matches!(active, Some(AppSheet::Run)),
        worker_notices: matches!(active, Some(AppSheet::WorkerNotices)),
        keybindings: matches!(active, Some(AppSheet::Keybindings)),
        settings: matches!(active, Some(AppSheet::Settings)),
        project_trust: matches!(active, Some(AppSheet::ProjectTrust)),
    }
}

impl SheetFlags {
    const fn any(self) -> bool {
        self.sessions
            || self.run
            || self.worker_notices
            || self.keybindings
            || self.settings
            || self.project_trust
    }
}

const fn should_capture_return_focus(flags: SheetFlags) -> bool {
    !flags.any()
}

#[cfg(test)]
const fn arriving_request_takes_focus(composer_slot_owns: bool) -> bool {
    composer_slot_owns
}

impl FarcasterApp {
    pub(in crate::app) fn recover_keyboard_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(focus) = self.keyboard_overlay_focus(window, cx) {
            focus.focus(window, cx);
        } else {
            self.request_active_surface_focus(None);
            self.apply_post_render_focus(window, cx);
        }
    }

    pub(in crate::app) fn keyboard_overlay_focus(
        &self,
        _window: &Window,
        cx: &gpui::App,
    ) -> Option<FocusHandle> {
        if let Some(pending) = &self.lifecycle.pending_quit {
            Some(pending.focus.clone())
        } else if let Some(dialog) = &self.workspace.send_to_chat
            && !self.overlays.view.project_trust
        {
            Some(dialog.picker.as_ref().map_or_else(
                || dialog.input.read(cx).focus_handle(cx),
                |picker| picker.list.read(cx).focus_handle(cx),
            ))
        } else if self.overlays.image_preview.is_some() {
            Some(self.overlays.image_preview_focus.clone())
        } else if let Some(pending) = &self.project.repository.edits.pending {
            Some(
                if pending.action == crate::repository::RepositoryEdit::Commit {
                    pending.input.read(cx).focus_handle(cx)
                } else {
                    pending.focus.clone()
                },
            )
        } else if let Some(pending) = &self.project.repository.pending_jj_init {
            Some(pending.focus.clone())
        } else if let Some(pending) = &self.sessions.pending_delete {
            Some(pending.focus.clone())
        } else if let Some(dialog) = &self.sessions.import {
            Some(dialog.focus.clone())
        } else if let Some(pending) = &self.sessions.pending_archive {
            Some(pending.focus.clone())
        } else if self.current_sheet_flags().any() {
            Some(self.overlays.sheet_focus.clone())
        } else if self.navigation.picker.is_some() {
            self.picker_focus(cx)
        } else if self.workspace.surface == AppSurface::Work {
            Some(self.views.workgraph.read(cx).focus_handle())
        } else {
            None
        }
    }

    pub(in crate::app) fn composer_region_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        let focus = if self.extensions.active.dialog.is_some() {
            &self.extensions.dialog_focus
        } else {
            &self.composer.focus
        };
        window.is_window_active() && focus.contains_focused(window, cx)
    }

    pub(in crate::app) fn composer_region_focus(&self, cx: &gpui::App) -> FocusHandle {
        match self.extensions.active.dialog {
            Some(ExtensionUiRequest::Input { .. } | ExtensionUiRequest::Editor { .. }) => {
                self.extensions.dialog_input.read(cx).focus_handle(cx)
            }
            Some(_) => self.extensions.dialog_focus.clone(),
            None => self.composer.focus.clone(),
        }
    }

    pub(in crate::app) fn restore_overlay_focus(
        &mut self,
        target: Option<FocusHandle>,
        closing: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let overlay = self.keyboard_overlay_focus(window, cx);
        let restored = crate::app::ui::focus::restore(
            target,
            closing,
            &self.navigation.chat.focus,
            overlay
                .clone()
                .unwrap_or_else(|| self.chat_composer_focus(cx)),
            window,
            cx,
        );
        if restored == crate::app::ui::focus::Restoration::Fallback
            && overlay.is_none()
            && matches!(
                self.workspace.surface,
                AppSurface::Editor | AppSurface::Terminal
            )
        {
            self.request_active_surface_focus(None);
        }
    }

    pub(in crate::app) fn set_surface(
        &mut self,
        surface: AppSurface,
        cx: &mut Context<Self>,
    ) -> bool {
        let changed = self.workspace.surface != surface;
        self.workspace.surface = surface;
        if changed {
            self.navigation.chat.activation.clear();
            self.workspace.editor.request_generation =
                self.workspace.editor.request_generation.wrapping_add(1);
            self.notify_session_rail_shell(cx);
            self.notify_run_panel(cx);
            cx.notify();
        }
        changed
    }

    pub(in crate::app) fn hide_native_workspace_surfaces(&self, cx: &mut Context<Self>) {
        self.hide_editor(cx);
        self.hide_terminal(cx);
    }

    pub(in crate::app) fn native_workspace_surface_ready(&self) -> bool {
        match self.workspace.surface {
            AppSurface::Editor => {
                self.workspace.editor.ready && self.workspace.editor.view.is_some()
            }
            AppSurface::Terminal => self.workspace.terminal.view.is_some(),
            AppSurface::Chat | AppSurface::Work => false,
        }
    }

    pub(in crate::app) fn cover_native_workspace_surface(&mut self, cx: &mut Context<Self>) {
        if !self.workspace.native_surface_covered {
            self.workspace.native_surface_covered = self.native_workspace_surface_ready();
            if self.workspace.native_surface_covered {
                self.workspace.native_surface_snapshot = match self.workspace.surface {
                    AppSurface::Editor => self
                        .workspace
                        .editor
                        .view
                        .as_ref()
                        .and_then(|editor| editor.update(cx, |editor, cx| editor.snapshot(cx)).ok())
                        .or_else(|| {
                            self.workspace
                                .editor
                                .terminal_editor_view
                                .as_ref()
                                .and_then(|editor| {
                                    editor.update(cx, |editor, cx| editor.snapshot(cx)).ok()
                                })
                        }),
                    AppSurface::Terminal => {
                        self.workspace.terminal.view.as_ref().and_then(|terminal| {
                            terminal.update(cx, |terminal, _| terminal.snapshot()).ok()
                        })
                    }
                    AppSurface::Chat | AppSurface::Work => None,
                };
            }
        }
        self.hide_native_workspace_surfaces(cx);
    }

    pub(in crate::app) fn restore_active_native_workspace_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let overlay_active = self.native_surface_obscured(window, cx);
        if !overlay_active {
            self.workspace.native_surface_covered = false;
            if let Some(snapshot) = self.workspace.native_surface_snapshot.take() {
                let _ = window.drop_image(snapshot);
            }
        }
        if overlay_active {
            self.hide_native_workspace_surfaces(cx);
            return;
        }
        self.restore_editor_visibility(cx);
        self.restore_terminal_visibility(cx);
    }

    pub(in crate::app) fn workspace_project(&self) -> PathBuf {
        root_session_for_path(
            &self.sessions.all,
            self.snapshot.selected_session.as_deref(),
        )
        .map(|root| root.project.clone())
        .or_else(|| {
            let selected = self.sessions.selected_draft.as_deref()?;
            self.sessions
                .drafts
                .iter()
                .find(|draft| draft.id == selected)
                .map(|draft| draft.project.clone())
        })
        .unwrap_or_else(|| self.project.path.clone())
    }

    pub(in crate::app) fn capture_center_surface(&mut self) {
        let target = self.composer.sessions.current_target().to_owned();
        match self.workspace.surface {
            AppSurface::Editor | AppSurface::Terminal => {
                self.workspace
                    .session_surfaces
                    .insert(target, self.workspace.surface);
            }
            AppSurface::Chat | AppSurface::Work => {
                self.workspace.session_surfaces.remove(&target);
            }
        }
    }

    pub(in crate::app) fn restore_center_surface(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.view.sessions {
            self.apply_sheet_flags(sheet_flags(None));
            self.overlays.view.pending_setup = false;
            self.overlays.sheet_return_focus = None;
        }
        if self.workspace.surface == AppSurface::Work {
            return;
        }
        let surface = self
            .workspace
            .session_surfaces
            .get(self.composer.sessions.current_target())
            .copied()
            .unwrap_or(AppSurface::Chat);
        self.activate_chat_center(cx);
        match surface {
            AppSurface::Editor => self.activate_editor_for_project(project, window, cx),
            AppSurface::Terminal => self.activate_terminal_for_project(project, window, cx),
            AppSurface::Chat | AppSurface::Work => {}
        }
    }

    pub(in crate::app) fn promote_center_surface(&mut self, from: &str, to: &str) {
        if let Some(tab) = self.workspace.editor.session_tabs.remove(from) {
            self.workspace
                .editor
                .session_tabs
                .insert(to.to_owned(), tab);
        }
        if let Some(surface) = self.workspace.session_surfaces.remove(from) {
            self.workspace
                .session_surfaces
                .insert(to.to_owned(), surface);
        }
    }

    pub(in crate::app) fn activate_chat_center(&mut self, cx: &mut Context<Self>) {
        if self.native_workspace_covered_by_overlay() {
            if self.workspace.surface != AppSurface::Chat {
                self.hide_native_workspace_surfaces(cx);
                self.set_surface(AppSurface::Chat, cx);
            }
            return;
        }
        let _ = self.enter_chat_surface(self.chat_composer_focus(cx), cx);
    }

    pub(in crate::app) fn reveal_native_center_surface(
        &mut self,
        surface: AppSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_worker_picker_for_surface_switch(window, cx);
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
        self.overlays.post_render_focus = Some(PostRenderFocus::ActiveSurface(chat));
    }

    pub(in crate::app) fn apply_post_render_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.overlays.post_render_focus.take() else {
            return;
        };
        match request {
            PostRenderFocus::ImagePreview => {
                if self.overlays.image_preview.is_some() {
                    self.overlays.image_preview_focus.focus(window, cx);
                }
            }
            PostRenderFocus::ActiveSurface(chat) => {
                if self.native_workspace_covered_by_overlay() {
                    return;
                }
                match self.workspace.surface {
                    AppSurface::Chat => chat
                        .filter(|focus| self.navigation.chat.focus.contains(focus, window))
                        .unwrap_or_else(|| self.chat_composer_focus(cx))
                        .focus(window, cx),
                    AppSurface::Editor => {
                        if self.workspace.editor.ready
                            && let Some(editor) = self.workspace.editor.view.as_ref()
                        {
                            editor.update(cx, |editor, cx| editor.focus(window, cx));
                        } else if self.workspace.editor.ready
                            && let Some(editor) =
                                self.workspace.editor.terminal_editor_view.as_ref()
                        {
                            editor.update(cx, |editor, cx| editor.focus(window, cx));
                        }
                    }
                    AppSurface::Terminal => {
                        if let Some(terminal) = self.workspace.terminal.view.as_ref() {
                            terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
                        }
                    }
                    AppSurface::Work => {}
                }
            }
        }
    }

    pub(in crate::app) fn native_workspace_modal_active(&self) -> bool {
        self.workspace.send_to_chat.is_some()
            || self.navigation.picker.is_some()
            || self.overlays.view.sessions
            || self.overlays.view.run
            || self.overlays.view.keybindings
            || self.overlays.view.settings
            || self.overlays.view.project_trust
            || self.lifecycle.pending_quit.is_some()
            || self.sessions.pending_archive.is_some()
            || self.sessions.pending_delete.is_some()
            || self.sessions.import.is_some()
            || self.overlays.image_preview.is_some()
            || self.project.repository.pending_jj_init.is_some()
            || self.project.repository.edits.pending.is_some()
    }

    pub(in crate::app) fn native_workspace_covered_by_overlay(&self) -> bool {
        self.native_workspace_modal_active()
            || self
                .extensions
                .active
                .dialog
                .as_ref()
                .is_some_and(|dialog| {
                    !matches!(dialog, ExtensionUiRequest::WorkerModel { .. })
                        || self.workspace.runtime_picker.open
                })
    }

    pub(in crate::app) fn native_surface_obscured(&self, window: &Window, cx: &gpui::App) -> bool {
        self.native_workspace_covered_by_overlay()
            || gpui_base::GlobalState::is_in_deferred_context(cx)
            || gpui_component::Root::tooltip_overlay(window, cx)
                .is_some_and(|overlay| overlay.read(cx).is_visible())
    }

    pub(in crate::app) fn watch_tooltip_overlay(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.tooltip_watch.is_some() {
            return;
        }
        let Some(overlay) = gpui_component::Root::tooltip_overlay(window, cx) else {
            return;
        };
        self.workspace.tooltip_watch = Some(cx.observe(&overlay, |_, _, cx| cx.notify()));
    }

    pub(in crate::app) fn center_surface_switch_blocked(&self) -> bool {
        self.native_workspace_modal_active()
            || self
                .extensions
                .active
                .dialog
                .as_ref()
                .is_some_and(|dialog| !matches!(dialog, ExtensionUiRequest::WorkerModel { .. }))
    }

    pub(in crate::app) fn workspace_switch_blocked(&self) -> bool {
        self.center_surface_switch_blocked() || self.workspace.surface == AppSurface::Work
    }

    pub(in crate::app) fn cycle_workspace_surface(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_switch_blocked() {
            return;
        }
        let target = match (self.workspace.surface, forward) {
            (AppSurface::Chat, true) | (AppSurface::Terminal, false) => AppSurface::Editor,
            (AppSurface::Editor, true) | (AppSurface::Chat, false) => AppSurface::Terminal,
            (AppSurface::Terminal, true) | (AppSurface::Editor, false) => AppSurface::Chat,
            (AppSurface::Work, _) => return,
        };
        match target {
            AppSurface::Chat => self.show_chat_surface(window, cx),
            AppSurface::Editor => self.show_editor_surface(window, cx),
            AppSurface::Terminal => self.show_terminal_surface(window, cx),
            AppSurface::Work => {}
        }
    }

    pub(in crate::app) fn respond_value(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.extensions.dialog_input.read(cx).value().to_string();
        self.respond_dialog_value(id, value, window, cx);
    }

    pub(in crate::app) fn respond_dialog_value(
        &mut self,
        id: String,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.respond_to_restored_dialog(&id, value.clone(), window, cx) {
            return;
        }
        if let Some(response) = self.extensions.active.respond_value(&id, value) {
            self.send(RuntimeCommand::ExtensionResponse(response), cx);
            self.advance_or_restore_dialog(window, cx);
        }
    }

    pub(in crate::app) fn respond_confirm(
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
        if let Some(response) = self.extensions.active.respond_confirm(&id, confirmed) {
            self.send(RuntimeCommand::ExtensionResponse(response), cx);
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
        if self.extensions.restored_dialog_id.as_deref() != Some(id) {
            return false;
        }
        if !self.can_submit() {
            return true;
        }
        let _ = self.extensions.active.cancel(id);
        self.extensions.restored_dialog_id = None;
        self.extensions.dismissed_restored_dialog_id = Some(id.to_owned());
        self.submit(value, PromptMode::Normal, window, cx);
        true
    }

    pub(in crate::app) fn cancel_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.runtime_picker.worker.is_some() {
            self.set_runtime_picker_open(false, window, cx);
        }
        let Some(id) = self
            .extensions
            .active
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id)
            .map(str::to_owned)
        else {
            return;
        };
        if self.extensions.restored_dialog_id.as_deref() == Some(id.as_str()) {
            let _ = self.extensions.active.cancel(&id);
            self.extensions.restored_dialog_id = None;
            self.extensions.dismissed_restored_dialog_id = Some(id);
            self.advance_or_restore_dialog(window, cx);
        } else if let Some(response) = self.extensions.active.cancel(&id) {
            self.send(RuntimeCommand::ExtensionResponse(response), cx);
            self.advance_or_restore_dialog(window, cx);
        }
    }

    pub(in crate::app) fn advance_or_restore_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.extensions.active.dialog.is_some() {
            self.extensions.pending_dialog_setup = true;
            cx.notify();
        } else {
            self.extensions.dialog_input.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
            self.restore_dialog_focus(window, cx);
        }
    }

    fn restore_dialog_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.extensions.dialog_return_focus.take();
        self.restore_overlay_focus(target, &self.extensions.dialog_focus.clone(), window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn open_sessions_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sheet(AppSheet::Sessions, window, cx);
    }

    pub(in crate::app) fn open_run_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_sheet(AppSheet::Run, window, cx);
    }

    pub(in crate::app) fn open_worker_notices(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sheet(AppSheet::WorkerNotices, window, cx);
    }

    pub(in crate::app) fn toggle_workgraph_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.surface == AppSurface::Work {
            self.show_chat_surface(window, cx);
        } else {
            self.open_workgraph_surface(window, cx);
        }
    }

    pub(in crate::app) fn enter_chat_surface(
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

    pub(in crate::app) fn show_chat_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_worker_picker_for_surface_switch(window, cx);
        if self.workspace.surface == AppSurface::Chat
            && self.extensions.active.dialog.is_some()
            && !self.native_workspace_modal_active()
        {
            self.composer_region_focus(cx).focus(window, cx);
            self.notify_composer(cx);
            return;
        }
        if self.enter_chat_surface(self.composer.focus.clone(), cx) {
            self.views
                .workgraph
                .update(cx, |view, cx| view.prepare_open(window, cx));
        }
    }

    pub(in crate::app) fn open_workgraph_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_worker_picker_for_surface_switch(window, cx);
        self.hide_native_workspace_surfaces(cx);
        if self.overlays.view.run {
            self.close_sheet(window, cx);
        }
        if self.workspace.surface != AppSurface::Work {
            self.refresh_workgraph_board(cx);
            self.set_surface(AppSurface::Work, cx);
        }
        self.views
            .workgraph
            .update(cx, |view, cx| view.prepare_open(window, cx));
    }

    pub(in crate::app) fn open_workgraph_node(
        &mut self,
        number: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_workgraph_surface(window, cx);
        self.views
            .workgraph
            .update(cx, |view, cx| view.select_node(number, cx));
    }

    pub(in crate::app) fn close_workgraph_inspector(&mut self, cx: &mut Context<Self>) {
        if self.views.workgraph_inspector_issue.take().is_some() {
            cx.notify();
        }
    }

    fn refresh_workgraph_board(&mut self, cx: &mut Context<Self>) {
        let project = self.project.path.clone();
        let active_session = self.active_workgraph_session();
        let session_goal = self.active_root_session_goal();
        self.views.workgraph.update(cx, |view, cx| {
            view.refresh_for(project, active_session, session_goal, cx);
        });
    }

    pub(in crate::app) fn refresh_workgraph_sidebar(&mut self, cx: &mut Context<Self>) {
        let project = self.project.path.clone();
        let session_id = self
            .active_workgraph_session()
            .map(|(session_id, _)| session_id);
        let session_goal = self.active_root_session_goal();
        self.views.workgraph_sidebar.update(cx, |view, cx| {
            view.refresh_for(project, session_id, session_goal, cx);
        });
    }

    pub(in crate::app) fn refresh_workgraph_goal(&mut self, cx: &mut Context<Self>) {
        let Some(goal) = self.active_root_session_goal() else {
            return;
        };
        self.views
            .workgraph
            .update(cx, |view, cx| view.set_session_goal(goal.clone(), cx));
        self.views
            .workgraph_sidebar
            .update(cx, |view, cx| view.set_session_goal(goal, cx));
    }

    pub(in crate::app) fn active_workgraph_session(&self) -> Option<(String, String)> {
        root_session_for_path(
            &self.sessions.all,
            self.snapshot.selected_session.as_deref(),
        )
        .map(|root| (root.id.clone(), root.path.display().to_string()))
    }

    fn active_root_session_goal(&self) -> Option<Option<crate::agents::SessionGoal>> {
        let root = root_session_for_path(
            &self.sessions.all,
            self.snapshot.selected_session.as_deref(),
        )?;
        (self.snapshot.selected_session.as_deref() == Some(root.path.as_path()))
            .then(|| self.snapshot.session_goal.clone())
    }

    pub(in crate::app) fn close_sessions_sheet_after_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.view.sessions {
            self.close_sheet(window, cx);
        }
    }

    pub(in crate::app) fn open_keybindings_help(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sheet(AppSheet::Keybindings, window, cx);
    }

    pub(in crate::app) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.proxy_save.is_some() {
            self.save_settings_proxy(cx);
        }
        match crate::app::persistence::open().and_then(|store| crate::access::load_proxy(&*store)) {
            Ok(proxy) => {
                self.settings.network_proxy_input.update(cx, |input, cx| {
                    input.set_value(proxy.unwrap_or_default(), window, cx);
                });
                self.settings.network_proxy_error = None;
            }
            Err(error) => self.settings.network_proxy_error = Some(error),
        }
        if let Err(error) = self.load_worker_profile_settings() {
            self.workspace.worker_profile_editor.error = Some(error);
        }
        self.settings.mcp_error = None;
        self.settings.editor_error = None;
        self.open_sheet(AppSheet::Settings, window, cx);
    }

    pub(in crate::app) fn clear_network_proxy(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings
            .network_proxy_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.save_settings_proxy(cx);
    }

    pub(in crate::app) fn toggle_settings_builtin_mcp(&mut self, cx: &mut Context<Self>) {
        let enabled = !crate::builtin_mcp::enabled();
        match crate::app::mcp_server::set_enabled(enabled) {
            Ok(()) => {
                self.settings.mcp_error = None;
            }
            Err(error) => self.settings.mcp_error = Some(error),
        }
        cx.notify();
    }

    pub(in crate::app) fn toggle_settings_transcript_folders(&mut self, cx: &mut Context<Self>) {
        let expanded = !self.settings.expand_transcript_folders;
        match crate::app::persistence::open()
            .and_then(|store| store.save_expand_transcript_folders(expanded))
        {
            Ok(()) => {
                self.settings.expand_transcript_folders = expanded;
                self.settings.transcript_error = None;
                self.views.transcript.update(cx, |transcript, cx| {
                    transcript.list.remeasure_items(0..transcript.rows.len());
                    cx.notify();
                });
            }
            Err(error) => self.settings.transcript_error = Some(error),
        }
        cx.notify();
    }

    pub(in crate::app) fn toggle_group_by_project(&mut self, cx: &mut Context<Self>) {
        let grouped = !self.sessions.group_by_project;
        match crate::app::persistence::open()
            .and_then(|store| store.save_group_sessions_by_project(grouped))
        {
            Ok(()) => {
                self.sessions.group_by_project = grouped;
                self.sessions.error = None;
            }
            Err(error) => self.sessions.error = Some(error),
        }
        cx.notify();
    }

    pub(in crate::app) fn toggle_project_group(
        &mut self,
        project: &std::path::Path,
        cx: &mut Context<Self>,
    ) {
        if !self.sessions.collapsed_projects.remove(project) {
            self.sessions
                .collapsed_projects
                .insert(project.to_path_buf());
        }
        cx.notify();
    }

    pub(in crate::app) fn select_editor(
        &mut self,
        choice: crate::storage::EditorChoice,
        cx: &mut Context<Self>,
    ) {
        match crate::app::persistence::open().and_then(|store| store.save_editor_choice(choice)) {
            Ok(()) => {
                self.settings.editor_choice = choice;
                self.settings.editor_error = None;
            }
            Err(error) => self.settings.editor_error = Some(error),
        }
        cx.notify();
    }

    pub(in crate::app) fn save_settings_proxy(&mut self, cx: &mut Context<Self>) {
        self.settings.proxy_save = None;
        let value = self
            .settings
            .network_proxy_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let proxy = (!value.is_empty()).then_some(value);
        let result = crate::app::persistence::open().and_then(|store| {
            if store.load_network_proxy()? == proxy {
                return Ok(false);
            }
            store.save_network_proxy(proxy.as_deref())?;
            Ok(true)
        });
        match result {
            Ok(changed) => {
                self.settings.network_proxy_error = None;
                if changed {
                    self.send(RuntimeCommand::SetAppProxy(proxy), cx);
                }
            }
            Err(error) => {
                self.settings.network_proxy_error = Some(error);
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn schedule_settings_proxy_save(&mut self, cx: &mut Context<Self>) {
        self.settings.proxy_save = Some(cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            let _ = weak.update(cx, |this, cx| this.save_settings_proxy(cx));
        }));
    }

    pub(in crate::app) fn open_project_trust(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project.trust_error = None;
        self.project.trust_project = Some(self.project.path.clone());
        self.project.trust_backend = None;
        self.project.pending_trust_command = None;
        self.project.pending_trust_action = None;
        self.open_sheet(AppSheet::ProjectTrust, window, cx);
    }

    fn open_sheet(&mut self, sheet: AppSheet, window: &mut Window, cx: &mut Context<Self>) {
        self.cover_native_workspace_surface(cx);
        let picker_return = self
            .navigation
            .picker
            .take()
            .map(|_| self.navigation.picker_return_focus.take());
        if should_capture_return_focus(self.current_sheet_flags()) {
            self.overlays.sheet_return_focus = picker_return.unwrap_or_else(|| window.focused(cx));
        }
        self.apply_sheet_flags(sheet_flags(Some(sheet)));
        self.overlays.view.pending_setup = true;
        cx.notify();
    }

    fn current_sheet_flags(&self) -> SheetFlags {
        SheetFlags {
            sessions: self.overlays.view.sessions,
            run: self.overlays.view.run,
            worker_notices: self.overlays.view.worker_notices,
            keybindings: self.overlays.view.keybindings,
            settings: self.overlays.view.settings,
            project_trust: self.overlays.view.project_trust,
        }
    }

    fn apply_sheet_flags(&mut self, flags: SheetFlags) {
        self.overlays.view.sessions = flags.sessions;
        self.overlays.view.run = flags.run;
        self.overlays.view.worker_notices = flags.worker_notices;
        self.overlays.view.keybindings = flags.keybindings;
        self.overlays.view.settings = flags.settings;
        self.overlays.view.project_trust = flags.project_trust;
    }

    pub(crate) fn open_image_preview(
        &mut self,
        image: Arc<Image>,
        index: usize,
        total: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.image_preview.is_none() {
            self.overlays.image_preview_return_focus = window.focused(cx);
        }
        self.cover_native_workspace_surface(cx);
        self.overlays.image_preview = Some(ImagePreview {
            image,
            index,
            total,
        });
        self.overlays.post_render_focus = Some(PostRenderFocus::ImagePreview);
        cx.notify();
    }

    pub(in crate::app) fn close_image_preview(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.image_preview.take().is_none() {
            return;
        }
        let target = self.overlays.image_preview_return_focus.take();
        self.restore_overlay_focus(
            target,
            &self.overlays.image_preview_focus.clone(),
            window,
            cx,
        );
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.current_sheet_flags().any() {
            return;
        }
        if self.overlays.view.settings && self.settings.proxy_save.is_some() {
            self.save_settings_proxy(cx);
        }
        self.apply_sheet_flags(sheet_flags(None));
        self.overlays.view.pending_setup = false;
        let target = self.overlays.sheet_return_focus.take();
        self.restore_overlay_focus(target, &self.overlays.sheet_focus.clone(), window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn dismiss_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.lifecycle.pending_quit.is_some() {
            self.close_quit_confirmation(window, cx);
        } else if self.workspace.send_to_chat.is_some() && !self.overlays.view.project_trust {
            self.close_send_to_chat(window, cx);
        } else if self.overlays.image_preview.is_some() {
            self.close_image_preview(window, cx);
        } else if self.project.repository.edits.pending.is_some() {
            self.close_repository_edit(window, cx);
        } else if self.project.repository.pending_jj_init.is_some() {
            self.close_jj_init_confirmation(window, cx);
        } else if self.sessions.pending_delete.is_some() {
            self.close_delete_confirmation(window, cx);
        } else if self.sessions.pending_move.is_some() {
            self.close_move_confirmation(window, cx);
        } else if self.sessions.import.is_some() {
            self.close_session_import(window, cx);
        } else if self.sessions.pending_archive.is_some() {
            self.close_archive_confirmation(window, cx);
        } else if self.overlays.view.project_trust {
            self.dismiss_project_trust(window, cx);
        } else if self.overlays.view.sessions
            || self.overlays.view.run
            || self.overlays.view.keybindings
            || self.overlays.view.settings
        {
            self.close_sheet(window, cx);
        } else if self.navigation.pending_model_access.is_some() {
            self.close_model_access_confirmation(window, cx);
        } else if self.navigation.picker.is_some() {
            self.close_picker(window, cx);
        } else if self.workspace.runtime_picker.open
            && self.workspace.runtime_picker.worker.is_some()
        {
            self.set_runtime_picker_open(false, window, cx);
        } else if self.extensions.active.dialog.is_some() {
            self.cancel_dialog(window, cx);
        }
    }
}

impl Drop for FarcasterApp {
    fn drop(&mut self) {
        let _ = self.runtime.send(RuntimeCommand::Shutdown);
    }
}

#[cfg(test)]
#[path = "surfaces_tests.rs"]
mod tests;
