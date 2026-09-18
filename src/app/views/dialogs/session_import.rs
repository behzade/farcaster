use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, ElementId, FontWeight, InteractiveElement as _, IntoElement as _,
    MouseButton, ParentElement as _, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::tooltip::Tooltip;

use crate::app::FarcasterApp;
use crate::{
    agents,
    app::OVERLAY_KEY_CONTEXT,
    app::session::import::import_harnesses,
    app::ui::{
        assets::AppIcon,
        primitives::{
            AppIconSize, ButtonTone, FeedbackTone, activates_button, app_icon, button, feedback,
            icon_control, modal,
        },
        theme::theme,
    },
    sessions::SessionSummary,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let dialog = app.sessions.import.as_ref().expect("visible import");
    let dismiss = entity.clone();
    let harness = dialog.harness;
    let harness_name = dialog
        .profile_id
        .as_deref()
        .and_then(|id| app.settings.harness_profiles.get(id).ok())
        .map_or_else(
            || agents::backend_display_name(harness).to_string(),
            |profile| profile.name,
        );
    let loading = dialog.loading;
    let error = dialog.error.clone();
    let candidates = dialog.candidates.clone();
    let selected = dialog.selected.clone();
    let selected_count = selected.len();
    modal(
        "import-sessions",
        "Import sessions",
        &dialog.focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| this.close_session_import(window, cx));
        },
        |surface| {
            let cancel = entity.clone();
            let confirm = entity.clone();
            surface.w(px(640.0)).max_w_full().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme().space.md)
                    .p(theme().space.md)
                    .child(
                        div()
                            .text_size(theme().type_scale.display)
                            .child("Import sessions"),
                    )
                    .child(
                        div()
                            .text_size(theme().type_scale.body)
                            .text_color(theme().colors.muted)
                            .child(
                                "Choose one harness, review what is on disk, then import the sessions you want. Farcaster does not watch session files.",
                            ),
                    )
                    .child(harness_picker(entity.clone(), harness, dialog.profile_id.as_deref(), app.settings.harness_profiles.list().unwrap_or_default()))
                    .when_some(error, |this, message| {
                        this.child(feedback("import-error", message, FeedbackTone::Error))
                    })
                    .child(if loading {
                        div()
                            .text_color(theme().colors.muted)
                            .child(format!("Looking for {harness_name} sessions…"))
                    } else if candidates.is_empty() {
                        div().text_color(theme().colors.muted).child(format!(
                            "No new {harness_name} sessions on disk."
                        ))
                    } else {
                        candidate_list(entity.clone(), &candidates, &selected)
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(theme().space.sm)
                            .child(button(
                                "cancel-session-import",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.close_session_import(window, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "confirm-session-import",
                                if selected_count == 0 {
                                    "Import".into()
                                } else {
                                    format!("Import {selected_count}")
                                },
                                ButtonTone::Accent,
                                selected_count > 0,
                                move |window, cx| {
                                    let _ = confirm.update(cx, |this, cx| {
                                        this.confirm_session_import(window, cx)
                                    });
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}

fn harness_picker(
    entity: WeakEntity<FarcasterApp>,
    selected: Option<crate::agents::Backend>,
    selected_profile: Option<&str>,
    profiles: Vec<agents::HarnessProfile>,
) -> gpui::Div {
    div()
        .flex()
        .flex_wrap()
        .gap(theme().space.xs)
        .children(import_harnesses().into_iter().map(|harness| {
            let active = Some(harness) == selected && selected_profile.is_none();
            let entity = entity.clone();
            let id = format!("import-harness-{harness}");
            let label = agents::backend_display_name(harness);
            button(
                id,
                label,
                if active {
                    ButtonTone::Accent
                } else {
                    ButtonTone::Neutral
                },
                true,
                move |_, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.select_session_import_harness(harness, cx);
                    });
                },
            )
        }))
        .children(profiles.into_iter().map(|profile| {
            let active = selected_profile == Some(profile.id.as_str());
            let entity = entity.clone();
            let id = format!("import-profile-{}", profile.id);
            button(
                id,
                profile.name,
                if active {
                    ButtonTone::Accent
                } else {
                    ButtonTone::Neutral
                },
                true,
                move |_, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.select_session_import_profile(profile.backend, profile.id.clone(), cx);
                    });
                },
            )
        }))
}

fn candidate_list(
    entity: WeakEntity<FarcasterApp>,
    candidates: &[SessionSummary],
    selected: &std::collections::HashSet<std::path::PathBuf>,
) -> gpui::Div {
    let all = entity.clone();
    let none = entity.clone();
    let selected_count = selected.len();
    div()
        .flex()
        .flex_col()
        .gap(theme().space.sm)
        .child(
            div()
                .flex()
                .items_center()
                .gap(theme().space.sm)
                .child(button(
                    "import-select-all",
                    "Select all",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        let _ = all.update(cx, |this, cx| {
                            this.set_session_import_selection(true, cx);
                        });
                    },
                ))
                .child(button(
                    "import-select-none",
                    "Select none",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        let _ = none.update(cx, |this, cx| {
                            this.set_session_import_selection(false, cx);
                        });
                    },
                ))
                .child(
                    div()
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.subtle)
                        .child(format!(
                            "{selected_count} selected · {} on disk",
                            candidates.len()
                        )),
                ),
        )
        .child(
            div()
                .id("import-session-list")
                .max_h(theme().layout.session_row_height * 7)
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .children(candidates.iter().map(|session| {
                    candidate_row(entity.clone(), session, selected.contains(&session.path))
                })),
        )
}

fn candidate_row(
    entity: WeakEntity<FarcasterApp>,
    session: &SessionSummary,
    checked: bool,
) -> Stateful<gpui::Div> {
    let path = session.path.clone();
    let title = collapsed_import_title(&session.title, session.parent_session.is_some());
    let project = project_name(&session.project);
    let age = relative_age(session.modified);
    let accessible = format!(
        "{} {} · {project}. Updated {age}",
        if checked { "Deselect" } else { "Select" },
        title
    );
    let tooltip = format!("{title} · {project}");
    let checkbox_id = format!("import-select-{}", session.path.display());
    let row_id = format!("import-session-{}", session.path.display());
    let toggle = path.clone();
    let row_entity = entity.clone();
    div()
        .id(row_id)
        .role(Role::Button)
        .aria_label(accessible)
        .aria_selected(checked)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .tab_index(0)
        .on_mouse_down(
            MouseButton::Left,
            crate::app::ui::primitives::preserve_pointer_focus,
        )
        .w_full()
        .h(theme().layout.session_row_height)
        .relative()
        .px(theme().space.sm)
        .py(theme().space.xs)
        .rounded(px(2.0))
        .flex()
        .items_center()
        .gap(theme().space.sm)
        .bg(if checked {
            theme().colors.session_selection
        } else {
            theme().colors.panel
        })
        .hover(move |row| {
            row.bg(if checked {
                theme().colors.session_selection
            } else {
                theme().colors.surface
            })
        })
        .when(checked, |row| {
            row.child(
                div()
                    .absolute()
                    .left_0()
                    .top(theme().space.xs)
                    .bottom(theme().space.xs)
                    .w(px(2.0))
                    .bg(theme().colors.accent),
            )
        })
        .focus(|row| {
            row.border(theme().border)
                .border_color(theme().colors.accent)
        })
        .cursor(CursorStyle::PointingHand)
        .on_click(move |_, _, cx| {
            let path = toggle.clone();
            let _ = row_entity.update(cx, |this, cx| {
                this.toggle_session_import_candidate(path, cx);
            });
        })
        .child(selection_checkbox(
            checkbox_id,
            checked,
            format!("{} {title}", if checked { "Deselect" } else { "Select" }),
            move |_, cx| {
                let path = path.clone();
                let _ = entity.update(cx, |this, cx| {
                    this.toggle_session_import_candidate(path, cx);
                });
            },
        ))
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
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(theme().type_scale.body_small)
                        .font_weight(if checked {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(theme().colors.text)
                        .child(title.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.subtle)
                        .child(project),
                ),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(theme().space.xs)
                .child(app_icon(
                    AppIcon::for_harness(session.harness),
                    AppIconSize::Inline,
                ))
                .child(
                    div()
                        .w(px(30.0))
                        .flex_none()
                        .whitespace_nowrap()
                        .text_align(gpui::TextAlign::Right)
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.subtle)
                        .child(age),
                ),
        )
}

fn selection_checkbox(
    id: impl Into<ElementId>,
    selected: bool,
    label: impl Into<SharedString>,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    let press = Rc::new(on_press);
    let click = Rc::clone(&press);
    icon_control(id, label)
        .size(px(20.0))
        .role(Role::CheckBox)
        .aria_toggled(if selected {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            click(window, cx);
        })
        .on_key_down(move |event, window, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                press(window, cx);
            }
        })
        .child(
            div()
                .size(px(14.0))
                .border(theme().border)
                .border_color(theme().colors.muted)
                .rounded(px(2.0))
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |checkbox| {
                    checkbox.bg(theme().colors.accent).child(
                        app_icon(AppIcon::Check, AppIconSize::Inline)
                            .text_color(theme().colors.surface),
                    )
                }),
        )
}

fn collapsed_import_title(title: &str, nested: bool) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if nested {
        format!("↳ {title}")
    } else {
        title
    }
}

fn project_name(project: &std::path::Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| project.display().to_string(), str::to_owned)
}

fn relative_age(modified: std::time::SystemTime) -> String {
    let age = std::time::SystemTime::now()
        .duration_since(modified)
        .unwrap_or(std::time::Duration::ZERO);
    if age < std::time::Duration::from_secs(60) {
        "now".into()
    } else if age < std::time::Duration::from_secs(60 * 60) {
        format!("{}m", age.as_secs() / 60)
    } else if age < std::time::Duration::from_secs(24 * 60 * 60) {
        format!("{}h", age.as_secs() / (60 * 60))
    } else {
        format!("{}d", age.as_secs() / (24 * 60 * 60))
    }
}

#[cfg(test)]
#[path = "session_import_tests.rs"]
mod tests;
