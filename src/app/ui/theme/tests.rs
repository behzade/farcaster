use super::{
    Appearance, ColorKey, FARSI_FONT_FAMILY, LengthKey, SyntaxKey, Theme, ThemeDefinition,
    ThemeLibrary, UI_FONT_FAMILY, builtin::BUILT_IN_THEMES, color_hex, highlight, parse_hex, px,
    ui_font,
};
use crate::app::ui::file_icons;
use gpui::Hsla;

fn default_theme() -> Theme {
    Theme::from_colors(BUILT_IN_THEMES[0].colors)
}

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
    assert_eq!(f32::from(default_theme().layout.session_rail), 286.0);
    assert_eq!(f32::from(default_theme().layout.session_rail_min), 220.0);
    assert_eq!(f32::from(default_theme().layout.session_rail_max), 430.0);
    assert_eq!(f32::from(default_theme().layout.run_panel), 332.0);
    assert_eq!(f32::from(default_theme().layout.run_panel_min), 220.0);
    assert_eq!(f32::from(default_theme().layout.run_panel_max), 430.0);
}

#[test]
fn icon_and_control_tokens_keep_icons_optically_proportional() {
    let theme = default_theme();
    assert!(theme.icons.inline >= theme.type_scale.body);
    assert!(theme.icons.control > theme.icons.inline);
    assert!(theme.icons.prominent >= theme.icons.control);
    assert!(theme.controls.icon_button > theme.icons.prominent);
    assert!(
        theme.controls.utility_row >= theme.controls.icon_button + theme.space.sm + theme.space.sm
    );
}

#[test]
fn bundled_themes_parse_and_keep_distinct_names() {
    assert_eq!(BUILT_IN_THEMES.len(), 3);
    let mut names = Vec::new();
    for definition in BUILT_IN_THEMES.iter() {
        definition.validate().expect("valid bundled theme");
        assert!(!names.contains(&definition.name));
        names.push(definition.name.clone());
    }
    assert_eq!(names, ["Farcaster", "White", "Black"]);
    assert_eq!(BUILT_IN_THEMES[1].appearance, Appearance::Light);
}

#[test]
fn the_default_library_starts_on_the_bundled_theme() {
    let library = ThemeLibrary::default();
    assert_eq!(library.selected_name(), BUILT_IN_THEMES[0].name);
    assert_eq!(library.selected(), BUILT_IN_THEMES[0]);
    assert_eq!(library.selected().colors, default_theme().colors);
    assert!(library.user_themes().is_empty());
}

#[test]
fn every_color_token_is_reachable_by_key() {
    let mut colors = default_theme().colors;
    for key in ColorKey::ALL.iter() {
        let color = colors.get(*key);
        let shifted = parse_hex(&color_hex(color)).expect("hex round trip");
        colors.set(*key, shifted);
        assert!(!key.label().is_empty());
    }
    assert_eq!(colors, default_theme().colors);
}

#[test]
fn color_hex_drops_opaque_alpha_and_keeps_translucent_alpha() {
    let opaque = parse_hex("#123456").expect("hex");
    assert_eq!(color_hex(opaque), "#123456");
    let translucent = parse_hex("#12345678").expect("hex");
    assert_eq!(color_hex(translucent), "#12345678");
    assert!(parse_hex("nonsense").is_none());
}

#[test]
fn a_palette_keeps_the_design_tokens_untouched() {
    let black = Theme::from_colors(BUILT_IN_THEMES[2].colors);
    assert_eq!(black.colors.canvas, parse_hex("#000000").expect("hex"));
    assert_eq!(black.space.md, default_theme().space.md);
    assert_eq!(black.type_scale.reading, default_theme().type_scale.reading);
    assert_eq!(
        black.layout.session_rail,
        default_theme().layout.session_rail
    );
}

#[test]
fn syntax_tokens_default_to_the_palette() {
    let colors = BUILT_IN_THEMES[0].colors;
    assert_eq!(SyntaxKey::keyword.default_color(colors), colors.accent);
    assert_eq!(SyntaxKey::string.default_color(colors), colors.success);
    assert_eq!(SyntaxKey::comment.default_color(colors), colors.subtle);
    assert_eq!(SyntaxKey::title.default_color(colors), colors.text);
}

#[test]
fn syntax_overrides_reach_the_highlight_theme() {
    let theme = default_theme();
    let keyword = parse_hex("#123456").expect("hex");
    let highlighted =
        highlight::for_theme(theme, Appearance::Dark, &[(SyntaxKey::keyword, keyword)]);
    assert_eq!(
        highlighted
            .style
            .syntax
            .keyword
            .and_then(|style| style.color),
        Some(Hsla::from(keyword))
    );
    assert_eq!(
        highlighted
            .style
            .syntax
            .string
            .and_then(|style| style.color),
        Some(Hsla::from(theme.colors.success))
    );
}

#[test]
fn every_editable_token_has_a_name_and_a_label() {
    let tokens = super::editable_tokens();
    assert_eq!(
        tokens.len(),
        ColorKey::ALL.len() + file_icons::ICON_NAMES.len() + SyntaxKey::ALL.len()
    );
    for token in tokens {
        assert!(!super::library::token_name(token).is_empty());
        assert!(!super::token_label(token).is_empty());
    }
}

#[test]
fn length_tokens_default_to_the_designed_structure() {
    let definition = BUILT_IN_THEMES[0].clone();
    let theme = Theme::from_definition(&definition);
    assert_eq!(f32::from(theme.space.xs), 4.0);
    assert_eq!(f32::from(theme.radius), 4.0);
    assert_eq!(f32::from(theme.size(24.0)), 24.0);
    assert_eq!(
        definition.length(LengthKey::from_name("space-xs").expect("token")),
        theme.space.xs
    );
    assert_eq!(
        definition.length(LengthKey::from_name("size-24").expect("token")),
        theme.size(24.0)
    );
    assert!(!definition.is_custom_length(LengthKey::from_name("radius").expect("token")));
}

#[test]
fn length_tokens_override_the_structure() {
    let mut definition = BUILT_IN_THEMES[0].clone();
    let space_xs = LengthKey::from_name("space-xs").expect("token");
    let size_24 = LengthKey::from_name("size-24").expect("token");
    definition.set_length(space_xs, px(6.0));
    definition.set_length(size_24, px(30.0));
    let theme = Theme::from_definition(&definition);
    assert_eq!(f32::from(theme.space.xs), 6.0);
    assert_eq!(f32::from(theme.size(24.0)), 30.0);
    assert_eq!(f32::from(theme.size(28.0)), 28.0);
    assert_eq!(f32::from(theme.type_scale.body), 13.0);
    assert!(definition.is_custom_length(space_xs));
}

#[test]
fn every_editable_length_has_a_name_and_a_label() {
    for key in super::editable_lengths() {
        assert!(!key.name().is_empty());
        assert!(!key.label().is_empty());
        assert_eq!(LengthKey::from_name(&key.name()), Some(key));
    }
}

#[test]
fn theme_definitions_round_trip_through_css() {
    let definition =
        ThemeDefinition::from_css(&BUILT_IN_THEMES[0].to_css().expect("encode bundled theme"))
            .expect("decode bundled theme");
    assert_eq!(definition, BUILT_IN_THEMES[0]);
}
