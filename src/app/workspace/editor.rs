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
        self.editor_request_generation = self.editor_request_generation.wrapping_add(1);
        if self.overlays.run {
            self.close_sheet(window, cx);
        }
        let project = self.workspace_project();
        let path = match resolve_editor_path(&project, &path) {
            Ok(path) => path,
            Err(error) => {
                self.notify_workspace_error("Neovim", error, cx);
                return;
            }
        };

        if let Some(editor) = self.reusable_editor(&project, cx) {
            let opened = editor.update(cx, |editor, cx| editor.open_file_at_line(path, line, cx));
            self.hide_terminal(cx);
            self.set_surface(AppSurface::Editor, cx);
            editor.update(cx, |editor, cx| editor.focus(window, cx));
            cx.notify();
            let generation = self.editor_request_generation;
            let target = self.composer_sessions.current_target().to_owned();
            cx.spawn(async move |weak, cx| {
                let Err(error) = opened.await else {
                    return;
                };
                zlog::warn!("Neovim file-open failed for {target}: {error}");
                let _ = weak.update(cx, |this, cx| {
                    if this.editor.as_ref() != Some(&editor)
                        || !editor_completion_is_current(
                            generation,
                            this.editor_request_generation,
                            &target,
                            this.composer_sessions.current_target(),
                            this.surface,
                        )
                    {
                        return;
                    }
                    this.notify_workspace_error("Neovim", error, cx);
                });
            })
            .detach();
            return;
        }

        let return_focus = window.focused(cx);
        if self.spawn_editor(nvim_options(project, path, line), window, cx) {
            if self.editor_return_focus.is_none() {
                self.editor_return_focus = return_focus;
            }
            self.hide_terminal(cx);
            self.set_surface(AppSurface::Editor, cx);
        }
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
        self.editor_request_generation = self.editor_request_generation.wrapping_add(1);
        if !self.repository.execution_allowed {
            self.notify_workspace_error(
                "Neovim",
                "Trust this project before opening Neovim.".to_owned(),
                cx,
            );
            return;
        }

        if self.reusable_editor(&project, cx).is_none()
            && !self.spawn_editor(nvim_options(project.clone(), project, None), window, cx)
        {
            return;
        }
        self.hide_terminal(cx);
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

    fn spawn_editor(
        &mut self,
        options: NvimOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match NvimEditor::spawn(options, window, cx) {
            Ok(editor) => {
                let editor = cx.new(|_| editor);
                self.editor = Some(editor.clone());
                self.monitor_native_process(window, cx, move |this, _window, cx| {
                    if this.editor.as_ref() != Some(&editor) {
                        return false;
                    }
                    if editor.read(cx).is_alive(cx) {
                        return true;
                    }
                    if this.surface == AppSurface::Editor {
                        this.close_editor(cx);
                    } else {
                        this.editor = None;
                        this.editor_return_focus = None;
                        this.request_repository_refresh(cx);
                    }
                    false
                });
                true
            }
            Err(error) => {
                self.notify_workspace_error("Neovim", error, cx);
                false
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
            && let Some(editor) = self.editor.as_ref()
        {
            editor.update(cx, |editor, cx| editor.set_visible(true, cx));
        }
    }

    pub(in crate::app) fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        let focus = self
            .editor_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone());
        self.enter_chat_surface(focus, cx);
        self.request_repository_refresh(cx);
    }
}

// The editor is shared across sessions. Entity identity alone does not establish
// ownership of a delayed file-open result, and completions must never navigate.
fn editor_completion_is_current(
    generation: u64,
    current_generation: u64,
    target: &str,
    current_target: &str,
    surface: AppSurface,
) -> bool {
    generation == current_generation && target == current_target && surface == AppSurface::Editor
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
    fn editor_completion_is_scoped_to_its_request_session_and_view() {
        assert!(editor_completion_is_current(
            1,
            1,
            "a",
            "a",
            AppSurface::Editor
        ));
        // Sessions in the same project can share the very same editor entity.
        assert!(!editor_completion_is_current(
            1,
            1,
            "a",
            "b",
            AppSurface::Editor
        ));
        for surface in [AppSurface::Chat, AppSurface::Terminal, AppSurface::Work] {
            assert!(!editor_completion_is_current(1, 1, "a", "a", surface));
        }
        // A newer open, reactivation, or leaving and returning invalidates it.
        assert!(!editor_completion_is_current(
            1,
            2,
            "a",
            "a",
            AppSurface::Editor
        ));
    }

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
