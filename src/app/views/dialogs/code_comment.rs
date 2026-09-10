use crate::app::{
    FarcasterApp, OVERLAY_KEY_CONTEXT,
    ui::{
        primitives::{ButtonTone, button, modal},
        theme::THEME,
    },
};
use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};
use gpui_component::input::Textarea;

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
    cx: &gpui::App,
) -> AnyElement {
    let comment = app.code_comment.as_ref().expect("visible code comment");
    let cancel = entity.clone();
    let submit = entity.clone();
    let enabled = !comment.input.read(cx).value().trim().is_empty();
    let title = if comment.task.is_some() {
        "Start task"
    } else {
        "Comment on code"
    };
    modal(
        "code-comment",
        title,
        &comment.focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = cancel.update(cx, |this, cx| this.close_code_comment(window, cx));
        },
        |surface| {
            surface.child(
                div()
                    .p(THEME.space.md)
                    .flex()
                    .flex_col()
                    .gap(THEME.space.sm)
                    .children(comment.task.as_ref().map(|settings| {
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(format!(
                                "{} · {}",
                                crate::agents::backend_display_name(&settings.harness),
                                settings
                                    .model
                                    .as_ref()
                                    .map_or("Default model", |model| model.name.as_str())
                            ))
                    }))
                    .child(Textarea::new(&comment.input).aria_label(title).w_full())
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
                            .child(button(
                                "cancel-code-comment",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = entity
                                        .update(cx, |this, cx| this.close_code_comment(window, cx));
                                },
                            ))
                            .child(button(
                                "add-code-comment",
                                if comment.task.is_some() {
                                    "Start task"
                                } else {
                                    "Add"
                                },
                                ButtonTone::Accent,
                                enabled,
                                move |window, cx| {
                                    let _ = submit
                                        .update(cx, |this, cx| this.add_code_comment(window, cx));
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}
