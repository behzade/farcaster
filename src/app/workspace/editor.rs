use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use gpui::{AppContext as _, Context, Window};

use super::{
    AppSurface, FarcasterApp,
    neovim::{EditorTarget, NvimEditor, new_session_tab},
    neovim_adapter::NeovimBackend,
    terminal_editor::{TerminalBackend, TerminalEditor},
    vscode::VsCodeBackend,
    zed::ZedBackend,
};
use crate::reviews::{Review, resolve_path};

pub(in crate::app) enum EditorRequest {
    Project(PathBuf),
    File {
        project: PathBuf,
        path: PathBuf,
        line: Option<u64>,
        diff: bool,
    },
    Review {
        project: PathBuf,
        review: Review,
        locations: Vec<(PathBuf, Option<u64>)>,
    },
}

pub(super) trait EditorBackend {
    fn open(
        &self,
        app: &mut FarcasterApp,
        request: EditorRequest,
        window: &mut Window,
        cx: &mut Context<FarcasterApp>,
    ) -> Result<(), String>;
}

fn backend(choice: crate::storage::EditorChoice) -> &'static dyn EditorBackend {
    static NEOVIM: NeovimBackend = NeovimBackend;
    static VSCODE: VsCodeBackend = VsCodeBackend;
    static ZED: ZedBackend = ZedBackend;
    static HELIX: TerminalBackend = TerminalBackend(crate::storage::EditorChoice::Helix);
    static VIM: TerminalBackend = TerminalBackend(crate::storage::EditorChoice::Vim);
    match choice {
        crate::storage::EditorChoice::Neovim => &NEOVIM,
        crate::storage::EditorChoice::VsCode => &VSCODE,
        crate::storage::EditorChoice::Zed => &ZED,
        crate::storage::EditorChoice::Helix => &HELIX,
        crate::storage::EditorChoice::Vim => &VIM,
    }
}

pub(in crate::app) fn editor_available(
    choice: crate::storage::EditorChoice,
    project: &Path,
) -> bool {
    choice.available(project, std::env::var_os("PATH").as_deref())
}

pub(in crate::app) fn effective_editor_choice(
    choice: crate::storage::EditorChoice,
    project: &Path,
) -> crate::storage::EditorChoice {
    std::iter::once(choice)
        .chain(crate::storage::EditorChoice::ALL)
        .find(|candidate| editor_available(*candidate, project))
        .unwrap_or(choice)
}

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
        let editor_name = effective_editor_choice(self.settings.editor_choice, &project).label();
        let path = match resolve_editor_path(&project, &path) {
            Ok(path) => path,
            Err(error) => {
                self.notify_workspace_error(editor_name, error, cx);
                return;
            }
        };
        self.open_editor_request(
            EditorRequest::File {
                project,
                path,
                line,
                diff,
            },
            window,
            cx,
        );
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
        self.open_editor_request(EditorRequest::Project(project), window, cx);
    }

    pub(crate) fn open_review_editor(
        &mut self,
        project: PathBuf,
        review: Review,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.center_surface_switch_blocked() {
            return;
        }
        let current = self.workspace_project();
        let locations = review.validate().and_then(|()| {
            let root = current.canonicalize().map_err(|error| error.to_string())?;
            if project != root {
                return Err("This review belongs to a different project.".into());
            }
            review
                .items
                .iter()
                .map(|item| {
                    resolve_path(&root, &item.path)
                        .map(|path| (path, item.start_line.map(u64::from)))
                })
                .collect::<Result<Vec<_>, _>>()
        });
        let locations = match locations {
            Ok(locations) => locations,
            Err(error) => {
                self.notify_workspace_error("Review", error, cx);
                return;
            }
        };
        self.open_editor_request(
            EditorRequest::Review {
                project: current,
                review,
                locations,
            },
            window,
            cx,
        );
    }

    pub(in crate::app) fn open_editor_request(
        &mut self,
        request: EditorRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = match &request {
            EditorRequest::Project(project)
            | EditorRequest::File { project, .. }
            | EditorRequest::Review { project, .. } => project,
        };
        let choice = effective_editor_choice(self.settings.editor_choice, project);
        if !self.project.repository.execution_allowed {
            self.request_project_trust_for_action(
                project.clone(),
                crate::app::project::trust::PendingTrustAction::Editor(request),
                window,
                cx,
            );
            return;
        }
        if let Err(error) = backend(choice).open(self, request, window, cx) {
            self.notify_workspace_error(choice.label(), error, cx);
        }
    }

    pub(in crate::app) fn activate_editor_tab(
        &mut self,
        project: PathBuf,
        editor_target: EditorTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.project.repository.execution_allowed {
            self.request_project_trust_for_action(
                project.clone(),
                crate::app::project::trust::PendingTrustAction::EditorTab {
                    project,
                    target: editor_target,
                },
                window,
                cx,
            );
            return;
        }
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
        let Some(editor) = self
            .workspace
            .editor
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
        self.workspace.editor.terminal_editor_view = None;
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<NvimEditor>> {
        match NvimEditor::spawn(project.clone(), window, cx) {
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
                self.notify_workspace_error("Neovim", error, cx);
                None
            }
        }
    }

    pub(super) fn activate_terminal_editor(
        &mut self,
        project: PathBuf,
        choice: crate::storage::EditorChoice,
        title: String,
        arguments: Vec<OsString>,
        temporary: Option<tempfile::NamedTempFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let project = project.canonicalize().unwrap_or(project);
        let key = (
            project.clone(),
            self.composer.sessions.current_target().to_owned(),
            choice,
        );
        let existing = self.workspace.editor.terminal_editors.get(&key).cloned();
        let editor = existing
            .clone()
            .unwrap_or_else(|| cx.new(|_| TerminalEditor::new(project, choice)));
        editor.update(cx, |editor, cx| {
            editor.open(arguments, title, temporary, window, cx)
        })?;
        if existing.is_none() {
            self.workspace
                .editor
                .terminal_editors
                .insert(key.clone(), editor.clone());
            let monitored = editor.clone();
            self.monitor_native_process(window, cx, move |this, _window, cx| {
                if this.workspace.editor.terminal_editors.get(&key) != Some(&monitored) {
                    return false;
                }
                if monitored.update(cx, |editor, cx| editor.retain_alive(cx)) {
                    return true;
                }
                this.workspace.editor.terminal_editors.remove(&key);
                if this.workspace.editor.terminal_editor_view.as_ref() == Some(&monitored) {
                    if this.workspace.surface == AppSurface::Editor {
                        this.close_editor(cx);
                    } else {
                        this.workspace.editor.terminal_editor_view = None;
                        this.workspace.editor.ready = false;
                        this.workspace.editor.return_focus = None;
                        this.request_repository_refresh(cx);
                    }
                }
                false
            });
        }
        self.retain_workspace_draft(cx);
        let switching = self.workspace.editor.terminal_editor_view.as_ref() != Some(&editor);
        if switching {
            self.hide_editor(cx);
        }
        if switching || self.workspace.surface != AppSurface::Editor {
            self.workspace.editor.return_focus = window.focused(cx);
        }
        self.workspace.editor.view = None;
        self.workspace.editor.terminal_editor_view = Some(editor);
        self.workspace.editor.active_review = None;
        self.hide_terminal(cx);
        self.workspace.editor.ready = true;
        self.reveal_native_center_surface(AppSurface::Editor, window, cx);
        self.notify_run_panel(cx);
        cx.notify();
        Ok(())
    }

    pub(in crate::app) fn hide_editor(&self, cx: &mut Context<Self>) {
        if let Some(editor) = self.workspace.editor.view.as_ref() {
            editor.update(cx, |editor, cx| editor.set_visible(false, cx));
        }
        if let Some(editor) = self.workspace.editor.terminal_editor_view.as_ref() {
            editor.update(cx, |editor, cx| editor.set_visible(false, cx));
        }
    }

    pub(in crate::app) fn restore_editor_visibility(&self, cx: &mut Context<Self>) {
        if self.workspace.surface == AppSurface::Editor
            && self.workspace.editor.ready
            && let Some(editor) = self.workspace.editor.view.as_ref()
        {
            editor.update(cx, |editor, cx| editor.set_visible(true, cx));
        } else if self.workspace.surface == AppSurface::Editor
            && self.workspace.editor.ready
            && let Some(editor) = self.workspace.editor.terminal_editor_view.as_ref()
        {
            editor.update(cx, |editor, cx| editor.set_visible(true, cx));
        }
    }

    pub(in crate::app) fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.hide_editor(cx);
        self.workspace.editor.view = None;
        self.workspace.editor.terminal_editor_view = None;
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
