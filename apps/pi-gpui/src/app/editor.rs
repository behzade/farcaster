//! In-app editor orchestration and project-file boundary checks.

use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context, Window};
use gpui_neovim::{NvimEditor, NvimOptions};

use super::{AppSurface, PiApp};

impl PiApp {
    pub(crate) fn open_file_editor(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file_editor_at_line(path, None, window, cx);
    }

    pub(crate) fn open_file_editor_at_line(
        &mut self,
        path: PathBuf,
        line: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.run_sheet {
            self.close_sheet(window, cx);
        }
        self.hide_terminal(cx);
        let project = self.workspace_project();
        let path = match resolve_editor_path(&project, &path) {
            Ok(path) => path,
            Err(error) => {
                self.hide_editor(cx);
                self.editor_error = Some(error);
                self.surface = AppSurface::Editor;
                cx.notify();
                return;
            }
        };

        if let Some(editor) = self.reusable_editor(&project, cx) {
            let opened = editor.update(cx, |editor, cx| editor.open_file_at_line(path, line, cx));
            match opened {
                Ok(()) => {
                    self.editor_error = None;
                    self.surface = AppSurface::Editor;
                    editor.update(cx, |editor, cx| editor.focus(window, cx));
                }
                Err(error) => {
                    editor.update(cx, |editor, cx| editor.set_visible(false, cx));
                    self.editor_error = Some(error);
                    self.surface = AppSurface::Editor;
                }
            }
            cx.notify();
            return;
        }

        if self.editor_return_focus.is_none() {
            self.editor_return_focus = window.focused(cx);
        }
        self.spawn_editor(nvim_options(project, path, line), window, cx);
        self.surface = AppSurface::Editor;
        cx.notify();
    }

    pub(super) fn show_editor_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace_switch_blocked() {
            return;
        }
        self.hide_terminal(cx);
        let project = self.workspace_project();
        if !self.repository.execution_allowed {
            self.editor = None;
            self.editor_error =
                Some("Trust this project before opening Neovim, then restart Pi.".to_owned());
            self.surface = AppSurface::Editor;
            cx.notify();
            return;
        }

        if self.reusable_editor(&project, cx).is_some() {
            self.editor_error = None;
        } else {
            self.spawn_editor(nvim_options(project.clone(), project, None), window, cx);
        }
        self.surface = AppSurface::Editor;
        if let Some(editor) = self.editor.as_ref() {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
        }
        cx.notify();
    }

    fn reusable_editor(&self, project: &Path, cx: &gpui::App) -> Option<gpui::Entity<NvimEditor>> {
        self.editor
            .as_ref()
            .filter(|editor| {
                let state = editor.read(cx);
                state.project() == project && state.is_alive(cx)
            })
            .cloned()
    }

    fn spawn_editor(&mut self, options: NvimOptions, window: &mut Window, cx: &mut Context<Self>) {
        match NvimEditor::spawn(options, window, cx) {
            Ok(editor) => {
                self.editor = Some(cx.new(|_| editor));
                self.editor_error = None;
            }
            Err(error) => {
                self.editor = None;
                self.editor_error = Some(error);
            }
        }
    }

    pub(super) fn hide_editor(&self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.as_ref() {
            editor.update(cx, |editor, cx| editor.set_visible(false, cx));
        }
    }

    pub(super) fn restore_editor_visibility(&self, cx: &mut Context<Self>) {
        if self.surface == AppSurface::Editor
            && self.editor_error.is_none()
            && let Some(editor) = self.editor.as_ref()
        {
            editor.update(cx, |editor, cx| editor.set_visible(true, cx));
        }
    }

    pub(super) fn close_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        self.editor_error = None;
        self.surface = AppSurface::Chat;
        self.editor_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone())
            .focus(window, cx);
        self.request_repository_refresh(cx);
        cx.notify();
    }
}

fn nvim_options(project: PathBuf, path: PathBuf, line: Option<u64>) -> NvimOptions {
    let mut options = NvimOptions::new(project, path);
    options.initial_line = line;
    if let Some(executable) = std::env::var_os("PI_GUI_NVIM") {
        options.executable = executable.into();
    }
    options
}

fn resolve_editor_path(project: &Path, path: &Path) -> Result<PathBuf, String> {
    let project = project
        .canonicalize()
        .map_err(|error| format!("resolve editor project {}: {error}", project.display()))?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project.join(path)
    };
    match std::fs::symlink_metadata(&candidate) {
        Ok(_) => {
            let candidate = candidate
                .canonicalize()
                .map_err(|error| format!("open {}: {error}", candidate.display()))?;
            if !candidate.starts_with(&project) {
                return Err(format!(
                    "refusing to open a file outside the project: {}",
                    candidate.display()
                ));
            }
            if !candidate.is_file() {
                return Err(format!(
                    "editor target is not a file: {}",
                    candidate.display()
                ));
            }
            return Ok(candidate);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("open {}: {error}", candidate.display())),
    }

    let file_name = candidate
        .file_name()
        .ok_or_else(|| format!("editor target is not a file: {}", candidate.display()))?;
    let parent = candidate
        .parent()
        .ok_or_else(|| format!("editor target has no parent: {}", candidate.display()))?
        .canonicalize()
        .map_err(|error| format!("open {}: {error}", candidate.display()))?;
    if !parent.starts_with(&project) {
        return Err(format!(
            "refusing to open a file outside the project: {}",
            candidate.display()
        ));
    }
    Ok(parent.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn editor_paths_are_confined_to_the_selected_project() -> Result<(), Box<dyn std::error::Error>>
    {
        let project = tempdir()?;
        let file = project.path().join("src.rs");
        std::fs::write(&file, "fn main() {}")?;
        assert_eq!(
            resolve_editor_path(project.path(), Path::new("src.rs"))?,
            file.canonicalize()?
        );
        assert_eq!(
            resolve_editor_path(project.path(), Path::new("deleted.rs"))?,
            project.path().canonicalize()?.join("deleted.rs")
        );

        let outside = tempdir()?;
        let outside_file = outside.path().join("outside.rs");
        std::fs::write(&outside_file, "")?;
        assert!(resolve_editor_path(project.path(), &outside_file).is_err());

        #[cfg(unix)]
        {
            let dangling = project.path().join("dangling.rs");
            std::os::unix::fs::symlink(outside.path().join("missing.rs"), &dangling)?;
            assert!(resolve_editor_path(project.path(), &dangling).is_err());
        }
        Ok(())
    }
}
