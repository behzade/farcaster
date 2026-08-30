use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};

use super::super::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    primitives::{ButtonTone, button, modal},
    theme::{MONO_FONT_FAMILY, THEME},
};

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let dismiss = entity.clone();
    let repository = app
        .repository
        .pending_jj_init
        .as_ref()
        .map(|pending| pending.repository.display().to_string())
        .unwrap_or_default();
    modal(
        "initialize-jj-repository",
        "Initialize Jujutsu repository?",
        &app.sheet_focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| {
                this.close_jj_init_confirmation(window, cx)
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
                            .child("This Git repository has not been initialized for Jujutsu. Run jj git init to use JJ here?"),
                    )
                    .child(
                        div()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.body_small)
                            .text_color(THEME.colors.subtle)
                            .child(repository),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
                            .child(button(
                                "cancel-jj-init",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.close_jj_init_confirmation(window, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "confirm-jj-init",
                                "Run jj git init",
                                ButtonTone::Accent,
                                true,
                                move |window, cx| {
                                    let _ = confirm.update(cx, |this, cx| {
                                        this.confirm_jj_init(window, cx)
                                    });
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}
