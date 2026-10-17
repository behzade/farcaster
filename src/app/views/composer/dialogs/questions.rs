use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};
use gpui_component::input::{Input, InputState};

use super::selectable_dialog_text;
use crate::{
    agents::Question,
    app::{
        FarcasterApp,
        ui::{
            primitives::{ButtonTone, button},
            theme::theme,
        },
    },
};

pub(super) fn render(
    id: &str,
    question: &Question,
    input: &gpui::Entity<InputState>,
    app: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let mut body = div()
        .flex()
        .flex_col()
        .gap(theme().space.sm)
        .child(selectable_dialog_text("question-prompt", &question.prompt));
    for (index, option) in question.options.iter().enumerate() {
        let owner = app.clone();
        let id = id.to_owned();
        let question_id = question.id.clone();
        let value = option.label.clone();
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap(theme().space.xs)
                .child(button(
                    ("question-option", index),
                    option.label.clone(),
                    ButtonTone::Neutral,
                    true,
                    move |window, cx| {
                        let _ = owner.update(cx, |app, cx| {
                            app.respond_question_value(
                                id.clone(),
                                question_id.clone(),
                                value.clone(),
                                window,
                                cx,
                            )
                        });
                    },
                ))
                .child(
                    selectable_dialog_text(("question-description", index), &option.description)
                        .text_color(theme().colors.muted),
                ),
        );
    }
    if question.allow_free_text {
        let id = id.to_owned();
        let question_id = question.id.clone();
        body = body.child(Input::new(input).w_full()).child(button(
            "question-submit",
            "Submit answer",
            ButtonTone::Accent,
            true,
            move |window, cx| {
                let _ = app.update(cx, |app, cx| {
                    app.respond_question_text(id.clone(), question_id.clone(), window, cx)
                });
            },
        ));
    }
    body.into_any_element()
}
