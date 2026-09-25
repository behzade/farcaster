use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};

use crate::app::{
    FarcasterApp, OVERLAY_KEY_CONTEXT,
    ui::{
        primitives::{ButtonTone, button, confirmation_modal},
        theme::theme,
    },
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let dismiss = entity.clone();
    let on_cancel = move |window: &mut gpui::Window, cx: &mut gpui::App| {
        let _ = dismiss.update(cx, |this, cx| this.close_quit_confirmation(window, cx));
    };
    let on_confirm = move |_: &mut gpui::Window, cx: &mut gpui::App| {
        let _ = entity.update(cx, |this, cx| this.confirm_application_quit(cx));
    };
    confirmation_modal(
        "quit-application",
        "Exit Farcaster?",
        &app.lifecycle.pending_quit.as_ref().expect("visible confirmation").focus,
        OVERLAY_KEY_CONTEXT,
        on_cancel.clone(),
        on_confirm.clone(),
        |surface| {
            surface.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme().space.md)
                    .p(theme().space.md)
                    .child(
                        div()
                            .text_size(theme().type_scale.body)
                            .text_color(theme().colors.text)
                            .child("Agents, subagents, or tool runs are still active. Exiting may interrupt this work."),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(theme().space.sm)
                            .child(button("cancel-application-quit", "Cancel", ButtonTone::Neutral, true, on_cancel))
                            .child(button("confirm-application-quit", "Exit", ButtonTone::Danger, true, on_confirm)),
                    ),
            )
        },
    )
    .into_any_element()
}
