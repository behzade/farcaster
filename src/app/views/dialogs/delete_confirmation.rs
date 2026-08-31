use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};

use crate::app::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    app::ui::primitives::{ButtonTone, button, modal},
    app::ui::theme::THEME,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let dismiss = entity.clone();
    modal(
        "delete-session",
        "Delete session permanently?",
        &app.sheet_focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| {
                this.close_delete_confirmation(window, cx)
            });
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
                            .child("This permanently deletes the session and all of its subagent sessions. This cannot be undone."),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
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
                                "confirm-session-delete",
                                "Delete permanently",
                                ButtonTone::Danger,
                                true,
                                move |window, cx| {
                                    let _ = confirm.update(cx, |this, cx| {
                                        this.delete_pending_session(window, cx)
                                    });
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}
