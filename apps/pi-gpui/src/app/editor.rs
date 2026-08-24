//! In-app editor orchestration and project-file boundary checks.

use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context, Window};

use super::{AppSurface, PiApp};
use crate::{editor_terminal::EditorTerminal, sessions::root_session_for_path};

impl PiApp {
    pub(crate) fn open_file_editor(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.editor_project();
        let path = match resolve_editor_path(&project, &path) {
            Ok(path) => path,
            Err(error) => {
                self.editor_error = Some(error);
                self.surface = AppSurface::Editor;
                cx.notify();
                return;
            }
        };

        let can_reuse = self.editor.as_ref().is_some_and(|editor| {
            let editor = editor.read(cx);
            editor.project() == project && editor.is_alive()
        });
        if can_reuse {
            let editor = self.editor.as_ref().expect("editor checked above").clone();
            let opened = editor.update(cx, |editor, _| editor.open_file(path));
            match opened {
                Ok(()) => {
                    self.editor_error = None;
                    self.surface = AppSurface::Editor;
                    editor.update(cx, |editor, cx| editor.focus(window, cx));
                }
                Err(error) => self.editor_error = Some(error),
            }
            cx.notify();
            return;
        }

        if self.editor_return_focus.is_none() {
            self.editor_return_focus = window.focused(cx);
        }
        match EditorTerminal::spawn(project, path, window, cx) {
            Ok(editor) => {
                self.editor = Some(cx.new(|_| editor));
                self.editor_error = None;
                self.surface = AppSurface::Editor;
            }
            Err(error) => {
                self.editor = None;
                self.editor_error = Some(error);
                self.surface = AppSurface::Editor;
            }
        }
        cx.notify();
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

    pub(super) fn editor_path(&self, cx: &gpui::App) -> Option<PathBuf> {
        self.editor
            .as_ref()
            .map(|editor| editor.read(cx).path().to_path_buf())
    }

    fn editor_project(&self) -> PathBuf {
        root_session_for_path(
            &self.all_sessions,
            self.snapshot.selected_session.as_deref(),
        )
        .map(|root| root.project.clone())
        .or_else(|| {
            let selected = self.selected_draft.as_deref()?;
            self.drafts
                .iter()
                .find(|draft| draft.id == selected)
                .map(|draft| draft.project.clone())
        })
        .unwrap_or_else(|| self.project.clone())
    }
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
    Ok(candidate)
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

        let outside = tempdir()?;
        let outside_file = outside.path().join("outside.rs");
        std::fs::write(&outside_file, "")?;
        assert!(resolve_editor_path(project.path(), &outside_file).is_err());
        Ok(())
    }
}
