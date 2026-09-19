use std::cell::RefCell;

use gpui::{
    InteractiveElement as _, IntoElement, ListState, ParentElement as _, Styled as _, WeakEntity,
    div, list, prelude::FluentBuilder as _, px,
};
use gpui_component::scroll::Scrollbar;

use super::{
    FarcasterApp,
    groups::{SessionRailKind, session_rail_lists},
    reconcile_list_rows,
    rendering::{
        ARCHIVED_LEADING_GAP, INACTIVE_PREVIEW_LIMIT, inactive_session_badge,
        session_section_drop_target, session_section_header, subagent_counts,
    },
    rows::{SessionRow, SessionRowInput},
    session_item_identity,
};
use crate::{
    app::session::status::roots_waiting_for_descendants, app::ui::primitives::disclosure_button,
    app::ui::theme::theme, sessions::root_session_for_path,
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_inactive_sessions(
        &self,
        entity: WeakEntity<Self>,
        kind: SessionRailKind,
        list_state: ListState,
        list_rows: &RefCell<Vec<String>>,
    ) -> gpui::AnyElement {
        debug_assert!(kind != SessionRailKind::Project);
        let selected_root = self.selected_rail_root().map(|session| session.id.clone());
        let live_root = root_session_for_path(
            &self.sessions.visible,
            self.snapshot.live_session.as_deref(),
        )
        .map(|session| session.id.clone());
        let waiting_roots = roots_waiting_for_descendants(&self.sessions.all);
        let lists = session_rail_lists(
            &self.sessions.visible,
            &self.sessions.drafts,
            self.sessions.project_filter.as_deref(),
            &self.sessions.order,
        );
        let (rows, expanded) = match kind {
            SessionRailKind::Archived => (lists.archived, self.sessions.archived_expanded),
            SessionRailKind::Project => unreachable!("active sessions use the main rail"),
        };
        let count = rows.len();
        let expanded = expanded && count > 0;
        let counts = subagent_counts(&self.sessions.all);
        reconcile_list_rows(
            &list_state,
            list_rows,
            rows.iter().map(session_item_identity).collect(),
        );

        let preview_elements = rows
            .iter()
            .take(INACTIVE_PREVIEW_LIMIT)
            .map(|item| {
                let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                let badge = inactive_session_badge(
                    kind,
                    item,
                    &self.activity.run_statuses,
                    live_root.as_deref(),
                    &self.snapshot.live_status,
                    &waiting_roots,
                );
                let editing = self
                    .sessions
                    .editing_title
                    .as_ref()
                    .is_some_and(|edit| edit.path == item.session.path);
                SessionRow::new(
                    item,
                    SessionRowInput {
                        title_editor: editing.then(|| self.sessions.title_input.clone()),
                        subagents: counts.get(item.session.id.as_str()).copied().unwrap_or(0),
                        row_height: theme().controls.archived_preview_row,
                        ..SessionRowInput::standard(selected, badge)
                    },
                    entity.clone(),
                )
            })
            .collect::<Vec<_>>();
        let row_entity = entity.clone();
        let editing_path = self
            .sessions
            .editing_title
            .as_ref()
            .map(|edit| edit.path.clone());
        let title_input = self.sessions.title_input.clone();
        let live_status = self.snapshot.live_status.clone();
        let run_statuses = self.activity.run_statuses.clone();
        let section_scrollbar = list_state.clone();
        let rows_list = list(list_state, move |index, _, _| match rows.get(index) {
            Some(item) => {
                let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                let badge = inactive_session_badge(
                    kind,
                    item,
                    &run_statuses,
                    live_root.as_deref(),
                    &live_status,
                    &waiting_roots,
                );
                let editing = editing_path.as_deref() == Some(item.session.path.as_path());
                SessionRow::new(
                    item,
                    SessionRowInput {
                        title_editor: editing.then(|| title_input.clone()),
                        subagents: counts.get(item.session.id.as_str()).copied().unwrap_or(0),
                        ..SessionRowInput::standard(selected, badge)
                    },
                    row_entity.clone(),
                )
                .into_any_element()
            }
            None => div().into_any_element(),
        })
        .size_full();

        let drop_entity = entity.clone();
        let toggle_entity = entity;
        let section = div()
            .id("archived-sessions")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .when(!expanded, |section| section.pt(px(ARCHIVED_LEADING_GAP)))
            .child(
                session_section_header()
                    .child(format!("Archived · {count}"))
                    .child(disclosure_button(
                        "toggle-archived-sessions",
                        expanded,
                        "Archived sessions",
                        move |_, cx| {
                            let _ = toggle_entity.update(cx, |this, cx| {
                                this.sessions.archived_expanded = !this.sessions.archived_expanded;
                                this.notify_session_rail(cx);
                            });
                        },
                    )),
            )
            .when(!expanded, |section| section.children(preview_elements))
            .when(expanded, |section| {
                section
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_hidden()
                            .child(rows_list),
                    )
                    .child(Scrollbar::vertical(&section_scrollbar))
            });
        session_section_drop_target(section, kind, drop_entity).into_any_element()
    }
}
