use std::{collections::BTreeSet, path::PathBuf};

use gpui::{AppContext as _, Context, Entity, FocusHandle, Focusable as _, Subscription, Window};
use gpui_component::input::{InputEvent, TextareaState};

use crate::{
    app::FarcasterApp,
    repository::{RepositoryEdit, RepositoryEditReview, WorkingCopySnapshot},
};

#[derive(Default)]
pub(in crate::app) struct FileSelection {
    pub active: bool,
    pub paths: BTreeSet<PathBuf>,
}

impl FileSelection {
    pub fn toggle_mode(&mut self) {
        self.active = !self.active;
        self.paths.clear();
    }

    pub fn toggle(&mut self, path: PathBuf) {
        if self.active && !self.paths.remove(&path) {
            self.paths.insert(path);
        }
    }

    pub fn retain(&mut self, snapshot: &WorkingCopySnapshot) {
        self.paths.retain(|path| {
            snapshot
                .changes
                .iter()
                .any(|change| &change.relative_path == path)
        });
    }
}

#[derive(Default)]
pub(in crate::app) struct RepositoryEditState {
    pub selection: FileSelection,
    pub pending: Option<PendingRepositoryEdit>,
    generation: u64,
}

impl RepositoryEditState {
    pub(super) fn clear(&mut self) {
        self.selection = Default::default();
        self.pending = None;
        self.generation = self.generation.saturating_add(1);
    }
}

pub(in crate::app) struct PendingRepositoryEdit {
    pub focus: FocusHandle,
    pub input: Entity<TextareaState>,
    pub action: RepositoryEdit,
    pub paths: Vec<PathBuf>,
    pub review: Option<RepositoryEditReview>,
    pub error: Option<String>,
    pub applying: bool,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

impl PendingRepositoryEdit {
    pub fn preparing(&self) -> bool {
        self.review.is_none() && self.error.is_none()
    }

    pub fn can_apply(&self, cx: &gpui::App) -> bool {
        self.review.is_some()
            && self.error.is_none()
            && !self.applying
            && (self.action != RepositoryEdit::Commit
                || !self.input.read(cx).value().trim().is_empty())
    }
}

impl FarcasterApp {
    pub(in crate::app) fn toggle_repository_selection(&mut self, cx: &mut Context<Self>) {
        if self.repository.edits.pending.is_none() {
            self.repository.edits.selection.toggle_mode();
            self.notify_run_panel(cx);
        }
    }

    pub(in crate::app) fn toggle_repository_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.repository.edits.pending.is_none() {
            self.repository.edits.selection.toggle(path);
            self.notify_run_panel(cx);
        }
    }

    pub(in crate::app) fn review_repository_edit(
        &mut self,
        action: RepositoryEdit,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.repository.execution_allowed
            || self.repository.sync.action.is_some()
            || self.repository.edits.pending.is_some()
        {
            return;
        }
        let (Some(backend), Some(snapshot)) = (
            self.repository.backend.clone(),
            self.repository.snapshot.clone(),
        ) else {
            return;
        };
        let selected = path
            .map(|path| BTreeSet::from([path]))
            .unwrap_or_else(|| self.repository.edits.selection.paths.clone());
        if selected.is_empty() {
            return;
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .submit_on_enter(false)
                .placeholder("Commit message")
        });
        let subscription = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
        let focus = cx.focus_handle();
        let return_focus = window.focused(cx);
        self.cover_native_workspace_surface(cx);
        if action == RepositoryEdit::Commit {
            input.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            focus.focus(window, cx);
        }
        self.repository.edits.generation = self.repository.edits.generation.saturating_add(1);
        let generation = self.repository.edits.generation;
        self.repository.edits.pending = Some(PendingRepositoryEdit {
            focus,
            input,
            action,
            paths: selected.iter().cloned().collect(),
            review: None,
            applying: false,
            error: None,
            return_focus,
            _subscription: subscription,
        });
        self.notify_run_panel(cx);
        cx.notify();
        let task = cx.background_spawn(async move { backend.prepare_edit(&snapshot, &selected) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.repository.edits.generation != generation {
                    return;
                }
                let Some(pending) = this.repository.edits.pending.as_mut() else {
                    return;
                };
                match result {
                    Ok(review) => {
                        pending.paths = review.paths().to_vec();
                        pending.review = Some(review);
                    }
                    Err(error) => {
                        pending.error = Some(format!(
                            "{error}\nClose this review and try again after the changes refresh."
                        ));
                        this.request_repository_refresh(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::app) fn close_repository_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .repository
            .edits
            .pending
            .as_ref()
            .is_some_and(|pending| pending.applying)
        {
            return;
        }
        let Some(pending) = self.repository.edits.pending.take() else {
            return;
        };
        self.repository.edits.generation = self.repository.edits.generation.saturating_add(1);
        self.restore_overlay_focus(pending.return_focus, &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        self.notify_run_panel(cx);
        cx.notify();
    }

    pub(in crate::app) fn confirm_repository_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.repository.execution_allowed || self.repository.sync.action.is_some() {
            return;
        }
        let Some(backend) = self.repository.backend.clone() else {
            return;
        };
        let Some(pending) = self.repository.edits.pending.as_mut() else {
            return;
        };
        if !pending.can_apply(cx) {
            return;
        }
        let Some(review) = pending.review.clone() else {
            return;
        };
        let action = pending.action;
        let message = pending.input.read(cx).value().to_string();
        pending.applying = true;
        let generation = self.repository.edits.generation;
        cx.notify();
        let task =
            cx.background_spawn(async move { backend.apply_edit(&review, action, &message) });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.repository.edits.generation != generation { return; }
                let Some(pending) = this.repository.edits.pending.as_mut() else { return; };
                pending.applying = false;
                match result {
                    Ok(()) => {
                        this.repository.edits.selection = Default::default();
                        this.close_repository_edit(window, cx);
                    }
                    Err(error) => { pending.error = Some(format!("{error}\nClose this review and inspect the refreshed changes before trying again.")); }
                }
                // A failed hook or command can still have changed repository state.
                this.request_repository_refresh(cx);
                this.notify_run_panel(cx);
                cx.notify();
            });
        }).detach();
    }
}

#[cfg(test)]
#[path = "edits_tests.rs"]
mod tests;
