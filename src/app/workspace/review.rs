use std::path::PathBuf;

use gpui::{Context, Window};

use super::{AppSurface, FarcasterApp, neovim::EditorTarget};
use crate::app::reviews::{Review, ReviewNavigation, resolve_path};

pub(in crate::app) struct ActiveReview {
    pub id: u64,
    pub target: String,
    pub project: PathBuf,
    pub review: Review,
    pub navigation: Option<ReviewNavigation>,
    pub inspecting: Option<usize>,
    pub error: Option<String>,
    pub pending: Option<u64>,
}

impl ActiveReview {
    pub(super) fn new(id: u64, target: String, project: PathBuf, review: Review) -> Self {
        Self {
            id,
            target,
            project,
            review,
            navigation: None,
            inspecting: None,
            error: None,
            pending: Some(id),
        }
    }

    fn is_visible(&self, surface: AppSurface, target: &str) -> bool {
        surface == AppSurface::Editor && self.target == target
    }

    pub(super) fn complete(
        &mut self,
        generation: u64,
        result: Result<ReviewNavigation, String>,
    ) -> bool {
        if self.pending != Some(generation) {
            return false;
        }
        self.pending = None;
        match result {
            Ok(navigation) => {
                if self.inspecting.is_none() {
                    self.inspecting = navigation
                        .selected
                        .or_else(|| (!self.review.items.is_empty()).then_some(0));
                }
                self.navigation = Some(navigation);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        true
    }
}

impl FarcasterApp {
    pub(in crate::app) fn visible_review(&self) -> Option<&ActiveReview> {
        self.active_review.as_ref().filter(|review| {
            review.is_visible(self.surface, self.composer_sessions.current_target())
        })
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
        let validation = review.validate().and_then(|()| {
            let root = current.canonicalize().map_err(|error| error.to_string())?;
            if project != root {
                return Err("This review belongs to a different project.".into());
            }
            for item in &review.items {
                resolve_path(&root, &item.path)?;
            }
            Ok(())
        });
        if let Err(error) = validation {
            self.notify_workspace_error("Review", error, cx);
            return;
        }
        self.activate_editor_tab(current, EditorTarget::Review(review), window, cx);
        if self.visible_review().is_some()
            && !crate::app::ui::layout::shows_right_inline(crate::app::ui::layout::layout_mode(
                window.viewport_size().width,
            ))
        {
            self.open_run_sheet(window, cx);
        }
    }

    pub(in crate::app) fn open_review_location(
        &mut self,
        id: u64,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active) = self
            .visible_review()
            .filter(|review| review.id == id && review.pending.is_none())
        else {
            return;
        };
        let Some(navigation) = &active.navigation else {
            return;
        };
        let Some(location) = active.review.items.get(index) else {
            return;
        };
        let resolved = resolve_path(&active.project, &location.path);
        let project = active.project.clone();
        let list_id = navigation.list_id;
        self.active_review
            .as_mut()
            .expect("visible review")
            .inspecting = Some(index);
        self.notify_run_panel(cx);
        let path = match resolved {
            Ok(path) => path,
            Err(error) => {
                self.notify_workspace_error("Review", error, cx);
                return;
            }
        };
        if self.overlays.run {
            self.close_sheet(window, cx);
        }
        self.activate_editor_tab(
            project,
            EditorTarget::ReviewLocation {
                list_id,
                index,
                path,
            },
            window,
            cx,
        );
    }

    pub(in crate::app) fn close_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_review = None;
        if self.overlays.run {
            self.close_sheet(window, cx);
        }
        if let Some(editor) = self.editor.clone() {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
        }
        self.notify_run_panel(cx);
        cx.notify();
    }
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
