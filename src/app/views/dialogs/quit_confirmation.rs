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
    let dismiss = entity.clone();
    let on_cancel = move |window: &mut gpui::Window, cx: &mut gpui::App| {
        let _ = dismiss.update(cx, |this, cx| this.close_quit_confirmation(window, cx));
    };
    modal(
        "quit-application",
        "Exit Farcaster?",
        &app.pending_quit.as_ref().expect("visible confirmation").focus,
        OVERLAY_KEY_CONTEXT,
        on_cancel.clone(),
        |surface| {
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
                            .child("Agents, subagents, or tool runs are still active. Exiting may interrupt this work."),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
                            .child(button("cancel-application-quit", "Cancel", ButtonTone::Neutral, true, on_cancel))
                            .child(button("confirm-application-quit", "Exit", ButtonTone::Danger, true, move |_, cx| {
                                let _ = entity.update(cx, |this, cx| this.confirm_application_quit(cx));
                            })),
                    ),
            )
        },
    )
    .into_any_element()
}
