mod worker_tasks;
use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    input::Input,
};

use super::super::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    app::ui::primitives::{ButtonTone, FeedbackTone, button, feedback, modal},
    app::ui::theme::THEME,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let dismiss = entity.clone();
    modal(
        "settings",
        "Settings",
        &app.sheet_focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| this.close_sheet(window, cx));
        },
        |surface| {
            let cancel = entity.clone();
            let clear = entity.clone();
            let save = entity.clone();
            surface
                .w(gpui::px(860.0))
                .max_w_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div()
                        .flex_none()
                        .px(gpui::px(24.0))
                        .py(THEME.space.md)
                        .border_b_1()
                        .border_color(THEME.colors.surface)
                        .text_size(THEME.type_scale.display)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Settings"),
                )
                .child(
                    div()
                        .id("settings-scroll")
                        .min_h_0()
                        .max_h(gpui::px(520.0))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(gpui::px(24.0))
                        .p(gpui::px(24.0))
                        .child(worker_tasks::render(app, entity.clone()))
                        .child(
                            div()
                                .pt(THEME.space.md)
                                .border_t_1()
                                .border_color(THEME.colors.surface)
                                .text_size(THEME.type_scale.reading)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Connections"),
                        )
                        .child(builtin_mcp_setting(
                            crate::builtin_mcp::enabled(),
                            entity.clone(),
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(THEME.space.sm)
                                .child(setting_label(
                                    "Network proxy",
                                    "Used when the project environment has no HTTP or HTTPS proxy.",
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(THEME.space.sm)
                                        .child(
                                            div()
                                                .flex_1()
                                                .child(Input::new(&app.network_proxy_input)),
                                        )
                                        .child(button(
                                            "clear-network-proxy",
                                            "Clear",
                                            ButtonTone::Quiet,
                                            true,
                                            move |window, cx| {
                                                let _ = clear.update(cx, |this, cx| {
                                                    this.clear_network_proxy(window, cx)
                                                });
                                            },
                                        )),
                                ),
                        ),
                )
                .when_some(app.network_proxy_error.clone(), |content, error| {
                    content.child(div().px(THEME.space.md).child(feedback(
                        "settings-error",
                        error,
                        FeedbackTone::Error,
                    )))
                })
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .justify_end()
                        .gap(THEME.space.sm)
                        .px(gpui::px(24.0))
                        .py(THEME.space.md)
                        .border_t_1()
                        .border_color(THEME.colors.surface)
                        .child(button(
                            "cancel-settings",
                            "Cancel",
                            ButtonTone::Neutral,
                            true,
                            move |window, cx| {
                                let _ = cancel.update(cx, |this, cx| this.close_sheet(window, cx));
                            },
                        ))
                        .child(button(
                            "save-settings",
                            "Save changes",
                            ButtonTone::Accent,
                            app.worker_task_editor.edit.is_none(),
                            move |window, cx| {
                                let _ = save.update(cx, |this, cx| this.save_settings(window, cx));
                            },
                        )),
                )
        },
    )
    .into_any_element()
}

fn builtin_mcp_setting(enabled: bool, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(THEME.space.md)
        .child(setting_label(
            "Built-in MCP",
            "Add local tools to new sessions. Turning this off disconnects existing MCP clients.",
        ))
        .child(
            Button::new("builtin-mcp-toggle")
                .label(if enabled { "On" } else { "Off" })
                .with_size(Size::Small)
                .toggled(enabled)
                .when(enabled, |button| button.primary())
                .when(!enabled, |button| button.secondary())
                .on_click(move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| this.toggle_settings_builtin_mcp(cx));
                }),
        )
        .into_any_element()
}

fn setting_label(title: &'static str, description: &'static str) -> AnyElement {
    div()
        .min_w_0()
        .max_w_full()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.body)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(THEME.colors.text)
                .child(title),
        )
        .child(
            div()
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.muted)
                .child(description),
        )
        .into_any_element()
}
