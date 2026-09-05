use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use gpui::{
    AnyElement, App, AppContext as _, CursorStyle, Entity, FontWeight, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Pixels, RenderOnce, Rgba, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::{Escape, Input, InputState},
    kbd::Kbd,
    menu::{DropdownMenu as _, PopupMenuItem},
    tooltip::Tooltip,
};

use super::{
    drag::DraggedSession,
    groups::{SessionRailItem, SessionRailKind},
    hover::{session_hover_details, session_hover_panel},
};
use crate::{
    app::ui::assets::AppIcon,
    app::ui::keybindings::application_modifier,
    app::ui::primitives::{
        AppIconSize, ContextMenuTrigger, ReorderPosition, ReorderTargetExt as _, app_icon,
    },
    app::ui::theme::THEME,
    app::{FarcasterApp, PickerScope, ProjectPickerIntent},
};

pub(super) fn session_badge(
    item: &SessionRailItem,
    explicit_status: Option<&str>,
    live_session_id: Option<&str>,
    live_status: &str,
    waiting_for_descendant: bool,
) -> Option<String> {
    let explicit_status = explicit_status.and_then(normalized_session_status);
    let live_status = (live_session_id == Some(item.session.id.as_str()))
        .then(|| normalized_session_status(live_status))
        .flatten();
    let status = explicit_status
        .filter(|status| status != "Done" || !waiting_for_descendant)
        .or_else(|| live_status.filter(|status| status != "Done" || !waiting_for_descendant))
        .or_else(|| waiting_for_descendant.then(|| "Waiting".into()))
        .or_else(|| item.session.is_running.then(|| "Working".into()))
        .or_else(|| (item.kind == SessionRailKind::Project).then(|| "Done".into()));
    match (item.kind, status.as_deref()) {
        (SessionRailKind::Archived, Some("Done")) | (_, None) => None,
        _ => status,
    }
}

fn normalized_session_status(status: &str) -> Option<String> {
    match status {
        "" | "Idle" | "Ready" => None,
        status => Some(status.into()),
    }
}

pub(super) struct SessionRowInput {
    pub(super) selected: bool,
    pub(super) status: Option<String>,
    pub(super) shortcut: Option<u8>,
    pub(super) drop_position: Option<ReorderPosition>,
    pub(super) draggable: bool,
    pub(super) title_editor: Option<Entity<InputState>>,
    pub(super) subagents: usize,
    pub(super) row_height: Pixels,
}

impl SessionRowInput {
    pub(super) fn standard(selected: bool, status: Option<String>) -> Self {
        Self {
            selected,
            status,
            shortcut: None,
            drop_position: None,
            draggable: true,
            title_editor: None,
            subagents: 0,
            row_height: THEME.layout.session_row_height,
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
                    status,
                    shortcut,
                    drop_position,
                    draggable,
                    title_editor,
                    subagents,
                    row_height,
                },
            entity,
        } = self;
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
            path: Some(session.path.clone()),
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
        let hover_id = format!("session-hover-{}", session.id);
        let action_group = format!("session-actions-{}", session.id);
        let archive_action = session_archive_action(
            &session.id,
            session.path.clone(),
            is_archived,
            action_group.clone(),
            entity.clone(),
        );
        let delete_action = session_delete_action(
            &session.id,
            session.path.clone(),
            action_group.clone(),
            entity.clone(),
        );
        let row = div()
            .id(format!("session-{}", session.id))
            .role(Role::Button)
            .aria_label(accessible_label)
            .aria_selected(selected)
            .tab_index(0)
            .size_full()
            .h(row_height)
            .relative()
            .flex()
            .items_stretch()
            .px(THEME.space.sm)
            .py(THEME.space.xs)
            .rounded(px(2.0))
            .group(action_group)
            .bg(if selected {
                THEME.colors.session_selection
            } else {
                THEME.colors.panel
            })
            .hover(move |row| {
                row.bg(if selected {
                    THEME.colors.session_selection
                } else {
                    THEME.colors.surface
                })
            })
            .when(selected, |row| {
                row.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(THEME.space.xs)
                        .bottom(THEME.space.xs)
                        .w(px(2.0))
                        .bg(THEME.colors.accent),
                )
            })
            .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
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
                    THEME.colors.accent,
                    THEME.colors.hover,
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
                        this.select_session(path.clone(), project.clone(), window, cx)
                    });
                }
            })
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_stretch()
                    .gap(THEME.space.sm)
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .overflow_hidden()
                            .child(session_row_title(
                                session.title.clone(),
                                selected,
                                is_archived,
                                title_editor,
                                cancel_entity,
                            ))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(THEME.space.xs)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .flex()
                                            .items_center()
                                            .gap(px(3.0))
                                            .text_size(THEME.type_scale.caption)
                                            .text_color(THEME.colors.subtle)
                                            .child(
                                                div()
                                                    .id(format!("move-project-{}", session.id))
                                                    .role(Role::Button)
                                                    .aria_label("Move session to another project")
                                                    .tab_index(0)
                                                    .min_w_0()
                                                    .rounded(THEME.radius)
                                                    .cursor(CursorStyle::PointingHand)
                                                    .hover(|icon| {
                                                        icon.text_color(THEME.colors.accent)
                                                    })
                                                    .focus(|icon| {
                                                        icon.border(THEME.border)
                                                            .border_color(THEME.colors.accent)
                                                    })
                                                    .tooltip(move |window, cx| {
                                                        Tooltip::new("Move to project…")
                                                            .build(window, cx)
                                                    })
                                                    .on_click(move |_, window, cx| {
                                                        cx.stop_propagation();
                                                        let _ =
                                                            move_entity.update(cx, |this, cx| {
                                                                this.open_picker(
                                                            PickerScope::Projects(
                                                                ProjectPickerIntent::MoveSession {
                                                                    path: move_path.clone(),
                                                                    source_project: move_project
                                                                        .clone(),
                                                                },
                                                            ),
                                                            window,
                                                            cx,
                                                        );
                                                            });
                                                    })
                                                    .child(
                                                        div()
                                                            .min_w_0()
                                                            .overflow_hidden()
                                                            .whitespace_nowrap()
                                                            .text_ellipsis()
                                                            .child(project_label(&session.project)),
                                                    ),
                                            ),
                                    )
                                    .child(session_row_metadata(
                                        &session.harness,
                                        target_app_session_id,
                                        &status_text,
                                        age,
                                        shortcut,
                                    )),
                            ),
                    ),
            )
            .child(archive_action)
            .when(is_archived, |row| row.child(delete_action));
        let context_menu = session_context_menu(
            &session.id,
            session.path.clone(),
            session.project.clone(),
            session.title.clone(),
            target_kind,
            entity,
            row.into_any_element(),
        );

        session_hover_panel(
            hover_id,
            hover_details,
            div()
                .h(row_height)
                .w_full()
                .px(px(2.0))
                .child(context_menu)
                .into_any_element(),
        )
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
            .pr(if is_archived { px(50.0) } else { px(24.0) })
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(THEME.type_scale.body_small)
            .font_weight(if selected {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .text_color(if is_archived && !selected {
                THEME.colors.muted
            } else {
                THEME.colors.text
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
    let label = if is_archived { "Restore" } else { "Archive" };
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
        .absolute()
        .top(px(4.0))
        .right(if is_archived { px(28.0) } else { px(5.0) })
        .size(px(21.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(THEME.radius)
        .opacity(0.0)
        .group_hover(action_group, |button| button.opacity(1.0))
        .focus(|button| {
            button
                .opacity(1.0)
                .border(THEME.border)
                .border_color(THEME.colors.accent)
        })
        .text_color(if is_archived {
            THEME.colors.success
        } else {
            THEME.colors.muted
        })
        .hover(|button| button.bg(THEME.colors.hover))
        .tooltip(move |window, cx| Tooltip::new(format!("{label} session")).build(window, cx))
        .child(app_icon(icon, AppIconSize::Control))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            let _ = entity.update(cx, |this, cx| {
                this.request_session_archive(path.clone(), !is_archived, window, cx);
            });
        })
        .into_any_element()
}

fn session_delete_action(
    id: &str,
    path: PathBuf,
    action_group: String,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    div()
        .id(format!("delete-{id}"))
        .role(Role::Button)
        .aria_label("Delete session permanently")
        .tab_index(0)
        .absolute()
        .top(px(4.0))
        .right(px(5.0))
        .size(px(21.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(THEME.radius)
        .opacity(0.0)
        .group_hover(action_group, |button| button.opacity(1.0))
        .focus(|button| {
            button
                .opacity(1.0)
                .border(THEME.border)
                .border_color(THEME.colors.accent)
        })
        .text_color(THEME.colors.danger)
        .hover(|button| button.bg(THEME.colors.hover))
        .tooltip(move |window, cx| Tooltip::new("Delete session permanently").build(window, cx))
        .child(app_icon(AppIcon::Trash, AppIconSize::Control))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            let _ = entity.update(cx, |this, cx| {
                this.request_session_delete(path.clone(), window, cx);
            });
        })
        .into_any_element()
}

fn session_context_menu(
    id: &str,
    path: PathBuf,
    project: PathBuf,
    title: String,
    kind: SessionRailKind,
    entity: WeakEntity<FarcasterApp>,
    row: AnyElement,
) -> AnyElement {
    ContextMenuTrigger::new(format!("session-context-trigger-{id}"), row)
        .size_full()
        .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
            let rename_path = path.clone();
            let rename_project = project.clone();
            let rename_title = title.clone();
            let rename_entity = entity.clone();
            let fork_path = path.clone();
            let fork_project = project.clone();
            let fork_entity = entity.clone();
            let mut menu = menu
                .min_w(px(190.0))
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
                .item(
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
                );

            if kind == SessionRailKind::Project {
                let archive_path = path.clone();
                let archive_entity = entity.clone();
                menu = menu.separator().item(
                    PopupMenuItem::new("Archive")
                        .icon(AppIcon::Archive)
                        .on_click(move |_, window, cx| {
                            let _ = archive_entity.update(cx, |this, cx| {
                                this.request_session_archive(
                                    archive_path.clone(),
                                    true,
                                    window,
                                    cx,
                                );
                            });
                        }),
                );
            } else {
                let restore_path = path.clone();
                let restore_entity = entity.clone();
                menu = menu.separator().item(
                    PopupMenuItem::new("Restore")
                        .icon(AppIcon::ArrowCounterClockwise)
                        .on_click(move |_, window, cx| {
                            let _ = restore_entity.update(cx, |this, cx| {
                                this.request_session_archive(
                                    restore_path.clone(),
                                    false,
                                    window,
                                    cx,
                                );
                            });
                        }),
                );
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

// Keep draft and persisted rows on the same trailing metadata columns.
pub(super) fn session_row_metadata(
    harness: &str,
    app_session_id: i64,
    status: &str,
    age: String,
    shortcut: Option<u8>,
) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .child(app_icon(AppIcon::for_harness(harness), AppIconSize::Inline))
        .child(
            div()
                .size(THEME.icons.inline)
                .flex_none()
                .when_some(status_icon(app_session_id, status), |slot, icon| {
                    slot.child(icon)
                }),
        )
        .child(
            div()
                .w(px(30.0))
                .flex_none()
                .whitespace_nowrap()
                .text_align(gpui::TextAlign::Right)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(age),
        )
        .when_some(shortcut, |metadata, number| {
            metadata.child(Kbd::new(
                gpui::Keystroke::parse(&format!("{}-{number}", application_modifier().prefix()))
                    .expect("fixed session shortcut must parse"),
            ))
        })
        .into_any_element()
}

fn status_icon(app_session_id: i64, status: &str) -> Option<AnyElement> {
    let (icon, color) = status_visual(status)?;
    let tooltip = status.to_owned();
    let icon = app_icon(icon, AppIconSize::Inline).into_any_element();
    Some(
        div()
            .id(format!("session-status-{app_session_id}"))
            .flex_none()
            .text_color(color)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(icon)
            .into_any_element(),
    )
}

pub(super) fn status_visual(status: &str) -> Option<(AppIcon, Rgba)> {
    match status {
        "" => None,
        "Done" => Some((AppIcon::CheckCircle, THEME.colors.success)),
        "Needs input" => Some((AppIcon::WarningCircle, THEME.colors.warning)),
        "Waiting" => Some((AppIcon::Hourglass, THEME.colors.accent)),
        "Failed" => Some((AppIcon::XCircle, THEME.colors.error)),
        "Working" => Some((AppIcon::SpinnerGap, THEME.colors.accent)),
        _ => Some((AppIcon::Question, THEME.colors.subtle)),
    }
}

pub(super) fn project_badge(project: &Path) -> AnyElement {
    let path = project.display().to_string();
    div()
        .id(format!("project-badge:{path}"))
        .max_w_full()
        .flex()
        .items_center()
        .gap(px(3.0))
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.subtle)
        .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx))
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
