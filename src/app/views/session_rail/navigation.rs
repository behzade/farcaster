use std::path::Path;

use gpui::{Context, Window};

use super::{
    FarcasterApp, VisibleSessionTarget, rendering::INACTIVE_PREVIEW_LIMIT, session_rail_lists,
};
use crate::{
    app::AppSurface,
    sessions::{SessionSummary, root_session_for_path},
};

fn selected_root<'a>(
    sessions: &'a [SessionSummary],
    confirmed: Option<&Path>,
    requested: Option<&Path>,
) -> Option<&'a SessionSummary> {
    root_session_for_path(sessions, requested.or(confirmed))
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

impl FarcasterApp {
    pub(super) fn selected_rail_root(&self) -> Option<&SessionSummary> {
        selected_root(
            &self.sessions,
            self.snapshot.selected_session.as_deref(),
            self.pending_session_switch
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
        if self.surface != AppSurface::Chat || self.keyboard_overlay_focus(window, cx).is_some() {
            return;
        }
        let active = self.visible_session_targets();
        let archived = session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        )
        .archived;
        // A held key may advance again before the runtime publishes the selection.
        let selected = self
            .selected_draft
            .as_ref()
            .and_then(|id| self.drafts.iter().find(|draft| &draft.id == id))
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
            self.archived_sessions_expanded |= index >= INACTIVE_PREVIEW_LIMIT;
            self.archived_session_rail_view.update(cx, |view, cx| {
                view.reveal = Some(key);
                cx.notify();
            });
        } else {
            // The expanded archive occupies the active list's space.
            self.archived_sessions_expanded = false;
            self.session_rail_view.update(cx, |view, cx| {
                view.reveal = Some(key);
                cx.notify();
            });
        }
        self.recover_keyboard_focus(window, cx);
        self.notify_session_rail(cx);
        cx.notify();
    }
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
