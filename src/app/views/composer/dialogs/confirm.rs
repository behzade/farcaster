use gpui::{
    App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, WeakEntity,
    Window, div, prelude::FluentBuilder as _,
};

use super::selectable_dialog_text;
use crate::{
    app::{
        FarcasterApp,
        ui::{
            primitives::{ButtonTone, button},
            theme::{MONO_FONT_FAMILY, theme},
        },
    },
    conversation,
};

#[derive(IntoElement)]
pub(super) struct ConfirmRequestView {
    id: String,
    title: SharedString,
    message: String,
    app: WeakEntity<FarcasterApp>,
}

impl ConfirmRequestView {
    pub(super) fn new(
        id: String,
        title: String,
        message: String,
        app: WeakEntity<FarcasterApp>,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            message,
            app,
        }
    }

    pub(super) fn title(&self) -> &SharedString {
        &self.title
    }
}

impl RenderOnce for ConfirmRequestView {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (reason, command) = conversation::split_command_block(&self.message)
            .map_or((self.message.as_str(), None), |(reason, command)| {
                (reason, Some(command))
            });
        let no_id = self.id.clone();
        let yes_id = self.id;
        let no = self.app.clone();
        let yes = self.app;

        div()
            .flex()
            .flex_col()
            .gap(theme().space.md)
            .when(!reason.is_empty(), |body| {
                body.child(
                    selectable_dialog_text("dialog-confirm-message", reason)
                        .text_size(theme().type_scale.body)
                        .text_color(theme().colors.muted),
                )
            })
            .when_some(command, |body, command| {
                body.child(
                    selectable_dialog_text("dialog-confirm-command", command)
                        .font_family(MONO_FONT_FAMILY)
                        .text_size(theme().type_scale.body_small)
                        .text_color(theme().colors.text),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(theme().space.xs)
                    .child(button(
                        "confirm-no",
                        "[n] No",
                        ButtonTone::Neutral,
                        true,
                        move |window, cx| {
                            let _ = no.update(cx, |this, cx| {
                                this.respond_confirm(no_id.clone(), false, window, cx)
                            });
                        },
                    ))
                    .child(button(
                        "confirm-yes",
                        "[y] Yes",
                        ButtonTone::Accent,
                        true,
                        move |window, cx| {
                            let _ = yes.update(cx, |this, cx| {
                                this.respond_confirm(yes_id.clone(), true, window, cx)
                            });
                        },
                    )),
            )
    }
}
