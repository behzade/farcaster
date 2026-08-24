//! Compact file-mutation actions for the transcript.

use crate::{
    assets::AppIcon,
    conversation::ToolPresentation,
    primitives::{ButtonTone, icon_button},
    theme::{MONO_FONT_FAMILY, THEME},
};
use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window,
    div, px,
};

pub(crate) fn render(
    label: &str,
    presentation: &ToolPresentation,
    key: usize,
    status_glyph: Option<&'static str>,
    on_open: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let (additions, deletions) = presentation.counts();
    div()
        .id(("tool-change", key))
        .w_full()
        .min_w_0()
        .h(px(34.0))
        .px(THEME.space.sm)
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .border_y(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.panel)
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
                .text_size(THEME.type_scale.caption)
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
            on_open,
        ))
        .children(status_glyph.map(|glyph| {
            div()
                .flex_none()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(glyph)
        }))
        .into_any_element()
}

fn change_count(label: String, color: gpui::Rgba) -> impl IntoElement {
    div()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .text_color(color)
        .child(label)
}
