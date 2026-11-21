#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use crate::sessions::SessionCatalog;
use gpui::{Context, Window};

use super::{FarcasterApp, active_item_identity};
use crate::{
    app::{
        views::SessionReveal,
        views::run_panel::{RECENT_WORKERS, worker_navigation_rows},
    },
    sessions::SessionSummary,
};

#[cfg(test)]
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
    active_count: usize,
    archived_count: usize,
    current: Option<usize>,
    direction: isize,
) -> Option<SessionStep> {
    let total = active_count + archived_count;
    let current = current?;
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

fn visible_worker_indices(total: usize, selected: Option<usize>, expanded: bool) -> Vec<usize> {
    let recent_end = (RECENT_WORKERS + 1).min(total);
    if expanded {
        (0..total).collect()
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
        if self.native_workspace_covered_by_overlay()
            || self.keyboard_overlay_focus(window, cx).is_some()
        {
            return;
        }
        let selected = self.sidebar_catalog_row(&self.sessions.all);
        let rows = worker_navigation_rows(
            &self.sessions.all,
            &self.activity.agents,
            self.sidebar_worker_root(),
        );
        let Some(root) = rows.first() else { return };
        let expanded = self
            .views
            .run_panel
            .read(cx)
            .workers_expanded_for(&root.catalog_key());
        let selected_index = rows
            .iter()
            .position(|session| selected.is_some_and(|row| row.same_record(session)));
        let visible = visible_worker_indices(rows.len(), selected_index, expanded);
        let current = visible
            .iter()
            .position(|index| Some(*index) == selected_index);
        let Some(index) = worker_step(visible.len(), current, direction) else {
            return;
        };
        let session = rows[visible[index]];
        self.select_rail_session(session.catalog_key(), window, cx);
    }

    pub(in crate::app) fn selected_rail_root(&self) -> Option<&SessionSummary> {
        if self.sessions.selected_draft.is_some() {
            return None;
        }
        let catalog = &self.sessions.visible;
        catalog.root_for_row(self.sidebar_catalog_row(catalog)?)
    }

    pub(in crate::app) fn switch_transcript_session(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.native_workspace_covered_by_overlay()
            || self.keyboard_overlay_focus(window, cx).is_some()
        {
            return;
        }
        let Some(selected) = self.selected_rail_identity() else {
            return;
        };
        let mut active = self.visible_session_targets();
        let active_index = active.iter().position(|item| selected.matches(item));
        // Stepping within the active section need not prepare a collapsed archive.
        let archived = if active_index.is_some_and(|i| {
            i.checked_add_signed(direction)
                .is_none_or(|next| next < active.len())
        }) {
            Default::default()
        } else {
            self.views
                .archived_session_rail
                .update(cx, |rail, _| rail.sorted_rows(self))
        };
        let current = active_index.or_else(|| {
            archived
                .iter()
                .position(|item| item.matches(&selected))
                .map(|i| active.len() + i)
        });
        let Some(step) = session_step(active.len(), archived.len(), current, direction) else {
            return;
        };
        let target = match step {
            SessionStep::Active(index) => active.swap_remove(index),
            SessionStep::Archived(index) => {
                let Some(target) = archived[index].resolve(self) else {
                    return;
                };
                target
            }
        };
        let key = active_item_identity(&target);
        self.select_visible_session(target, window, cx);
        if matches!(step, SessionStep::Archived(_)) {
            self.sessions.archived_expanded = true;
            self.views.archived_session_rail.update(cx, |view, cx| {
                view.reveal = Some(SessionReveal::SessionID(key));
                cx.notify();
            });
        } else {
            self.reveal_active_session_row(key, cx);
        }
        self.notify_session_rail_shell(cx);
        cx.notify();
    }
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
