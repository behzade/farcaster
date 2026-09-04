use crate::app::ui::theme::THEME;
use gpui::{
    Div, FontWeight, InteractiveElement as _, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, div,
};

pub(crate) fn panel() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.panel)
}

pub(crate) fn section_heading(title: impl Into<SharedString>) -> impl gpui::IntoElement {
    let title = title.into();
    div()
        .id(title.clone())
        .role(Role::Heading)
        .aria_label(title.clone())
        .aria_level(2)
        .text_size(THEME.type_scale.body)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(THEME.colors.muted)
        .child(title)
}
