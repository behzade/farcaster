use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context, Window};

use super::{
    AppSurface, FarcasterApp,
    editor_session::{EditorFile, EditorSession, EditorTarget, new_session_tab},
};
use crate::app::ui::assets::AppIcon;
use crate::editors::EditorCommand;

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
        if self.overlays.view.run {
            self.close_sheet(window, cx);
        }
        let project = self.workspace_project();
        let path = match resolve_editor_path(&project, &path) {
            Ok(path) => path,
            Err(error) => {
                self.notify_workspace_error("Editor", error, cx);
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

    pub(in crate::app) fn activate_editor_tab(
        &mut self,
        project: PathBuf,
        editor_target: EditorTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = match self.editor_command(cx) {
            Some(command) => command,
            None => return,
        };
        let name = command.name();
        if !self.workspace_trusted() {
            self.notify_workspace_error(
                &name,
                format!("Trust this project before opening {name}."),
                cx,
            );
            return;
        }
        let neovim = command.is_neovim();
        let Some(editor_target) = editor_target_for_editor(neovim, editor_target) else {
            self.notify_workspace_error(
                &name,
                format!("{name} cannot open this view. Choose Neovim in Settings to open it here."),
                cx,
            );
            return;
        };
        self.workspace.editor.request_generation =
            self.workspace.editor.request_generation.wrapping_add(1);

        let project = project.canonicalize().unwrap_or(project);
        let target = self.composer.sessions.current_target().to_owned();
        let tab = *self
            .workspace
            .editor
            .session_tabs
            .entry(target.clone())
            .or_insert_with(new_session_tab);
        let file = if neovim { None } else { editor_target.file() };
        let resume = matches!(editor_target, EditorTarget::Resume);
        let key = (project.clone(), tab);
        let Some(editor) = self
            .workspace
            .editor
            .project_editors
            .get(&key)
            .filter(|editor| {
                let editor = editor.read(cx);
                editor.is_alive(cx)
                    && editor.command() == &command
                    && (resume || editor.file() == file.as_ref())
            })
            .cloned()
            .or_else(|| self.spawn_editor(project.clone(), tab, command, file, window, cx))
        else {
            return;
        };
        // Reusing the native terminal must not unmap/remap it: both file jumps
        // and repeated Open editor commands come through this path.
        let switching_editor = self.workspace.editor.view.as_ref() != Some(&editor);
        if switching_editor {
            self.hide_editor(cx);
        }
        if switching_editor || self.workspace.surface != AppSurface::Editor {
            self.workspace.editor.return_focus = window.focused(cx);
        }
        let review_request = matches!(
            &editor_target,
            EditorTarget::Review(_) | EditorTarget::ReviewLocation { .. }
        );
        self.workspace.editor.view = Some(editor.clone());
        self.hide_terminal(cx);
        // Startup prompts can block remote requests until the user responds.
        // Show the terminal before waiting so those prompts remain accessible.
        self.workspace.editor.ready = true;
        self.reveal_native_center_surface(AppSurface::Editor, window, cx);
        let generation = self.workspace.editor.request_generation;
        match &editor_target {
            EditorTarget::Review(review) => {
                self.workspace.editor.active_review = Some(super::review::ActiveReview::new(
                    generation,
                    target.clone(),
                    project,
                    review.clone(),
                ));
            }
            EditorTarget::ReviewLocation { .. } => {
                if let Some(review) = self.workspace.editor.active_review.as_mut() {
                    review.pending = Some(generation);
                    review.error = None;
                }
            }
            _ => {}
        }
        if !neovim {
            self.notify_run_panel(cx);
            cx.notify();
            return;
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
                    if let Some(review) = this.workspace.editor.active_review.as_mut()
                        && review.target == target
                        && review.complete(generation, completion)
                    {
                        this.notify_run_panel(cx);
                        cx.notify();
                    }
                }
                if this.workspace.editor.view.as_ref() != Some(&editor)
                    || this.composer.sessions.current_target() != target
                {
                    return;
                }
                let Err(error) = result else { return };
                zlog::warn!("Neovim session-view request failed for {target}: {error}");
                if !editor_completion_is_current(
                    generation,
                    this.workspace.editor.request_generation,
                    tab,
                    this.workspace
                        .editor
                        .session_tabs
                        .get(this.composer.sessions.current_target())
                        .copied(),
                    this.workspace.surface,
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
        command: EditorCommand,
        file: Option<EditorFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<EditorSession>> {
        match EditorSession::spawn(project.clone(), command, file, window, cx) {
            Ok(editor) => {
                let editor = cx.new(|_| editor);
                let key = (project, tab);
                self.workspace
                    .editor
                    .project_editors
                    .insert(key.clone(), editor.clone());
                let monitored = editor.clone();
                self.monitor_native_process(window, cx, move |this, _window, cx| {
                    if this.workspace.editor.project_editors.get(&key) != Some(&monitored) {
                        return false;
                    }
                    if monitored.read(cx).is_alive(cx) {
                        if this.workspace.editor.view.as_ref() == Some(&monitored)
                            && let Some(selection) =
                                monitored.update(cx, |editor, _| editor.take_review_selection())
                            && let Some(review) = this.workspace.editor.active_review.as_mut()
                            && review.target == this.composer.sessions.current_target()
                            && review.project == key.0
                            && review.select_from_editor(selection.list_id, selection.selected)
                        {
                            this.notify_run_panel(cx);
                            cx.notify();
                        }
                        return true;
                    }
                    this.workspace.editor.project_editors.remove(&key);
                    if this.workspace.editor.view.as_ref() != Some(&monitored) {
                        return false;
                    }
                    if this.workspace.surface == AppSurface::Editor {
                        this.close_editor(cx);
                    } else {
                        this.workspace.editor.view = None;
                        this.workspace.editor.ready = false;
                        this.workspace.editor.return_focus = None;
                        this.request_repository_refresh(cx);
                    }
                    false
                });
                Some(editor)
            }
            Err(error) => {
                self.notify_workspace_error("Editor", error, cx);
                None
            }
        }
    }

    fn editor_command(&mut self, cx: &mut Context<Self>) -> Option<EditorCommand> {
        let command = match self.text_editor_command() {
            Ok(command) => command,
            Err(error) => {
                self.notify_workspace_error("Editor", error, cx);
                return None;
            }
        };
        if command.available() {
            return Some(command);
        }
        self.notify_workspace_error(
            "Editor",
            format!(
                "{} was not found. Install it or set an editor command in Settings.",
                command.program
            ),
            cx,
        );
        None
    }

    fn workspace_trusted(&self) -> bool {
        self.project.repository.execution_allowed
    }

    pub(in crate::app) fn text_editor_command(&self) -> Result<EditorCommand, String> {
        crate::editors::resolve_text_editor(self.settings.text_editor.as_deref())
    }

    pub(in crate::app) fn text_editor_name(&self) -> String {
        self.text_editor_command()
            .map_or_else(|_| "Editor".to_owned(), |command| command.name())
    }

    pub(in crate::app) fn text_editor_icon(&self) -> AppIcon {
        self.text_editor_command()
            .map_or(AppIcon::Code, |command| AppIcon::for_editor(command.icon()))
    }

    pub(in crate::app) fn reset_editor_sessions(&mut self, cx: &mut Context<Self>) {
        if self.workspace.editor.project_editors.is_empty() && self.workspace.editor.view.is_none()
        {
            return;
        }
        if self.workspace.editor.view.is_some() {
            self.close_editor(cx);
        }
        self.workspace.editor.project_editors.clear();
        self.workspace.editor.session_tabs.clear();
    }

    pub(in crate::app) fn hide_editor(&self, cx: &mut Context<Self>) {
        if let Some(editor) = self.workspace.editor.view.as_ref() {
            editor.update(cx, |editor, cx| editor.set_visible(false, cx));
        }
    }

    pub(in crate::app) fn restore_editor_visibility(&self, cx: &mut Context<Self>) {
        if self.workspace.surface == AppSurface::Editor
            && self.workspace.editor.ready
            && let Some(editor) = self.workspace.editor.view.as_ref()
        {
            editor.update(cx, |editor, cx| editor.set_visible(true, cx));
        }
    }

    pub(in crate::app) fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.hide_editor(cx);
        self.workspace.editor.view = None;
        self.workspace.editor.ready = false;
        let focus = self
            .workspace
            .editor
            .return_focus
            .take()
            .unwrap_or_else(|| self.chat_composer_focus(cx));
        self.enter_chat_surface(focus, cx);
        self.request_repository_refresh(cx);
    }
}

fn editor_target_for_editor(neovim: bool, target: EditorTarget) -> Option<EditorTarget> {
    if neovim {
        return Some(target);
    }
    match target {
        EditorTarget::Resume | EditorTarget::File(..) => Some(target),
        EditorTarget::Diff(path, line) => Some(EditorTarget::File(path, line)),
        EditorTarget::Transcript(_)
        | EditorTarget::Review(_)
        | EditorTarget::ReviewLocation { .. } => None,
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
