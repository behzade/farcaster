//! Session rail rendering and interaction.

use std::{collections::HashMap, path::PathBuf};

use gpui::{
    Anchor, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, list,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::{
    super::PiApp,
    session_groups::{
        ActiveSessionItem, SessionRailItem, merge_visible_session_order, recent_archived_sessions,
        reordered_session_ids, roots_waiting_for_descendants, session_rail_lists,
    },
    session_rows::{
        draft_session_row, project_label, session_badge, session_row, session_row_with_height,
    },
};
#[cfg(test)]
use super::{
    session_groups::SessionRailKind,
    session_rows::{session_accessible_label, status_visual},
};
use crate::{
    app::{PickerScope, ProjectPickerIntent},
    assets::AppIcon,
    primitives::{
        AppIconSize, ButtonTone, FeedbackTone, ReorderPosition, app_icon, disclosure_button,
        dropdown_button, feedback, icon_button,
    },
    projects::{self, DraftSession},
    sessions::{SessionSummary, root_session_for_path},
    theme::THEME,
};

const ARCHIVED_PREVIEW_LIMIT: usize = 3;

impl PiApp {
    pub(super) fn render_sessions(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let new_entity = entity.clone();
        let actions_entity = entity.clone();
        let cancel_drop_entity = entity.clone();
        let search_focus = self.search_focus.clone();
        let selected_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let live_root =
            root_session_for_path(&self.sessions, self.snapshot.live_session.as_deref())
                .map(|session| session.id.clone());
        let waiting_roots = roots_waiting_for_descendants(&self.all_sessions);
        let lists = session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        );
        let active_entry_count = lists.active.len();
        let archived_entry_count = lists.archived.len();
        let active_rows = lists.active;
        let session_shortcuts = visible_session_shortcuts(&active_rows);
        reconcile_list_rows(
            &self.session_list,
            &self.session_list_rows,
            active_rows.iter().map(active_item_identity).collect(),
        );

        let selected_draft = self.selected_draft.clone();
        let submitted_drafts = self.submitted_drafts.clone();
        let active_selected_root = selected_root.clone();
        let active_live_root = live_root.clone();
        let active_live_status = self.snapshot.live_status.clone();
        let active_run_statuses = self.run_statuses.clone();
        let active_waiting_roots = waiting_roots.clone();
        let active_row_entity = entity.clone();
        let active_editing_path = self
            .editing_session_title
            .as_ref()
            .map(|edit| edit.path.clone());
        let active_title_input = self.session_title_input.clone();
        let active_drop_target = self.session_drop_target;
        let shortcuts_visible = self.session_shortcuts_visible;
        let active_list = list(
            self.session_list.clone(),
            move |index, _, _| match active_rows.get(index) {
                Some(ActiveSessionItem::Draft(draft)) => {
                    let selected = selected_draft.as_deref() == Some(draft.id.as_str());
                    let status = crate::app::drafts::resolved_draft_status(
                        &draft.id,
                        &submitted_drafts,
                        &active_run_statuses,
                    );
                    let shortcut = shortcuts_visible
                        .then(|| session_shortcuts.get(&draft.app_session_id).copied())
                        .flatten();
                    let drop_position = active_drop_target
                        .filter(|(target, _)| *target == draft.app_session_id)
                        .map(|(_, position)| position);
                    draft_session_row(
                        draft,
                        selected,
                        &status,
                        shortcut,
                        drop_position,
                        active_row_entity.clone(),
                    )
                }
                Some(ActiveSessionItem::Session(item)) => {
                    let selected =
                        active_selected_root.as_deref() == Some(item.session.id.as_str());
                    let target = format!("session:{}", item.session.path.display());
                    let badge = session_badge(
                        item,
                        active_run_statuses.get(&target).map(String::as_str),
                        active_live_root.as_deref(),
                        &active_live_status,
                        active_waiting_roots.contains(&item.session.id),
                    );
                    let shortcut = shortcuts_visible
                        .then(|| session_shortcuts.get(&item.session.app_session_id).copied())
                        .flatten();
                    let editing =
                        active_editing_path.as_deref() == Some(item.session.path.as_path());
                    let drop_position = active_drop_target
                        .filter(|(target, _)| *target == item.session.app_session_id)
                        .map(|(_, position)| position);
                    session_row(
                        item,
                        selected,
                        badge,
                        shortcut,
                        drop_position,
                        true,
                        editing.then(|| active_title_input.clone()),
                        active_row_entity.clone(),
                    )
                }
                None => div().into_any_element(),
            },
        )
        .size_full();

        let archived_expanded = self.archived_sessions_expanded && archived_entry_count > 0;
        let archived_session_rail = self
            .archived_session_rail_view
            .clone()
            .cached(gpui::StyleRefinement::default());
        let projects = self.available_projects();
        let project_filter_entity = entity.clone();
        let filter_label = self
            .session_project_filter
            .as_deref()
            .map(project_label)
            .unwrap_or_else(|| "All".into());

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(THEME.colors.panel)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .p(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .child(
                        div()
                            .h(THEME.controls.utility_row)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(icon_button(
                                "session-actions",
                                AppIcon::List,
                                "Actions",
                                ButtonTone::Quiet,
                                move |window, cx| {
                                    let _ = actions_entity.update(cx, |this, cx| {
                                        this.open_picker(PickerScope::Actions, window, cx);
                                    });
                                },
                            ))
                            .child(icon_button(
                                "new-session",
                                AppIcon::Plus,
                                "New session",
                                ButtonTone::Quiet,
                                move |window, cx| {
                                    let _ = new_entity.update(cx, |this, cx| {
                                        this.open_picker(
                                            PickerScope::Projects(ProjectPickerIntent::NewSession),
                                            window,
                                            cx,
                                        );
                                    });
                                },
                            )),
                    )
                    .child(
                        div()
                            .id("session-search-surface")
                            .h(THEME.controls.utility_row)
                            .flex()
                            .items_center()
                            .gap(THEME.space.xs)
                            .pl(THEME.space.sm)
                            .rounded(THEME.radius)
                            .bg(THEME.colors.surface)
                            .text_color(THEME.colors.muted)
                            .on_click(move |_, window, cx| search_focus.focus(window, cx))
                            .child(app_icon(AppIcon::MagnifyingGlass, AppIconSize::Prominent))
                            .child(
                                Input::new(&self.search)
                                    .flex_1()
                                    .min_w_0()
                                    .appearance(false),
                            )
                            .child(
                                dropdown_button(
                                    "project-filter",
                                    filter_label,
                                    ButtonTone::Quiet,
                                    true,
                                )
                                .flex_none()
                                .dropdown_menu_with_anchor(
                                    Anchor::TopRight,
                                    move |menu, _, _| {
                                        let all_entity = project_filter_entity.clone();
                                        let mut menu = menu
                                            .min_w(px(220.0))
                                            .max_h(px(420.0))
                                            .label("Projects")
                                            .item(PopupMenuItem::new("All").on_click(
                                                move |_, _, cx| {
                                                    let _ = all_entity.update(cx, |this, cx| {
                                                        this.set_session_project_filter(None, cx);
                                                    });
                                                },
                                            ));
                                        for project in &projects {
                                            let target = project.clone();
                                            let filter_entity = project_filter_entity.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(project_label(project))
                                                    .on_click(move |_, _, cx| {
                                                        let _ =
                                                            filter_entity.update(cx, |this, cx| {
                                                                this.set_session_project_filter(
                                                                    Some(target.clone()),
                                                                    cx,
                                                                );
                                                            });
                                                    }),
                                            );
                                        }
                                        menu
                                    },
                                ),
                            ),
                    ),
            )
            .when_some(self.sessions_error.clone(), |rail, error| {
                rail.child(feedback("sessions-error", error, FeedbackTone::Error))
            })
            .child(
                div()
                    .id("session-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_hidden()
                    .on_mouse_up_out(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = cancel_drop_entity
                            .update(cx, |this, cx| this.clear_session_drop_target(cx));
                    })
                    .when(!archived_expanded, |lists| {
                        lists.child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_hidden()
                                .child(active_list),
                        )
                    })
                    .when(archived_entry_count > 0, |lists| {
                        lists.child(archived_session_rail)
                    }),
            )
            .when(
                active_entry_count == 0
                    && (!archived_expanded || archived_entry_count == 0)
                    && self.sessions_error.is_none(),
                |rail| {
                    rail.child(
                        div()
                            .px(THEME.space.md)
                            .py(THEME.space.sm)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child("No matching sessions"),
                    )
                },
            )
            .into_any_element()
    }

    pub(super) fn render_archived_sessions(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let selected_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let live_root = root_session_for_path(&self.sessions, self.snapshot.live_session.as_deref())
            .map(|session| session.id.clone());
        let waiting_roots = roots_waiting_for_descendants(&self.all_sessions);
        let archived_rows = session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        )
        .archived;
        let archived_entry_count = archived_rows.len();
        let archived_preview = recent_archived_sessions(&archived_rows, ARCHIVED_PREVIEW_LIMIT);
        reconcile_list_rows(
            &self.archived_session_list,
            &self.archived_session_list_rows,
            archived_rows.iter().map(archived_item_identity).collect(),
        );

        let archived_preview_elements = archived_preview
            .iter()
            .map(|item| {
                let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                let target = format!("session:{}", item.session.path.display());
                let badge = session_badge(
                    item,
                    self.run_statuses.get(&target).map(String::as_str),
                    live_root.as_deref(),
                    &self.snapshot.live_status,
                    waiting_roots.contains(&item.session.id),
                );
                let editing = self
                    .editing_session_title
                    .as_ref()
                    .is_some_and(|edit| edit.path == item.session.path);
                session_row_with_height(
                    item,
                    selected,
                    badge,
                    None,
                    None,
                    false,
                    editing.then(|| self.session_title_input.clone()),
                    THEME.controls.archived_preview_row,
                    entity.clone(),
                )
            })
            .collect::<Vec<_>>();
        let archived_live_status = self.snapshot.live_status.clone();
        let archived_run_statuses = self.run_statuses.clone();
        let archived_waiting_roots = waiting_roots;
        let archived_row_entity = entity.clone();
        let archived_editing_path = self
            .editing_session_title
            .as_ref()
            .map(|edit| edit.path.clone());
        let archived_title_input = self.session_title_input.clone();
        let archived_list = list(
            self.archived_session_list.clone(),
            move |index, _, _| match archived_rows.get(index) {
                Some(item) => {
                    let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                    let target = format!("session:{}", item.session.path.display());
                    let badge = session_badge(
                        item,
                        archived_run_statuses.get(&target).map(String::as_str),
                        live_root.as_deref(),
                        &archived_live_status,
                        archived_waiting_roots.contains(&item.session.id),
                    );
                    let editing =
                        archived_editing_path.as_deref() == Some(item.session.path.as_path());
                    session_row(
                        item,
                        selected,
                        badge,
                        None,
                        None,
                        false,
                        editing.then(|| archived_title_input.clone()),
                        archived_row_entity.clone(),
                    )
                }
                None => div().into_any_element(),
            },
        )
        .size_full();

        let archived_expanded = self.archived_sessions_expanded && archived_entry_count > 0;
        let archive_toggle_entity = entity;
        div()
            .id("archived-sessions")
            .min_h_0()
            .flex()
            .flex_col()
            .when(archived_expanded, |archived| archived.flex_1())
            .when(!archived_expanded, |archived| archived.flex_none())
            .child(
                div()
                    .h(THEME.controls.utility_row)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(THEME.space.md)
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
                    .text_size(THEME.type_scale.caption)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(THEME.colors.muted)
                    .child(format!("Archived · {archived_entry_count}"))
                    .child(disclosure_button(
                        "toggle-archived-sessions",
                        archived_expanded,
                        "Archived sessions",
                        move |_, cx| {
                            let _ = archive_toggle_entity.update(cx, |this, cx| {
                                this.archived_sessions_expanded =
                                    !this.archived_sessions_expanded;
                                this.notify_session_rail(cx);
                            });
                        },
                    )),
            )
            .when(!archived_expanded, |archived| {
                archived.children(archived_preview_elements)
            })
            .when(archived_expanded, |archived| {
                archived.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_hidden()
                        .child(archived_list),
                )
            })
            .into_any_element()
    }
}

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

fn archived_item_identity(item: &SessionRailItem) -> String {
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

fn visible_session_shortcuts(rows: &[ActiveSessionItem]) -> HashMap<i64, u8> {
    rows.iter()
        .filter_map(|row| match row {
            ActiveSessionItem::Draft(draft) if draft.submitted => Some(draft.app_session_id),
            ActiveSessionItem::Session(item) => Some(item.session.app_session_id),
            ActiveSessionItem::Draft(_) => None,
        })
        .filter(|id| *id > 0)
        .take(9)
        .enumerate()
        .map(|(index, id)| (id, (index + 1) as u8))
        .collect()
}

impl PiApp {
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

    fn select_visible_session(
        &mut self,
        target: VisibleSessionTarget,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
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

    pub(super) fn begin_session_drag(&mut self, cx: &mut gpui::Context<Self>) {
        if self.session_drop_target.take().is_some() {
            self.notify_session_rail(cx);
        }
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
        if self.session_drop_target.take().is_some() {
            self.notify_session_rail(cx);
        }
    }

    pub(super) fn complete_session_drop(&mut self, source: i64, cx: &mut gpui::Context<Self>) {
        let Some((target, position)) = self.session_drop_target.take() else {
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
        if let Some(order) = reordered_session_ids(&visible, source, target, position) {
            let all = session_rail_lists(&self.sessions, &self.drafts, None, &self.session_order)
                .active
                .iter()
                .map(ActiveSessionItem::app_session_id)
                .collect::<Vec<_>>();
            self.session_order = merge_visible_session_order(&all, &order);
            if let Err(error) = projects::save_app_session_order(&self.session_order) {
                self.sessions_error = Some(error);
            }
        }
        self.notify_session_rail(cx);
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

#[cfg(test)]
#[path = "session_rail_tests.rs"]
mod tests;
