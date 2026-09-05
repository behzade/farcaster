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
        let project = project.canonicalize().unwrap_or(project);
        self.select_editor_for_project(project.clone(), cx);
        let reusable = self.editor.as_ref().is_some_and(|editor| {
            editor.read(cx).project() == project && editor.read(cx).is_alive(cx)
        });
        if !reusable && !self.spawn_editor(project, window, cx) {
            return;
        }
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let target = self.composer_sessions.current_target().to_owned();
        let tab = *self
            .session_editor_tabs
            .entry(target.clone())
            .or_insert_with(new_session_tab);
        self.editor_request_generation = self.editor_request_generation.wrapping_add(1);
        self.editor_ready = false;
        self.hide_editor(cx);
        self.hide_terminal(cx);
        if self.editor_return_focus.is_none() {
            self.editor_return_focus = window.focused(cx);
        }
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

    pub(in crate::app) fn select_editor_for_project(
        &mut self,
        project: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let project = project.canonicalize().unwrap_or(project);
        // Hide even when the project/entity is unchanged: its active tab still
        // belongs to the previous session until the queued request completes.
        self.hide_editor(cx);
        self.editor = self.project_editors.get(&project).cloned();
        self.editor_ready = false;
        self.editor_return_focus = None;
        self.editor_request_generation = self.editor_request_generation.wrapping_add(1);
    }

    fn spawn_editor(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match NvimEditor::spawn(project.clone(), window, cx) {
            Ok(editor) => {
                let editor = cx.new(|_| editor);
                self.project_editors.insert(project, editor.clone());
                self.editor = Some(editor.clone());
                self.monitor_native_process(window, cx, move |this, _window, cx| {
                    if !this.project_editors.values().any(|entry| entry == &editor) {
                        return false;
                    }
                    if editor.read(cx).is_alive(cx) {
                        return true;
                    }
                    this.project_editors.retain(|_, entry| entry != &editor);
                    if this.editor.as_ref() != Some(&editor) {
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
            .unwrap_or_else(|| self.composer_focus.clone());
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
        // Sessions share an entity, but not ownership of delayed completions.
        assert!(!editor_completion_is_current(
            1,
            1,
            11,
            Some(22),
            AppSurface::Editor
        ));
        assert!(!editor_completion_is_current(
            1,
            1,
            11,
            None,
            AppSurface::Editor
        ));
        for surface in [AppSurface::Chat, AppSurface::Terminal, AppSurface::Work] {
            assert!(!editor_completion_is_current(1, 1, 11, Some(11), surface));
        }
        assert!(!editor_completion_is_current(
            1,
            2,
            11,
            Some(11),
            AppSurface::Editor
        ));
        // Draft promotion changes the routing key, not the native tab identity.
        let mut tabs = std::collections::HashMap::from([("draft", 11)]);
        let tab = tabs.remove("draft").expect("draft has a tab");
        tabs.insert("session", tab);
        assert!(editor_completion_is_current(
            1,
            1,
            11,
            tabs.get("session").copied(),
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
