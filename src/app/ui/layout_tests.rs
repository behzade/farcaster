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
