use gpui::{BoxShadow, Div, Hsla, Pixels, Styled as _, div, hsla, px};

use crate::app::ui::theme::{Appearance, appearance, theme};

/// Floating surfaces share a shape and depth; inner sections use spacing.
pub(crate) fn surface_radius() -> Pixels {
    theme().radius.max(theme().size(10.0))
}

pub(crate) fn surface_shadow() -> Vec<BoxShadow> {
    let dark = appearance() == Appearance::Dark;
    vec![
        BoxShadow {
            color: hsla(0.0, 0.0, 0.0, if dark { 0.32 } else { 0.12 }),
            offset: gpui::point(px(0.0), px(12.0)),
            blur_radius: px(16.0),
            spread_radius: px(-4.0),
            inset: false,
        },
        BoxShadow {
            color: hsla(0.0, 0.0, 0.0, if dark { 0.18 } else { 0.06 }),
            offset: gpui::point(px(0.0), px(2.0)),
            blur_radius: px(3.0),
            spread_radius: px(0.0),
            inset: false,
        },
    ]
}

pub(crate) fn floating_surface() -> Div {
    div()
        .rounded(surface_radius())
        .bg(theme().colors.panel)
        .shadow(surface_shadow())
}

/// Pane edges remain draggable without the contrast of an outlined surface.
pub(crate) fn separator_color() -> Hsla {
    let mut color = Hsla::from(theme().colors.border);
    color.a *= 0.35;
    color
}
