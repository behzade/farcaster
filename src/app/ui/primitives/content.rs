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

pub(crate) fn folder_change_summary(count: usize, counts: Option<(usize, usize)>) -> Div {
    div()
        .flex_none()
        .whitespace_nowrap()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .child(div().text_color(THEME.colors.muted).child(format!(
            "{count} {}",
            if count == 1 { "file" } else { "files" }
        )))
        .child(
            div()
                .text_color(THEME.colors.success)
                .child(counts.map_or_else(|| "+—".to_owned(), |(added, _)| format!("+{added}"))),
        )
        .child(
            div().text_color(THEME.colors.error).child(
                counts.map_or_else(|| "−—".to_owned(), |(_, removed)| format!("−{removed}")),
            ),
        )
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
