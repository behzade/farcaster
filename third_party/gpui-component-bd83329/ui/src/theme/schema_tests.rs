use gpui::{linear_color_stop, linear_gradient, px};

use crate::{Theme, ThemeConfig, ThemeMode, try_parse_color};

#[test]
fn test_semantic_theme_config_parses_and_roundtrips() {
    let value = serde_json::json!({
        "name": "Semantic",
        "mode": "dark",
        "tokens": {
            "colors": {
                "surface": "#111827",
                "surface_foreground": "#f9fafb",
                "primary": "#2563eb",
                "destructive": "#dc2626"
            },
            "radius": { "sm": 2.0, "md": 6.0, "lg": 10.0 },
            "spacing": { "xs": 4.0, "md": 12.0, "xl": 24.0 },
            "typography": {
                "sans": "Inter",
                "md": { "size": 15.0, "line_height": 22.0 }
            }
        }
    });
    let config: super::SemanticThemeConfigFile = serde_json::from_value(value).unwrap();
    let serialized = serde_json::to_string(&config).unwrap();
    let reparsed: super::SemanticThemeConfigFile = serde_json::from_str(&serialized).unwrap();
    let semantic = reparsed.tokens;

    assert_eq!(semantic.colors.surface.as_deref(), Some("#111827"));
    assert_eq!(semantic.colors.destructive.as_deref(), Some("#dc2626"));
    assert_eq!(semantic.radius.lg, Some(10.0));
    assert_eq!(semantic.spacing.xl, Some(24.0));
    assert_eq!(semantic.typography.sans.as_deref(), Some("Inter"));
    assert_eq!(semantic.typography.md.line_height, Some(22.0));

    let mut theme = Theme::default();
    let resolved = theme.apply_semantic_config_str(&serialized).unwrap();
    assert_eq!(theme.primary, try_parse_color("#2563eb").unwrap());
    assert_eq!(resolved.spacing.xl, px(24.));
    assert_eq!(resolved.typography.md.line_height, px(22.));
}

#[test]
fn test_semantic_tokens_override_legacy_generic_fields_only() {
    let config = serde_json::from_value::<super::SemanticThemeConfigFile>(serde_json::json!({
        "tokens": {
            "colors": { "primary": "#2563eb", "destructive": "#b91c1c" },
            "spacing": { "md": 14.0 },
            "typography": { "md": { "size": 15.0 } }
        }
    }))
    .unwrap();
    let mut theme = Theme::default();
    let component_color = theme.button_primary;
    let resolved = theme.apply_semantic_config(&config.tokens);

    assert_eq!(theme.primary, try_parse_color("#2563eb").unwrap());
    assert_eq!(theme.danger, try_parse_color("#b91c1c").unwrap());
    assert_eq!(theme.button_primary, component_color);
    assert_eq!(resolved.spacing.md, px(14.));
    assert_eq!(resolved.typography.md.size, px(15.));
}

#[test]
fn test_legacy_config_without_semantic_tokens_is_unchanged() {
    let config = serde_json::from_value::<ThemeConfig>(serde_json::json!({
        "name": "Legacy",
        "mode": "light",
        "radius": 7,
        "colors": { "primary.background": "#7c3aed" }
    }))
    .unwrap();
    let mut theme = Theme::default();
    theme.apply_config(&std::rc::Rc::new(config));
    assert_eq!(theme.primary, try_parse_color("#7c3aed").unwrap());
    assert_eq!(theme.radius, px(7.));
    assert_eq!(theme.semantic_tokens().spacing, Default::default());
}

#[test]
fn test_apply_config_preserves_gradient_background_and_solid_color_fallback() {
    let config = serde_json::from_value::<ThemeConfig>(serde_json::json!({
        "name": "Gradient",
        "mode": "light",
        "colors": {
            "primary.background": "linear-gradient(135deg, #4F46E5, #06B6D4)",
            "button.primary.hover.background": "linear-gradient(to right, red-500 25%, blue-600 75%)"
        }
    }))
    .unwrap();

    let mut theme = Theme::default();
    theme.apply_config(&std::rc::Rc::new(config));

    let primary_from = try_parse_color("#4F46E5").unwrap();
    let primary_to = try_parse_color("#06B6D4").unwrap();
    assert_eq!(theme.primary, primary_from);
    assert_eq!(theme.tokens.primary.color, primary_from);
    assert_eq!(
        theme.tokens.primary.background,
        linear_gradient(
            135.,
            linear_color_stop(primary_from, 0.),
            linear_color_stop(primary_to, 1.)
        )
    );
    assert_eq!(
        theme.tokens.button_primary.background,
        theme.tokens.primary.background
    );
    assert_eq!(
        theme.tokens.button_primary_hover.background,
        linear_gradient(
            90.,
            linear_color_stop(crate::red_500(), 0.25),
            linear_color_stop(crate::blue_600(), 0.75)
        )
    );
    assert_eq!(theme.mode, ThemeMode::Light);
}

#[test]
fn test_apply_config_clamps_highlight_alpha_per_gradient_stop() {
    let config = serde_json::from_value::<ThemeConfig>(serde_json::json!({
        "name": "Highlight",
        "mode": "light",
        "colors": {
            // Solid above the cap: must be capped to 0.2, not attenuated twice.
            "list.active.background": "#3b82f6",
            // Gradient with a faint `from` stop and an opaque `to` stop: the
            // `to` stop must be clamped independently, not left at full alpha.
            "table.active.background": "linear-gradient(#bfdbfe33, #3b82f6)",
            // Gradient with a transparent `from` stop: the opaque `to` stop
            // must still be clamped (the `base == 0` factor fallback used to
            // leave it untouched).
            "selection.background": "linear-gradient(#3b82f600, #3b82f6)",
        }
    }))
    .unwrap();

    let mut theme = Theme::default();
    theme.apply_config(&std::rc::Rc::new(config));

    // Solid: representative color and rendered background both capped at 0.2.
    let blue = try_parse_color("#3b82f6").unwrap();
    assert_eq!(theme.list_active, blue.alpha(0.2));
    assert_eq!(theme.tokens.list_active.background, blue.alpha(0.2).into());

    // Gradient: the opaque `to` stop is clamped to 0.2, not left fully opaque.
    let faint = try_parse_color("#bfdbfe33").unwrap();
    assert_eq!(
        theme.tokens.table_active.background,
        linear_gradient(
            180.,
            linear_color_stop(faint.alpha(faint.a.min(0.2)), 0.),
            linear_color_stop(blue.alpha(0.2), 1.),
        )
    );

    // Gradient: a transparent `from` stop stays transparent while the opaque
    // `to` stop is still clamped to 0.3 (selection cap).
    let clear = try_parse_color("#3b82f600").unwrap();
    assert_eq!(
        theme.tokens.selection.background,
        linear_gradient(
            180.,
            linear_color_stop(clear.alpha(clear.a.min(0.3)), 0.),
            linear_color_stop(blue.alpha(0.3), 1.),
        )
    );
}
