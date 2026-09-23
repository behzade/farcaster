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
    modal(
        "move-active-session",
        "Session has work or pending messages",
        &app.sessions.pending_move.as_ref().expect("visible confirmation").focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| this.close_move_confirmation(window, cx));
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
                            .child("Moving this session will stop its active work and discard pending messages in the session family."),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
                            .child(button(
                                "cancel-active-session-move",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.close_move_confirmation(window, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "stop-and-move-session",
                                "Stop, discard pending, and move",
                                ButtonTone::Danger,
                                true,
                                move |window, cx| {
                                    let _ = confirm.update(cx, |this, cx| {
                                        this.stop_and_move_pending_session(window, cx)
                                    });
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}
