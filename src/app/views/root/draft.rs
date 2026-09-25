use gpui::{
    AnyElement, FocusHandle, FontWeight, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};

use super::super::session_rail;
use crate::app::{
    FarcasterApp, PickerScope, ProjectPickerIntent,
    ui::{
        primitives::{ButtonTone, button},
        theme::THEME,
    },
};

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;

pub(super) fn render_body(
    composer: impl IntoElement,
    heading: Option<AnyElement>,
    composer_focus: FocusHandle,
) -> impl IntoElement {
    div()
        .id("chat-body")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            if !window.default_prevented() {
                composer_focus.focus(window, cx);
                window.prevent_default();
            }
        })
        .child(
            div()
                .w_full()
                .min_h(gpui::relative(1.0))
                .py(THEME.space.md)
                .flex()
                .flex_col()
                .justify_center()
                .items_center()
                .child(
                    div()
                        .w_full()
                        .max_w(THEME.layout.conversation_width)
                        .px(THEME.space.md)
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .when_some(heading, |body, heading| body.child(heading))
                        .child(composer),
                ),
        )
}

pub(super) fn render_heading(
    project: std::path::PathBuf,
    entity: WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    let label = session_rail::project_label(&project);
    div().flex().w_full().items_center().child(
        button(
            "draft-project",
            label,
            ButtonTone::Quiet,
            true,
            move |window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.open_picker(
                        PickerScope::Projects(ProjectPickerIntent::ChangeDraft),
                        window,
                        cx,
                    );
                });
            },
        )
        .tooltip(project.display().to_string())
        .max_w_full()
        .px_0()
        .text_size(THEME.type_scale.display)
        .font_weight(FontWeight::MEDIUM)
        .text_color(THEME.colors.accent),
    )
}
