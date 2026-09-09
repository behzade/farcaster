use crate::app::{
    FarcasterApp, OVERLAY_KEY_CONTEXT,
    ui::{
        primitives::{ButtonTone, button, modal},
        theme::{MONO_FONT_FAMILY, THEME},
    },
};
use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, px,
};
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
    let preview_lines = comment
        .context
        .text
        .lines()
        .take(12)
        .collect::<Vec<_>>()
        .join("\n");
    let preview = preview_lines.chars().take(4096).collect::<String>();
    let shortened = preview.len() < comment.context.text.len();
    modal(
        "code-comment",
        "Comment on code",
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
                    .child(
                        div()
                            .text_size(THEME.type_scale.body)
                            .child("Comment on code"),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.muted)
                            .child(format!("Add to {}", comment.session_label)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(comment.context.location()),
                    )
                    .child(
                        div()
                            .id("code-comment-preview")
                            .max_h(px(180.0))
                            .overflow_y_scroll()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.body_small)
                            .p(THEME.space.sm)
                            .bg(THEME.colors.surface)
                            .child(preview),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(if shortened {
                                "Preview shortened; the full selection will be included."
                            } else if comment.context.modified {
                                "Includes unsaved buffer text."
                            } else {
                                "Captured from the editor."
                            }),
                    )
                    .child(
                        Textarea::new(&comment.input)
                            .aria_label("Comment on selected code")
                            .w_full(),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child("Enter to add · Shift+Enter for a new line · Esc to cancel"),
                    )
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
                                "Add to composer",
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
