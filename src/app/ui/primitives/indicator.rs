use gpui::{AnyElement, IntoElement, Rgba, Styled as _, div};

use crate::app::ui::theme::theme;

/// Paint a focus edge without changing the row's content bounds.
pub(crate) fn focus_outline(color: Rgba) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: color.into(),
        offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
        blur_radius: gpui::px(0.0),
        spread_radius: theme().border,
        inset: true,
    }]
}

#[derive(Clone, Copy)]
pub(crate) enum IndicatorEdge {
    Bottom,
    Leading,
}

pub(crate) fn line_indicator(edge: IndicatorEdge, color: Rgba) -> AnyElement {
    let thickness = theme().size(2.0);
    match edge {
        IndicatorEdge::Bottom => div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(thickness)
            .bg(color)
            .into_any_element(),
        IndicatorEdge::Leading => div()
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .w(thickness)
            .bg(color)
            .into_any_element(),
    }
}
