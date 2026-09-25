use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{
    Context, Entity, IntoElement, ParentElement as _, Render, RenderImage, Styled as _, Window, div,
};
use gpui_component::{Sizable as _, Size, button::Button};
use gpui_libghostty::TerminalOptions;

use super::{
    FarcasterApp, Terminal,
    editor::{EditorBackend, EditorRequest},
    external_editor,
};
use crate::{app::infrastructure::editor_launch, storage::EditorChoice};

pub(super) struct TerminalBackend(pub(super) EditorChoice);

impl EditorBackend for TerminalBackend {
    fn open(
        &self,
        app: &mut FarcasterApp,
        request: EditorRequest,
        window: &mut Window,
        cx: &mut Context<FarcasterApp>,
    ) -> Result<(), String> {
        let choice = self.0;
        let vim = choice == EditorChoice::Vim;
        let (project, title, args, temporary) = match request {
            EditorRequest::Project(project) => {
                let args = if vim {
                    vec![project.clone().into_os_string()]
                } else {
                    vec![]
                };
                (project, "Project".to_owned(), args, None)
            }
            EditorRequest::File {
                project,
                path,
                line,
                diff,
            } => {
                let title = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "File".into());
                if diff {
                    let base = external_editor::head_tempfile(&path, choice.label())?;
                    let mut args = vec![if vim { "-d".into() } else { "--vsplit".into() }];
                    args.extend([base.path().as_os_str().to_owned(), path.into_os_string()]);
                    (project, format!("Diff: {title}"), args, Some(base))
                } else {
                    let args = if vim {
                        let mut args = Vec::new();
                        if let Some(line) = line {
                            args.push(format!("+{}", line.max(1)).into());
                        }
                        args.push(path.into_os_string());
                        args
                    } else {
                        vec![external_editor::location(&path, line).into()]
                    };
                    (project, title, args, None)
                }
            }
            EditorRequest::Review {
                project, locations, ..
            } => {
                let args = if vim {
                    let lines = locations
                        .iter()
                        .map(|(_, line)| line.unwrap_or(1).max(1).to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    let mut args = vec!["-p".into()];
                    if !locations.is_empty() {
                        args.extend([
                            "-c".into(),
                            format!("tabdo call cursor([{lines}][tabpagenr()-1], 1)").into(),
                            "-c".into(),
                            "tabfirst".into(),
                        ]);
                    }
                    args.extend(locations.into_iter().map(|(path, _)| path.into_os_string()));
                    args
                } else {
                    locations
                        .iter()
                        .map(|(path, line)| external_editor::location(path, *line).into())
                        .collect()
                };
                (project, "Review".into(), args, None)
            }
        };
        app.activate_terminal_editor(project, choice, title, args, temporary, window, cx)
    }
}

struct TerminalTab {
    key: Vec<std::ffi::OsString>,
    title: String,
    terminal: Entity<Terminal>,
    _directory: tempfile::TempDir,
    _temporary: Option<tempfile::NamedTempFile>,
}

pub(in crate::app) struct TerminalEditor {
    project: PathBuf,
    choice: EditorChoice,
    tabs: Vec<TerminalTab>,
    active: usize,
    visible: bool,
}

impl TerminalEditor {
    pub(super) fn new(project: PathBuf, choice: EditorChoice) -> Self {
        Self {
            project,
            choice,
            tabs: Vec::new(),
            active: 0,
            visible: false,
        }
    }

    pub(super) fn open(
        &mut self,
        arguments: Vec<std::ffi::OsString>,
        title: String,
        temporary: Option<tempfile::NamedTempFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.key == arguments && tab.terminal.read(cx).is_alive())
        {
            self.select(index, cx);
            return Ok(());
        }
        let directory = tempfile::Builder::new()
            .prefix("farcaster-editor-")
            .tempdir()
            .map_err(|error| format!("prepare {}: {error}", self.choice.label()))?;
        let launch_file = directory.path().join("launch.json");
        editor_launch::prepare(
            &launch_file,
            self.choice
                .program(&self.project, std::env::var_os("PATH").as_deref()),
            arguments.clone(),
            self.project.clone(),
        )?;
        let executable = std::env::current_exe()
            .map_err(|error| format!("resolve {} launcher: {error}", self.choice.label()))?;
        let command = format!(
            "{} {} {}",
            shell_quote(&executable),
            editor_launch::ARGUMENT,
            shell_quote(&launch_file)
        );
        let terminal = Terminal::spawn(
            TerminalOptions::new(command, self.project.clone()),
            window,
            cx,
        )?;
        terminal.update(cx, |terminal, _| terminal.set_visible(false));
        self.tabs.push(TerminalTab {
            key: arguments,
            title,
            terminal,
            _directory: directory,
            _temporary: temporary,
        });
        self.select(self.tabs.len() - 1, cx);
        Ok(())
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.terminal
                .update(cx, |terminal, _| terminal.set_visible(false));
        }
        self.active = index;
        if self.visible
            && let Some(tab) = self.tabs.get(self.active)
        {
            tab.terminal
                .update(cx, |terminal, _| terminal.set_visible(true));
        }
        cx.notify();
    }

    pub(super) fn retain_alive(&mut self, cx: &mut Context<Self>) -> bool {
        let previous_count = self.tabs.len();
        let active = self.tabs.get(self.active).map(|tab| tab.terminal.clone());
        self.tabs.retain(|tab| tab.terminal.read(cx).is_alive());
        self.active = active
            .and_then(|terminal| self.tabs.iter().position(|tab| tab.terminal == terminal))
            .unwrap_or_else(|| self.tabs.len().saturating_sub(1));
        if self.tabs.len() != previous_count {
            self.apply_visibility(cx);
            cx.notify();
        }
        !self.tabs.is_empty()
    }

    pub(super) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.terminal
                .update(cx, |terminal, cx| terminal.focus(window, cx));
        }
    }

    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        self.apply_visibility(cx);
    }

    fn apply_visibility(&self, cx: &mut Context<Self>) {
        for (index, tab) in self.tabs.iter().enumerate() {
            tab.terminal.update(cx, |terminal, _| {
                terminal.set_visible(self.visible && index == self.active)
            });
        }
    }

    pub(super) fn snapshot(&mut self, cx: &mut Context<Self>) -> Result<Arc<RenderImage>, String> {
        self.tabs
            .get(self.active)
            .ok_or_else(|| format!("{} has no open tab", self.choice.label()))?
            .terminal
            .update(cx, |terminal, _| terminal.snapshot())
    }
}

impl Render for TerminalEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let active = self.active;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap(gpui::px(4.0))
                    .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                        let entity = entity.clone();
                        Button::new(format!("editor-tab-{index}"))
                            .label(tab.title.clone())
                            .with_size(Size::Small)
                            .toggled(index == active)
                            .on_click(move |_, window, cx| {
                                let _ = entity.update(cx, |editor, cx| {
                                    editor.select(index, cx);
                                    editor.focus(window, cx);
                                });
                            })
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .children(self.tabs.get(self.active).map(|tab| tab.terminal.clone())),
            )
    }
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
