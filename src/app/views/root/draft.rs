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
