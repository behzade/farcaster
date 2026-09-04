mod active_rail;
mod draft_row;
mod drag;
mod groups;
mod hover;
mod inactive_rail;
mod rendering;
mod rows;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use gpui::{Pixels, px};

use self::{
    drag::DraggedSession,
    groups::{
        ActiveSessionItem, SessionRailItem, merge_visible_session_order, reordered_session_ids,
        session_rail_lists,
    },
};
use super::super::FarcasterApp;
use crate::{
    app::ui::primitives::ReorderPosition,
    app::ui::theme::THEME,
    projects::DraftSession,
    sessions::{SessionSummary, root_session_for_path},
};

pub(in crate::app) use groups::{SessionRailKind, roots_waiting_for_descendants};
pub(in crate::app) use rows::project_label;

#[cfg(test)]
use self::{
    rendering::{ARCHIVED_LEADING_GAP, collapsed_inactive_rail_height, subagent_counts},
    rows::{session_accessible_label, session_badge, status_visual},
};

fn clamped_session_rail_width(width: f32) -> Pixels {
    px(width.clamp(
        f32::from(THEME.layout.session_rail_min),
        f32::from(THEME.layout.session_rail_max),
    ))
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum VisibleSessionTarget {
    Draft(DraftSession),
    Persisted(SessionSummary),
}

impl VisibleSessionTarget {
    fn app_session_id(&self) -> i64 {
        match self {
            Self::Draft(draft) => draft.app_session_id,
            Self::Persisted(session) => session.app_session_id,
        }
    }
}

fn active_item_identity(item: &ActiveSessionItem) -> String {
    match item {
        ActiveSessionItem::Draft(draft) => format!("draft:{}", draft.id),
        ActiveSessionItem::Session(item) => format!("session:{}", item.session.id),
    }
}

fn session_item_identity(item: &SessionRailItem) -> String {
    format!("session:{}", item.session.id)
}

fn reconcile_list_rows(
    list: &gpui::ListState,
    current: &std::cell::RefCell<Vec<String>>,
    next: Vec<String>,
) {
    let mut current = current.borrow_mut();
    if let Some((range, count)) = minimal_row_splice(&current, &next) {
        list.splice(range, count);
        *current = next;
    }
}

fn minimal_row_splice<T: Eq>(current: &[T], next: &[T]) -> Option<(std::ops::Range<usize>, usize)> {
    let prefix = current
        .iter()
        .zip(next)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = current[prefix..]
        .iter()
        .rev()
        .zip(next[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let old_end = current.len().saturating_sub(suffix);
    let replacement_count = next.len().saturating_sub(prefix + suffix);
    (prefix != old_end || replacement_count != 0).then_some((prefix..old_end, replacement_count))
}

fn replacement_index_after_close(len: usize, current: usize) -> Option<usize> {
    (current + 1 < len)
        .then_some(current + 1)
        .or_else(|| current.checked_sub(1))
}

fn first_unsubmitted_draft(rows: &[ActiveSessionItem]) -> Option<&DraftSession> {
    rows.iter().find_map(|row| match row {
        ActiveSessionItem::Draft(draft) if !draft.submitted => Some(draft),
        ActiveSessionItem::Draft(_) | ActiveSessionItem::Session(_) => None,
    })
}

fn visible_session_shortcuts(rows: &[ActiveSessionItem]) -> HashMap<i64, u8> {
    let mut shortcuts = rows
        .iter()
        .filter_map(|row| match row {
            ActiveSessionItem::Draft(draft) if draft.submitted => Some(draft.app_session_id),
            ActiveSessionItem::Session(item) => Some(item.session.app_session_id),
            ActiveSessionItem::Draft(_) => None,
        })
        .filter(|id| *id > 0)
        .take(9)
        .enumerate()
        .map(|(index, id)| (id, (index + 1) as u8))
        .collect::<HashMap<_, _>>();
    if let Some(draft) = first_unsubmitted_draft(rows)
        && draft.app_session_id > 0
    {
        shortcuts.insert(draft.app_session_id, 0);
    }
    shortcuts
}

impl FarcasterApp {
    pub(super) fn switch_to_first_unsubmitted_draft(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let lists = session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        );
        if let Some(draft) = first_unsubmitted_draft(&lists.active).cloned() {
            self.select_visible_session(VisibleSessionTarget::Draft(draft), window, cx);
        }
    }

    pub(super) fn switch_to_session_number(
        &mut self,
        number: usize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(target) = self
            .visible_session_targets()
            .get(number.saturating_sub(1))
            .cloned()
        {
            self.select_visible_session(target, window, cx);
        }
    }

    pub(super) fn archive_selected_session_and_advance(
        &mut self,
        path: std::path::PathBuf,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let sessions = self.visible_session_targets();
        let selected_id = root_session_for_path(&self.sessions, Some(&path))
            .map(|session| session.app_session_id);
        let replacement = selected_id
            .and_then(|id| {
                sessions
                    .iter()
                    .position(|session| session.app_session_id() == id)
            })
            .and_then(|index| replacement_index_after_close(sessions.len(), index))
            .and_then(|index| sessions.get(index))
            .map(VisibleSessionTarget::app_session_id);

        self.request_session_archive_and_advance(path, replacement, window, cx);
    }

    pub(super) fn switch_relative_session(
        &mut self,
        direction: isize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let sessions = self.visible_session_targets();
        let selected_id = self
            .selected_draft
            .as_deref()
            .and_then(|id| self.drafts.iter().find(|draft| draft.id == id))
            .map(|draft| draft.app_session_id)
            .or_else(|| {
                root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                    .map(|session| session.app_session_id)
            });
        let selected = selected_id.and_then(|selected_id| {
            sessions
                .iter()
                .position(|session| session.app_session_id() == selected_id)
        });
        let Some(current) = selected else { return };
        let next = current as isize + direction;
        if next >= 0
            && let Some(target) = sessions.get(next as usize).cloned()
        {
            self.select_visible_session(target, window, cx);
        }
    }

    pub(in crate::app) fn select_visible_app_session(
        &mut self,
        app_session_id: i64,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(target) = self
            .visible_session_targets()
            .into_iter()
            .find(|target| target.app_session_id() == app_session_id)
        {
            self.select_visible_session(target, window, cx);
        }
    }

    fn select_visible_session(
        &mut self,
        target: VisibleSessionTarget,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.workspace_switch_blocked() {
            return;
        }
        match target {
            VisibleSessionTarget::Draft(draft) => {
                self.resume_draft(draft.id, draft.project, window, cx);
            }
            VisibleSessionTarget::Persisted(session) => {
                self.select_session(session.path, session.project, window, cx);
            }
        }
    }

    fn visible_session_targets(&self) -> Vec<VisibleSessionTarget> {
        session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        )
        .active
        .into_iter()
        .filter_map(|row| match row {
            ActiveSessionItem::Draft(draft) if draft.submitted => {
                Some(VisibleSessionTarget::Draft(draft))
            }
            ActiveSessionItem::Session(item) => Some(VisibleSessionTarget::Persisted(item.session)),
            ActiveSessionItem::Draft(_) => None,
        })
        .collect()
    }

    pub(super) fn begin_session_rail_resize(
        &mut self,
        pointer_x: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        self.view.session_rail.resize_start = Some((pointer_x, self.view.session_rail.width));
        cx.notify();
    }

    pub(super) fn update_session_rail_resize(
        &mut self,
        pointer_x: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((start_x, start_width)) = self.view.session_rail.resize_start else {
            return;
        };
        let width = clamped_session_rail_width(
            f32::from(start_width) + f32::from(pointer_x) - f32::from(start_x),
        );
        if width != self.view.session_rail.width {
            self.view.session_rail.width = width;
            cx.notify();
        }
    }

    pub(super) fn finish_session_rail_resize(&mut self, cx: &mut gpui::Context<Self>) {
        if self.view.session_rail.resize_start.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn begin_session_drag(&mut self, cx: &mut gpui::Context<Self>) {
        self.session_drop_target = None;
        self.notify_session_rail(cx);
    }

    pub(super) fn update_session_drop_target(
        &mut self,
        target: i64,
        position: ReorderPosition,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = Some((target, position));
        if self.session_drop_target != next {
            self.session_drop_target = next;
            self.notify_session_rail(cx);
        }
    }

    pub(super) fn clear_session_drop_target(&mut self, cx: &mut gpui::Context<Self>) {
        self.session_drop_target = None;
        self.notify_session_rail(cx);
    }

    fn complete_session_row_drop(
        &mut self,
        drag: &DraggedSession,
        target_kind: SessionRailKind,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if drag.can_move_to(target_kind) {
            self.complete_session_category_drop(drag, target_kind, window, cx);
            return;
        }
        if target_kind != SessionRailKind::Project {
            self.clear_session_drop_target(cx);
            return;
        }
        let Some((target, position)) = self.session_drop_target.take() else {
            self.clear_session_drop_target(cx);
            return;
        };
        let visible = session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        )
        .active
        .iter()
        .map(ActiveSessionItem::app_session_id)
        .collect::<Vec<_>>();
        if let Some(order) = reordered_session_ids(&visible, drag.app_session_id, target, position)
        {
            let all = session_rail_lists(&self.sessions, &self.drafts, None, &self.session_order)
                .active
                .iter()
                .map(ActiveSessionItem::app_session_id)
                .collect::<Vec<_>>();
            let active_order = merge_visible_session_order(&all, &order);
            let active_ids = all.into_iter().collect::<HashSet<_>>();
            self.session_order.retain(|id| !active_ids.contains(id));
            self.session_order.extend(active_order);
            if let Err(error) =
                crate::app::project::registry::save_app_session_order(&self.session_order)
            {
                self.sessions_error = Some(error);
            }
        }
        self.notify_session_rail(cx);
    }

    fn complete_session_category_drop(
        &mut self,
        drag: &DraggedSession,
        target_kind: SessionRailKind,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.session_drop_target = None;
        let Some(path) = drag.path.clone() else {
            self.notify_session_rail(cx);
            return;
        };
        match target_kind {
            SessionRailKind::Project => self.set_session_active(path, cx),
            SessionRailKind::Archived => {
                self.notify_session_rail(cx);
                self.request_session_archive(path, true, window, cx);
            }
        }
    }

    fn set_session_project_filter(
        &mut self,
        project: Option<PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.session_project_filter != project {
            self.session_project_filter = project;
            self.archived_sessions_expanded = false;
            self.notify_session_rail(cx);
        }
    }
}
