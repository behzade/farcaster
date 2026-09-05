use std::path::PathBuf;

use gpui::{Context, Window};
use gpui_libghostty::{Terminal, TerminalConfiguration, TerminalOptions};

use super::{AppSurface, FarcasterApp};

impl FarcasterApp {
    pub(in crate::app) fn show_terminal_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.center_surface_switch_blocked() {
            return;
        }
        self.activate_terminal_for_project(self.workspace_project(), window, cx);
    }

    pub(in crate::app) fn activate_terminal_for_project(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.repository.execution_allowed {
            self.notify_workspace_error(
                "Terminal",
                "Trust this project before opening its terminal.".to_owned(),
                cx,
            );
            return;
        }

        let can_reuse = self.terminal_project.as_ref() == Some(&project)
            && self
                .terminal
                .as_ref()
                .is_some_and(|terminal| terminal.read(cx).is_alive());
        if !can_reuse {
            let mut options = TerminalOptions::new(
                crate::app::infrastructure::shell_environment::terminal_login_shell_command(),
                project.clone(),
            );
            options.configuration =
                TerminalConfiguration::Custom(crate::app::ui::theme::terminal_theme());
            match Terminal::spawn(options, window, cx) {
                Ok(terminal) => {
                    self.terminal = Some(terminal.clone());
                    self.terminal_project = Some(project);
                    self.monitor_native_process(window, cx, move |this, window, cx| {
                        if this.terminal.as_ref() != Some(&terminal) {
                            return false;
                        }
                        if terminal.read(cx).is_alive() {
                            return true;
                        }
                        if this.surface == AppSurface::Terminal {
                            this.close_terminal(window, cx);
                        } else {
                            this.clear_terminal_process();
                        }
                        false
                    });
                }
                Err(error) => {
                    self.notify_workspace_error("Terminal", error, cx);
                    return;
                }
            }
        }

        self.hide_editor(cx);
        self.reveal_native_center_surface(AppSurface::Terminal, window, cx);
    }

    fn clear_terminal_process(&mut self) {
        self.terminal = None;
        self.terminal_project = None;
    }

    pub(in crate::app) fn hide_terminal(&self, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminal.as_ref() {
            terminal.update(cx, |terminal, _| terminal.set_visible(false));
        }
    }

    pub(in crate::app) fn restore_terminal_visibility(&self, cx: &mut Context<Self>) {
        if self.surface == AppSurface::Terminal
            && let Some(terminal) = self.terminal.as_ref()
        {
            terminal.update(cx, |terminal, _| terminal.set_visible(true));
        }
    }

    pub(in crate::app) fn close_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_terminal_process();
        self.show_chat_surface(window, cx);
    }
}
