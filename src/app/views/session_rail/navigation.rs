use std::path::Path;

use gpui::{Context, Window};

use super::{
    FarcasterApp, VisibleSessionTarget, rendering::INACTIVE_PREVIEW_LIMIT, session_rail_lists,
};
use crate::{
    app::{
        AppSurface,
        views::run_panel::{RECENT_WORKERS, worker_navigation_rows},
    },
    sessions::{SessionCatalog, SessionSummary},
};

fn selected_root<'a>(
    sessions: &'a SessionCatalog,
    confirmed: Option<&Path>,
    requested: Option<&Path>,
) -> Option<&'a SessionSummary> {
    sessions.root_for_path(requested.or(confirmed))
}

#[derive(Debug, PartialEq, Eq)]
enum SessionStep {
    Active(usize),
    Archived(usize),
}

fn session_step(
    active: impl IntoIterator<Item = i64, IntoIter: ExactSizeIterator>,
    archived: impl IntoIterator<Item = i64, IntoIter: ExactSizeIterator>,
    selected: i64,
    direction: isize,
) -> Option<SessionStep> {
    let active = active.into_iter();
    let archived = archived.into_iter();
    let active_count = active.len();
    let total = active_count + archived.len();
    let current = active.chain(archived).position(|id| id == selected)?;
    let next = current.checked_add_signed(direction)?;
    if next < active_count {
        Some(SessionStep::Active(next))
    } else if next < total {
        Some(SessionStep::Archived(next - active_count))
    } else {
        None
    }
}

fn worker_step(count: usize, selected: Option<usize>, direction: isize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let current = selected.unwrap_or(if direction > 0 { count - 1 } else { 0 });
    Some((current as isize + direction).rem_euclid(count as isize) as usize)
}

fn visible_worker_indices(total: usize, selected: Option<usize>, older_open: bool) -> Vec<usize> {
    let recent_end = (RECENT_WORKERS + 1).min(total);
    if older_open {
        (recent_end..total).collect()
    } else {
        (0..recent_end)
            .chain(selected.filter(|index| *index >= recent_end))
            .collect()
    }
}

impl FarcasterApp {
    pub(in crate::app) fn switch_worker(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.surface != AppSurface::Chat
            || self.keyboard_overlay_focus(window, cx).is_some()
        {
            return;
        }
        let selected = self
            .lifecycle
            .pending_session_switch
            .as_ref()
            .map(|(path, _)| path.as_path())
            .or(self.snapshot.selected_session.as_deref());
        let rows = worker_navigation_rows(&self.sessions.all, &self.activity.agents, selected);
        let Some(root) = rows.first() else { return };
        let older_open = self
            .views
            .run_panel
            .read(cx)
            .older_workers_open_for(&root.path);
        let selected_index = rows
            .iter()
            .position(|session| Some(session.path.as_path()) == selected);
        let visible = visible_worker_indices(rows.len(), selected_index, older_open);
        let current = visible
            .iter()
            .position(|index| Some(*index) == selected_index);
        let Some(index) = worker_step(visible.len(), current, direction) else {
            return;
        };
        let session = rows[visible[index]];
        let path = session.path.clone();
        let project = session.project.clone();
        self.select_session_restoring_center(path, project, false, window, cx);
        self.recover_keyboard_focus(window, cx);
    }

    pub(super) fn selected_rail_root(&self) -> Option<&SessionSummary> {
        selected_root(
            &self.sessions.visible,
            self.snapshot.selected_session.as_deref(),
            self.lifecycle
                .pending_session_switch
                .as_ref()
                .map(|(path, _)| path.as_path()),
        )
    }

    pub(in crate::app) fn switch_transcript_session(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.surface != AppSurface::Chat
            || self.keyboard_overlay_focus(window, cx).is_some()
        {
            return;
        }
        let active = self.visible_session_targets();
        let archived = session_rail_lists(
            &self.sessions.visible,
            &self.sessions.drafts,
            self.sessions.project_filter.as_deref(),
            &self.sessions.order,
        )
        .archived;
        // A held key may advance again before the runtime publishes the selection.
        let selected = self
            .sessions
            .selected_draft
            .as_ref()
            .and_then(|id| self.sessions.drafts.iter().find(|draft| &draft.id == id))
            .map(|draft| draft.app_session_id)
            .or_else(|| {
                self.selected_rail_root()
                    .map(|session| session.app_session_id)
            });
        let Some(selected) = selected else { return };
        let step = session_step(
            active.iter().map(VisibleSessionTarget::app_session_id),
            archived.iter().map(|item| item.session.app_session_id),
            selected,
            direction,
        );
        let (target, archived_index) = match step {
            Some(SessionStep::Active(index)) => (active[index].clone(), None),
            Some(SessionStep::Archived(index)) => (
                VisibleSessionTarget::Persisted(archived[index].session.clone()),
                Some(index),
            ),
            None => return,
        };
        let key = match &target {
            VisibleSessionTarget::Draft(draft) => format!("draft:{}", draft.id),
            VisibleSessionTarget::Persisted(session) => format!("session:{}", session.id),
        };
        // Browse archived history; never restore it or switch to its saved editor.
        match target {
            VisibleSessionTarget::Draft(draft) => {
                self.resume_draft_restoring_center(draft.id, draft.project, false, window, cx)
            }
            VisibleSessionTarget::Persisted(session) => self.select_session_restoring_center(
                session.path,
                session.project,
                false,
                window,
                cx,
            ),
        }
        if let Some(index) = archived_index {
            self.sessions.archived_expanded |= index >= INACTIVE_PREVIEW_LIMIT;
            self.views.archived_session_rail.update(cx, |view, cx| {
                view.reveal = Some(key);
                cx.notify();
            });
        } else {
            // The expanded archive occupies the active list's space.
            self.reveal_active_session_row(key, cx);
        }
        self.recover_keyboard_focus(window, cx);
        self.notify_session_rail(cx);
        cx.notify();
    }
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
