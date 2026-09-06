use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context, Window};

use super::{
    AppSurface, FarcasterApp,
    neovim::{NvimEditor, new_session_tab},
};

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
        self.activate_editor_tab(project, Some(path), line, window, cx);
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
        self.activate_editor_tab(project, None, None, window, cx);
    }

    fn activate_editor_tab(
        &mut self,
        project: PathBuf,
        path: Option<PathBuf>,
        line: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.repository.execution_allowed {
            self.notify_workspace_error(
                "Neovim",
                "Trust this project before opening Neovim.".to_owned(),
                cx,
            );
            return;
        }
        // Hide the previous session's tab until this request completes, even
        // when both sessions use the same project server.
        self.hide_editor(cx);
        self.editor = None;
        self.editor_ready = false;
        self.editor_return_focus = window.focused(cx);
        self.editor_request_generation = self.editor_request_generation.wrapping_add(1);

        let project = project.canonicalize().unwrap_or(project);
        let Some(editor) = self
            .project_editors
            .get(&project)
            .filter(|editor| editor.read(cx).is_alive(cx))
            .cloned()
            .or_else(|| self.spawn_editor(project, window, cx))
        else {
            return;
        };
        self.editor = Some(editor.clone());
        let target = self.composer_sessions.current_target().to_owned();
        let tab = *self
            .session_editor_tabs
            .entry(target.clone())
            .or_insert_with(new_session_tab);
        self.hide_terminal(cx);
        self.set_surface(AppSurface::Editor, cx);
        let generation = self.editor_request_generation;
        let opened = editor.update(cx, |editor, cx| editor.activate_tab(tab, path, line, cx));
        cx.spawn_in(window, async move |weak, cx| {
            let result = opened.await;
            if let Err(error) = &result {
                zlog::warn!("Neovim session-view request failed for {target}: {error}");
            }
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.editor.as_ref() != Some(&editor)
                    || !editor_completion_is_current(
                        generation,
                        this.editor_request_generation,
                        tab,
                        this.session_editor_tabs
                            .get(this.composer_sessions.current_target())
                            .copied(),
                        this.surface,
                    )
                {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.editor_ready = true;
                        this.reveal_native_center_surface(AppSurface::Editor, window, cx);
                    }
                    Err(error) => this.notify_workspace_error("Neovim", error, cx),
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn spawn_editor(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<NvimEditor>> {
        match NvimEditor::spawn(project.clone(), window, cx) {
            Ok(editor) => {
                let editor = cx.new(|_| editor);
                self.project_editors.insert(project.clone(), editor.clone());
                let monitored = editor.clone();
                self.monitor_native_process(window, cx, move |this, _window, cx| {
                    if this.project_editors.get(&project) != Some(&monitored) {
                        return false;
                    }
                    if monitored.read(cx).is_alive(cx) {
                        return true;
                    }
                    this.project_editors.remove(&project);
                    if this.editor.as_ref() != Some(&monitored) {
                        return false;
                    }
                    if this.surface == AppSurface::Editor {
                        this.close_editor(cx);
                    } else {
                        this.editor = None;
                        this.editor_ready = false;
                        this.editor_return_focus = None;
                        this.request_repository_refresh(cx);
                    }
                    false
                });
                Some(editor)
            }
            Err(error) => {
                self.notify_workspace_error("Neovim", error, cx);
                None
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
            && self.editor_ready
            && let Some(editor) = self.editor.as_ref()
        {
            editor.update(cx, |editor, cx| editor.set_visible(true, cx));
        }
    }

    pub(in crate::app) fn close_editor(&mut self, cx: &mut Context<Self>) {
        // Closing a surface must not kill other sessions' tabs or shared unsaved
        // buffers. Keep the project server until Neovim exits (or the app does).
        self.hide_editor(cx);
        self.editor = None;
        self.editor_ready = false;
        let focus = self
            .editor_return_focus
            .take()
            .unwrap_or_else(|| self.preferred_chat_focus());
        self.enter_chat_surface(focus, cx);
        self.request_repository_refresh(cx);
    }
}

// Returning to an editor does not establish ownership of an older delayed
// request, and completions must never navigate back from another surface.
fn editor_completion_is_current(
    generation: u64,
    current_generation: u64,
    tab: u64,
    current_tab: Option<u64>,
    surface: AppSurface,
) -> bool {
    generation == current_generation && Some(tab) == current_tab && surface == AppSurface::Editor
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
            11,
            Some(11),
            AppSurface::Editor
        ));
        // A shared process is not enough: the request, tab, and view must match.
        for (generation, tab, surface) in [
            (2, Some(11), AppSurface::Editor),
            (1, Some(22), AppSurface::Editor),
            (1, None, AppSurface::Editor),
            (1, Some(11), AppSurface::Chat),
            (1, Some(11), AppSurface::Terminal),
            (1, Some(11), AppSurface::Work),
        ] {
            assert!(!editor_completion_is_current(
                1, generation, 11, tab, surface
            ));
        }
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
