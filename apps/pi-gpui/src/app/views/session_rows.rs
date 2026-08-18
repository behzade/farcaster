//! Session-rail row presentation and row-local interaction.

use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use gpui::{
    AnyElement, AppContext as _, CursorStyle, Empty, Entity, FontWeight, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Pixels, Role, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
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
    primitives::{AppIconSize, app_icon, disclosure_indicator, icon_control},
    projects::DraftSession,
    theme::THEME,
};

#[derive(Clone)]
struct DraggedSession(String);

pub(super) fn project_heading(
    project: &Path,
    collapsed: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let project_path = project.to_path_buf();
    div()
        .id(format!("project-group:{}", project.display()))
        .role(Role::Button)
        .aria_label(format!(
            "{} project {}",
            if collapsed { "Expand" } else { "Collapse" },
            project_label(project)
        ))
        .aria_expanded(!collapsed)
        .tab_index(0)
        .h(THEME.controls.project_row)
        .w_full()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .px(THEME.space.md)
        .text_size(THEME.type_scale.body_small)
        .font_weight(FontWeight::MEDIUM)
        .text_color(THEME.colors.muted)
        .hover(|heading| heading.bg(THEME.colors.hover))
        .focus(|heading| {
            heading
                .border(THEME.border)
                .border_color(THEME.colors.accent)
        })
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.toggle_project_group(&project_path, cx);
            });
        })
        .child(disclosure_indicator(!collapsed))
        .child(app_icon(AppIcon::Folder, AppIconSize::Inline))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(project_label(project)),
        )
        .into_any_element()
}

pub(super) fn draft_session_row(
    draft: &DraftSession,
    selected: bool,
    status: &str,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let id = draft.id.clone();
    let discard_id = id.clone();
    let project = draft.project.clone();
    let discard_entity = entity.clone();
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
                .px(THEME.space.sm)
                .rounded(THEME.radius)
                .bg(if selected {
                    THEME.colors.surface
                } else {
                    THEME.colors.panel
                })
                .hover(|row| row.bg(THEME.colors.hover))
                .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
                .cursor(CursorStyle::PointingHand)
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.resume_draft(id.clone(), project.clone(), window, cx);
                    });
                })
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(THEME.space.xs)
                        .child(
                            div()
                                .flex_none()
                                .whitespace_nowrap()
                                .text_size(THEME.type_scale.caption)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if status == "Draft" {
                                    THEME.colors.subtle
                                } else {
                                    THEME.colors.accent
                                })
                                .child(status.to_owned()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(THEME.type_scale.body)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(THEME.colors.text)
                                .child("New session"),
                        )
                        .child(
                            icon_control(format!("discard-{discard_id}"), "Discard draft")
                                .hover(|button| button.bg(THEME.colors.hover))
                                .child(app_icon(AppIcon::X, AppIconSize::Control))
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    let _ = discard_entity.update(cx, |this, cx| {
                                        this.discard_draft(&discard_id, window, cx);
                                    });
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
) -> Option<String> {
    let status = explicit_status
        .and_then(normalized_session_status)
        .or_else(|| {
            (live_session_id == Some(item.session.id.as_str()))
                .then(|| normalized_session_status(live_status))
                .flatten()
        })
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
    title_editor: Option<Entity<InputState>>,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    session_row_with_height(
        item,
        selected,
        status,
        shortcut,
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
    let drop_entity = entity.clone();
    let dragged_id = session.id.clone();
    let drop_target_id = session.id.clone();
    let settle_path = session.path.clone();
    let settle_entity = entity.clone();
    let age = relative_age(session.modified);
    let is_settled = item.kind == SessionRailKind::Settled;
    let status_color = match status.as_deref() {
        Some("Done") => THEME.colors.success,
        Some("Needs input") => THEME.colors.warning,
        Some(_) => THEME.colors.accent,
        None => THEME.colors.subtle,
    };
    let status_text = status.unwrap_or_default();
    let show_status = !status_text.is_empty();
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
        .on_drag(DraggedSession(dragged_id), |_, _, _, cx| cx.new(|_| Empty))
        .on_drop(move |dragged: &DraggedSession, _, cx| {
            let _ = drop_entity.update(cx, |this, cx| {
                this.move_session_to(&dragged.0, &drop_target_id, cx);
            });
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
                    this.resume(path.clone(), project.clone(), window, cx)
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
                .child(if let Some(title_input) = title_editor {
                    div()
                        .min_w_0()
                        .flex_1()
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
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_normal()
                        .line_clamp(2)
                        .line_height(THEME.type_scale.line_body)
                        .text_size(THEME.type_scale.body)
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
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_end()
                        .justify_between()
                        .when(show_status, |column| {
                            column.child(
                                div()
                                    .whitespace_nowrap()
                                    .text_size(THEME.type_scale.caption)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(status_color)
                                    .child(format!("{status_text} · {age}")),
                            )
                        })
                        .when(!show_status, |column| {
                            column.child(
                                div()
                                    .text_size(THEME.type_scale.caption)
                                    .text_color(THEME.colors.subtle)
                                    .child(age),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_end()
                                .gap(THEME.space.xs)
                                .when_some(shortcut, |actions, number| {
                                    actions.child(Kbd::new(
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
                                        .h(THEME.controls.icon_button)
                                        .px(THEME.space.xs)
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(THEME.radius)
                                        .opacity(0.65)
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
                                        .gap(THEME.space.xs)
                                        .text_size(THEME.type_scale.caption)
                                        .child(app_icon(settle_icon, AppIconSize::Control))
                                        .child(settle_label)
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
