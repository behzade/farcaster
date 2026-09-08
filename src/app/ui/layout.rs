use gpui::Pixels;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LayoutMode {
    Wide,
    Compact,
    Narrow,
}

pub(crate) const WIDE_MIN_WIDTH: f32 = 1_180.0;
pub(crate) const COMPACT_MIN_WIDTH: f32 = 960.0;

pub(crate) fn draft_top_padding(height: Pixels) -> Pixels {
    gpui::px((f32::from(height) * 0.18).clamp(24.0, 160.0))
}

pub(crate) fn composer_bottom_clearance(height: Pixels) -> Pixels {
    gpui::px(((f32::from(height) - 400.0) * 0.06).clamp(12.0, 28.0))
}

pub(crate) fn layout_mode(width: Pixels) -> LayoutMode {
    let width = f32::from(width);
    if width >= WIDE_MIN_WIDTH {
        LayoutMode::Wide
    } else if width >= COMPACT_MIN_WIDTH {
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
