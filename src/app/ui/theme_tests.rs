use super::{FARSI_FONT_FAMILY, THEME, UI_FONT_FAMILY, ui_font};

#[test]
fn ui_font_uses_plex_sans_with_a_persian_fallback() {
    let font = ui_font();
    assert_eq!(font.family, UI_FONT_FAMILY);
    assert_eq!(
        font.fallbacks
            .expect("UI font should have a Persian fallback")
            .fallback_list(),
        &[FARSI_FONT_FAMILY]
    );
}

#[test]
fn sidebar_widths_match_the_design_bounds() {
    assert_eq!(f32::from(THEME.layout.session_rail), 286.0);
    assert_eq!(f32::from(THEME.layout.session_rail_min), 220.0);
    assert_eq!(f32::from(THEME.layout.session_rail_max), 430.0);
    assert_eq!(f32::from(THEME.layout.run_panel), 332.0);
    assert_eq!(f32::from(THEME.layout.run_panel_min), 220.0);
    assert_eq!(f32::from(THEME.layout.run_panel_max), 430.0);
}

#[test]
fn icon_and_control_tokens_keep_icons_optically_proportional() {
    assert!(THEME.icons.inline >= THEME.type_scale.body);
    assert!(THEME.icons.control > THEME.icons.inline);
    assert!(THEME.icons.prominent >= THEME.icons.control);
    assert!(THEME.controls.icon_button > THEME.icons.prominent);
    assert!(
        THEME.controls.utility_row >= THEME.controls.icon_button + THEME.space.sm + THEME.space.sm
    );
}
