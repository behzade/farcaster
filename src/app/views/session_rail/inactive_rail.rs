use std::cell::RefCell;

use gpui::{
    InteractiveElement as _, IntoElement, ListState, ParentElement as _, Styled as _, WeakEntity,
    div, list,
};
use gpui_component::scroll::Scrollbar;

use super::{
    FarcasterApp,
    draft_row::{DraftRow, DraftRowInput},
    groups::{ActiveSessionItem, SessionRailKind, session_rail_lists},
    reconcile_list_rows,
    rendering::{inactive_session_badge, session_section_drop_target, subagent_counts},
    rows::{SessionRow, SessionRowInput},
    session_item_identity,
};

/// Archived chats are chat rows whatever they are — a draft that was filed away
/// before anything was ever sent is still a chat.
fn archived_item_identity(item: &ActiveSessionItem) -> String {
    match item {
        ActiveSessionItem::Draft(draft) => format!("draft:{}", draft.id),
        ActiveSessionItem::Session(item) => session_item_identity(item),
    }
}
use crate::{app::session::status::roots_waiting_for_descendants, sessions::root_session_for_path};

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
        let rows = match kind {
            SessionRailKind::Archived => lists.archived,
            SessionRailKind::Project => unreachable!("active sessions use the main rail"),
        };
        let counts = subagent_counts(&self.sessions.all);
        reconcile_list_rows(
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
        let live_status = self.snapshot.live_status.clone();
        let run_statuses = self.activity.run_statuses.clone();
        let section_scrollbar = list_state.clone();
        let rows_list = list(list_state, move |index, _, _| match rows.get(index) {
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
                        nested: false,
                    },
                    row_entity.clone(),
                )
                .into_any_element()
            }
            Some(ActiveSessionItem::Session(item)) => {
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
                        project_badge: false,
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
        let section = div()
            .id("archived-sessions")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_y_hidden()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_hidden()
                    .child(rows_list),
            )
            .child(Scrollbar::vertical(&section_scrollbar));
        session_section_drop_target(section, kind, drop_entity).into_any_element()
    }
}
