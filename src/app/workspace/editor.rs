use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context, Window};

use super::{
    AppSurface, FarcasterApp,
    neovim::{EditorTarget, NvimEditor, new_session_tab},
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
        self.open_file_editor_with_diff(path, line, false, window, cx);
    }

    pub(crate) fn open_file_editor_with_diff(
        &mut self,
        path: PathBuf,
        line: Option<u64>,
        diff: bool,
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
        let target = if diff {
            EditorTarget::Diff(path, line)
        } else {
            EditorTarget::File(path, line)
        };
        self.activate_editor_tab(project, target, window, cx);
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

    pub(in crate::app) fn open_transcript_scratch(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.center_surface_switch_blocked() {
            return;
        }
        let items = &self.snapshot.conversation.items;
        let text = crate::app::views::transcript::transcript_scratch_text(items);
        self.activate_editor_tab(
            self.workspace_project(),
            EditorTarget::Transcript(text),
            window,
            cx,
        );
    }

    pub(in crate::app) fn activate_editor_for_project(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_editor_tab(project, EditorTarget::Resume, window, cx);
    }

    pub(super) fn activate_editor_tab(
        &mut self,
        project: PathBuf,
        editor_target: EditorTarget,
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
        self.editor_request_generation = self.editor_request_generation.wrapping_add(1);

        let project = project.canonicalize().unwrap_or(project);
        let target = self.composer_sessions.current_target().to_owned();
        let tab = *self
            .session_editor_tabs
            .entry(target.clone())
            .or_insert_with(new_session_tab);
        let Some(editor) = self
            .project_editors
            .get(&(project.clone(), tab))
            .filter(|editor| editor.read(cx).is_alive(cx))
            .cloned()
            .or_else(|| self.spawn_editor(project.clone(), tab, window, cx))
        else {
            return;
        };
        self.retain_workspace_draft(cx);
        // Reusing the native terminal must not unmap/remap it: both file jumps
        // and repeated Open editor commands come through this path.
        let switching_editor = self.editor.as_ref() != Some(&editor);
        if switching_editor {
            self.hide_editor(cx);
        }
        if switching_editor || self.surface != AppSurface::Editor {
            self.editor_return_focus = window.focused(cx);
        }
        let review_request = matches!(
            &editor_target,
            EditorTarget::Review(_) | EditorTarget::ReviewLocation { .. }
        );
        self.editor = Some(editor.clone());
        self.hide_terminal(cx);
        // Startup prompts can block remote requests until the user responds.
        // Show the terminal before waiting so those prompts remain accessible.
        self.editor_ready = true;
        self.reveal_native_center_surface(AppSurface::Editor, window, cx);
        let generation = self.editor_request_generation;
        match &editor_target {
            EditorTarget::Review(review) => {
                self.active_review = Some(super::review::ActiveReview::new(
                    generation,
                    target.clone(),
                    project,
                    review.clone(),
                ));
            }
            EditorTarget::ReviewLocation { .. } => {
                if let Some(review) = self.active_review.as_mut() {
                    review.pending = Some(generation);
                    review.error = None;
                }
            }
            _ => {}
        }
        let opened = editor.update(cx, |editor, cx| editor.activate_tab(tab, editor_target, cx));
        cx.spawn_in(window, async move |weak, cx| {
            let result = opened.await;
            let _ = weak.update_in(cx, |this, _window, cx| {
                if review_request {
                    let completion = match &result {
                        Ok(Some(navigation)) => Ok(navigation.clone()),
                        Err(error) => Err(error.clone()),
                        Ok(None) => Err("Editor returned no review locations".into()),
                    };
                    if let Some(review) = this.active_review.as_mut()
                        && review.target == target
                        && review.complete(generation, completion)
                    {
                        this.notify_run_panel(cx);
                        cx.notify();
                    }
                }
                if this.editor.as_ref() != Some(&editor)
                    || this.composer_sessions.current_target() != target
                {
                    return;
                }
                let Err(error) = result else { return };
                zlog::warn!("Neovim session-view request failed for {target}: {error}");
                if !editor_completion_is_current(
                    generation,
                    this.editor_request_generation,
                    tab,
                    this.session_editor_tabs
                        .get(this.composer_sessions.current_target())
                        .copied(),
                    this.surface,
                ) {
                    return;
                }
                if !editor.read(cx).is_alive(cx) {
                    this.close_editor(cx);
                }
                this.notify_workspace_error("Neovim", error, cx);
            });
        })
        .detach();
        self.notify_run_panel(cx);
        cx.notify();
    }

    fn spawn_editor(
        &mut self,
        project: PathBuf,
        tab: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<NvimEditor>> {
        match NvimEditor::spawn(project.clone(), window, cx) {
            Ok(editor) => {
                let editor = cx.new(|_| editor);
                let key = (project, tab);
                self.project_editors.insert(key.clone(), editor.clone());
                let monitored = editor.clone();
                self.monitor_native_process(window, cx, move |this, _window, cx| {
                    if this.project_editors.get(&key) != Some(&monitored) {
                        return false;
                    }
                    if monitored.read(cx).is_alive(cx) {
                        return true;
                    }
                    this.project_editors.remove(&key);
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
        self.hide_editor(cx);
        self.editor = None;
        self.editor_ready = false;
        let focus = self
            .editor_return_focus
            .take()
            .unwrap_or_else(|| self.chat_composer_focus(cx));
        self.enter_chat_surface(focus, cx);
        self.request_repository_refresh(cx);
    }
}

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
#[path = "editor_tests.rs"]
mod tests;
