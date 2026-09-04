use gpui::{
    Anchor, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, list,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::{
    FarcasterApp, active_item_identity,
    draft_row::draft_session_row,
    groups::{
        ActiveSessionItem, SessionRailKind, roots_waiting_for_descendants, session_rail_lists,
    },
    reconcile_list_rows,
    rendering::{inactive_rail_style, session_section_drop_target, subagent_counts},
    rows::{project_label, session_badge, session_row},
    visible_session_shortcuts,
};
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{
        AppIconSize, ButtonTone, FeedbackTone, app_icon, dropdown_button, feedback, icon_button,
    },
    app::ui::theme::THEME,
    app::{PickerScope, ProjectPickerIntent},
    sessions::root_session_for_path,
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_sessions(
        &self,
        entity: WeakEntity<Self>,
        session_drag_active: bool,
    ) -> impl IntoElement {
        let new_entity = entity.clone();
        let actions_entity = entity.clone();
        let cancel_drop_entity = entity.clone();
        let cancel_drop_out_entity = entity.clone();
        let active_drop_entity = entity.clone();
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
        let counts = subagent_counts(&self.all_sessions);
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
        let shortcuts_visible = self.view.session_rail.shortcuts_visible;
        let active_list = list(
            self.session_list.clone(),
            move |index, _, _| match active_rows.get(index) {
                Some(ActiveSessionItem::Draft(draft)) => {
                    let selected = selected_draft.as_deref() == Some(draft.id.as_str());
                    let status = crate::app::session::drafts::resolved_draft_status(
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
                        counts.get(item.session.id.as_str()).copied().unwrap_or(0),
                        active_row_entity.clone(),
                    )
                }
                None => div().into_any_element(),
            },
        )
        .size_full();

        let archived_expanded =
            !session_drag_active && self.archived_sessions_expanded && archived_entry_count > 0;
        let archived_session_rail_style =
            inactive_rail_style(archived_expanded, archived_entry_count, true);
        let archived_session_rail = self
            .archived_session_rail_view
            .clone()
            .cached(archived_session_rail_style);
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
                    .px(px(10.0))
                    .pb(px(10.0))
                    .child(
                        div().h(px(47.0)).flex().items_center().justify_end().child(
                            div()
                                .flex()
                                .items_center()
                                .gap(THEME.space.xs)
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
                                                PickerScope::Projects(
                                                    ProjectPickerIntent::NewSession,
                                                ),
                                                window,
                                                cx,
                                            );
                                        });
                                    },
                                )),
                        ),
                    )
                    .child(
                        div()
                            .id("session-search-surface")
                            .h(px(36.0))
                            .flex()
                            .items_center()
                            .gap(THEME.space.xs)
                            .pl(px(10.0))
                            .rounded(px(5.0))
                            .border(THEME.border)
                            .border_color(THEME.colors.hover)
                            .bg(THEME.colors.surface)
                            .text_color(THEME.colors.muted)
                            .on_click(move |_, window, cx| search_focus.focus(window, cx))
                            .child(app_icon(AppIcon::MagnifyingGlass, AppIconSize::Inline))
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
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_hidden()
                    .on_mouse_up(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = cancel_drop_entity
                            .update(cx, |this, cx| this.clear_session_drop_target(cx));
                    })
                    .on_mouse_up_out(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = cancel_drop_out_entity
                            .update(cx, |this, cx| this.clear_session_drop_target(cx));
                    })
                    .when(!archived_expanded, |lists| {
                        lists.child(session_section_drop_target(
                            div()
                                .id("active-session-drop-area")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_hidden()
                                .child(active_list),
                            SessionRailKind::Project,
                            active_drop_entity,
                        ))
                    })
                    .when(archived_entry_count > 0, |lists| {
                        lists.child(archived_session_rail)
                    }),
            )
            .when(
                active_entry_count == 0
                    && archived_entry_count == 0
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
}
