use std::{cell::RefCell, rc::Rc};

use gpui::{
    InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, UniformListScrollHandle,
    WeakEntity, div, point, px, uniform_list,
};
use gpui_component::scroll::Scrollbar;

use super::{
    FarcasterApp,
    draft_row::{DraftRow, DraftRowInput},
    groups::{
        ActiveSessionItem, SessionRailItem, SessionRailKind, archived_session_rows_for_roots,
    },
    rendering::session_section_drop_target,
    rows::{SessionRow, SessionRowInput, session_row_height},
    session_item_identity,
};
use crate::{app::session::status::roots_waiting_for_descendants, app::ui::theme::theme};

pub(in crate::app::views) enum ArchivedItem {
    Draft(String, i64),
    Session(crate::sessions::SessionKey),
}

impl ArchivedItem {
    pub(super) fn matches(&self, selected: &super::RailSessionIdentity) -> bool {
        use super::RailSessionIdentity;
        use crate::sessions::SessionKey;
        match (self, selected) {
            (Self::Draft(_, id), RailSessionIdentity::App(selected)) => *id == selected.get(),
            (Self::Draft(id, _), RailSessionIdentity::Draft(selected)) => id == selected,
            (Self::Session(SessionKey::App(id)), RailSessionIdentity::App(selected)) => {
                id == selected
            }
            (
                Self::Session(SessionKey::Locator {
                    harness,
                    profile_id,
                    path,
                }),
                RailSessionIdentity::Session(selected, native),
            ) => path == selected && *harness == native.harness && profile_id == &native.profile_id,
            _ => false,
        }
    }

    pub(super) fn resolve(&self, app: &FarcasterApp) -> Option<ActiveSessionItem> {
        match self {
            Self::Draft(id, _) => app
                .sessions
                .drafts
                .iter()
                .find(|row| &row.id == id)
                .cloned()
                .map(ActiveSessionItem::Draft),
            Self::Session(key) => Some(ActiveSessionItem::Session(SessionRailItem {
                session: app.sessions.visible.by_key(key)?.clone(),
                kind: SessionRailKind::Archived,
            })),
        }
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
    let row_height = session_row_height(false);
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
    pub(in crate::app::views) fn prepare_archived_rows(
        &self,
        list_state: &UniformListScrollHandle,
        list_rows: &RefCell<Vec<String>>,
    ) -> Rc<Vec<ArchivedItem>> {
        let items = archived_session_rows_for_roots(
            self.sessions.visible.roots(),
            &self.sessions.drafts,
            self.sessions.project_filter.as_deref(),
        );
        let mut keys = Vec::with_capacity(items.len());
        let mut identities = Vec::with_capacity(items.len());
        for item in items {
            match item {
                ActiveSessionItem::Draft(draft) => {
                    identities.push(format!("draft:{}", draft.id));
                    keys.push(ArchivedItem::Draft(draft.id, draft.app_session_id));
                }
                ActiveSessionItem::Session(item) => {
                    identities.push(session_item_identity(&item));
                    keys.push(ArchivedItem::Session(item.session.catalog_key()));
                }
            }
        }
        reconcile_archived_rows(list_state, list_rows, identities);
        Rc::new(keys)
    }

    pub(in crate::app::views) fn render_inactive_sessions(
        &self,
        entity: WeakEntity<Self>,
        kind: SessionRailKind,
        list_state: UniformListScrollHandle,
        rows: Rc<Vec<ArchivedItem>>,
    ) -> gpui::AnyElement {
        debug_assert!(kind != SessionRailKind::Project);
        let selected_root = self
            .selected_rail_root()
            .map(|session| session.catalog_key());
        let waiting_roots = roots_waiting_for_descendants(&self.sessions.all);
        let empty = rows.is_empty();

        let row_entity = entity.clone();
        let section_scrollbar = list_state.clone();
        let rows_list = uniform_list("archived-session-rows", rows.len(), move |range, _, cx| {
            let Some(entity) = row_entity.upgrade() else {
                return Vec::new();
            };
            let app = entity.read(cx);
            range
                .map(|index| match rows.get(index) {
                    Some(ArchivedItem::Draft(id, _)) => {
                        let Some(draft) = app.sessions.drafts.iter().find(|draft| &draft.id == id)
                        else {
                            return div().into_any_element();
                        };
                        let selected = app.sessions.selected_draft.as_ref() == Some(id);
                        let status = app.draft_rail_status(draft);
                        DraftRow::new(
                            draft,
                            DraftRowInput {
                                selected,
                                status,
                                archived: true,
                                drop_position: None,
                                compact: false,
                            },
                            row_entity.clone(),
                        )
                        .into_any_element()
                    }
                    Some(ArchivedItem::Session(key)) => {
                        let Some(session) = app.sessions.visible.by_key(key) else {
                            return div().into_any_element();
                        };
                        let selected = selected_root.as_ref() == Some(key);
                        let status = app.rail_row_status(session, waiting_roots.contains(key));
                        let badge = (status != crate::runtime::RunStatus::Done).then_some(status);
                        let editing = app
                            .sessions
                            .editing_title
                            .as_ref()
                            .is_some_and(|edit| edit.path == session.path);
                        SessionRow::new(
                            SessionRailItem {
                                session: session.clone(),
                                kind,
                            },
                            SessionRowInput {
                                compact: false,
                                title_editor: editing.then(|| app.sessions.title_input.clone()),
                                subagents: app.sessions.all.child_count(session),
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
