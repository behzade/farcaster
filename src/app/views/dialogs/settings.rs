use gpui::{
    AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::super::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    app::ui::keybindings::ApplicationModifier,
    app::ui::primitives::{ButtonTone, FeedbackTone, button, dropdown_button, feedback, modal},
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
            surface.w(gpui::px(520.0)).max_w_full().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.md)
                    .p(THEME.space.md)
                    .child(modifier_setting(
                        app.settings_application_modifier,
                        entity.clone(),
                    ))
                    .child(builtin_mcp_setting(
                        crate::builtin_mcp::enabled(),
                        entity.clone(),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(THEME.space.xs)
                            .child(
                                div()
                                    .text_size(THEME.type_scale.body)
                                    .text_color(THEME.colors.text)
                                    .child("Network proxy"),
                            )
                            .child(
                                div()
                                    .text_size(THEME.type_scale.body_small)
                                    .text_color(THEME.colors.subtle)
                                    .child("Used only when the project environment does not set HTTP_PROXY, HTTPS_PROXY, http_proxy, or https_proxy."),
                            )
                            .child(Input::new(&app.network_proxy_input)),
                    )
                    .when_some(app.network_proxy_error.clone(), |content, error| {
                        content.child(feedback("network-proxy-error", error, FeedbackTone::Error))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
                            .child(button(
                                "cancel-settings",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.close_sheet(window, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "clear-network-proxy",
                                "Clear",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = clear.update(cx, |this, cx| {
                                        this.clear_network_proxy(window, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "save-settings",
                                "Save",
                                ButtonTone::Accent,
                                true,
                                move |window, cx| {
                                    let _ = save.update(cx, |this, cx| {
                                        this.save_settings(window, cx)
                                    });
                                },
                            )),
                    ),
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
            "Adds Farcaster tools to new agent sessions.",
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
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.body)
                .text_color(THEME.colors.text)
                .child(title),
        )
        .child(
            div()
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.subtle)
                .child(description),
        )
        .into_any_element()
}

fn modifier_setting(selected: ApplicationModifier, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(THEME.space.md)
        .child(setting_label(
            "Keybinding modifier",
            "Used for application shortcuts.",
        ))
        .child(
            dropdown_button(
                "keybinding-modifier",
                selected.label(),
                ButtonTone::Neutral,
                true,
            )
            .flex_none()
            .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
                ApplicationModifier::platform_choices().into_iter().fold(
                    menu.min_w(gpui::px(150.0)),
                    |menu, modifier| {
                        let entity = entity.clone();
                        menu.item(
                            PopupMenuItem::new(modifier.label())
                                .checked(modifier == selected)
                                .on_click(move |_, _, cx| {
                                    let _ = entity.update(cx, |this, cx| {
                                        this.select_settings_application_modifier(modifier, cx);
                                    });
                                }),
                        )
                    },
                )
            }),
        )
        .into_any_element()
}
