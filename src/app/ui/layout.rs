use gpui::Pixels;

use crate::app::ui::theme::theme;

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

pub(crate) fn draft_top_padding(height: Pixels) -> Pixels {
    let ratio = 0.18;
    gpui::px((f32::from(height) * ratio).clamp(
        f32::from(theme().size(24.0)),
        f32::from(theme().size(160.0)),
    ))
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
