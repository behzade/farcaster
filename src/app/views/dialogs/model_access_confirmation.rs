use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};

use crate::app::{
    FarcasterApp, OVERLAY_KEY_CONTEXT,
    ui::{
        primitives::{ButtonTone, button, modal},
        theme::THEME,
    },
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let pending = app
        .navigation
        .pending_model_access
        .as_ref()
        .expect("visible confirmation");
    let model = pending.model.name.clone();
    let modes = pending.modes.clone();
    let dismiss = entity.clone();
    modal(
        "model-access-choice",
        "Choose access mode",
        &pending.focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| this.close_model_access_confirmation(window, cx));
        },
        |surface| {
            let cancel = entity.clone();
            let confirm = entity;
            surface.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.md)
                    .p(THEME.space.md)
                    .child(
                        div()
                            .text_size(THEME.type_scale.body)
                            .text_color(THEME.colors.text)
                            .child(format!("{model} needs a different access mode. Choose one to apply the model and its preset.")),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
                            .child(button(
                                "cancel-model-full-access",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.close_model_access_confirmation(window, cx)
                                    });
                                },
                            ))
                            .children(modes.into_iter().enumerate().map(|(index, mode)| {
                                let confirm = confirm.clone();
                                let label = match mode {
                                    crate::runtime::HarnessAccessMode::Sandboxed => "Use Sandbox",
                                    crate::runtime::HarnessAccessMode::Auto => "Use Auto",
                                    crate::runtime::HarnessAccessMode::Full => "Use Full access",
                                };
                                button(
                                    format!("confirm-model-access-{index}"),
                                    label,
                                    if mode == crate::runtime::HarnessAccessMode::Full {
                                        ButtonTone::Danger
                                    } else {
                                        ButtonTone::Neutral
                                    },
                                    true,
                                    move |window, cx| {
                                        let _ = confirm.update(cx, |this, cx| {
                                            this.confirm_model_access(mode, window, cx)
                                        });
                                    },
                                )
                            })),
                    ),
            )
        },
    )
    .into_any_element()
}
