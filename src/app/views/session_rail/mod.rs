mod active_cache;
mod active_rail;
pub(in crate::app::views) use active_cache::PreparedActiveRows;
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

use crate::utility::persistent_vec::PersistentVec;
use gpui::{Pixels, px};

use self::{
    drag::DraggedSession,
    groups::{
        ActiveSessionItem, SessionRailItem, merge_visible_session_order, reordered_session_ids,
    },
    rendering::archived_panel_slot,
};
use super::super::FarcasterApp;
use super::SessionReveal;
use crate::{
    app::session::archive::RailSessionIdentity,
    app::ui::primitives::{ReorderPosition, ResizeBounds, panel_bounds},
    app::ui::theme::theme,
    sessions::{AppSessionId, DraftSession, SessionSummary},
};

pub(in crate::app) use group_drag::GroupTarget;
pub(in crate::app) use groups::SessionRailKind;
pub(in crate::app) use hover::{session_hover_details, session_tooltip_content};
#[cfg(test)]
pub(in crate::app::views) use inactive_rail::ArchivedItem;
pub(in crate::app::views) use inactive_rail::{
    ArchivedRow, reconcile_archived_rows, remap_archived_scroll,
};
pub(in crate::app) use rows::project_label;
pub(super) use rows::session_row_height;

pub(super) fn rail_inset() -> Pixels {
    crate::app::ui::grid::sidebar_inset()
}

#[cfg(test)]
use self::rows::session_accessible_label;

pub(super) fn clamped_session_rail_width(width: f32) -> Pixels {
    px(width.clamp(
        f32::from(theme().layout.session_rail_min),
        f32::from(theme().layout.session_rail_max),
    ))
}

impl RailSessionIdentity {
    fn for_draft(draft: &DraftSession) -> Self {
        AppSessionId::new(draft.app_session_id)
            .map_or_else(|| Self::Draft(draft.id.clone()), Self::App)
    }

    fn for_session(session: &SessionSummary) -> Self {
        AppSessionId::new(session.app_session_id).map_or_else(
            || Self::Session(session.path.clone(), session.native_identity()),
            Self::App,
        )
    }

    fn for_item(item: &ActiveSessionItem) -> Self {
        match item {
            ActiveSessionItem::Draft(draft) => Self::for_draft(draft),
            ActiveSessionItem::Session(item) => Self::for_session(&item.session),
        }
    }

    fn matches(&self, item: &ActiveSessionItem) -> bool {
        match (self, item) {
            (Self::App(id), item) => item.app_session_id() == id.get(),
            (Self::Draft(id), ActiveSessionItem::Draft(draft)) => draft.id == *id,
            (Self::Session(path, native), ActiveSessionItem::Session(item)) => {
                item.session.path == *path && item.session.native_identity() == *native
            }
            _ => false,
        }
    }
}

fn session_from_row(row: folders::FolderRow) -> Option<ActiveSessionItem> {
    match row {
        folders::FolderRow::Session(item) => Some(*item),
        folders::FolderRow::Header(..)
        | folders::FolderRow::Project { .. }
        | folders::FolderRow::New => None,
    }
}

fn active_item_identity(item: &ActiveSessionItem) -> String {
    match item {
        ActiveSessionItem::Draft(draft) => format!("draft:{}", draft.id),
        ActiveSessionItem::Session(item) => session_item_identity(item),
    }
}

fn session_item_identity(item: &SessionRailItem) -> String {
    session_row_identity(&item.session)
}

pub(in crate::app::views) fn session_row_identity(session: &SessionSummary) -> String {
    session_key_identity(&session.catalog_key())
}

fn session_key_identity(key: &crate::sessions::SessionKey) -> String {
    format!("session:{key:?}")
}

fn reconcile_list_rows(
    list: &gpui::ListState,
    current: &std::cell::RefCell<PersistentVec<String>>,
    next: Vec<String>,
) {
    let mut current = current.borrow_mut();
    if let Some((range, count)) = minimal_row_splice(&current, &next) {
        list.splice(range.clone(), count);
        current.splice(
            range.clone(),
            next.into_iter().skip(range.start).take(count),
        );
    }
}

fn minimal_row_splice<T: Eq + Clone>(
    current: &PersistentVec<T>,
    next: &[T],
) -> Option<(std::ops::Range<usize>, usize)> {
    let prefix = current
        .iter()
        .zip(next)
        .take_while(|(left, right)| left == right)
        .count();
    if prefix == current.len() && prefix == next.len() {
        return None;
    }
    let suffix = current
        .iter_rev()
        .zip(next.iter().rev())
        .take(current.len().min(next.len()) - prefix)
        .take_while(|(left, right)| left == right)
        .count();
    let old_end = current.len().saturating_sub(suffix);
    let replacement_count = next.len().saturating_sub(prefix + suffix);
    (prefix != old_end || replacement_count != 0).then_some((prefix..old_end, replacement_count))
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
                self.select_visible_session(ActiveSessionItem::Draft(draft), window, cx);
            }
            return;
        }
        let rows = self.visible_session_targets();
        if let Some(&target) = numbered_session_items(&rows).get(number - 1) {
            self.select_visible_session(target.clone(), window, cx);
        }
    }

    fn selected_rail_identity(&self) -> Option<RailSessionIdentity> {
        self.sessions
            .selected_draft
            .as_deref()
            .and_then(|id| self.sessions.drafts.iter().find(|draft| draft.id == id))
            .map(RailSessionIdentity::for_draft)
            .or_else(|| {
                self.selected_rail_root()
                    .map(RailSessionIdentity::for_session)
            })
    }

    pub(in crate::app) fn archive_selected_session_and_advance(
        &mut self,
        path: std::path::PathBuf,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let prepared = self.prepared_active_rows(cx);
        let selected = self
            .selected_rail_root()
            .map(RailSessionIdentity::for_session);
        let replacement = selected.and_then(|selected| prepared.replacement(&selected));

        if let Some(key) = self
            .sidebar_catalog_row(&self.sessions.all)
            .map(SessionSummary::catalog_key)
        {
            self.request_rail_archive(key, true, window, cx);
        } else if !self.sessions.all.has_shared_path(&path)
            && (self.lifecycle.pending_session_switch.is_none()
                || self.lifecycle.requested_session_key.is_none())
        {
            self.request_session_archive(path, true, window, cx);
        } else {
            return;
        }
        if let Some(pending) = self.sessions.pending_archive.as_mut() {
            pending.replacement = replacement.as_ref().map(RailSessionIdentity::for_item);
        } else if let Some(target) = replacement {
            self.select_visible_session(target, window, cx);
        }
    }

    pub(in crate::app) fn switch_relative_session(
        &mut self,
        direction: isize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut sessions = self.visible_session_targets();
        let Some(selected) = self.selected_rail_identity() else {
            return;
        };
        let Some(current) = sessions
            .iter()
            .position(|session| selected.matches(session))
        else {
            return;
        };
        if let Some(next) = current.checked_add_signed(direction)
            && next < sessions.len()
        {
            self.select_visible_session(sessions.swap_remove(next), window, cx);
        }
    }

    pub(in crate::app) fn select_visible_replacement(
        &mut self,
        replacement: RailSessionIdentity,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(target) = self.prepared_active_rows(cx).selectable(&replacement) {
            self.select_visible_session(target, window, cx);
        }
    }

    fn select_visible_session(
        &mut self,
        target: ActiveSessionItem,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.native_workspace_modal_active() {
            return;
        }
        match target {
            ActiveSessionItem::Draft(draft) => {
                self.resume_draft_and_focus(draft.id, draft.project, window, cx);
            }
            ActiveSessionItem::Session(item) => {
                self.select_rail_session(item.session.catalog_key(), window, cx);
            }
        }
    }

    fn prepared_active_rows(&self, cx: &mut gpui::Context<Self>) -> PreparedActiveRows {
        self.views
            .session_rail
            .update(cx, |view, _| view.prepared_rows(self))
    }

    fn session_folder_rows(&self, items: Vec<ActiveSessionItem>) -> Vec<folders::FolderRow> {
        if self.settings.group_sessions_by_project {
            folders::project_group_rows(
                items,
                &self.sessions.collapsed_projects,
                &self.sessions.folders.project_order,
            )
        } else {
            folders::folder_rows_with_pending(
                items,
                &self.sessions.folders,
                &self.sessions.pending_folder_drafts,
            )
        }
    }

    fn visible_session_targets(&self) -> Vec<ActiveSessionItem> {
        self.session_folder_rows(self.visible_active_items())
            .into_iter()
            .filter_map(session_from_row)
            .collect()
    }

    fn active_rail_lists(
        &self,
        project_filter: Option<&std::path::Path>,
    ) -> groups::SessionRailLists {
        groups::active_session_rail_lists_for_roots(
            self.sessions.visible.roots(),
            &self.sessions.drafts,
            project_filter,
            &self.sessions.order,
            |draft| self.present_rail_draft(draft),
        )
    }

    fn present_rail_draft(&self, draft: &mut DraftSession) {
        if draft.submitted || self.composer.pending_submissions.is_empty() {
            return;
        }
        if let Some(pending) = self
            .composer
            .pending_submissions
            .values()
            .filter(|pending| {
                pending.result.is_none()
                    && crate::sessions::draft_id(&pending.submitted_target)
                        == Some(draft.id.as_str())
            })
            .min_by_key(|pending| (pending.submitted_at, &pending.id))
        {
            draft.submitted = true;
            if draft.title.is_none() {
                draft.title = crate::sessions::provisional_session_title(&pending.text);
            }
        }
    }

    fn visible_active_items(&self) -> Vec<ActiveSessionItem> {
        self.active_rail_lists(self.sessions.project_filter.as_deref())
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

    fn panel_budget(&self) -> Pixels {
        self.views
            .panel_space
            .unwrap_or(theme().layout.window_height)
    }

    fn archive_bounds(&self, collapsed: bool, archived: usize) -> ResizeBounds {
        panel_bounds(
            self.panel_budget(),
            archived_panel_slot(archived, collapsed),
        )
    }

    pub(in crate::app::views) fn toggle_archive(&mut self, cx: &mut gpui::Context<Self>) {
        self.sessions.archived_expanded = !self.sessions.archived_expanded;
        if self.sessions.archived_expanded {
            self.views.archived_session_rail.update(cx, |view, _| {
                view.reveal = Some(SessionReveal::Index(0));
            });
        }
        self.save_panel_layout();
        self.notify_session_rail_shell(cx);
        self.repaint_archived_session_rail(cx);
    }

    fn begin_archive_resize(&mut self, pointer_y: Pixels, cx: &mut gpui::Context<Self>) {
        let count = self.prepared_active_rows(cx).archived_count;
        let bounds = self.archive_bounds(false, count);
        self.views.archived_panel.begin_resize(bounds, pointer_y);
        self.notify_session_rail_shell(cx);
    }

    pub(super) fn update_rail_panel_resize(
        &mut self,
        pointer_y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let count = self.prepared_active_rows(cx).archived_count;
        let bounds = self.archive_bounds(false, count);
        if self.views.archived_panel.update_resize(bounds, pointer_y) {
            self.notify_session_rail_shell(cx);
        }
    }

    pub(super) fn finish_rail_panel_resize(&mut self, cx: &mut gpui::Context<Self>) {
        if self.views.archived_panel.finish_resize() {
            self.save_panel_layout();
            self.notify_session_rail_shell(cx);
        }
    }

    pub(super) fn begin_session_drag(&mut self, cx: &mut gpui::Context<Self>) {
        self.sessions.drop_target = None;
        self.sessions.group_drop_target = None;
        self.notify_session_rail_shell(cx);
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
            self.notify_session_rail_shell(cx);
        }
    }

    pub(super) fn clear_session_drop_target(&mut self, cx: &mut gpui::Context<Self>) {
        self.sessions.drop_target = None;
        self.sessions.group_drop_target = None;
        self.notify_session_rail_shell(cx);
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
        let visible = self
            .active_rail_lists(self.sessions.project_filter.as_deref())
            .active
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>();
        if let Some(order) = reordered_session_ids(&visible, drag.app_session_id, target, position)
        {
            let all = self
                .active_rail_lists(None)
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
