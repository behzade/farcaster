mod active_rail;
mod colors;
mod draft_row;
mod drag;
mod folders;
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
        session_rail_lists,
    },
};
use super::super::FarcasterApp;
use crate::{
    app::session_folders::SessionFolders,
    app::ui::primitives::{PanelBounds, ReorderPosition},
    app::ui::theme::theme,
    sessions::DraftSession,
    sessions::{SessionSummary, root_session_for_path},
};

pub(in crate::app) use groups::SessionRailKind;
pub(in crate::app) use hover::{session_hover_details, session_tooltip_content};
pub(in crate::app) use rows::{project_label, status_visual};

#[cfg(test)]
use self::{
    rendering::{ARCHIVED_LEADING_GAP, collapsed_inactive_rail_height, subagent_counts},
    rows::session_accessible_label,
};

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
            folders::FolderRow::Header(..) => None,
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

#[cfg(test)]
fn first_unsubmitted_draft(rows: &[ActiveSessionItem]) -> Option<&DraftSession> {
    rows.iter().find_map(|row| match row {
        ActiveSessionItem::Draft(draft) if !draft.submitted => Some(draft),
        ActiveSessionItem::Draft(_) | ActiveSessionItem::Session(_) => None,
    })
}

/// The chats a session number can reach, in rail order. Numbers run from 1 to
/// 10 so `0` reaches the tenth chat, and a folder with fewer chats simply has
/// fewer reachable numbers.
fn numbered_session_items<'a>(
    items: &'a [ActiveSessionItem],
    folders: &SessionFolders,
    only: Option<u64>,
) -> Vec<(u64, &'a ActiveSessionItem)> {
    let mut numbered = Vec::new();
    for folder in &folders.folders {
        if only.is_some_and(|only| only != folder.id) {
            continue;
        }
        for item in items {
            if numbered.len() == 10 {
                return numbered;
            }
            if folders.folder_for_session(item.app_session_id(), item.project()) == Some(folder.id)
            {
                numbered.push((folder.id, item));
            }
        }
    }
    numbered
}

impl FarcasterApp {
    /// Selects the chat a session number points at. Numbers above nine arrive as
    /// `0`, which addresses the tenth chat.
    pub(in crate::app) fn switch_to_session_number(
        &mut self,
        number: usize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let number = if number == 0 { 10 } else { number };
        let Some((folder_id, target)) = self
            .numbered_session_targets()
            .get(number.saturating_sub(1))
            .cloned()
        else {
            return;
        };
        self.expand_folder(folder_id, cx);
        self.select_visible_session(target, window, cx);
    }

    fn numbered_session_targets(&self) -> Vec<(u64, VisibleSessionTarget)> {
        let items = self.visible_active_items();
        numbered_session_items(&items, &self.sessions.folders, self.current_folder_id())
            .into_iter()
            .filter_map(|(id, item)| {
                VisibleSessionTarget::from_item(item).map(|target| (id, target))
            })
            .collect()
    }

    /// The folder holding the chat that is open right now, when there is one.
    /// Without it the numbers address every folder's chats in rail order.
    fn current_folder_id(&self) -> Option<u64> {
        let selected = self.selected_app_session_id()?;
        let items = self.visible_active_items();
        let item = items
            .iter()
            .find(|item| item.app_session_id() == selected)?;
        self.sessions
            .folders
            .folder_for_session(item.app_session_id(), item.project())
    }

    fn expand_folder(&mut self, folder_id: u64, cx: &mut gpui::Context<Self>) {
        let collapsed = self
            .sessions
            .folders
            .folders
            .iter()
            .any(|folder| folder.id == folder_id && folder.collapsed);
        if collapsed {
            self.set_folder_collapsed(folder_id, false, cx);
        }
    }

    fn selected_app_session_id(&self) -> Option<i64> {
        self.sessions
            .selected_draft
            .as_deref()
            .and_then(|id| self.sessions.drafts.iter().find(|draft| draft.id == id))
            .map(|draft| draft.app_session_id)
            .or_else(|| {
                root_session_for_path(
                    &self.sessions.visible,
                    self.snapshot.selected_session.as_deref(),
                )
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
        let selected_id = root_session_for_path(&self.sessions.visible, Some(&path))
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

    fn visible_session_targets(&self) -> Vec<VisibleSessionTarget> {
        folders::folder_rows(self.visible_active_items(), &self.sessions.folders)
            .into_iter()
            .filter_map(VisibleSessionTarget::from_row)
            .collect()
    }

    fn visible_active_items(&self) -> Vec<ActiveSessionItem> {
        session_rail_lists(
            &self.sessions.visible,
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

    fn notification_panel_bounds(&self) -> PanelBounds {
        PanelBounds {
            height: theme().layout.notice_panel,
            min_height: theme().layout.notice_panel_min,
            max_height: theme().layout.notice_panel_max,
        }
    }

    pub(super) fn toggle_notification_panel(&mut self, cx: &mut gpui::Context<Self>) {
        let collapsed = self.views.notification_panel.is_collapsed();
        self.views.notification_panel.set_collapsed(!collapsed);
        if collapsed {
            self.extensions.active.mark_notifications_seen();
        }
        self.notify_session_rail_shell(cx);
    }

    pub(super) fn begin_notification_panel_resize(
        &mut self,
        pointer_y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let bounds = self.notification_panel_bounds();
        self.views
            .notification_panel
            .begin_resize(bounds, pointer_y);
        self.notify_session_rail_shell(cx);
    }

    pub(super) fn update_notification_panel_resize(
        &mut self,
        pointer_y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let bounds = self.notification_panel_bounds();
        if self
            .views
            .notification_panel
            .update_resize(bounds, pointer_y)
        {
            self.notify_session_rail_shell(cx);
        }
    }

    pub(super) fn finish_notification_panel_resize(&mut self, cx: &mut gpui::Context<Self>) {
        if self.views.notification_panel.finish_resize() {
            self.notify_session_rail_shell(cx);
        }
    }

    pub(super) fn begin_session_drag(&mut self, cx: &mut gpui::Context<Self>) {
        self.sessions.drop_target = None;
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
        let target_folder = self.sessions.folders.folder_for(target);
        if !self.assign_session_folder(drag.app_session_id, target_folder, cx) {
            return;
        }
        let visible = session_rail_lists(
            &self.sessions.visible,
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
            let all = session_rail_lists(
                &self.sessions.visible,
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
        if self.sessions.project_filter != project {
            self.sessions.project_filter = project;
            self.sessions.archived_expanded = false;
            self.notify_session_rail(cx);
        }
    }
}
