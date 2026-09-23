use gpui::{Context, Window};
use std::path::PathBuf;

use super::{
    FarcasterApp,
    editor::{EditorBackend, EditorRequest},
    neovim::EditorTarget,
};

pub(super) struct NeovimBackend;

impl EditorBackend for NeovimBackend {
    fn name(&self) -> &'static str {
        "Neovim"
    }

    fn program(&self) -> PathBuf {
        super::neovim::nvim_executable()
    }

    fn open(
        &self,
        app: &mut FarcasterApp,
        request: EditorRequest,
        window: &mut Window,
        cx: &mut Context<FarcasterApp>,
    ) -> Result<(), String> {
        match request {
            EditorRequest::Project(project) => {
                app.activate_editor_tab(project, EditorTarget::Resume, window, cx);
            }
            EditorRequest::File {
                project,
                path,
                line,
                diff,
            } => {
                let target = if diff {
                    EditorTarget::Diff(path, line)
                } else {
                    EditorTarget::File(path, line)
                };
                app.activate_editor_tab(project, target, window, cx);
            }
            EditorRequest::Review {
                project, review, ..
            } => {
                app.activate_editor_tab(project, EditorTarget::Review(review), window, cx);
                if app.visible_review().is_some()
                    && !crate::app::ui::layout::shows_right_inline(
                        crate::app::ui::layout::layout_mode(window.viewport_size().width),
                    )
                {
                    app.open_run_sheet(window, cx);
                }
            }
        }
        Ok(())
    }
}
