use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};

use crate::app::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    app::ui::primitives::{ButtonTone, button, modal},
    app::ui::theme::theme,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let dismiss = entity.clone();
    modal(
        "delete-session",
        "Delete session permanently?",
        &app.sessions.pending_delete.as_ref().expect("visible confirmation").focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| {
                this.close_delete_confirmation(window, cx)
            });
        },
        |surface| {
            let cancel = entity.clone();
            let stop_and_delete = entity;
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
                            .child("This stops active work, discards pending messages, and permanently deletes the session and its subagent sessions. This cannot be undone."),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(theme().space.sm)
                            .child(button(
                                "cancel-session-delete",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.close_delete_confirmation(window, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "stop-and-delete-session",
                                "Stop work and delete permanently",
                                ButtonTone::Danger,
                                true,
                                move |window, cx| {
                                    let _ = stop_and_delete.update(cx, |this, cx| {
                                        this.stop_and_delete_pending_session(window, cx)
                                    });
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}
