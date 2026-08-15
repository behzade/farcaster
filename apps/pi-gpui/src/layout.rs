//! Pure responsive shell policy.

use gpui::Pixels;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LayoutMode {
    Wide,
    Compact,
    Narrow,
}

pub(crate) const WIDE_MIN_WIDTH: f32 = 1_320.0;
pub(crate) const COMPACT_MIN_WIDTH: f32 = 960.0;

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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn exact_layout_boundaries_are_stable() {
        assert_eq!(layout_mode(px(959.0)), LayoutMode::Narrow);
        assert_eq!(layout_mode(px(960.0)), LayoutMode::Compact);
        assert_eq!(layout_mode(px(1_319.0)), LayoutMode::Compact);
        assert_eq!(layout_mode(px(1_320.0)), LayoutMode::Wide);
    }
}
