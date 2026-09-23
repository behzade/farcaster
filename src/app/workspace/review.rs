use std::path::PathBuf;

use gpui::{Context, Window};

use super::{AppSurface, FarcasterApp, neovim::EditorTarget};
use crate::reviews::{Review, ReviewNavigation, resolve_path};

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

    pub(super) fn select_from_editor(&mut self, list_id: u64, selected: usize) -> bool {
        let Some(navigation) = self.navigation.as_mut() else {
            return false;
        };
        if self.pending.is_some()
            || navigation.list_id != list_id
            || selected >= self.review.items.len()
            || (self.inspecting == Some(selected) && navigation.selected == Some(selected))
        {
            return false;
        }
        self.inspecting = Some(selected);
        navigation.selected = Some(selected);
        true
    }
}

impl FarcasterApp {
    pub(in crate::app) fn visible_review(&self) -> Option<&ActiveReview> {
        self.workspace
            .editor
            .active_review
            .as_ref()
            .filter(|review| {
                review.is_visible(
                    self.workspace.surface,
                    self.composer.sessions.current_target(),
                )
            })
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
        self.workspace
            .editor
            .active_review
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
        if self.overlays.view.run {
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
        self.workspace.editor.active_review = None;
        if self.overlays.view.run {
            self.close_sheet(window, cx);
        }
        if let Some(editor) = self.workspace.editor.view.clone() {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
        }
        self.notify_run_panel(cx);
        cx.notify();
    }
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
