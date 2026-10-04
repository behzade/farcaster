use std::cell::RefCell;

use gpui::{
    InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, UniformListScrollHandle,
    WeakEntity, div, point, px, uniform_list,
};
use gpui_component::scroll::Scrollbar;

use super::{
    FarcasterApp,
    draft_row::{DraftRow, DraftRowInput},
    groups::{ActiveSessionItem, SessionRailKind, session_rail_lists_for_roots},
    rendering::{inactive_session_badge, session_section_drop_target, subagent_counts},
    rows::{SessionRow, SessionRowInput, session_row_height},
    session_item_identity,
};
use crate::{app::session::status::roots_waiting_for_descendants, app::ui::theme::theme};

fn archived_item_identity(item: &ActiveSessionItem) -> String {
    match item {
        ActiveSessionItem::Draft(draft) => format!("draft:{}", draft.id),
        ActiveSessionItem::Session(item) => session_item_identity(item),
    }
}

fn reconcile_archived_rows(
    scroll: &UniformListScrollHandle,
    current: &RefCell<Vec<String>>,
    next: Vec<String>,
) {
    let mut current = current.borrow_mut();
    if *current == next {
        return;
    }
    let mut state = scroll.0.borrow_mut();
    let offset = state.base_handle.offset();
    let row_height = session_row_height(true);
    let top = (-offset.y / row_height).floor().max(0.0) as usize;
    let new_top = current
        .get(top)
        .and_then(|key| next.iter().position(|row| row == key))
        .unwrap_or_else(|| top.min(next.len().saturating_sub(1)));
    let within_row = -offset.y - row_height * top as f32;
    let y = if next.is_empty() {
        px(0.0)
    } else {
        -(row_height * new_top as f32 + within_row)
    };
    state.base_handle.set_offset(point(offset.x, y));
    state.deferred_scroll_to_item = state.deferred_scroll_to_item.and_then(|mut pending| {
        let key = current.get(pending.item_index)?;
        pending.item_index = next.iter().position(|row| row == key)?;
        Some(pending)
    });
    *current = next;
}
impl FarcasterApp {
    pub(in crate::app::views) fn render_inactive_sessions(
        &self,
        entity: WeakEntity<Self>,
        kind: SessionRailKind,
        list_state: UniformListScrollHandle,
        list_rows: &RefCell<Vec<String>>,
    ) -> gpui::AnyElement {
        debug_assert!(kind != SessionRailKind::Project);
        let selected_root = self
            .selected_rail_root()
            .map(|session| session.path.clone());
        let live_root = self
            .sessions
            .visible
            .root_for_path(self.snapshot.live_session.as_deref())
            .map(|session| session.path.clone());
        let waiting_roots = roots_waiting_for_descendants(&self.sessions.all);
        let lists = session_rail_lists_for_roots(
            self.sessions.visible.roots(),
            &self.sessions.drafts,
            self.sessions.project_filter.as_deref(),
            &self.sessions.order,
        );
        let rows = match kind {
            SessionRailKind::Archived => lists.archived,
            SessionRailKind::Project => unreachable!("active sessions use the main rail"),
        };
        let empty = rows.is_empty();
        let counts = subagent_counts(&self.sessions.all);
        reconcile_archived_rows(
            &list_state,
            list_rows,
            rows.iter().map(archived_item_identity).collect(),
        );

        let row_entity = entity.clone();
        let selected_draft = self.sessions.selected_draft.clone();
        let submitted_drafts = self.sessions.submitted_drafts.clone();
        let editing_path = self
            .sessions
            .editing_title
            .as_ref()
            .map(|edit| edit.path.clone());
        let title_input = self.sessions.title_input.clone();
        let live_status = self.snapshot.live_status;
        let run_statuses = self.activity.run_statuses.clone();
        let section_scrollbar = list_state.clone();
        let rows_list = uniform_list("archived-session-rows", rows.len(), move |range, _, _| {
            range
                .map(|index| match rows.get(index) {
                    Some(ActiveSessionItem::Draft(draft)) => {
                        let selected = selected_draft.as_deref() == Some(draft.id.as_str());
                        let status = crate::app::session::drafts::resolved_draft_status(
                            &draft.id,
                            &submitted_drafts,
                            &run_statuses,
                        );
                        DraftRow::new(
                            draft,
                            DraftRowInput {
                                selected,
                                status,
                                archived: true,
                                drop_position: None,
                                compact: true,
                                shortcut: None,
                            },
                            row_entity.clone(),
                        )
                        .into_any_element()
                    }
                    Some(ActiveSessionItem::Session(item)) => {
                        let selected =
                            selected_root.as_deref() == Some(item.session.path.as_path());
                        let badge = inactive_session_badge(
                            kind,
                            item,
                            &run_statuses,
                            live_root.as_deref(),
                            live_status,
                            &waiting_roots,
                        );
                        let editing = editing_path.as_deref() == Some(item.session.path.as_path());
                        SessionRow::new(
                            item,
                            SessionRowInput {
                                compact: true,
                                title_editor: editing.then(|| title_input.clone()),
                                subagents: counts
                                    .get(item.session.path.as_path())
                                    .copied()
                                    .unwrap_or(0),
                                ..SessionRowInput::standard(selected, badge)
                            },
                            row_entity.clone(),
                        )
                        .into_any_element()
                    }
                    None => div().into_any_element(),
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&list_state)
        .size_full();

        let drop_entity = entity.clone();
        let body = if empty {
            div()
                .px(theme().space.md)
                .py(theme().space.sm)
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.subtle)
                .child("No archived sessions")
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_hidden()
                .child(rows_list)
                .into_any_element()
        };
        let section = div()
            .id("archived-sessions")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_y_hidden()
            .child(body)
            .child(Scrollbar::vertical(&section_scrollbar));
        session_section_drop_target(section, kind, drop_entity).into_any_element()
    }
}

#[cfg(test)]
#[path = "inactive_rail_tests.rs"]
mod tests;
