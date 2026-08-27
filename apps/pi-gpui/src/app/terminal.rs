//! Project terminal lifecycle and shell selection.

use std::path::PathBuf;

use gpui::{Context, Window};
use gpui_libghostty::{Terminal, TerminalOptions};

use super::{AppSurface, PiApp};

impl PiApp {
    pub(super) fn show_terminal_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace_switch_blocked() {
            return;
        }
        self.activate_terminal_for_project(self.workspace_project(), window, cx);
    }

    pub(super) fn activate_terminal_for_project(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_editor(cx);

        if !self.repository.execution_allowed {
            self.clear_terminal_process();
            self.terminal_error =
                Some("Trust this project before opening its terminal, then restart Pi.".to_owned());
            self.surface = AppSurface::Terminal;
            cx.notify();
            return;
        }

        let can_reuse = self.terminal_project.as_ref() == Some(&project)
            && self
                .terminal
                .as_ref()
                .is_some_and(|terminal| terminal.read(cx).is_alive());
        if can_reuse {
            self.terminal_error = None;
        } else {
            self.clear_terminal_process();
            let options = TerminalOptions::new(
                crate::shell_environment::terminal_login_shell_command(),
                project.clone(),
            );
            match Terminal::spawn(options, window, cx) {
                Ok(terminal) => {
                    self.terminal = Some(terminal);
                    self.terminal_project = Some(project);
                    self.terminal_error = None;
                }
                Err(error) => self.terminal_error = Some(error),
            }
        }

        self.reveal_native_center_surface(AppSurface::Terminal, window, cx);
    }

    fn clear_terminal_process(&mut self) {
        self.terminal = None;
        self.terminal_project = None;
    }

    pub(super) fn hide_terminal(&self, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminal.as_ref() {
            terminal.update(cx, |terminal, _| terminal.set_visible(false));
        }
    }

    pub(super) fn restore_terminal_visibility(&self, cx: &mut Context<Self>) {
        if self.surface == AppSurface::Terminal
            && let Some(terminal) = self.terminal.as_ref()
        {
            terminal.update(cx, |terminal, _| terminal.set_visible(true));
        }
    }

    pub(super) fn close_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_terminal_process();
        self.terminal_error = None;
        self.show_chat_surface(window, cx);
    }
}
