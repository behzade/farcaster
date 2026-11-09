use gpui::{Div, Pixels, Styled as _, div};

use crate::app::ui::theme::theme;

pub(crate) fn chat_column() -> Div {
    div()
        .w_full()
        .max_w(theme().size(1120.0))
        .mx_auto()
        .px(theme().size(40.0))
}

// Transcript rows include their own inset for message backgrounds and hit areas.
// Keep their content on the composer's grid without caller-supplied arithmetic.
pub(crate) fn transcript_column() -> Div {
    chat_column().px(theme().size(40.0) - super::grid::transcript_inset())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LayoutMode {
    Wide,
    Compact,
    Narrow,
}

pub(crate) fn wide_min_width() -> f32 {
    f32::from(theme().layout.wide_min_width)
}

pub(crate) fn compact_min_width() -> f32 {
    f32::from(theme().layout.compact_min_width)
}

pub(crate) fn composer_bottom_clearance(height: Pixels) -> Pixels {
    let ratio = 0.06;
    gpui::px(
        ((f32::from(height) - f32::from(theme().size(400.0))) * ratio)
            .clamp(f32::from(theme().size(12.0)), f32::from(theme().size(28.0))),
    )
}

pub(crate) fn layout_mode(width: Pixels) -> LayoutMode {
    let width = f32::from(width);
    if width >= wide_min_width() {
        LayoutMode::Wide
    } else if width >= compact_min_width() {
        LayoutMode::Compact
    } else {
        LayoutMode::Narrow
    }
}

pub(crate) const TRAFFIC_LIGHT_INSET: f32 = 88.0;

pub(crate) const fn shows_left_inline(mode: LayoutMode) -> bool {
    !matches!(mode, LayoutMode::Narrow)
}

pub(crate) const fn shows_right_inline(mode: LayoutMode) -> bool {
    matches!(mode, LayoutMode::Wide)
}

pub(crate) const fn shows_session_sheet_button(mode: LayoutMode) -> bool {
    matches!(mode, LayoutMode::Narrow)
}

pub(crate) const fn shows_run_sheet_button(mode: LayoutMode) -> bool {
    !matches!(mode, LayoutMode::Wide)
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
