//! Pure responsive shell policy.

use gpui::Pixels;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LayoutMode {
    Wide,
    Compact,
    Narrow,
}

// Preserve a 956px chat canvas for the fixed-width composer footer when sidebars are inline.
pub(crate) const WIDE_MIN_WIDTH: f32 = 1_540.0;
pub(crate) const COMPACT_MIN_WIDTH: f32 = 1_240.0;
pub(crate) const SPLIT_DIFF_MIN_WIDTH: f32 = 760.0;

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

pub(crate) fn shows_split_diff(width: Pixels) -> bool {
    f32::from(width) >= SPLIT_DIFF_MIN_WIDTH
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
        assert_eq!(layout_mode(px(1_239.0)), LayoutMode::Narrow);
        assert_eq!(layout_mode(px(1_240.0)), LayoutMode::Compact);
        assert_eq!(layout_mode(px(1_539.0)), LayoutMode::Compact);
        assert_eq!(layout_mode(px(1_540.0)), LayoutMode::Wide);
    }

    #[test]
    fn inline_sidebars_preserve_the_fixed_composer_footer() {
        let wide_chat = WIDE_MIN_WIDTH
            - f32::from(crate::theme::THEME.layout.session_rail)
            - f32::from(crate::theme::THEME.layout.run_panel);
        let compact_chat =
            COMPACT_MIN_WIDTH - f32::from(crate::theme::THEME.layout.session_rail);

        assert!(wide_chat >= 956.0);
        assert!(compact_chat >= 956.0);
    }

    #[test]
    fn diff_mode_tracks_the_space_available_to_the_diff() {
        assert!(!shows_split_diff(px(759.0)));
        assert!(shows_split_diff(px(760.0)));
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
