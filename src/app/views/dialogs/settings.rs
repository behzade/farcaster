use gpui::{
    AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::Input;

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
            let save = entity;
            surface.w(gpui::px(520.0)).max_w_full().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.md)
                    .p(THEME.space.md)
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
                                        this.save_network_proxy(window, cx)
                                    });
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}
