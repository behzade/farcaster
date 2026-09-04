use gpui::{
    App, Entity, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _,
    WeakEntity, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{Textarea, TextareaState};

use super::selectable_dialog_text;
use crate::app::{
    FarcasterApp,
    ui::{
        primitives::{ButtonTone, button},
        theme::{MONO_FONT_FAMILY, THEME},
    },
};

#[derive(IntoElement)]
pub(super) struct TextRequestView {
    id: String,
    title: SharedString,
    hint: Option<String>,
    technical: bool,
    input: Entity<TextareaState>,
    app: WeakEntity<FarcasterApp>,
}

impl TextRequestView {
    pub(super) fn new(
        id: String,
        title: String,
        hint: Option<String>,
        technical: bool,
        input: Entity<TextareaState>,
        app: WeakEntity<FarcasterApp>,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            hint,
            technical,
            input,
            app,
        }
    }

    pub(super) fn title(&self) -> &SharedString {
        &self.title
    }
}

impl RenderOnce for TextRequestView {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let submit_id = self.id;
        let submit = self.app;

        div()
            .flex()
            .flex_col()
            .gap(THEME.space.md)
            .when_some(self.hint, |body, hint| {
                body.child(
                    selectable_dialog_text("dialog-input-hint", hint)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle),
                )
            })
            .child(
                div()
                    .when(self.technical, |input| input.font_family(MONO_FONT_FAMILY))
                    .child(Textarea::new(&self.input).w_full()),
            )
            .child(div().flex().justify_end().child(button(
                "dialog-submit",
                "Continue",
                ButtonTone::Accent,
                true,
                move |window, cx| {
                    let _ = submit.update(cx, |this, cx| {
                        this.respond_value(submit_id.clone(), window, cx)
                    });
                },
            )))
    }
}
