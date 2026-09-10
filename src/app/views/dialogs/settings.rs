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
    cx: &gpui::App,
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
            let close = entity.clone();
            let clear = entity.clone();
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
                        .child(transcript_font_size(app.transcript_view.read(cx).font_size, entity.clone()))
                        .child(toggle_setting(
                            "transcript-folders-toggle",
                            "Expand changed folders in transcript",
                            "Start change folders expanded. Your manual folder choices stay as you left them.",
                            app.expand_transcript_folders,
                            entity.clone(),
                            FarcasterApp::toggle_settings_transcript_folders,
                        ))
                        .when_some(app.settings_transcript_error.clone(), |content, error| {
                            content.child(feedback(
                                "settings-transcript-error",
                                error,
                                FeedbackTone::Error,
                            ))
                        })
                        .child(
                            div()
                                .pt(THEME.space.md)
                                .border_t_1()
                                .border_color(THEME.colors.surface)
                                .text_size(THEME.type_scale.reading)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Connections"),
                        )
                        .child(toggle_setting(
                            "builtin-mcp-toggle",
                            "Built-in MCP",
                            "Add local tools to new sessions. Turning this off disconnects existing MCP clients.",
                            crate::builtin_mcp::enabled(),
                            entity.clone(),
                            FarcasterApp::toggle_settings_builtin_mcp,
                        ))
                        .when_some(app.settings_mcp_error.clone(), |content, error| {
                            content.child(feedback(
                                "settings-mcp-error",
                                error,
                                FeedbackTone::Error,
                            ))
                        })
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
                                )
                                .when_some(app.network_proxy_error.clone(), |content, error| {
                                    content.child(feedback(
                                        "settings-proxy-error",
                                        error,
                                        FeedbackTone::Error,
                                    ))
                                }),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(THEME.space.sm)
                        .px(gpui::px(24.0))
                        .py(THEME.space.md)
                        .border_t_1()
                        .border_color(THEME.colors.surface)
                        .child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child("Valid changes save automatically."),
                        )
                        .child(button(
                            "close-settings",
                            "Close",
                            ButtonTone::Neutral,
                            true,
                            move |window, cx| {
                                let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                            },
                        )),
                )
        },
    )
    .into_any_element()
}

fn toggle_setting(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    enabled: bool,
    entity: WeakEntity<FarcasterApp>,
    toggle: fn(&mut FarcasterApp, &mut gpui::Context<FarcasterApp>),
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(THEME.space.md)
        .child(setting_label(title, description))
        .child(
            Button::new(id)
                .label(if enabled { "On" } else { "Off" })
                .with_size(Size::Small)
                .toggled(enabled)
                .when(enabled, |button| button.primary())
                .when(!enabled, |button| button.secondary())
                .on_click(move |_, _, cx| {
                    let _ = entity.update(cx, toggle);
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

fn transcript_font_size(size: gpui::Pixels, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    use crate::app::ui::theme::TRANSCRIPT_FONT_SIZE_RANGE;

    let size = f32::from(size);
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(THEME.space.md)
        .child(setting_label(
            "Transcript font size",
            if cfg!(target_os = "macos") {
                "Applies to all sessions. Cmd+- / Cmd+= also adjust the size."
            } else {
                "Applies to all sessions. Ctrl+- / Ctrl+= also adjust the size."
            },
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(THEME.space.sm)
                .child(format!("{size} px"))
                .children(
                    [
                        (
                            "transcript-font-smaller",
                            "−",
                            size - 1.0,
                            size > *TRANSCRIPT_FONT_SIZE_RANGE.start(),
                        ),
                        (
                            "transcript-font-larger",
                            "+",
                            size + 1.0,
                            size < *TRANSCRIPT_FONT_SIZE_RANGE.end(),
                        ),
                        (
                            "transcript-font-reset",
                            "Reset",
                            f32::from(THEME.type_scale.reading),
                            size != f32::from(THEME.type_scale.reading),
                        ),
                    ]
                    .into_iter()
                    .map(|(id, label, next, enabled)| {
                        let entity = entity.clone();
                        button(id, label, ButtonTone::Neutral, enabled, move |_, cx| {
                            let _ = entity
                                .update(cx, |this, cx| this.set_transcript_font_size(next, cx));
                        })
                        .accessibility_label(match label {
                            "−" => "Decrease transcript font size",
                            "+" => "Increase transcript font size",
                            _ => "Reset transcript font size",
                        })
                    }),
                ),
        )
        .into_any_element()
}
