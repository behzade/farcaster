use crate::app::{
    FarcasterApp, OVERLAY_KEY_CONTEXT,
    ui::{
        primitives::{ButtonTone, button, modal, submit_textarea},
        theme::THEME,
    },
};
use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};
use gpui_component::{input::Textarea, list::List};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
    cx: &gpui::App,
) -> AnyElement {
    let comment = app.code_comment.as_ref().expect("visible code comment");
    let destination = comment.destination();
    let cancel = entity.clone();
    let submit = entity.clone();
    let choose = entity.clone();
    let enabled = !comment.input.read(cx).value().trim().is_empty();
    let title = if comment.picker.is_some() {
        "Send to"
    } else if destination.is_none() {
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
            if let Some(picker) = &comment.picker {
                return surface.child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().p(THEME.space.sm).child(button(
                            "code-destination-back",
                            "Back",
                            ButtonTone::Quiet,
                            true,
                            move |window, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.close_code_destination_picker(window, cx)
                                });
                            },
                        )))
                        .child(
                            List::new(&picker.list)
                                .search_placeholder("Search chats in this project…")
                                .max_h(gpui::px(360.0)),
                        ),
                );
            }
            surface.child(
                div()
                    .p(THEME.space.md)
                    .flex()
                    .flex_col()
                    .gap(THEME.space.sm)
                    .child(button(
                        "code-destination",
                        format!(
                            "To: {}",
                            destination
                                .map_or("New task", |destination| destination.label.as_str())
                        ),
                        ButtonTone::Neutral,
                        true,
                        move |window, cx| {
                            let _ = choose
                                .update(cx, |this, cx| this.choose_code_destination(window, cx));
                        },
                    ))
                    .children(destination.is_none().then(|| {
                        let settings = &comment.settings;
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
                    .child(submit_textarea(Textarea::new(&comment.input).aria_label(title)))
                    .children(comment.error.as_ref().map(|error| {
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.danger)
                            .child(error.clone())
                    }))
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
                                if destination.is_none() {
                                    "Start task"
                                } else {
                                    "Send"
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
