#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoardLayoutMode {
    Wide,
    Narrow,
}

pub(crate) const BOARD_WIDTH: f32 = 1000.0;
pub(crate) const DETAIL_WIDTH: f32 = 400.0;
pub(crate) const DETAIL_MIN_WIDTH: f32 = 360.0;

pub(super) fn board_layout(viewport_width: f32) -> BoardLayoutMode {
    // The modal backdrop reserves 16px on either side.
    if (viewport_width - 32.0).min(BOARD_WIDTH) >= 760.0 {
        BoardLayoutMode::Wide
    } else {
        BoardLayoutMode::Narrow
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
