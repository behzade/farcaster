use gpui::{FontWeight, IntoElement, ParentElement as _, Styled as _, div};

use super::super::session_rail;
use crate::app::ui::theme::THEME;

pub(super) fn render_heading(project: std::path::PathBuf) -> impl IntoElement {
    let label = session_rail::project_label(&project);
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.sm)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.muted)
                .child("New session"),
        )
        .child(
            div()
                .text_size(THEME.type_scale.display)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(THEME.colors.text)
                .text_ellipsis()
                .child(label),
        )
}
