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

        let project = project.canonicalize().unwrap_or(project);
        let cached = self
            .project_terminals
            .get(&project)
            .filter(|terminal| terminal.read(cx).is_alive())
            .cloned();
        let terminal = if let Some(terminal) = cached {
            terminal
        } else {
            let mut options = TerminalOptions::new(
                crate::app::infrastructure::shell_environment::terminal_login_shell_command(),
                project.clone(),
            );
            options.configuration =
                TerminalConfiguration::Custom(crate::app::ui::theme::terminal_theme());
            let terminal = match Terminal::spawn(options, window, cx) {
                Ok(terminal) => terminal,
                Err(error) => {
                    self.notify_workspace_error("Terminal", error, cx);
                    return;
                }
            };
            self.project_terminals
                .insert(project.clone(), terminal.clone());
            let monitored = terminal.downgrade();
            let monitored_project = project.clone();
            self.monitor_native_process(window, cx, move |this, window, cx| {
                let Some(monitored) = monitored.upgrade() else {
                    return false;
                };
                if this.project_terminals.get(&monitored_project) != Some(&monitored) {
                    return false;
                }
                if monitored.read(cx).is_alive() {
                    return true;
                }
                if this.terminal.as_ref() != Some(&monitored) {
                    this.project_terminals.remove(&monitored_project);
                } else if this.surface == AppSurface::Terminal {
                    this.close_terminal(window, cx);
                } else {
                    this.clear_terminal_process();
                }
                false
            });
            terminal
        };

        self.hide_terminal(cx);
        self.terminal = Some(terminal);
        self.terminal_project = Some(project);
        self.hide_editor(cx);
        self.reveal_native_center_surface(AppSurface::Terminal, window, cx);
    }

    fn clear_terminal_process(&mut self) {
        if let Some(project) = self.terminal_project.take() {
            self.project_terminals.remove(&project);
        }
        self.terminal = None;
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
        self.hide_terminal(cx);
        self.clear_terminal_process();
        self.show_chat_surface(window, cx);
    }
}
