use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};

use super::super::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    app::ui::primitives::{ButtonTone, button, modal},
    app::ui::theme::THEME,
};

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let dismiss = entity.clone();
    modal(
        "archive-active-session",
        "Session is active",
        &app.sheet_focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| {
                this.close_archive_confirmation(window, cx)
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
                            .child("This session still has active work. Do you want to stop all of it and archive the session?"),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
                            .child(button(
                                "cancel-active-session-archive",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.close_archive_confirmation(window, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "stop-and-archive-session",
                                "Stop all and archive",
                                ButtonTone::Danger,
                                true,
                                move |window, cx| {
                                    let _ = confirm.update(cx, |this, cx| {
                                        this.stop_and_archive_pending_session(window, cx)
                                    });
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}
