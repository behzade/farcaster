mod active_rail;
mod colors;
mod draft_row;
mod drag;
mod folders;
mod group_drag;
mod groups;
mod hover;
mod inactive_rail;
mod navigation;
mod rendering;
mod rows;
#[cfg(test)]
mod tests;

use std::{collections::HashSet, path::PathBuf};

use gpui::{Pixels, px};

use self::{
    drag::DraggedSession,
    groups::{
        ActiveSessionItem, SessionRailItem, merge_visible_session_order, reordered_session_ids,
        session_rail_lists_for_roots,
    },
    rendering::{archived_panel_rows, rail_panel_slot},
};
use super::super::FarcasterApp;
use crate::{
    app::ui::primitives::{
        PanelSlot, ReorderPosition, ResizeBounds, ResizeState, panel_bounds, panel_resized,
        panel_room,
    },
    app::ui::theme::theme,
    sessions::DraftSession,
    sessions::SessionSummary,
};

pub(in crate::app) use group_drag::GroupTarget;
pub(in crate::app) use groups::SessionRailKind;
pub(in crate::app) use hover::{session_hover_details, session_tooltip_content};
pub(super) use rows::session_row_height;
pub(in crate::app) use rows::{project_label, status_visual};

#[cfg(test)]
use self::{rendering::subagent_counts, rows::session_accessible_label};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RailPanel {
    Archived,
    Notifications,
}

impl RailPanel {
    pub(super) const ALL: [Self; 2] = [Self::Archived, Self::Notifications];

    fn labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Archived => ("archived-panel", "Archived"),
            Self::Notifications => ("notification-panel", "Notifications"),
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

pub(super) fn clamped_session_rail_width(width: f32) -> Pixels {
    px(width.clamp(
        f32::from(theme().layout.session_rail_min),
        f32::from(theme().layout.session_rail_max),
    ))
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum VisibleSessionTarget {
    Draft(DraftSession),
    Persisted(SessionSummary),
}

impl VisibleSessionTarget {
    fn from_row(row: folders::FolderRow) -> Option<Self> {
        match row {
            folders::FolderRow::Session(item) => Self::from_item(&item),
            folders::FolderRow::Header(..)
            | folders::FolderRow::Project { .. }
            | folders::FolderRow::New => None,
        }
    }

    fn from_item(item: &ActiveSessionItem) -> Option<Self> {
        match item {
            ActiveSessionItem::Draft(draft) => Some(Self::Draft(draft.clone())),
            ActiveSessionItem::Session(item) => Some(Self::Persisted(item.session.clone())),
        }
    }

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

fn numbered_session_items<'a>(
    items: impl IntoIterator<Item = &'a ActiveSessionItem>,
) -> Vec<&'a ActiveSessionItem> {
    items
        .into_iter()
        .filter(|item| {
            item.app_session_id() > 0
                && !matches!(item, ActiveSessionItem::Draft(draft) if !draft.submitted)
        })
        .take(9)
        .collect()
}

impl FarcasterApp {
    pub(in crate::app) fn switch_to_session_number(
        &mut self,
        number: usize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if number == 0 {
            if let Some(draft) = first_unsubmitted_draft(&self.visible_active_items()).cloned() {
                self.select_visible_session(VisibleSessionTarget::Draft(draft), window, cx);
            }
            return;
        }
        let rows = self
            .session_folder_rows(self.visible_active_items())
            .into_iter()
            .filter_map(|row| match row {
                folders::FolderRow::Session(item) => Some(*item),
                folders::FolderRow::Header(_)
                | folders::FolderRow::Project { .. }
                | folders::FolderRow::New => None,
            })
            .collect::<Vec<_>>();
        if let Some(target) = numbered_session_items(&rows)
            .get(number - 1)
            .and_then(|item| VisibleSessionTarget::from_item(item))
        {
            self.select_visible_session(target, window, cx);
        }
    }

    fn selected_app_session_id(&self) -> Option<i64> {
        self.sessions
            .selected_draft
            .as_deref()
            .and_then(|id| self.sessions.drafts.iter().find(|draft| draft.id == id))
            .map(|draft| draft.app_session_id)
            .or_else(|| {
                self.sessions
                    .visible
                    .root_for_path(self.snapshot.selected_session.as_deref())
                    .map(|session| session.app_session_id)
            })
    }

    pub(in crate::app) fn archive_selected_session_and_advance(
        &mut self,
        path: std::path::PathBuf,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let sessions = self.selectable_session_targets();
        let selected_id = self
            .sessions
            .visible
            .root_for_path(Some(&path))
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

    pub(in crate::app) fn switch_relative_session(
        &mut self,
        direction: isize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let sessions = self.visible_session_targets();
        let selected_id = self.selected_app_session_id();
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
            .selectable_session_targets()
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
        if self.native_workspace_modal_active() {
            return;
        }
        match target {
            VisibleSessionTarget::Draft(draft) => {
                self.resume_draft_and_focus(draft.id, draft.project, window, cx);
            }
            VisibleSessionTarget::Persisted(session) => {
                self.select_session_and_focus(session.path, session.project, window, cx);
            }
        }
    }

    fn selectable_session_targets(&self) -> Vec<VisibleSessionTarget> {
        let mut targets = self.visible_session_targets();
        targets.retain(
            |target| !matches!(target, VisibleSessionTarget::Draft(draft) if !draft.submitted),
        );
        targets
    }

    fn session_folder_rows(&self, items: Vec<ActiveSessionItem>) -> Vec<folders::FolderRow> {
        if self.settings.group_sessions_by_project {
            folders::project_group_rows(
                items,
                &self.sessions.collapsed_projects,
                &self.sessions.folders.project_order,
            )
        } else {
            folders::folder_rows(items, &self.sessions.folders)
        }
    }

    fn visible_session_targets(&self) -> Vec<VisibleSessionTarget> {
        self.session_folder_rows(self.visible_active_items())
            .into_iter()
            .filter_map(VisibleSessionTarget::from_row)
            .collect()
    }

    fn visible_active_items(&self) -> Vec<ActiveSessionItem> {
        session_rail_lists_for_roots(
            self.sessions.visible.roots(),
            &self.sessions.drafts,
            self.sessions.project_filter.as_deref(),
            &self.sessions.order,
        )
        .active
    }

    pub(super) fn begin_session_rail_resize(
        &mut self,
        pointer_x: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        self.views
            .session_rail
            .update(cx, |view, _| view.begin_resize(pointer_x));
        cx.notify();
    }

    pub(super) fn update_session_rail_resize(
        &mut self,
        pointer_x: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let changed = self.views.session_rail.update(cx, |view, cx| {
            let changed = view.update_resize(pointer_x);
            if changed {
                cx.notify();
            }
            changed
        });
        if changed {
            cx.notify();
        }
    }

    pub(super) fn finish_session_rail_resize(&mut self, cx: &mut gpui::Context<Self>) {
        if self
            .views
            .session_rail
            .update(cx, |view, _| view.finish_resize())
        {
            cx.notify();
        }
    }

    fn panel_state(&self, panel: RailPanel) -> ResizeState {
        match panel {
            RailPanel::Archived => self.views.archived_panel,
            RailPanel::Notifications => self.views.notification_panel,
        }
    }

    fn panel_state_mut(&mut self, panel: RailPanel) -> &mut ResizeState {
        match panel {
            RailPanel::Archived => &mut self.views.archived_panel,
            RailPanel::Notifications => &mut self.views.notification_panel,
        }
    }

    fn panel_collapsed(&self, panel: RailPanel) -> bool {
        match panel {
            RailPanel::Archived => !self.sessions.archived_expanded,
            RailPanel::Notifications => self.views.notification_panel.is_collapsed(),
        }
    }

    fn panel_slot(&self, panel: RailPanel, collapsed: bool) -> PanelSlot {
        rail_panel_slot(panel, self.archived_session_count(), collapsed)
    }

    fn rail_panel_sizes(&self) -> (Vec<Pixels>, Vec<Pixels>) {
        RailPanel::ALL
            .into_iter()
            .map(|panel| {
                let slot = self.panel_slot(panel, self.panel_collapsed(panel));
                (
                    self.panel_state(panel).height_or(slot.preferred),
                    slot.floor,
                )
            })
            .unzip()
    }

    fn panel_budget(&self) -> Pixels {
        self.views
            .panel_space
            .unwrap_or_else(|| theme().layout.notice_panel_max)
    }

    pub(super) fn panel_bounds(&self, panel: RailPanel, collapsed: bool) -> ResizeBounds {
        let (sizes, floors) = self.rail_panel_sizes();
        let index = panel.index();
        let resizing = self.panel_state(panel).is_resizing();
        let room = panel_room(&sizes, &floors, self.panel_budget(), index, resizing);
        panel_bounds(room, self.panel_slot(panel, collapsed))
    }

    fn archived_session_count(&self) -> usize {
        session_rail_lists_for_roots(
            self.sessions.visible.roots(),
            &self.sessions.drafts,
            self.sessions.project_filter.as_deref(),
            &self.sessions.order,
        )
        .archived
        .len()
    }
    fn archived_visible_rows(&self) -> usize {
        if !self.sessions.archived_expanded {
            return 0;
        }
        archived_panel_rows(
            self.panel_state(RailPanel::Archived)
                .height(self.panel_bounds(RailPanel::Archived, false)),
        )
    }

    pub(super) fn toggle_rail_panel(&mut self, panel: RailPanel, cx: &mut gpui::Context<Self>) {
        match panel {
            RailPanel::Archived => {
                self.sessions.archived_expanded = !self.sessions.archived_expanded;
            }
            RailPanel::Notifications => {
                let collapsed = self.views.notification_panel.is_collapsed();
                self.views.notification_panel.set_collapsed(!collapsed);
                if collapsed {
                    self.extensions.active.mark_notifications_seen();
                }
            }
        }
        self.save_panel_layout();
        self.notify_session_rail(cx);
    }

    pub(super) fn begin_rail_panel_resize(
        &mut self,
        panel: RailPanel,
        pointer_y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let bounds = self.panel_bounds(panel, false);
        self.panel_state_mut(panel).begin_resize(bounds, pointer_y);
        self.notify_session_rail_shell(cx);
    }

    pub(super) fn update_rail_panel_resize(
        &mut self,
        pointer_y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let (mut sizes, floors) = self.rail_panel_sizes();
        for panel in RailPanel::ALL {
            if !self.panel_state(panel).is_resizing() {
                continue;
            }
            let index = panel.index();
            let bounds = self.panel_bounds(panel, false);
            sizes[index] = self.panel_state(panel).height(bounds);
            if !self.panel_state_mut(panel).update_resize(bounds, pointer_y) {
                continue;
            }
            let after = self.panel_state(panel).height(bounds);
            for (panel, height) in RailPanel::ALL.into_iter().zip(panel_resized(
                &sizes,
                &floors,
                self.panel_budget(),
                index,
                after,
            )) {
                self.panel_state_mut(panel).set_height(height);
            }
            self.notify_session_rail_shell(cx);
        }
    }

    pub(super) fn finish_rail_panel_resize(&mut self, cx: &mut gpui::Context<Self>) {
        for panel in RailPanel::ALL {
            if self.panel_state_mut(panel).finish_resize() {
                self.save_panel_layout();
                self.notify_session_rail_shell(cx);
            }
        }
    }

    pub(super) fn begin_session_drag(&mut self, cx: &mut gpui::Context<Self>) {
        self.sessions.drop_target = None;
        self.sessions.group_drop_target = None;
        self.notify_session_rail(cx);
    }

    pub(super) fn update_session_drop_target(
        &mut self,
        target: i64,
        position: ReorderPosition,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = Some((target, position));
        if self.sessions.drop_target != next {
            self.sessions.drop_target = next;
            self.notify_session_rail(cx);
        }
    }

    pub(super) fn clear_session_drop_target(&mut self, cx: &mut gpui::Context<Self>) {
        self.sessions.drop_target = None;
        self.sessions.group_drop_target = None;
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
        let Some((target, position)) = self.sessions.drop_target.take() else {
            self.clear_session_drop_target(cx);
            return;
        };
        if !self.settings.group_sessions_by_project {
            let target_folder = self.sessions.folders.folder_for(target);
            if !self.assign_session_folder(drag.app_session_id, target_folder, cx) {
                return;
            }
        }
        let visible = session_rail_lists_for_roots(
            self.sessions.visible.roots(),
            &self.sessions.drafts,
            self.sessions.project_filter.as_deref(),
            &self.sessions.order,
        )
        .active
        .iter()
        .map(ActiveSessionItem::app_session_id)
        .collect::<Vec<_>>();
        if let Some(order) = reordered_session_ids(&visible, drag.app_session_id, target, position)
        {
            let all = session_rail_lists_for_roots(
                self.sessions.visible.roots(),
                &self.sessions.drafts,
                None,
                &self.sessions.order,
            )
            .active
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>();
            let active_order = merge_visible_session_order(&all, &order);
            let active_ids = all.into_iter().collect::<HashSet<_>>();
            self.sessions.order.retain(|id| !active_ids.contains(id));
            self.sessions.order.extend(active_order);
            if let Err(error) =
                crate::app::project::registry::save_app_session_order(&self.sessions.order)
            {
                self.sessions.error = Some(error);
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
        self.sessions.drop_target = None;
        self.request_chat_archive(
            drag.app_session_id,
            target_kind == SessionRailKind::Archived,
            window,
            cx,
        );
    }

    fn set_session_project_filter(
        &mut self,
        project: Option<PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.sessions.project_filter != project {
            self.sessions.project_filter = project;
            self.sessions.archived_expanded = false;
            self.notify_session_rail(cx);
        }
    }
}
