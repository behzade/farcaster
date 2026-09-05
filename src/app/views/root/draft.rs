use gpui::{FontWeight, IntoElement, ParentElement as _, Styled as _, WeakEntity, div};

use super::super::session_rail;
use crate::app::{
    FarcasterApp, PickerScope, ProjectPickerIntent,
    ui::{
        primitives::{ButtonTone, button},
        theme::THEME,
    },
};

pub(super) fn render_heading(
    project: std::path::PathBuf,
    entity: WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    let label = session_rail::project_label(&project);
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(THEME.space.sm)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.muted)
                .child("New session"),
        )
        .child(
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
            .dropdown_caret(true)
            .tooltip(project.display().to_string())
            .h_auto()
            .px_0()
            .text_size(THEME.type_scale.display)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(THEME.colors.text),
        )
}
