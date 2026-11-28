#[cfg(target_os = "macos")]
use gpui::ParentElement as _;
use gpui::{
    BoxShadow, Div, Pixels, Styled as _, div, hsla, linear_color_stop, linear_gradient, px,
};

use crate::app::ui::theme::{Appearance, appearance, theme};

#[cfg(target_os = "macos")]
mod macos;

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

pub(crate) fn control_shadow() -> Vec<BoxShadow> {
    let dark = appearance() == Appearance::Dark;
    vec![
        BoxShadow {
            color: hsla(0.0, 0.0, 0.0, if dark { 0.30 } else { 0.09 }),
            offset: gpui::point(px(0.0), px(1.0)),
            blur_radius: px(2.0),
            spread_radius: px(0.0),
            inset: false,
        },
        BoxShadow {
            color: hsla(0.0, 0.0, 0.0, if dark { 0.16 } else { 0.05 }),
            offset: gpui::point(px(0.0), px(0.0)),
            blur_radius: px(1.0),
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

#[derive(Clone, Copy)]
pub(crate) enum RaisedEdge {
    Top,
    Left,
    Right,
}

pub(crate) fn raised_edge_shadow(edge: RaisedEdge, native_surface: bool) -> Div {
    let depth = theme().size(8.0);
    let opacity = if appearance() == Appearance::Dark {
        0.22
    } else {
        0.08
    };
    let (strip, angle) = match edge {
        RaisedEdge::Top => (div().top_0().left_0().right_0().h(depth), 180.0),
        RaisedEdge::Left => (div().top_0().bottom_0().left_0().w(depth), 90.0),
        RaisedEdge::Right => (div().top_0().bottom_0().right_0().w(depth), 270.0),
    };
    let strip = strip.absolute();
    // Native Ghostty views sit above GPUI's Metal drawing. Use a decoration
    // layer above those views, with no view or hitbox to intercept editor input.
    #[cfg(target_os = "macos")]
    if native_surface {
        return strip.child(macos::shadow(edge, opacity));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = native_surface;
    strip.bg(linear_gradient(
        angle,
        linear_color_stop(hsla(0.0, 0.0, 0.0, opacity), 0.0),
        linear_color_stop(hsla(0.0, 0.0, 0.0, 0.0), 1.0),
    ))
}
