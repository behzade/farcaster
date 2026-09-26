use crate::agents::Backend;
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use gpui::{
    AnyElement, App, AppContext as _, CursorStyle, Entity, FontWeight, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Pixels, RenderOnce, Rgba, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    input::{Escape, Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::{
    colors::{ColorTarget, color_menu},
    drag::DraggedSession,
    groups::{SessionRailItem, SessionRailKind},
    hover::{session_hover_details, session_tooltip_content},
};
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{
        AppIconSize, AppTooltip as _, ContextMenuTrigger, DeleteButton, IndicatorEdge,
        ReorderPosition, ReorderTargetExt as _, app_icon, line_indicator, number_slot,
    },
    app::ui::theme::theme,
    app::{FarcasterApp, PickerScope, ProjectPickerIntent},
};

pub(super) struct SessionRowInput {
    pub(super) selected: bool,
    pub(super) color: Option<Rgba>,
    pub(super) status: Option<String>,
    pub(super) drop_position: Option<ReorderPosition>,
    pub(super) draggable: bool,
    pub(super) title_editor: Option<Entity<InputState>>,
    pub(super) subagents: usize,
    pub(super) compact: bool,
}

impl SessionRowInput {
    pub(super) fn standard(selected: bool, status: Option<String>) -> Self {
        Self {
            selected,
            color: None,
            status,
            drop_position: None,
            draggable: true,
            title_editor: None,
            subagents: 0,
            compact: false,
        }
    }
}

#[derive(IntoElement)]
pub(super) struct SessionRow {
    item: SessionRailItem,
    input: SessionRowInput,
    entity: WeakEntity<FarcasterApp>,
}

impl SessionRow {
    pub(super) fn new(
        item: &SessionRailItem,
        input: SessionRowInput,
        entity: WeakEntity<FarcasterApp>,
    ) -> Self {
        Self {
            item: item.clone(),
            input,
            entity,
        }
    }
}

impl RenderOnce for SessionRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            item,
            input:
                SessionRowInput {
                    selected,
                    color,
                    status,
                    drop_position,
                    draggable,
                    title_editor,
                    subagents,
                    compact,
                },
            entity,
        } = self;
        let row_height = session_row_height(compact);
        let session = &item.session;
        let path = session.path.clone();
        let project = session.project.clone();
        let open_entity = entity.clone();
        let edit_entity = entity.clone();
        let cancel_entity = entity.clone();
        let edit_path = path.clone();
        let edit_project = project.clone();
        let edit_title = session.title.clone();
        let move_path = session.path.clone();
        let move_project = session.project.clone();
        let move_entity = entity.clone();
        let target_app_session_id = session.app_session_id;
        let drag = DraggedSession {
            app_session_id: target_app_session_id,
            kind: item.kind,
            title: session.title.clone(),
            project: project_label(&session.project),
        };
        let drag_move_entity = entity.clone();
        let drop_entity = entity.clone();
        let drag_entity = entity.clone();
        let age = relative_age(session.modified);
        let target_kind = item.kind;
        let is_archived = target_kind == SessionRailKind::Archived;
        let status_text = status.unwrap_or_default();
        let accessible_state = if is_archived {
            "Archived"
        } else {
            status_text.as_str()
        };
        let accessible_label = session_accessible_label(&session.title, accessible_state, &age);
        let hover_details = session_hover_details(session, accessible_state, &age, subagents);
        let action_group = format!("session-actions-{}", session.id);
        let archive_action = session_archive_action(
            &session.id,
            session.path.clone(),
            is_archived,
            action_group.clone(),
            entity.clone(),
        );
        let delete_action = is_archived.then(|| {
            session_delete_action(
                &session.id,
                session.path.clone(),
                action_group.clone(),
                entity.clone(),
            )
        });
        let row = div()
            .id(format!("session-{}", session.id))
            .role(Role::Button)
            .aria_label(accessible_label)
            .aria_selected(selected)
            .tab_index(0)
            .on_mouse_down(
                MouseButton::Left,
                crate::app::ui::primitives::preserve_pointer_focus,
            )
            .size_full()
            .h(row_height)
            .relative()
            .flex()
            .items_stretch()
            .px(theme().space.sm)
            .rounded(theme().radius)
            .group(action_group.clone())
            .bg(if selected {
                theme().colors.highlight
            } else {
                theme().colors.panel
            })
            .hover(|row| row.bg(theme().colors.highlight))
            .when(selected || color.is_some(), |row| {
                row.child(line_indicator(
                    IndicatorEdge::Leading,
                    color.unwrap_or(theme().colors.indicator),
                ))
            })
            .focus(|row| {
                row.border(theme().border)
                    .border_color(theme().colors.indicator)
            })
            .cursor(CursorStyle::PointingHand)
            .when(draggable, move |row| {
                row.on_drag(drag, move |drag, _, _, cx| {
                    let _ = drag_entity.update(cx, |this, cx| this.begin_session_drag(cx));
                    cx.new(|_| drag.clone())
                })
                .can_drop(move |value, _, _| {
                    value
                        .downcast_ref::<DraggedSession>()
                        .is_some_and(|drag| drag.can_drop_on(target_kind, target_app_session_id))
                })
                .reorder_target::<DraggedSession>(
                    drop_position,
                    theme().colors.indicator,
                    theme().colors.highlight,
                    move |position, _, cx| {
                        let _ = drag_move_entity.update(cx, |this, cx| {
                            this.update_session_drop_target(target_app_session_id, position, cx);
                        });
                    },
                    move |drag, window, cx| {
                        cx.stop_propagation();
                        let _ = drop_entity.update(cx, |this, cx| {
                            this.complete_session_row_drop(drag, target_kind, window, cx);
                        });
                    },
                )
            })
            .on_click(move |event, window, cx| {
                if event.click_count() >= 2 {
                    cx.stop_propagation();
                    let _ = edit_entity.update(cx, |this, cx| {
                        this.begin_session_title_edit(
                            edit_path.clone(),
                            edit_project.clone(),
                            edit_title.clone(),
                            window,
                            cx,
                        );
                    });
                } else {
                    let _ = open_entity.update(cx, |this, cx| {
                        this.select_session_and_focus(path.clone(), project.clone(), window, cx)
                    });
                }
            })
            .child(session_row_content(
                compact,
                session_row_title(
                    session.title.clone(),
                    selected,
                    is_archived,
                    title_editor,
                    cancel_entity,
                ),
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(theme().size(3.0))
                    .text_size(theme().type_scale.caption)
                    .text_color(theme().colors.subtle)
                    .child(
                        div()
                            .id(format!("move-project-{}", session.id))
                            .min_w_0()
                            .when(
                                crate::agents::supports_session_move(session.harness),
                                |label| {
                                    label
                                        .role(Role::Button)
                                        .aria_label("Move session to another project")
                                        .tab_index(0)
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            crate::app::ui::primitives::preserve_pointer_focus,
                                        )
                                        .rounded(theme().radius)
                                        .cursor(CursorStyle::PointingHand)
                                        .hover(|icon| icon.text_color(theme().colors.indicator))
                                        .focus(|icon| {
                                            icon.border(theme().border)
                                                .border_color(theme().colors.indicator)
                                        })
                                        .app_tooltip("Move to project…")
                                        .on_click(move |_, window, cx| {
                                            cx.stop_propagation();
                                            let _ = move_entity.update(cx, |this, cx| {
                                                this.open_picker(
                                                    PickerScope::Projects(
                                                        ProjectPickerIntent::MoveSession {
                                                            path: move_path.clone(),
                                                            source_project: move_project.clone(),
                                                        },
                                                    ),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        })
                                },
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(project_label(&session.project)),
                            ),
                    )
                    .into_any_element(),
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(theme().space.xs)
                    .child(archive_action)
                    .when_some(
                        session_status_icon(target_app_session_id, &status_text),
                        |cluster, icon| cluster.child(icon),
                    )
                    .child(session_provider_slot(
                        session.harness,
                        action_group.clone(),
                        delete_action,
                    ))
                    .child(session_row_age(age))
                    .into_any_element(),
            ));
        let row = row.app_tooltip_element(move |_, _| session_tooltip_content(&hover_details));
        let hover_entity = entity.clone();
        let hover_path = session.path.clone();
        let hover_project = session.project.clone();
        let context_menu =
            session_context_menu(session, target_kind, entity, row.into_any_element());

        div()
            .id(format!("session-hover-{}", session.id))
            .h(row_height)
            .w_full()
            .on_hover(move |hovered: &bool, _, cx| {
                if *hovered {
                    let _ = hover_entity.update(cx, |this, cx| {
                        this.prefetch_session(hover_path.clone(), hover_project.clone(), cx);
                    });
                }
            })
            .child(context_menu)
            .into_any_element()
    }
}

fn session_row_title(
    title: String,
    selected: bool,
    is_archived: bool,
    title_editor: Option<Entity<InputState>>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    if let Some(title_input) = title_editor {
        div()
            .min_w_0()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| cx.stop_propagation())
            .on_action(move |_: &Escape, _, cx| {
                cx.stop_propagation();
                let _ = entity.update(cx, |this, cx| this.cancel_session_title_edit(cx));
            })
            .child(Input::new(&title_input).w_full().appearance(false))
            .into_any_element()
    } else {
        div()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(theme().type_scale.body_small)
            .font_weight(if selected {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .text_color(if is_archived && !selected {
                theme().colors.muted
            } else {
                theme().colors.text
            })
            .child(title)
            .into_any_element()
    }
}

fn session_archive_action(
    id: &str,
    path: PathBuf,
    is_archived: bool,
    action_group: String,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    archive_action(id, is_archived, action_group, move |apply, window, cx| {
        let _ = entity.update(cx, |this, cx| {
            this.request_session_archive(path.clone(), apply, window, cx);
        });
    })
}

/// The archive control every chat row leads with, so a draft filed away before
/// anything was sent behaves like a chat one did.
pub(super) fn archive_action(
    id: &str,
    is_archived: bool,
    action_group: String,
    on_press: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label = if is_archived { "Restore" } else { "Archive" };
    // The control names the state it moves the chat to, so one place decides
    // the toggle and no row can disagree with the label it shows.
    let apply = !is_archived;
    let icon = if is_archived {
        AppIcon::ArrowCounterClockwise
    } else {
        AppIcon::Archive
    };
    div()
        .id(format!("archive-{id}"))
        .role(Role::Button)
        .aria_label(format!("{label} session"))
        .tab_index(0)
        .on_mouse_down(
            MouseButton::Left,
            crate::app::ui::primitives::preserve_pointer_focus,
        )
        .flex_none()
        .size(theme().controls.icon_button)
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme().radius)
        .opacity(0.0)
        .group_hover(action_group, |button| button.opacity(1.0))
        .focus(|button| {
            button
                .opacity(1.0)
                .border(theme().border)
                .border_color(theme().colors.indicator)
        })
        .text_color(if is_archived {
            theme().colors.success
        } else {
            theme().colors.muted
        })
        .hover(|button| button.bg(theme().colors.highlight))
        .app_tooltip(format!("{label} session"))
        .child(app_icon(icon, AppIconSize::Control))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_press(apply, window, cx);
        })
        .into_any_element()
}

fn session_delete_action(
    id: &str,
    path: PathBuf,
    action_group: String,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    DeleteButton::new(format!("delete-{id}"), "Delete session permanently")
        .reveal_on(action_group)
        .on_delete(move |window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.request_session_delete(path.clone(), window, cx);
            });
        })
        .into_any_element()
}

fn session_context_menu(
    session: &crate::sessions::SessionSummary,
    kind: SessionRailKind,
    entity: WeakEntity<FarcasterApp>,
    row: AnyElement,
) -> AnyElement {
    let path = session.path.clone();
    let project = session.project.clone();
    let title = session.title.clone();
    let can_fork = crate::agents::supports_session_fork(session.harness);
    let app_session_id = session.app_session_id;
    ContextMenuTrigger::new(format!("session-context-trigger-{}", session.id), row)
        .size_full()
        .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, window, cx| {
            let rename_path = path.clone();
            let rename_project = project.clone();
            let rename_title = title.clone();
            let rename_entity = entity.clone();
            let mut menu = menu
                .min_w(theme().size(190.0))
                .item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                    let _ = rename_entity.update(cx, |this, cx| {
                        this.begin_session_title_edit(
                            rename_path.clone(),
                            rename_project.clone(),
                            rename_title.clone(),
                            window,
                            cx,
                        );
                    });
                }))
                .when(can_fork, |menu| {
                    let fork_path = path.clone();
                    let fork_project = project.clone();
                    let fork_entity = entity.clone();
                    menu.item(
                        PopupMenuItem::new("Fork session")
                            .icon(AppIcon::GitFork)
                            .on_click(move |_, window, cx| {
                                let _ = fork_entity.update(cx, |this, cx| {
                                    this.fork_session(
                                        fork_path.clone(),
                                        fork_project.clone(),
                                        window,
                                        cx,
                                    );
                                });
                            }),
                    )
                });

            let move_entity = entity.clone();
            let move_path = path.clone();
            let colour_entity = entity.clone();
            menu = menu
                .separator()
                .submenu("Colour", window, cx, move |menu, _, cx| {
                    let current = colour_entity.upgrade().and_then(|app| {
                        app.read(cx).sessions.folders.session_color(app_session_id)
                    });
                    color_menu(
                        menu,
                        current,
                        colour_entity.clone(),
                        ColorTarget::Session(app_session_id),
                    )
                });
            let show_folders = entity
                .upgrade()
                .is_some_and(|app| !app.read(cx).settings.group_sessions_by_project);
            if show_folders {
                menu = menu.submenu("Move to folder", window, cx, move |mut menu, _, cx| {
                    use crate::app::session_folders::FolderDestination;
                    let Some(app) = move_entity.upgrade() else {
                        return menu;
                    };
                    let folders = &app.read(cx).sessions.folders;
                    let current =
                        folders.destination(app_session_id, kind == SessionRailKind::Archived);
                    for (destination, label) in folders.destinations() {
                        let target_entity = move_entity.clone();
                        let target_path = move_path.clone();
                        menu = menu.item(
                            PopupMenuItem::new(label)
                                .checked(destination == current)
                                .disabled(
                                    destination == current
                                        || (app_session_id <= 0
                                            && matches!(destination, FolderDestination::Folder(_))),
                                )
                                .on_click(move |_, window, cx| {
                                    let _ = target_entity.update(cx, |this, cx| {
                                        this.move_session_to_folder(
                                            app_session_id,
                                            target_path.clone(),
                                            destination,
                                            kind == SessionRailKind::Archived,
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                        );
                    }
                    menu
                });
            }

            if kind == SessionRailKind::Archived {
                let delete_path = path.clone();
                let delete_entity = entity.clone();
                menu = menu.separator().item(
                    PopupMenuItem::new("Delete permanently")
                        .icon(AppIcon::Trash)
                        .on_click(move |_, window, cx| {
                            let _ = delete_entity.update(cx, |this, cx| {
                                this.request_session_delete(delete_path.clone(), window, cx);
                            });
                        }),
                );
            }
            menu
        })
        .mouse_button(MouseButton::Right)
        .anchor_to_cursor()
        .into_any_element()
}

pub(super) fn session_accessible_label(title: &str, state: &str, age: &str) -> String {
    format!("Resume session: {title}. State: {state}. Updated {age}")
}

/// Archived rows replace the provider icon with Delete on hover.
pub(super) fn session_provider_slot(
    harness: impl Into<Option<Backend>>,
    reveal_group: String,
    delete_action: Option<AnyElement>,
) -> AnyElement {
    div()
        .relative()
        .w(theme().controls.icon_button)
        .h(theme().controls.icon_button)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .when(delete_action.is_some(), |icon| {
                    icon.group_hover(reveal_group, |icon| icon.opacity(0.0))
                })
                .child(app_icon(AppIcon::for_harness(harness), AppIconSize::Inline)),
        )
        .when_some(delete_action, |slot, delete_action| {
            slot.child(
                div()
                    .debug_selector(|| "session-delete-action".into())
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(delete_action),
            )
        })
        .into_any_element()
}

pub(super) fn session_row_age(age: String) -> AnyElement {
    number_slot(age, theme().layout.session_age_slot)
}

/// A chat reports its state immediately left of the provider icon, in the same
/// slot a failed chat and any future reload action share.
pub(super) fn session_status_icon(app_session_id: i64, status: &str) -> Option<AnyElement> {
    let (icon, color) = status_visual(status)?;
    let tooltip = status.to_owned();
    let icon = app_icon(icon, AppIconSize::Inline).into_any_element();
    Some(
        div()
            .id(format!("session-status-{app_session_id}"))
            .flex_none()
            .text_color(color)
            .app_tooltip(tooltip.clone())
            .child(icon)
            .into_any_element(),
    )
}

pub(in crate::app) fn status_visual(status: &str) -> Option<(AppIcon, Rgba)> {
    match status {
        "" => None,
        "Done" | "Complete" => Some((AppIcon::CheckCircle, theme().colors.success)),
        "Needs input" | "Delivery unknown" | "Incomplete" => {
            Some((AppIcon::WarningCircle, theme().colors.warning))
        }
        "Waiting" => Some((AppIcon::Hourglass, theme().colors.indicator)),
        "Failed" => Some((AppIcon::XCircle, theme().colors.error)),
        "Working" => Some((AppIcon::SpinnerGap, theme().colors.indicator)),
        "Compacting" => Some((AppIcon::ArrowsClockwise, theme().colors.indicator)),
        _ => Some((AppIcon::Question, theme().colors.subtle)),
    }
}

pub(super) fn project_badge(project: &Path) -> AnyElement {
    let path = project.display().to_string();
    div()
        .id(format!("project-badge:{path}"))
        .max_w_full()
        .flex()
        .items_center()
        .gap(theme().size(3.0))
        .text_size(theme().type_scale.caption)
        .text_color(theme().colors.subtle)
        .app_tooltip(path.clone())
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(project_label(project)),
        )
        .into_any_element()
}

pub(in crate::app) fn project_label(project: &Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| project.display().to_string(), str::to_owned)
}

pub(super) fn relative_age(modified: SystemTime) -> String {
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    if age < Duration::from_secs(60) {
        "now".into()
    } else if age < Duration::from_secs(60 * 60) {
        format!("{}m", age.as_secs() / 60)
    } else if age < Duration::from_secs(24 * 60 * 60) {
        format!("{}h", age.as_secs() / (60 * 60))
    } else {
        format!("{}d", age.as_secs() / (24 * 60 * 60))
    }
}

pub(in crate::app::views) fn session_row_height(compact: bool) -> Pixels {
    if compact {
        theme().layout.session_row_height
    } else {
        theme().layout.session_row_height + theme().controls.icon_button
    }
}

pub(super) fn session_row_content(
    compact: bool,
    title: AnyElement,
    project: AnyElement,
    actions: AnyElement,
) -> AnyElement {
    let title = div().min_w_0().overflow_hidden().child(title);
    if compact {
        div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(theme().space.sm)
            .child(title.flex_1())
            .child(actions)
            .into_any_element()
    } else {
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .justify_center()
            .child(title.w_full())
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(theme().space.sm)
                    .child(project)
                    .child(actions),
            )
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
