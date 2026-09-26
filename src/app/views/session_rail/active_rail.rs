use std::cell::RefCell;

use gpui::{
    Anchor, AnyElement, InteractiveElement as _, IntoElement, ListState, MouseButton,
    ParentElement as _, Styled as _, WeakEntity, div, list, prelude::FluentBuilder as _,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::scroll::Scrollbar;

use super::{
    FarcasterApp, RailPanel, active_item_identity,
    colors::palette_color,
    draft_row::{DraftRow, DraftRowInput},
    folders::{FolderRow, folder_header, new_folder_row, project_header},
    groups::{ActiveSessionItem, session_rail_lists_for_roots},
    numbered_session_items, reconcile_list_rows,
    rendering::{active_session_drop_target, subagent_counts},
    rows::{SessionRow, SessionRowInput, project_label},
};
use crate::{
    app::PickerScope,
    app::ProjectPickerIntent,
    app::session::status::{resolved_session_status, roots_waiting_for_active_descendants},
    app::ui::assets::AppIcon,
    app::ui::layout::TRAFFIC_LIGHT_INSET,
    app::ui::primitives::{
        AppIconSize, ButtonTone, ContextMenuTrigger, FeedbackTone, Panel, SearchField, app_icon,
        feedback, icon_button, panel_space,
    },
    app::ui::theme::theme,
};

fn notification_tone(tone: crate::protocol::NotifyTone) -> FeedbackTone {
    match tone {
        crate::protocol::NotifyTone::Error => FeedbackTone::Error,
        crate::protocol::NotifyTone::Warning => FeedbackTone::Warning,
        crate::protocol::NotifyTone::Info => FeedbackTone::Info,
    }
}

impl FarcasterApp {
    fn render_rail_notices(&self, entity: WeakEntity<Self>) -> Option<AnyElement> {
        let task_notice = self.render_code_task_notice(entity)?;
        Some(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .px(theme().size(10.0))
                .pb(theme().space.sm)
                .child(task_notice)
                .into_any_element(),
        )
    }

    fn render_rail_panel(
        &self,
        panel: RailPanel,
        collapsed: bool,
        entity: &WeakEntity<Self>,
        children: impl IntoIterator<Item = AnyElement>,
    ) -> Panel {
        let (id, title) = panel.labels();
        let mut state = self.panel_state(panel);
        state.set_collapsed(collapsed);
        let toggle_entity = entity.clone();
        let resize_entity = entity.clone();
        Panel::new(id, &state, self.panel_bounds(panel, collapsed), title)
            .on_toggle(move |_, cx| {
                let _ = toggle_entity.update(cx, |this, cx| this.toggle_rail_panel(panel, cx));
            })
            .on_resize(move |event, _, cx| {
                let _ = resize_entity.update(cx, |this, cx| {
                    this.begin_rail_panel_resize(panel, event.position.y, cx)
                });
            })
            .children(children)
    }

    fn render_notification_panel(&self, entity: WeakEntity<Self>) -> AnyElement {
        let collapsed = self.panel_state(RailPanel::Notifications).is_collapsed();
        let notifications = self
            .extensions
            .active
            .notification_history
            .iter()
            .rev()
            .enumerate()
            .map(|(index, notice)| {
                feedback(
                    ("rail-notification", index),
                    notice.message.clone(),
                    notification_tone(notice.tone),
                )
            })
            .collect::<Vec<_>>();
        self.render_rail_panel(RailPanel::Notifications, collapsed, &entity, notifications)
            .badge(self.extensions.active.unseen_notifications())
            .into_any_element()
    }

    pub(in crate::app::views) fn render_sessions(
        &self,
        entity: WeakEntity<Self>,
        session_drag_active: bool,
        session_list: ListState,
        session_list_rows: &RefCell<Vec<String>>,
    ) -> impl IntoElement {
        let new_entity = entity.clone();
        let actions_entity = entity.clone();
        let cancel_drop_entity = entity.clone();
        let cancel_drop_out_entity = entity.clone();
        let active_drop_entity = entity.clone();
        let selected_root = self.selected_rail_root().map(|session| session.id.clone());
        let live_root = self
            .sessions
            .visible
            .root_for_path(self.snapshot.live_session.as_deref())
            .map(|session| session.id.clone());
        let waiting_roots =
            roots_waiting_for_active_descendants(&self.sessions.all, &self.activity.agents);
        let lists = session_rail_lists_for_roots(
            self.sessions.visible.roots(),
            &self.sessions.drafts,
            self.sessions.project_filter.as_deref(),
            &self.sessions.order,
        );
        let counts = subagent_counts(&self.sessions.all);
        let session_colors = self.sessions.folders.session_colors.clone();
        let active_entry_count = lists.active.len();
        let archived_entry_count = lists.archived.len();
        let active_rows = lists.active;
        let last_active_row = if !self.settings.group_sessions_by_project
            && self.sessions.folders.folders.is_empty()
        {
            active_rows
                .last()
                .map(|item| (active_entry_count - 1, item.app_session_id()))
        } else {
            None
        };
        let active_drop_list = session_list.clone();
        let mut active_rows = self.session_folder_rows(active_rows);
        let compact = self.settings.group_sessions_by_project;
        if !compact {
            active_rows.push(FolderRow::New);
        }
        let shortcuts = numbered_session_items(active_rows.iter().filter_map(|row| match row {
            FolderRow::Session(item) => Some(item.as_ref()),
            _ => None,
        }))
        .into_iter()
        .enumerate()
        .map(|(index, item)| (item.app_session_id(), (index + 1) as u8))
        .collect::<std::collections::HashMap<_, _>>();
        let group_drop_target = self.sessions.group_drop_target.clone();
        let project_colors = self.sessions.folders.project_colors.clone();
        let editing_folder = self.sessions.editing_folder.map(|edit| edit.id);
        reconcile_list_rows(
            &session_list,
            session_list_rows,
            active_rows
                .iter()
                .map(|row| match row {
                    FolderRow::Session(item) => active_item_identity(item),
                    FolderRow::Header(folder) => format!("folder:{}", folder.id),
                    FolderRow::New => "new-folder".into(),
                    FolderRow::Project { path, .. } => format!("project:{}", path.display()),
                })
                .collect(),
        );

        let selected_draft = self.sessions.selected_draft.clone();
        let submitted_drafts = self.sessions.submitted_drafts.clone();
        let active_selected_root = selected_root.clone();
        let active_live_root = live_root.clone();
        let active_live_status = self.snapshot.live_status.clone();
        let active_run_statuses = self.activity.run_statuses.clone();
        let active_waiting_roots = waiting_roots.clone();
        let active_row_entity = entity.clone();
        let active_editing_path = self
            .sessions
            .editing_title
            .as_ref()
            .map(|edit| edit.path.clone());
        let active_title_input = self.sessions.title_input.clone();
        let active_drop_target = self.sessions.drop_target;
        let rail_scrollbar = session_list.clone();
        let active_list = list(session_list, move |index, _, _| {
            match active_rows.get(index) {
                Some(FolderRow::Session(item)) => match item.as_ref() {
                    ActiveSessionItem::Draft(draft) => {
                        let selected = selected_draft.as_deref() == Some(draft.id.as_str());
                        let status = crate::app::session::drafts::resolved_draft_status(
                            &draft.id,
                            &submitted_drafts,
                            &active_run_statuses,
                        );
                        let drop_position = active_drop_target
                            .filter(|(target, _)| *target == draft.app_session_id)
                            .map(|(_, position)| position);
                        DraftRow::new(
                            draft,
                            DraftRowInput {
                                selected,
                                status,
                                archived: false,
                                drop_position,
                                compact,
                                shortcut: shortcuts.get(&draft.app_session_id).copied(),
                            },
                            active_row_entity.clone(),
                        )
                        .into_any_element()
                    }
                    ActiveSessionItem::Session(item) => {
                        let selected =
                            active_selected_root.as_deref() == Some(item.session.id.as_str());
                        let target = format!("session:{}", item.session.path.display());
                        let badge = Some(resolved_session_status(
                            &item.session,
                            active_run_statuses.get(&target).map(String::as_str),
                            active_live_root.as_deref(),
                            &active_live_status,
                            active_waiting_roots.contains(&item.session.id),
                        ));
                        let editing =
                            active_editing_path.as_deref() == Some(item.session.path.as_path());
                        let drop_position = active_drop_target
                            .filter(|(target, _)| *target == item.session.app_session_id)
                            .map(|(_, position)| position);
                        SessionRow::new(
                            item,
                            SessionRowInput {
                                selected,
                                color: session_colors
                                    .get(&item.session.app_session_id)
                                    .copied()
                                    .map(palette_color),
                                status: badge,
                                drop_position,
                                draggable: true,
                                title_editor: editing.then(|| active_title_input.clone()),
                                subagents: counts
                                    .get(item.session.id.as_str())
                                    .copied()
                                    .unwrap_or(0),
                                compact,
                                shortcut: shortcuts.get(&item.session.app_session_id).copied(),
                            },
                            active_row_entity.clone(),
                        )
                        .into_any_element()
                    }
                },
                Some(FolderRow::Header(folder)) => folder_header(
                    (**folder).clone(),
                    editing_folder == Some(Some(folder.id)),
                    active_title_input.clone(),
                    group_drop_target
                        .as_ref()
                        .filter(|(target, _)| target == &super::GroupTarget::Folder(folder.id))
                        .map(|(_, position)| *position),
                    active_row_entity.clone(),
                ),
                Some(FolderRow::Project {
                    path,
                    label,
                    collapsed,
                    count,
                }) => project_header(
                    path.clone(),
                    label.clone(),
                    *collapsed,
                    *count,
                    project_colors.get(path).copied(),
                    group_drop_target
                        .as_ref()
                        .filter(|(target, _)| target == &super::GroupTarget::Project(path.clone()))
                        .map(|(_, position)| *position),
                    active_row_entity.clone(),
                ),
                Some(FolderRow::New) => new_folder_row(
                    editing_folder == Some(None),
                    active_title_input.clone(),
                    active_row_entity.clone(),
                ),
                None => div().into_any_element(),
            }
        })
        .size_full();
        let archived_panel = self
            .render_rail_panel(
                RailPanel::Archived,
                !self.sessions.archived_expanded || session_drag_active,
                &entity,
                [div()
                    .flex_1()
                    .min_h_0()
                    .child(self.views.archived_session_rail.clone())
                    .into_any_element()],
            )
            .count(archived_entry_count)
            .flush_body();
        let projects = self.available_projects();
        let project_filter_entity = entity.clone();
        let filter_label = self
            .sessions
            .project_filter
            .as_deref()
            .map(project_label)
            .unwrap_or_else(|| "All".into());

        let measure_entity = entity.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme().colors.panel)
            .on_children_prepainted(move |bounds, _, cx| {
                let space = bounds
                    .last()
                    .map(|stack| panel_space(stack.size.height, theme().layout.folders_min));
                let _ = measure_entity.update(cx, |this, cx| {
                    if this.views.panel_space != space {
                        this.views.panel_space = space;
                        cx.notify();
                    }
                });
            })
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(theme().space.xs)
                    .px(theme().size(10.0))
                    .pb(theme().size(10.0))
                    .child(
                        div()
                            .h(theme().size(38.0))
                            .flex()
                            .items_center()
                            .justify_end()
                            .when(cfg!(target_os = "macos"), |row| {
                                row.pl(theme().size(TRAFFIC_LIGHT_INSET))
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .h_full()
                                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                                        window.start_window_move()
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(theme().space.xs)
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
                        SearchField::new("session-search", &self.navigation.search)
                            .accessible_label("Search sessions")
                            .trailing(
                                ContextMenuTrigger::new(
                                    "project-filter-menu",
                                    div()
                                        .id("project-filter-button")
                                        .h_full()
                                        .flex()
                                        .items_center()
                                        .gap(theme().space.xs)
                                        .px(theme().space.sm)
                                        .border_l(theme().border)
                                        .border_color(theme().colors.border)
                                        .cursor_pointer()
                                        .text_size(theme().type_scale.caption)
                                        .text_color(theme().colors.muted)
                                        .hover(|filter| filter.bg(theme().colors.highlight))
                                        .child(filter_label)
                                        .child(app_icon(AppIcon::CaretDown, AppIconSize::Inline))
                                        .into_any_element(),
                                )
                                .h_full()
                                .dropdown_menu_with_anchor(
                                    Anchor::TopRight,
                                    move |menu, _, _| {
                                        let all_entity = project_filter_entity.clone();
                                        let mut menu = menu
                                            .min_w(theme().size(220.0))
                                            .max_h(theme().size(420.0))
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
            .when_some(self.sessions.error.clone(), |rail, error| {
                rail.child(feedback("sessions-error", error, FeedbackTone::Error))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
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
                            .child(active_session_drop_target(
                                div()
                                    .id("active-session-drop-area")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_hidden()
                                    .child(active_list),
                                active_drop_list,
                                last_active_row,
                                active_drop_entity,
                            ))
                            .child(Scrollbar::vertical(&rail_scrollbar))
                            .when(
                                active_entry_count == 0
                                    && !self.sessions.visible.is_empty()
                                    && self.sessions.error.is_none(),
                                |list| {
                                    list.child(
                                        div()
                                            .px(theme().space.md)
                                            .py(theme().space.sm)
                                            .text_size(theme().type_scale.caption)
                                            .text_color(theme().colors.subtle)
                                            .child("No matching sessions"),
                                    )
                                },
                            ),
                    )
                    .child(archived_panel)
                    .when_some(
                        self.render_rail_notices(entity.clone()),
                        |stack, notices| stack.child(notices),
                    )
                    .child(self.render_notification_panel(entity)),
            )
            .into_any_element()
    }
}
