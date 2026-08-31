use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context, Window};
use gpui_neovim::{NvimEditor, NvimOptions};

use super::{AppSurface, FarcasterApp};

impl FarcasterApp {
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
                self.set_surface(AppSurface::Editor, cx);
                return;
            }
        };

        if let Some(editor) = self.reusable_editor(&project, cx) {
            let opened = editor.update(cx, |editor, cx| editor.open_file_at_line(path, line, cx));
            self.editor_error = None;
            self.set_surface(AppSurface::Editor, cx);
            editor.update(cx, |editor, cx| editor.focus(window, cx));
            cx.notify();
            cx.spawn(async move |weak, cx| {
                let Err(error) = opened.await else {
                    return;
                };
                let _ = weak.update(cx, |this, cx| {
                    if this.editor.as_ref() != Some(&editor) {
                        return;
                    }
                    editor.update(cx, |editor, cx| editor.set_visible(false, cx));
                    this.editor_error = Some(error);
                    this.set_surface(AppSurface::Editor, cx);
                    cx.notify();
                });
            })
            .detach();
            return;
        }

        if self.editor_return_focus.is_none() {
            self.editor_return_focus = window.focused(cx);
        }
        self.spawn_editor(nvim_options(project, path, line), window, cx);
        self.set_surface(AppSurface::Editor, cx);
    }

    pub(in crate::app) fn show_editor_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.center_surface_switch_blocked() {
            return;
        }
        self.activate_editor_for_project(self.workspace_project(), window, cx);
    }

    pub(in crate::app) fn activate_editor_for_project(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_terminal(cx);
        if !self.repository.execution_allowed {
            self.editor = None;
            self.editor_error =
                Some("Trust this project before opening Neovim, then restart Pi.".to_owned());
            self.set_surface(AppSurface::Editor, cx);
            return;
        }

        if self.reusable_editor(&project, cx).is_some() {
            self.editor_error = None;
        } else {
            self.spawn_editor(nvim_options(project.clone(), project, None), window, cx);
        }
        self.reveal_native_center_surface(AppSurface::Editor, window, cx);
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

    pub(in crate::app) fn hide_editor(&self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.as_ref() {
            editor.update(cx, |editor, cx| editor.set_visible(false, cx));
        }
    }

    pub(in crate::app) fn restore_editor_visibility(&self, cx: &mut Context<Self>) {
        if self.surface == AppSurface::Editor
            && self.editor_error.is_none()
            && let Some(editor) = self.editor.as_ref()
        {
            editor.update(cx, |editor, cx| editor.set_visible(true, cx));
        }
    }

    pub(in crate::app) fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        self.editor_error = None;
        let focus = self
            .editor_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone());
        self.enter_chat_surface(focus, cx);
        self.request_repository_refresh(cx);
    }
}

fn nvim_options(project: PathBuf, path: PathBuf, line: Option<u64>) -> NvimOptions {
    let mut options = NvimOptions::new(project, path);
    options.initial_line = line;
    if let Some(executable) = std::env::var_os("FARCASTER_NVIM") {
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
    Ok(parent.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn editor_paths_allow_targets_outside_the_selected_project()
    -> Result<(), Box<dyn std::error::Error>> {
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
        assert_eq!(
            resolve_editor_path(project.path(), &outside_file)?,
            outside_file.canonicalize()?
        );
        let new_outside_file = outside.path().join("new.rs");
        assert_eq!(
            resolve_editor_path(project.path(), &new_outside_file)?,
            outside.path().canonicalize()?.join("new.rs")
        );

        #[cfg(unix)]
        {
            let dangling = project.path().join("dangling.rs");
            std::os::unix::fs::symlink(outside.path().join("missing.rs"), &dangling)?;
            assert!(resolve_editor_path(project.path(), &dangling).is_err());
        }
        Ok(())
    }
}
