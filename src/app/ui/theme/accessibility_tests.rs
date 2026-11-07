use super::*;
use gpui::Hsla;

fn luminance(color: Rgba) -> f64 {
    [color.r, color.g, color.b]
        .into_iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(channel, weight)| {
            let channel = f64::from(channel);
            weight
                * if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

fn assert_contrast(fg: Rgba, bg: Rgba, minimum: f64) {
    assert_eq!(fg.a, 1.0);
    assert_eq!(bg.a, 1.0);
    let a = luminance(fg);
    let b = luminance(bg);
    let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
    assert!(
        ratio >= minimum,
        "{} on {}: {ratio:.4}:1 < {minimum}:1",
        color_hex(fg),
        color_hex(bg)
    );
}

#[test]
fn default_text_syntax_icons_and_focus_keep_contrast_on_selected_rows() {
    let definition = builtin::default_definition();
    let colors = definition.colors;
    for background in [
        colors.canvas,
        colors.panel,
        colors.inspector,
        colors.composer,
        colors.surface,
        colors.highlight,
    ] {
        for foreground in [
            colors.text,
            colors.muted,
            colors.subtle,
            colors.accent,
            colors.link,
            colors.code,
            colors.skill,
            colors.file,
            colors.warning,
            colors.error,
            colors.danger,
            colors.success,
            colors.indicator,
        ] {
            assert_contrast(foreground, background, 4.5);
        }
        for &key in SyntaxKey::ALL {
            assert_contrast(definition.color(ThemeToken::Syntax(key)), background, 4.5);
        }
        for index in 0..crate::app::ui::file_icons::ICON_NAMES.len() {
            assert_contrast(definition.color(ThemeToken::Icon(index)), background, 3.0);
        }
        assert_contrast(colors.focus_border, background, 3.0);
    }
    assert_contrast(colors.terminal_green, colors.canvas, 4.5);
}

#[test]
fn default_strings_and_terminal_green_have_their_own_colors() {
    let definition = builtin::default_definition();
    let string = definition.color(ThemeToken::Syntax(SyntaxKey::string));
    assert_ne!(string, definition.colors.success);
    assert_ne!(definition.colors.terminal_green, definition.colors.success);
    let highlighted = highlight::for_theme(
        Theme::from_definition(definition),
        definition.appearance,
        &syntax_overrides(&definition.tokens),
    );
    assert_eq!(
        highlighted.style.syntax.string.unwrap().color,
        Some(string.into())
    );

    let mut colors = definition.colors;
    let original = terminal_theme_for(colors);
    colors.success = parse_hex("#ff00ff").unwrap();
    assert_eq!(terminal_theme_for(colors), original);

    colors.terminal_green = parse_hex("#b1c47b").unwrap();
    let changed = terminal_theme_for(colors);
    for index in 0..16 {
        if matches!(index, 2 | 10) {
            assert_eq!(
                changed.palette[index],
                terminal_color(colors.terminal_green)
            );
            assert_ne!(changed.palette[index], original.palette[index]);
        } else {
            assert_eq!(changed.palette[index], original.palette[index]);
        }
    }
}

#[test]
fn older_themes_keep_their_terminal_green_when_the_token_is_missing() {
    for definition in builtin::BUILT_IN_THEMES.iter() {
        let css = definition.to_css().unwrap().replace(
            &format!(
                "  --terminal_green: {};\n",
                color_hex(definition.colors.terminal_green)
            ),
            "",
        );
        let legacy = ThemeDefinition::from_css(&css).unwrap();
        assert_eq!(legacy.colors.terminal_green, legacy.colors.success);
        let saved = ThemeDefinition::from_css(&legacy.to_css().unwrap()).unwrap();
        assert_eq!(saved, legacy);
    }
}

#[gpui::test]
fn installed_button_tokens_use_the_palette_in_every_state(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        install_component_theme(cx);
        let colors = theme().colors;
        let component = ComponentTheme::global(cx);
        assert_eq!(component.input, Hsla::from(colors.focus_border));
        for background in [colors.canvas, colors.panel, colors.composer] {
            assert_contrast(component.input.into(), background, 3.0);
            assert_contrast(component.ring.into(), background, 3.0);
        }
        for (fill, hover, active, foreground, expected_states, expected_foreground) in [
            (
                component.button_primary,
                component.button_primary_hover,
                component.button_primary_active,
                component.button_primary_foreground,
                [colors.accent, colors.accent_hover, colors.accent_active],
                colors.canvas,
            ),
            (
                component.button_secondary,
                component.button_secondary_hover,
                component.button_secondary_active,
                component.button_secondary_foreground,
                [colors.surface, colors.highlight, colors.highlight],
                colors.text,
            ),
            (
                component.button_danger,
                component.button_danger_hover,
                component.button_danger_active,
                component.button_danger_foreground,
                [colors.danger; 3],
                colors.canvas,
            ),
            (
                component.button_warning,
                component.button_warning_hover,
                component.button_warning_active,
                component.button_warning_foreground,
                [colors.warning; 3],
                colors.canvas,
            ),
            (
                component.button_success,
                component.button_success_hover,
                component.button_success_active,
                component.button_success_foreground,
                [colors.success; 3],
                colors.canvas,
            ),
        ] {
            assert_eq!(foreground, Hsla::from(expected_foreground));
            for (background, expected) in [fill, hover, active].into_iter().zip(expected_states) {
                assert_eq!(background, Hsla::from(expected));
                assert_contrast(foreground.into(), background.into(), 4.5);
            }
        }
    });
}
