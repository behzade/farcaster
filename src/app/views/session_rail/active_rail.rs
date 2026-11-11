use crate::app::ui::primitives::DropdownMenu as _;
use std::cell::RefCell;

use gpui::{
    Anchor, AnyElement, InteractiveElement as _, IntoElement, ListState, MouseButton,
    ParentElement as _, Styled as _, WeakEntity, div, list, prelude::FluentBuilder as _,
};
use gpui_component::menu::PopupMenuItem;
use gpui_component::scroll::Scrollbar;

use super::{
    FarcasterApp, active_item_identity,
    colors::palette_color,
    draft_row::{DraftRow, DraftRowInput},
    folders::{FolderRow, folder_header, new_folder_row, project_header},
    groups::{ActiveSessionItem, active_session_rail_lists_for_roots},
    reconcile_list_rows,
    rendering::{active_session_drop_target, subagent_counts},
    rows::{SessionRow, SessionRowInput, project_label},
};
use crate::{
    app::PickerScope,
    app::ProjectPickerIntent,
    app::infrastructure::performance::{OperationKind, OperationTiming},
    app::session::status::{resolved_session_status, roots_waiting_for_active_descendants},
    app::ui::assets::AppIcon,
    app::ui::layout::TRAFFIC_LIGHT_INSET,
    app::ui::primitives::{
        AppIconSize, ButtonTone, ContextMenuTrigger, FeedbackTone, Panel, SearchField, app_icon,
        feedback, icon_button, icon_button_with_size, icon_control, panel_space,
    },
    app::ui::theme::theme,
};

impl FarcasterApp {
    fn render_archive_panel(
        &self,
        collapsed: bool,
        archived_count: usize,
        entity: &WeakEntity<Self>,
        children: impl IntoIterator<Item = AnyElement>,
    ) -> Panel {
        let mut state = self.views.archived_panel;
        state.set_collapsed(collapsed);
        let toggle_entity = entity.clone();
        let resize_entity = entity.clone();
        let bounds = self.archive_bounds(collapsed, archived_count);
        Panel::new("archived-panel", &state, bounds, "Archived")
            .on_toggle(move |_, cx| {
                let _ = toggle_entity.update(cx, |this, cx| this.toggle_archive(cx));
            })
            .on_resize(move |event, _, cx| {
                let _ = resize_entity.update(cx, |this, cx| {
                    this.begin_archive_resize(event.position.y, cx)
                });
            })
            .children(children)
    }

    fn session_search_controls(&self, entity: WeakEntity<Self>) -> AnyElement {
        let projects = self.available_projects();
        let selected = self.sessions.project_filter.clone();
        let filter_label = selected.as_deref().map_or_else(
            || "Filter sessions by project".to_owned(),
            |path| format!("Filter: {}", project_label(path)),
        );
        let filter_entity = entity.clone();
        let project_entity = entity.clone();
        let filter = ContextMenuTrigger::new(
            "project-filter-menu",
            icon_control("project-filter-button", filter_label)
                .text_color(if selected.is_some() {
                    theme().colors.indicator
                } else {
                    theme().colors.muted
                })
                .hover(|control| control.bg(theme().colors.highlight))
                .child(app_icon(AppIcon::FunnelSimple, AppIconSize::Control))
                .into_any_element(),
        )
        .w(theme().controls.icon_button)
        .flex_none()
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let all_entity = filter_entity.clone();
            let mut menu = menu
                .min_w(theme().size(220.0))
                .max_w(theme().size(300.0))
                .max_h(theme().size(420.0))
                .scrollable(true)
                .label("Projects")
                .item(
                    PopupMenuItem::new("All projects")
                        .checked(selected.is_none())
                        .on_click(move |_, _, cx| {
                            let _ = all_entity
                                .update(cx, |this, cx| this.set_session_project_filter(None, cx));
                        }),
                );
            for project in &projects {
                let target = project.clone();
                let target_entity = filter_entity.clone();
                menu = menu.item(
                    PopupMenuItem::new(project_label(project))
                        .checked(selected.as_ref() == Some(project))
                        .on_click(move |_, _, cx| {
                            let _ = target_entity.update(cx, |this, cx| {
                                this.set_session_project_filter(Some(target.clone()), cx);
                            });
                        }),
                );
            }
            menu
        });
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(theme().space.xs)
            .child(filter)
            .child(icon_button(
                "add-project",
                AppIcon::FolderPlus,
                "Add project",
                ButtonTone::Quiet,
                move |window, cx| {
                    let _ = project_entity.update(cx, |this, cx| {
                        this.choose_project_folder(None, window, cx);
                    });
                },
            ))
            .child(icon_button(
                "new-session",
                AppIcon::PencilSimple,
                "New session",
                ButtonTone::Quiet,
                move |window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.open_picker(
                            PickerScope::Projects(ProjectPickerIntent::NewSession),
                            window,
                            cx,
                        );
                    });
                },
            ))
            .into_any_element()
    }

    pub(in crate::app::views) fn render_sessions(
        &self,
        entity: WeakEntity<Self>,
        session_drag_active: bool,
        session_list: ListState,
        session_list_rows: &RefCell<Vec<String>>,
    ) -> impl IntoElement {
        let settings_entity = entity.clone();
        let plan_entity = entity.clone();
        let notice_entity = entity.clone();
        let notice_count = self.worker_notices.snapshot(&self.project.path).len();
        let cancel_drop_entity = entity.clone();
        let cancel_drop_out_entity = entity.clone();
        let active_drop_entity = entity.clone();
        let selected_root = self
            .selected_rail_root()
            .map(|session| session.path.clone());
        let live_session = self.snapshot.live_session.clone();
        let waiting_roots = {
            let _timing =
                OperationTiming::new(OperationKind::SidebarFamilyStatus, self.sessions.all.len());
            roots_waiting_for_active_descendants(&self.sessions.all, &self.activity.agents)
        };
        let lists = {
            let _timing = OperationTiming::new(
                OperationKind::SidebarSessionLists,
                self.sessions.visible.len(),
            );
            active_session_rail_lists_for_roots(
                self.sessions.visible.roots(),
                &self.sessions.drafts,
                self.sessions.project_filter.as_deref(),
                &self.sessions.order,
            )
        };
        let counts = subagent_counts(&self.sessions.all);
        let session_colors = self.sessions.folders.session_colors.clone();
        let active_entry_count = lists.active.len();
        let archived_entry_count = lists.archived_count;
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
        let active_live_session = live_session.clone();
        let active_live_status = self.snapshot.live_status;
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
                            },
                            active_row_entity.clone(),
                        )
                        .into_any_element()
                    }
                    ActiveSessionItem::Session(item) => {
                        let selected =
                            active_selected_root.as_deref() == Some(item.session.path.as_path());
                        let target = format!("session:{}", item.session.path.display());
                        let badge = Some(resolved_session_status(
                            &item.session,
                            active_run_statuses.get(&target).copied(),
                            active_live_session.as_deref(),
                            active_live_status,
                            active_waiting_roots.contains(&item.session.path),
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
                                    .get(item.session.path.as_path())
                                    .copied()
                                    .unwrap_or(0),
                                compact,
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
            .render_archive_panel(
                !self.sessions.archived_expanded || session_drag_active,
                archived_entry_count,
                &entity,
                [div()
                    .flex_1()
                    .min_h_0()
                    .child(self.views.archived_session_rail.clone())
                    .into_any_element()],
            )
            .count(archived_entry_count)
            .flush_body();
        let measure_entity = entity.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme().colors.panel)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .px(super::rail_inset())
                    .pb(theme().space.sm + theme().space.xs)
                    .child(
                        div()
                            .h(crate::app::ui::grid::header_height())
                            .flex()
                            .items_center()
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
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "session-search-header".into())
                            .h(crate::app::ui::grid::header_height())
                            .flex_none()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(
                                SearchField::new("session-search", &self.navigation.search)
                                    .accessible_label("Search sessions")
                                    .inset(gpui::px(0.0))
                                    .trailing(self.session_search_controls(entity.clone())),
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
                    .on_children_prepainted(move |bounds, _, cx| {
                        let space = bounds.first().zip(bounds.last()).map(|(first, last)| {
                            panel_space(last.bottom() - first.top(), theme().layout.folders_min)
                        });
                        let _ = measure_entity.update(cx, |this, cx| {
                            if this.views.panel_space != space {
                                this.views.panel_space = space;
                                cx.notify();
                            }
                        });
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
                    .child(div().flex_none().pt(theme().space.sm).child(archived_panel)),
            )
            .child(
                div()
                    .debug_selector(|| "session-sidebar-footer".into())
                    .flex_none()
                    .h(theme().controls.icon_button + theme().size(12.0))
                    .px(super::rail_inset())
                    .flex()
                    .items_center()
                    .gap(theme().space.xs)
                    .child(
                        icon_button_with_size(
                            "sidebar-settings",
                            AppIcon::GearSix,
                            "Settings",
                            AppIconSize::Control,
                            ButtonTone::Quiet,
                            move |window, cx| {
                                let _ = settings_entity.update(cx, |this, cx| {
                                    this.open_settings(window, cx);
                                });
                            },
                        )
                        .debug_selector(|| "sidebar-settings-control".into())
                        // Align the visible gear, including its SVG inset, with the row labels.
                        .ml(-((theme().controls.icon_button
                            - theme().icons.control)
                            / 2.0
                            + theme().icons.control / 16.0)),
                    )
                    .child(
                        icon_button_with_size(
                            "open-project-work",
                            AppIcon::ListChecks,
                            "Project plan",
                            AppIconSize::Control,
                            ButtonTone::Quiet,
                            move |window, cx| {
                                let _ = plan_entity.update(cx, |this, cx| {
                                    if this.overlays.view.sessions {
                                        this.close_sheet(window, cx);
                                    }
                                    this.open_workgraph_surface(window, cx);
                                });
                            },
                        )
                        .debug_selector(|| "sidebar-plan-control".into()),
                    )
                    .child(
                        icon_button_with_size(
                            "open-worker-notices",
                            AppIcon::Chats,
                            format!("Worker notices — {notice_count} active"),
                            AppIconSize::Control,
                            ButtonTone::Quiet,
                            move |window, cx| {
                                let _ = notice_entity.update(cx, |this, cx| {
                                    this.open_worker_notices(window, cx);
                                });
                            },
                        )
                        .debug_selector(|| "sidebar-notices-control".into())
                        .relative()
                        .when(notice_count > 0, |control| {
                            control.child(
                                div()
                                    .absolute()
                                    .top(theme().size(2.0))
                                    .right(theme().size(2.0))
                                    .size(theme().size(6.0))
                                    .rounded_full()
                                    .bg(theme().colors.indicator)
                                    .border(theme().border)
                                    .border_color(theme().colors.panel),
                            )
                        }),
                    ),
            )
            .into_any_element()
    }
}
