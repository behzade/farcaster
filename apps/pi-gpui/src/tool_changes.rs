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
    disclosure: AnyElement,
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
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .child(disclosure)
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
                    on_open,
                ))
                .children(status_glyph.map(|glyph| {
                    div()
                        .flex_none()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child(glyph)
                })),
        )
        .children(detail.map(|detail| {
            div()
                .ml(px(22.0))
                .mt(THEME.space.xs)
                .min_w_0()
                .child(detail)
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
