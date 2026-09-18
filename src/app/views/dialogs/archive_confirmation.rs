use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};

use crate::app::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    app::ui::primitives::{ButtonTone, button, confirmation_modal},
    app::ui::theme::theme,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let dismiss = entity.clone();
    let on_cancel = move |window: &mut gpui::Window, cx: &mut gpui::App| {
        let _ = dismiss.update(cx, |this, cx| this.close_archive_confirmation(window, cx));
    };
    let on_confirm = move |window: &mut gpui::Window, cx: &mut gpui::App| {
        let _ = entity.update(cx, |this, cx| {
            this.stop_and_archive_pending_session(window, cx)
        });
    };
    confirmation_modal(
        "archive-active-session",
        "Session is active",
        &app.sessions.pending_archive.as_ref().expect("visible confirmation").focus,
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
                            .child("This session still has active work. Do you want to stop all of it and archive the session?"),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(theme().space.sm)
                            .child(button(
                                "cancel-active-session-archive",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                on_cancel,
                            ))
                            .child(button(
                                "stop-and-archive-session",
                                "Stop all and archive",
                                ButtonTone::Danger,
                                true,
                                on_confirm,
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}
