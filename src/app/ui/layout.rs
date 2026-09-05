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

/// Leave breathing room without wasting transcript space in short windows.
pub(crate) fn composer_bottom_clearance(height: Pixels) -> Pixels {
    gpui::px(((f32::from(height) - 400.0) * 0.06).clamp(12.0, 28.0))
}

pub(crate) const fn shows_draft_inspector(mode: LayoutMode, enabled: bool) -> bool {
    shows_right_inline(mode) && enabled
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
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn composer_clearance_adapts_and_stays_bounded() {
        assert_eq!(composer_bottom_clearance(px(300.0)), px(12.0));
        assert_eq!(composer_bottom_clearance(px(600.0)), px(12.0));
        assert_eq!(composer_bottom_clearance(px(800.0)), px(24.0));
        assert_eq!(composer_bottom_clearance(px(2000.0)), px(28.0));
    }

    #[test]
    fn draft_inspector_only_uses_inline_space_when_requested_and_wide() {
        assert!(!shows_draft_inspector(LayoutMode::Wide, false));
        assert!(shows_draft_inspector(LayoutMode::Wide, true));
        assert!(!shows_draft_inspector(LayoutMode::Compact, true));
        assert!(!shows_draft_inspector(LayoutMode::Narrow, true));
    }

    #[test]
    fn draft_start_spacing_is_bounded_for_short_and_tall_windows() {
        assert_eq!(draft_top_padding(px(100.0)), px(24.0));
        assert_eq!(draft_top_padding(px(2000.0)), px(160.0));
        assert!(draft_top_padding(px(600.0)) < draft_top_padding(px(820.0)));
    }

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
