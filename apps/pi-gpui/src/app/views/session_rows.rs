//! Session-rail row presentation and row-local interaction.

use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use gpui::{
    AnyElement, Context, CursorStyle, Entity, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Pixels, Render, Rgba, Role, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::{Escape, Input, InputState},
    kbd::Kbd,
    tooltip::Tooltip,
};

use super::{
    super::PiApp,
    session_groups::{SessionRailItem, SessionRailKind},
};
use crate::{
    assets::AppIcon,
    primitives::{
        AppIconSize, ReorderPosition, ReorderTargetExt as _, app_icon, icon_control, reorder_handle,
    },
    projects::DraftSession,
    theme::THEME,
};

#[derive(Clone)]
struct DraggedSession {
    app_session_id: i64,
    title: String,
    project: String,
}

impl Render for DraggedSession {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(260.0))
            .px(THEME.space.md)
            .py(THEME.space.sm)
            .rounded(THEME.radius)
            .bg(THEME.colors.surface)
            .border(THEME.border)
            .border_color(THEME.colors.accent)
            .shadow_md()
            .child(
                div()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(THEME.colors.text)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .mt(px(2.0))
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.subtle)
                    .child(self.project.clone()),
            )
    }
}

fn session_drag_handle(drag: DraggedSession, entity: WeakEntity<PiApp>) -> AnyElement {
    let id = drag.app_session_id;
    reorder_handle(
        format!("drag-session-{id}"),
        "Drag to reorder session",
        drag,
        move |cx| {
            let _ = entity.update(cx, |this, cx| this.begin_session_drag(cx));
        },
    )
}

pub(super) fn draft_session_row(
    draft: &DraftSession,
    selected: bool,
    status: &str,
    shortcut: Option<u8>,
    drop_position: Option<ReorderPosition>,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let id = draft.id.clone();
    let discard_id = id.clone();
    let project = draft.project.clone();
    let discard_entity = entity.clone();
    let title = draft.title.as_deref().unwrap_or("New session").to_owned();
    let target_app_session_id = draft.app_session_id;
    let drag = DraggedSession {
        app_session_id: target_app_session_id,
        title: title.clone(),
        project: project_label(&draft.project),
    };
    let drag_move_entity = entity.clone();
    let drop_entity = entity.clone();
    let drag_handle_entity = entity.clone();
    let action_group = format!("draft-actions-{id}");
    div()
        .h(THEME.layout.session_row_height)
        .w_full()
        .px(THEME.space.sm)
        .child(
            div()
                .id(format!("session-{id}"))
                .role(Role::Button)
                .aria_label(format!("Open {status} session in {}", project.display()))
                .aria_selected(selected)
                .tab_index(0)
                .size_full()
                .h(THEME.layout.session_row_height)
                .flex()
                .items_center()
                .gap(THEME.space.sm)
                .px(THEME.space.sm)
                .rounded(THEME.radius)
                .group(action_group.clone())
                .bg(if selected {
                    THEME.colors.surface
                } else {
                    THEME.colors.panel
                })
                .hover(|row| row.bg(THEME.colors.hover))
                .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
                .cursor(CursorStyle::PointingHand)
                .reorder_target::<DraggedSession>(
                    drop_position,
                    THEME.colors.accent,
                    THEME.colors.hover,
                    move |position, _, cx| {
                        let _ = drag_move_entity.update(cx, |this, cx| {
                            this.update_session_drop_target(target_app_session_id, position, cx);
                        });
                    },
                    move |drag, _, cx| {
                        cx.stop_propagation();
                        let _ = drop_entity.update(cx, |this, cx| {
                            this.complete_session_drop(drag.app_session_id, cx);
                        });
                    },
                )
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.resume_draft(id.clone(), project.clone(), window, cx);
                    });
                })
                .child(session_drag_handle(drag, drag_handle_entity))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .overflow_hidden()
                        .child(
                            div()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(THEME.type_scale.body_small)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(THEME.colors.text)
                                .child(title),
                        )
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
                                        .child(project_badge(&draft.project)),
                                )
                                .child(draft_badge())
                                .when(status != "Draft", |metadata| {
                                    metadata.when_some(
                                        status_icon(target_app_session_id, status),
                                        |metadata, icon| metadata.child(icon),
                                    )
                                })
                                .when_some(shortcut, |metadata, number| {
                                    metadata.child(Kbd::new(
                                        gpui::Keystroke::parse(&format!("cmd-{number}"))
                                            .expect("fixed session shortcut must parse"),
                                    ))
                                })
                                .when(!draft.submitted, |metadata| {
                                    metadata.child(
                                        icon_control(
                                            format!("discard-{discard_id}"),
                                            "Discard draft",
                                        )
                                        .opacity(0.0)
                                        .group_hover(action_group, |button| button.opacity(1.0))
                                        .focus(|button| button.opacity(1.0))
                                        .hover(|button| button.bg(THEME.colors.hover))
                                        .child(app_icon(AppIcon::X, AppIconSize::Control))
                                        .on_click(
                                            move |_, window, cx| {
                                                cx.stop_propagation();
                                                let _ = discard_entity.update(cx, |this, cx| {
                                                    this.discard_draft(&discard_id, window, cx);
                                                });
                                            },
                                        ),
                                    )
                                }),
                        ),
                ),
        )
        .into_any_element()
}

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
        (SessionRailKind::Settled, Some("Done")) | (_, None) => None,
        _ => status,
    }
}

fn normalized_session_status(status: &str) -> Option<String> {
    match status {
        "" | "Idle" | "Ready" => None,
        status => Some(status.into()),
    }
}

pub(super) fn session_row(
    item: &SessionRailItem,
    selected: bool,
    status: Option<String>,
    shortcut: Option<u8>,
    drop_position: Option<ReorderPosition>,
    draggable: bool,
    title_editor: Option<Entity<InputState>>,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    session_row_with_height(
        item,
        selected,
        status,
        shortcut,
        drop_position,
        draggable,
        title_editor,
        THEME.layout.session_row_height,
        entity,
    )
}

pub(super) fn session_row_with_height(
    item: &SessionRailItem,
    selected: bool,
    status: Option<String>,
    shortcut: Option<u8>,
    drop_position: Option<ReorderPosition>,
    draggable: bool,
    title_editor: Option<Entity<InputState>>,
    row_height: Pixels,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let session = &item.session;
    let path = session.path.clone();
    let project = session.project.clone();
    let open_entity = entity.clone();
    let edit_entity = entity.clone();
    let cancel_entity = entity.clone();
    let edit_path = path.clone();
    let edit_project = project.clone();
    let edit_title = session.title.clone();
    let settle_path = session.path.clone();
    let settle_entity = entity.clone();
    let target_app_session_id = session.app_session_id;
    let drag = DraggedSession {
        app_session_id: target_app_session_id,
        title: session.title.clone(),
        project: project_label(&session.project),
    };
    let drag_move_entity = entity.clone();
    let drop_entity = entity.clone();
    let drag_handle_entity = entity.clone();
    let age = relative_age(session.modified);
    let is_settled = item.kind == SessionRailKind::Settled;
    let status_text = status.unwrap_or_default();
    let accessible_state = if status_text.is_empty() {
        "Archived"
    } else {
        status_text.as_str()
    };
    let accessible_label = session_accessible_label(&session.title, accessible_state, &age);
    let settle_label = if is_settled { "Restore" } else { "Settle" };
    let settle_icon = if is_settled {
        AppIcon::ArrowCounterClockwise
    } else {
        AppIcon::Archive
    };
    let row = div()
        .id(format!("session-{}", session.id))
        .role(Role::Button)
        .aria_label(accessible_label)
        .aria_selected(selected)
        .tab_index(0)
        .size_full()
        .h(row_height)
        .flex()
        .items_stretch()
        .px(THEME.space.sm)
        .py(THEME.space.xs)
        .rounded(THEME.radius)
        .group(format!("session-actions-{}", session.id))
        .bg(if selected {
            THEME.colors.surface
        } else {
            THEME.colors.panel
        })
        .hover(|row| row.bg(THEME.colors.hover))
        .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .cursor(CursorStyle::PointingHand)
        .when(draggable, move |row| {
            row.reorder_target::<DraggedSession>(
                drop_position,
                THEME.colors.accent,
                THEME.colors.hover,
                move |position, _, cx| {
                    let _ = drag_move_entity.update(cx, |this, cx| {
                        this.update_session_drop_target(target_app_session_id, position, cx);
                    });
                },
                move |drag, _, cx| {
                    cx.stop_propagation();
                    let _ = drop_entity.update(cx, |this, cx| {
                        this.complete_session_drop(drag.app_session_id, cx);
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
                .when(draggable, |content| {
                    content.child(session_drag_handle(drag, drag_handle_entity))
                })
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .overflow_hidden()
                        .child(if let Some(title_input) = title_editor {
                            div()
                                .min_w_0()
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .on_action(move |_: &Escape, _, cx| {
                                    cx.stop_propagation();
                                    let _ = cancel_entity.update(cx, |this, cx| {
                                        this.cancel_session_title_edit(cx);
                                    });
                                })
                                .child(Input::new(&title_input).w_full().appearance(false))
                                .into_any_element()
                        } else {
                            div()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(THEME.type_scale.body_small)
                                .font_weight(if selected || !is_settled {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if is_settled && !selected {
                                    THEME.colors.muted
                                } else {
                                    THEME.colors.text
                                })
                                .child(session.title.clone())
                                .into_any_element()
                        })
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
                                        .child(project_badge(&session.project)),
                                )
                                .when_some(
                                    status_icon(target_app_session_id, &status_text),
                                    |metadata, icon| metadata.child(icon),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .whitespace_nowrap()
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(THEME.colors.subtle)
                                        .child(age),
                                )
                                .when_some(shortcut, |metadata, number| {
                                    metadata.child(Kbd::new(
                                        gpui::Keystroke::parse(&format!("cmd-{number}"))
                                            .expect("fixed session shortcut must parse"),
                                    ))
                                })
                                .child(
                                    div()
                                        .id(format!("settle-{}", session.id))
                                        .role(Role::Button)
                                        .aria_label(format!("{settle_label} session"))
                                        .tab_index(0)
                                        .size(THEME.controls.icon_button)
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(THEME.radius)
                                        .opacity(0.0)
                                        .group_hover(
                                            format!("session-actions-{}", session.id),
                                            |button| button.opacity(1.0),
                                        )
                                        .focus(|button| {
                                            button
                                                .opacity(1.0)
                                                .border(THEME.border)
                                                .border_color(THEME.colors.accent)
                                        })
                                        .text_color(if is_settled {
                                            THEME.colors.success
                                        } else {
                                            THEME.colors.muted
                                        })
                                        .hover(|button| button.bg(THEME.colors.hover))
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(format!("{settle_label} session"))
                                                .build(window, cx)
                                        })
                                        .child(app_icon(settle_icon, AppIconSize::Control))
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            let _ = settle_entity.update(cx, |this, cx| {
                                                this.set_session_settled(
                                                    settle_path.clone(),
                                                    !is_settled,
                                                    cx,
                                                );
                                            });
                                        }),
                                ),
                        ),
                ),
        );
    div()
        .h(row_height)
        .w_full()
        .px(THEME.space.sm)
        .child(row)
        .into_any_element()
}

pub(super) fn session_accessible_label(title: &str, state: &str, age: &str) -> String {
    format!("Resume session: {title}. State: {state}. Updated {age}")
}

fn status_icon(app_session_id: i64, status: &str) -> Option<AnyElement> {
    let (icon, color) = status_visual(status)?;
    let tooltip = status.to_owned();
    Some(
        div()
            .id(format!("session-status-{app_session_id}"))
            .flex_none()
            .text_color(color)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(app_icon(icon, AppIconSize::Inline))
            .into_any_element(),
    )
}

pub(super) fn status_visual(status: &str) -> Option<(AppIcon, Rgba)> {
    match status {
        "" => None,
        "Done" => Some((AppIcon::CheckCircle, THEME.colors.success)),
        "Needs input" | "Waiting" => Some((AppIcon::WarningCircle, THEME.colors.warning)),
        "Failed" => Some((AppIcon::XCircle, THEME.colors.error)),
        "Working" => Some((AppIcon::SpinnerGap, THEME.colors.accent)),
        _ => Some((AppIcon::Question, THEME.colors.subtle)),
    }
}

fn draft_badge() -> AnyElement {
    div()
        .flex_none()
        .px(THEME.space.xs)
        .rounded(px(3.0))
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.muted)
        .child("Draft")
        .into_any_element()
}

fn project_badge(project: &Path) -> AnyElement {
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
        .child(app_icon(AppIcon::Folder, AppIconSize::Inline))
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

pub(super) fn project_label(project: &Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| project.display().to_string(), str::to_owned)
}

fn relative_age(modified: SystemTime) -> String {
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
