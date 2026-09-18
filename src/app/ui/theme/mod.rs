use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use gpui::{App, Font, FontFallbacks, Pixels, Rgba, px};
use gpui_component::{
    highlighter::HighlightTheme,
    theme::{Theme as ComponentTheme, ThemeTokens},
};
use gpui_libghostty::{TerminalColor, TerminalTheme};
use serde::{Deserialize, Serialize};

pub(crate) mod builtin;
mod highlight;
mod library;

#[cfg(test)]
mod tests;

pub(crate) use highlight::SyntaxKey;
pub(crate) use library::{
    Appearance, ThemeDefinition, ThemeLibrary, color_hex, parse_hex, suggested_file_name,
    token_label,
};

pub(crate) const TRANSCRIPT_FONT_SIZE_RANGE: std::ops::RangeInclusive<f32> = 10.0..=32.0;

pub(crate) const UI_FONT_FAMILY: &str = "IBM Plex Sans";
pub(crate) const FARSI_FONT_FAMILY: &str = "Vazirmatn";
pub(crate) const MONO_FONT_FAMILY: &str = "Lilex";

pub(crate) fn ui_font() -> Font {
    Font {
        family: UI_FONT_FAMILY.into(),
        fallbacks: Some(FontFallbacks::from_fonts(vec![FARSI_FONT_FAMILY.into()])),
        ..Font::default()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Theme {
    pub colors: Colors,
    pub space: Space,
    pub type_scale: TypeScale,
    pub icons: IconScale,
    pub controls: ControlScale,
    pub radius: Pixels,
    pub border: Pixels,
    pub layout: Layout,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Colors {
    pub canvas: Rgba,
    pub panel: Rgba,
    pub inspector: Rgba,
    pub composer: Rgba,
    pub surface: Rgba,
    pub hover: Rgba,
    pub selection: Rgba,
    pub session_selection: Rgba,
    pub text_selection: Rgba,
    pub border: Rgba,
    pub focus_border: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub subtle: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_active: Rgba,
    pub link: Rgba,
    pub code: Rgba,
    pub skill: Rgba,
    pub file: Rgba,
    pub warning: Rgba,
    pub error: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub backdrop: Rgba,
}

macro_rules! color_keys {
    ($($field:ident),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        #[allow(non_camel_case_types)]
        pub(crate) enum ColorKey {
            $($field),*
        }

        impl ColorKey {
            pub(crate) const ALL: &'static [ColorKey] = &[$(ColorKey::$field),*];

            pub(crate) fn from_name(name: &str) -> Option<ColorKey> {
                match name {
                    $(stringify!($field) => Some(ColorKey::$field),)*
                    _ => None,
                }
            }

            pub(crate) fn name(self) -> &'static str {
                match self {
                    $(ColorKey::$field => stringify!($field)),*
                }
            }

            pub(crate) fn label(self) -> String {
                label(self.name())
            }
        }

        impl Colors {
            pub(crate) const EMPTY: Colors = Colors {
                $($field: Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }),*
            };

            pub(crate) fn get(self, key: ColorKey) -> Rgba {
                match key {
                    $(ColorKey::$field => self.$field),*
                }
            }

            pub(crate) fn set(&mut self, key: ColorKey, color: Rgba) {
                match key {
                    $(ColorKey::$field => self.$field = color),*
                }
            }
        }
    };
}

pub(crate) fn label(name: &str) -> String {
    name.replace('-', "_")
        .split('_')
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), characters.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

color_keys!(
    canvas,
    panel,
    inspector,
    composer,
    surface,
    hover,
    selection,
    session_selection,
    text_selection,
    border,
    focus_border,
    text,
    muted,
    subtle,
    accent,
    accent_hover,
    accent_active,
    link,
    code,
    skill,
    file,
    warning,
    error,
    danger,
    success,
    backdrop,
);

#[derive(Clone, Copy)]
pub(crate) struct Space {
    pub xs: Pixels,
    pub sm: Pixels,
    pub md: Pixels,
}

#[derive(Clone, Copy)]
pub(crate) struct TypeScale {
    pub caption: Pixels,
    pub body_small: Pixels,
    pub body: Pixels,
    pub reading: Pixels,
    pub display: Pixels,
    pub line_body: Pixels,
    pub line_reading: Pixels,
    pub line_composer: Pixels,
}

#[derive(Clone, Copy)]
pub(crate) struct IconScale {
    pub inline: Pixels,
    pub control: Pixels,
    pub prominent: Pixels,
}

#[derive(Clone, Copy)]
pub(crate) struct ControlScale {
    pub icon_button: Pixels,
    pub utility_row: Pixels,
    pub archived_preview_row: Pixels,
}
#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub window_width: Pixels,
    pub window_height: Pixels,
    pub session_rail: Pixels,
    pub session_rail_min: Pixels,
    pub session_rail_max: Pixels,
    pub run_panel: Pixels,
    pub run_panel_min: Pixels,
    pub run_panel_max: Pixels,
    pub transcript_overdraw: Pixels,
    pub composer_min: Pixels,
    pub conversation_width: Pixels,
    pub dialog_width: Pixels,
    pub dialog_max_height: Pixels,
    pub tool_max_height: Pixels,
    pub session_row_height: Pixels,
    pub status_row_height: Pixels,
}

#[derive(Clone, Copy)]
struct Structure {
    space: Space,
    type_scale: TypeScale,
    icons: IconScale,
    controls: ControlScale,
    radius: Pixels,
    border: Pixels,
    layout: Layout,
}

const STRUCTURE: Structure = Structure {
    space: Space {
        xs: px(4.0),
        sm: px(8.0),
        md: px(16.0),
    },
    type_scale: TypeScale {
        caption: px(12.0),
        body_small: px(13.0),
        body: px(13.0),
        reading: px(15.0),
        display: px(18.0),
        line_body: px(19.0),
        line_reading: px(23.0),
        line_composer: px(22.0),
    },
    icons: IconScale {
        inline: px(16.0),
        control: px(18.0),
        prominent: px(20.0),
    },
    controls: ControlScale {
        icon_button: px(28.0),
        utility_row: px(44.0),
        archived_preview_row: px(49.0),
    },
    radius: px(4.0),
    border: px(1.0),
    layout: Layout {
        window_width: px(1240.0),
        window_height: px(820.0),
        session_rail: px(286.0),
        session_rail_min: px(220.0),
        session_rail_max: px(430.0),
        run_panel: px(332.0),
        run_panel_min: px(220.0),
        run_panel_max: px(430.0),
        transcript_overdraw: px(160.0),
        composer_min: px(184.0),
        conversation_width: px(1040.0),
        dialog_width: px(560.0),
        dialog_max_height: px(680.0),
        tool_max_height: px(220.0),
        session_row_height: px(49.0),
        status_row_height: px(24.0),
    },
};

impl Theme {
    pub(crate) fn from_colors(colors: Colors) -> Self {
        Self {
            colors,
            space: STRUCTURE.space,
            type_scale: STRUCTURE.type_scale,
            icons: STRUCTURE.icons,
            controls: STRUCTURE.controls,
            radius: STRUCTURE.radius,
            border: STRUCTURE.border,
            layout: STRUCTURE.layout,
        }
    }
}

#[derive(Clone, Copy)]
struct ActiveTheme {
    theme: Theme,
    appearance: Appearance,
}

static ACTIVE: LazyLock<RwLock<ActiveTheme>> = LazyLock::new(|| {
    let definition = builtin::default_definition();
    RwLock::new(ActiveTheme {
        theme: Theme::from_colors(definition.colors),
        appearance: definition.appearance,
    })
});

static HIGHLIGHT: LazyLock<RwLock<Arc<HighlightTheme>>> = LazyLock::new(|| {
    let definition = builtin::default_definition();
    RwLock::new(highlight::for_theme(
        Theme::from_colors(definition.colors),
        definition.appearance,
        &syntax_overrides(&definition.tokens),
    ))
});

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ThemeToken {
    Palette(ColorKey),
    Syntax(SyntaxKey),
    Icon(usize),
}

static TOKENS: LazyLock<RwLock<Vec<(ThemeToken, Rgba)>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

pub(crate) fn editable_tokens() -> Vec<ThemeToken> {
    ColorKey::ALL
        .iter()
        .map(|key| ThemeToken::Palette(*key))
        .chain((0..crate::app::ui::file_icons::ICON_NAMES.len()).map(ThemeToken::Icon))
        .chain(SyntaxKey::ALL.iter().map(|key| ThemeToken::Syntax(*key)))
        .collect()
}

pub(crate) fn theme_tint(index: usize) -> Option<Rgba> {
    TOKENS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(token, _)| *token == ThemeToken::Icon(index))
        .map(|(_, color)| *color)
}

fn syntax_overrides(tokens: &[(ThemeToken, Rgba)]) -> Vec<(SyntaxKey, Rgba)> {
    tokens
        .iter()
        .filter_map(|(token, color)| match token {
            ThemeToken::Syntax(key) => Some((*key, *color)),
            _ => None,
        })
        .collect()
}

pub(crate) fn theme() -> Theme {
    active().theme
}

pub(crate) fn appearance() -> Appearance {
    active().appearance
}

pub(crate) fn highlight_theme() -> Arc<HighlightTheme> {
    HIGHLIGHT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn active() -> ActiveTheme {
    *ACTIVE.read().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn set_active(theme: Theme, appearance: Appearance, tokens: Vec<(ThemeToken, Rgba)>) {
    let overrides = syntax_overrides(&tokens);
    *ACTIVE.write().unwrap_or_else(PoisonError::into_inner) = ActiveTheme { theme, appearance };
    *HIGHLIGHT.write().unwrap_or_else(PoisonError::into_inner) =
        highlight::for_theme(theme, appearance, &overrides);
    *TOKENS.write().unwrap_or_else(PoisonError::into_inner) = tokens;
}

pub(crate) fn terminal_theme() -> TerminalTheme {
    let colors = theme().colors;
    TerminalTheme::new(
        terminal_color(colors.canvas),
        terminal_color(colors.text),
        [
            terminal_color(colors.panel),
            terminal_color(colors.error),
            terminal_color(colors.success),
            terminal_color(colors.code),
            terminal_color(colors.file),
            terminal_color(colors.skill),
            terminal_color(colors.accent_active),
            terminal_color(colors.muted),
            terminal_color(colors.subtle),
            terminal_color(colors.error),
            terminal_color(colors.success),
            terminal_color(colors.warning),
            terminal_color(colors.link),
            terminal_color(colors.skill),
            terminal_color(colors.accent),
            terminal_color(colors.text),
        ],
    )
}

fn terminal_color(color: Rgba) -> TerminalColor {
    let channel = |value: f32| (value * 255.0).round() as u8;
    TerminalColor::new(channel(color.r), channel(color.g), channel(color.b))
}

pub(crate) fn install_component_theme(cx: &mut App) {
    let theme = theme();
    let colors = theme.colors;
    let appearance = appearance();
    ComponentTheme::change(appearance.mode(), None, cx);
    let component = ComponentTheme::global_mut(cx);
    component.font_family = UI_FONT_FAMILY.into();
    component.font_size = theme.type_scale.body;
    component.mono_font_family = MONO_FONT_FAMILY.into();
    component.mono_font_size = theme.type_scale.body_small;
    component.highlight_theme = highlight_theme();
    component.radius = theme.radius;
    component.radius_lg = theme.radius;
    component.shadow = true;
    let component_colors = &mut component.colors;
    component_colors.background = colors.canvas.into();
    component_colors.foreground = colors.text.into();
    component_colors.accent = colors.surface.into();
    component_colors.accent_foreground = colors.text.into();
    component_colors.link = colors.link.into();
    component_colors.link_active = colors.link.into();
    component_colors.link_hover = colors.link.into();
    component_colors.border = colors.border.into();
    component_colors.input = colors.border.into();
    component_colors.muted = colors.panel.into();
    component_colors.muted_foreground = colors.muted.into();
    component_colors.popover = colors.panel.into();
    component_colors.popover_foreground = colors.text.into();
    component_colors.primary = colors.surface.into();
    component_colors.primary_hover = colors.hover.into();
    component_colors.primary_active = colors.hover.into();
    component_colors.primary_foreground = colors.text.into();
    component_colors.secondary = colors.surface.into();
    component_colors.secondary_hover = colors.hover.into();
    component_colors.secondary_active = colors.hover.into();
    component_colors.secondary_foreground = colors.text.into();
    component_colors.button = colors.surface.into();
    component_colors.button_hover = colors.hover.into();
    component_colors.button_active = colors.hover.into();
    component_colors.button_foreground = colors.text.into();
    component_colors.button_primary = colors.accent.into();
    component_colors.button_primary_hover = colors.accent_hover.into();
    component_colors.button_primary_active = colors.accent_active.into();
    component_colors.button_primary_foreground = colors.canvas.into();
    component_colors.danger = colors.danger.into();
    component_colors.danger_foreground = colors.canvas.into();
    component_colors.warning = colors.warning.into();
    component_colors.warning_foreground = colors.canvas.into();
    component_colors.success = colors.success.into();
    component_colors.success_foreground = colors.canvas.into();
    component_colors.ring = colors.accent.into();
    component_colors.caret = colors.text.into();
    component_colors.selection = colors.text_selection.into();
    component.tokens = ThemeTokens::from(component.colors);
}
