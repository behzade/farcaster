use crate::{
    sessions::{DraftSession, SessionKey, SessionSummary},
    utility::persistent_vec::PersistentVec,
};

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    UniformListScrollHandle, WeakEntity, div, point, px, uniform_list,
};
use gpui_component::scroll::Scrollbar;

use super::{
    FarcasterApp,
    draft_row::{DraftRow, DraftRowInput},
    groups::{
        ActiveSessionItem, SessionRailItem, SessionRailKind, archived_session_rows_for_roots,
        draft_archive_order, session_archive_order,
    },
    rendering::session_section_drop_target,
    rows::{SessionRow, SessionRowInput, session_row_height},
};
use crate::{app::session::status::roots_waiting_for_descendants, app::ui::theme::theme};

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app::views) enum ArchivedItem {
    Draft(String, i64),
    Session(crate::sessions::SessionKey),
}

#[derive(Clone)]
pub(in crate::app::views) struct ArchivedRow {
    pub(in crate::app::views) item: ArchivedItem,
    pub(in crate::app::views) order: (u64, i64),
    pub(in crate::app::views) identity: String,
}

impl ArchivedRow {
    fn draft(row: &DraftSession) -> Self {
        Self {
            identity: format!("draft:{}", row.id),
            item: ArchivedItem::Draft(row.id.clone(), row.app_session_id),
            order: draft_archive_order(row),
        }
    }

    fn session(row: &SessionSummary) -> Self {
        let key = row.catalog_key();
        Self {
            identity: super::session_key_identity(&key),
            item: ArchivedItem::Session(key),
            order: session_archive_order(row),
        }
    }
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

pub(in crate::app::views) fn reconcile_archived_rows(
    scroll: &UniformListScrollHandle,
    previous: &PersistentVec<ArchivedRow>,
    next: &PersistentVec<ArchivedRow>,
) {
    remap_archived_scroll(scroll, next.len(), |index| {
        let old = previous.get(index)?;
        next.position(|row| row.identity == old.identity)
    });
}

pub(in crate::app::views) fn remap_archived_scroll(
    scroll: &UniformListScrollHandle,
    next_len: usize,
    new_index: impl Fn(usize) -> Option<usize>,
) {
    let mut state = scroll.0.borrow_mut();
    let offset = state.base_handle.offset();
    let row_height = session_row_height(false);
    let top = (-offset.y / row_height).floor().max(0.0) as usize;
    let new_top = new_index(top).unwrap_or_else(|| top.min(next_len.saturating_sub(1)));
    let within_row = -offset.y - row_height * top as f32;
    let y = if next_len == 0 {
        px(0.0)
    } else {
        -(row_height * new_top as f32 + within_row)
    };
    state.base_handle.set_offset(point(offset.x, y));
    state.deferred_scroll_to_item = state.deferred_scroll_to_item.and_then(|mut pending| {
        pending.item_index = new_index(pending.item_index)?;
        Some(pending)
    });
}
impl FarcasterApp {
    pub(in crate::app) fn update_archive_membership(
        &self,
        session: Option<&SessionKey>,
        draft: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if !self.views.archived_session_rail.read(cx).has_sorted_rows() {
            self.repaint_archived_session_rail(cx);
            return;
        }
        let catalog = &self.sessions.visible;
        let filter = self.sessions.project_filter.as_deref();
        let (item, archived) = if let Some(id) = draft {
            let Some(row) = self.sessions.drafts.iter().find(|row| row.id == id) else {
                self.notify_archived_session_rail(cx);
                return;
            };
            if session.is_some()
                || (row.app_session_id > 0 && catalog.by_app_id(row.app_session_id).is_some())
            {
                self.notify_archived_session_rail(cx);
                return;
            }
            if filter.is_some_and(|project| project != row.project) {
                return;
            }
            (ArchivedRow::draft(row), row.archived)
        } else if let Some(key) = session {
            let Some(row) = catalog.by_key(key) else {
                self.notify_archived_session_rail(cx);
                return;
            };
            if !catalog.is_root_row(row) || filter.is_some_and(|project| project != row.project) {
                return;
            }
            if row.app_session_id > 0
                && self.sessions.drafts.iter().any(|d| {
                    d.archived
                        && d.app_session_id == row.app_session_id
                        && filter.is_none_or(|project| project == d.project)
                })
            {
                return;
            }
            (ArchivedRow::session(row), row.archived)
        } else {
            return;
        };
        self.views.archived_session_rail.update(cx, |rail, cx| {
            rail.set_membership(item, archived);
            cx.notify();
        });
    }

    pub(in crate::app::views) fn prepare_archived_rows(&self) -> PersistentVec<ArchivedRow> {
        archived_session_rows_for_roots(
            self.sessions.visible.roots(),
            &self.sessions.drafts,
            self.sessions.project_filter.as_deref(),
        )
        .into_iter()
        .map(|item| match item {
            ActiveSessionItem::Draft(draft) => ArchivedRow::draft(&draft),
            ActiveSessionItem::Session(item) => ArchivedRow::session(&item.session),
        })
        .collect()
    }

    pub(in crate::app::views) fn render_inactive_sessions(
        &self,
        entity: WeakEntity<Self>,
        kind: SessionRailKind,
        list_state: UniformListScrollHandle,
        rows: PersistentVec<ArchivedRow>,
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
                .map(|index| match rows.get(index).map(|row| &row.item) {
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
