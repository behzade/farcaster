use crate::{
    assets::AppIcon,
    conversation::ToolPresentation,
    primitives::{ButtonTone, disclosure_detail, disclosure_title_row, icon_button},
    theme::{MONO_FONT_FAMILY, THEME},
};
use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window,
    div, px,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    label: &str,
    presentation: &ToolPresentation,
    key: usize,
    status_glyph: Option<&'static str>,
    expanded: bool,
    disclosure_label: String,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
    detail: Option<AnyElement>,
    on_open: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let (additions, deletions) = presentation.counts();
    div()
        .id(("tool-change", key))
        .w_full()
        .px(THEME.space.md)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            disclosure_title_row(
                ("tool-change-title", key),
                key,
                expanded,
                true,
                disclosure_label,
                on_toggle,
            )
            .child(
                div()
                    .flex_none()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.muted)
                    .child(label.to_owned()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.text)
                    .child(presentation.path().to_owned()),
            )
            .child(change_count(format!("+{additions}"), THEME.colors.success))
            .child(change_count(format!("-{deletions}"), THEME.colors.error))
            .child(icon_button(
                ("open-tool-change", key),
                AppIcon::Code,
                "Open in Neovim",
                ButtonTone::Quiet,
                move |window, cx| {
                    cx.stop_propagation();
                    on_open(window, cx);
                },
            ))
            .children(status_glyph.map(|glyph| {
                div()
                    .flex_none()
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.subtle)
                    .child(glyph)
            })),
        )
        .children(detail.map(|detail| disclosure_detail().min_w_0().child(detail)))
        .into_any_element()
}

fn change_count(label: String, color: gpui::Rgba) -> impl IntoElement {
    div()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .text_color(color)
        .child(label)
}
