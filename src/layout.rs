use gpui::Pixels;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LayoutMode {
    Wide,
    Compact,
    Narrow,
}

pub(crate) const WIDE_MIN_WIDTH: f32 = 1_180.0;
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
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn exact_layout_boundaries_are_stable() {
        assert_eq!(layout_mode(px(959.0)), LayoutMode::Narrow);
        assert_eq!(layout_mode(px(960.0)), LayoutMode::Compact);
        assert_eq!(layout_mode(px(1_179.0)), LayoutMode::Compact);
        assert_eq!(layout_mode(px(1_180.0)), LayoutMode::Wide);
    }

    #[test]
    fn compact_moves_only_the_right_panel_and_narrow_moves_both() {
        assert!(shows_left_inline(LayoutMode::Wide));
        assert!(shows_right_inline(LayoutMode::Wide));
        assert!(!shows_run_sheet_button(LayoutMode::Wide));

        assert!(shows_left_inline(LayoutMode::Compact));
        assert!(!shows_right_inline(LayoutMode::Compact));
        assert!(!shows_session_sheet_button(LayoutMode::Compact));
        assert!(shows_run_sheet_button(LayoutMode::Compact));

        assert!(!shows_left_inline(LayoutMode::Narrow));
        assert!(!shows_right_inline(LayoutMode::Narrow));
        assert!(shows_session_sheet_button(LayoutMode::Narrow));
        assert!(shows_run_sheet_button(LayoutMode::Narrow));
    }
}
