use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{
    App, Context, Entity, IntoElement as _, ParentElement as _, Render, RenderImage, Styled as _,
    Window, div,
};
use gpui_component::{
    Sizable as _, Size,
    button::Button,
};
use gpui_libghostty::TerminalOptions;

use super::{
    FarcasterApp, Terminal,
    editor::{EditorBackend, EditorRequest},
    external_editor,
};
use crate::app::infrastructure::editor_launch;

pub(super) struct HelixBackend;

impl EditorBackend for HelixBackend {
    fn name(&self) -> &'static str {
        "Helix"
    }

    fn program(&self, project: &Path) -> PathBuf {
        farcaster_editors::EditorChoice::Helix.program(project, None)
    }

    fn open(
        &self,
        app: &mut FarcasterApp,
        request: EditorRequest,
        window: &mut Window,
        cx: &mut Context<FarcasterApp>,
    ) -> Result<(), String> {
        let (project, title, args, temporary) = match request {
            EditorRequest::Project(project) => (project, "Project".to_owned(), vec![], None),
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
                    let base = external_editor::head_tempfile(&path, "Helix")?;
                    let args = vec![
                        "--vsplit".into(),
                        base.path().as_os_str().to_owned(),
                        path.into_os_string(),
                    ];
                    (project, format!("Diff: {title}"), args, Some(base))
                } else {
                    (project, title, vec![location(&path, line).into()], None)
                }
            }
            EditorRequest::Review {
                project, locations, ..
            } => {
                let args = locations
                    .iter()
                    .map(|(path, line)| location(path, *line).into())
                    .collect();
                (project, "Review".into(), args, None)
            }
        };
        app.activate_helix_editor(project, title, args, temporary, window, cx)
    }
}

fn location(path: &Path, line: Option<u64>) -> String {
    match line {
        Some(line) => format!("{}:{}", path.display(), line.max(1)),
        None => path.to_string_lossy().into_owned(),
    }
}

struct HelixTab {
    key: Vec<std::ffi::OsString>,
    title: String,
    terminal: Entity<Terminal>,
    _directory: tempfile::TempDir,
    _temporary: Option<tempfile::NamedTempFile>,
}

pub(in crate::app) struct HelixEditor {
    project: PathBuf,
    tabs: Vec<HelixTab>,
    active: usize,
    visible: bool,
}

impl HelixEditor {
    pub(super) fn new(project: PathBuf) -> Self {
        Self {
            project,
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
            .prefix("farcaster-helix-")
            .tempdir()
            .map_err(|error| format!("prepare Helix: {error}"))?;
        let launch_file = directory.path().join("launch.json");
        editor_launch::prepare(
            &launch_file,
            "hx".into(),
            arguments.clone(),
            self.project.clone(),
        )?;
        let executable =
            std::env::current_exe().map_err(|error| format!("resolve Helix launcher: {error}"))?;
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
        self.tabs.push(HelixTab {
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
        if self.visible {
            if let Some(tab) = self.tabs.get(self.active) {
                tab.terminal
                    .update(cx, |terminal, _| terminal.set_visible(true));
            }
        }
        cx.notify();
    }

    pub(super) fn retain_alive(&mut self, cx: &mut Context<Self>) -> bool {
        let previous_count = self.tabs.len();
        let active = self.tabs.get(self.active).map(|tab| tab.key.clone());
        self.tabs.retain(|tab| tab.terminal.read(cx).is_alive());
        self.active = active
            .and_then(|key| self.tabs.iter().position(|tab| tab.key == key))
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
            .ok_or("Helix has no open tab")?
            .terminal
            .update(cx, |terminal, _| terminal.snapshot())
    }
}

impl Render for HelixEditor {
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
                        Button::new(format!("helix-tab-{index}"))
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
