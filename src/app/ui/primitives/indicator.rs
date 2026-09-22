use gpui::{AnyElement, IntoElement, Rgba, Styled as _, div};

use crate::app::ui::theme::theme;

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
