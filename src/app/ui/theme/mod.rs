use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use gpui::{App, Font, FontFallbacks, Pixels, Rgba, px};
use gpui_component::{
    highlighter::HighlightTheme,
    theme::{Theme as ComponentTheme, ThemeTokens},
};
use gpui_libghostty::{TerminalColor, TerminalTheme};
use serde::{Deserialize, Serialize};

pub(crate) mod builtin;
mod library;

#[cfg(test)]
mod tests;

pub(crate) use crate::app::ui::primitives::{SyntaxKey, highlight};
pub(crate) use library::{
    Appearance, ThemeDefinition, ThemeLibrary, color_hex, length_label, parse_hex,
    suggested_file_name, token_label,
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
    pub scrollbar: ScrollbarScale,
    sizes: [Pixels; SIZE_VALUES.len()],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Colors {
    pub canvas: Rgba,
    pub panel: Rgba,
    pub inspector: Rgba,
    pub composer: Rgba,
    pub surface: Rgba,
    pub highlight: Rgba,
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
    pub indicator: Rgba,
}

macro_rules! color_keys {
    ($($required:ident),* $(,)? ; $($optional:ident),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        #[allow(non_camel_case_types)]
        pub(crate) enum ColorKey {
            $($required,)*
            $($optional,)*
        }

        impl ColorKey {
            pub(crate) const REQUIRED: &'static [ColorKey] = &[$(ColorKey::$required),*];
            pub(crate) const OPTIONAL: &'static [ColorKey] = &[$(ColorKey::$optional),*];
            pub(crate) const ALL: &'static [ColorKey] =
                &[$(ColorKey::$required,)* $(ColorKey::$optional,)*];

            pub(crate) fn from_name(name: &str) -> Option<ColorKey> {
                match name {
                    $(stringify!($required) => Some(ColorKey::$required),)*
                    $(stringify!($optional) => Some(ColorKey::$optional),)*
                    _ => None,
                }
            }

            pub(crate) fn name(self) -> &'static str {
                match self {
                    $(ColorKey::$required => stringify!($required),)*
                    $(ColorKey::$optional => stringify!($optional),)*
                }
            }

            pub(crate) fn label(self) -> String {
                label(self.name())
            }
        }

        impl Colors {
            pub(crate) const EMPTY: Colors = Colors {
                $($required: Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                },)*
                $($optional: Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                },)*
            };

            pub(crate) fn get(self, key: ColorKey) -> Rgba {
                match key {
                    $(ColorKey::$required => self.$required,)*
                    $(ColorKey::$optional => self.$optional,)*
                }
            }

            pub(crate) fn set(&mut self, key: ColorKey, color: Rgba) {
                match key {
                    $(ColorKey::$required => self.$required = color,)*
                    $(ColorKey::$optional => self.$optional = color,)*
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
    highlight,
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
    backdrop;
    indicator
);

pub(crate) fn default_optional_color(colors: Colors, key: ColorKey) -> Rgba {
    match key {
        ColorKey::indicator => colors.muted,
        _ => colors.text,
    }
}

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
    pub wide_min_width: Pixels,
    pub compact_min_width: Pixels,
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
pub(crate) struct MetricScale {
    pub radius: Pixels,
    pub border_width: Pixels,
}

#[derive(Clone, Copy)]
pub(crate) struct ScrollbarScale {
    pub width: Pixels,
    pub inset: Pixels,
}

const SIZE_VALUES: [u16; 94] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 28,
    30, 32, 34, 36, 38, 40, 44, 47, 48, 49, 50, 56, 60, 65, 72, 80, 90, 100, 110, 116, 120, 130,
    132, 140, 150, 160, 180, 184, 190, 200, 211, 220, 240, 252, 260, 264, 280, 286, 300, 320, 332,
    360, 400, 420, 421, 430, 480, 500, 520, 560, 600, 620, 640, 680, 700, 800, 820, 860, 900, 959,
    960, 1040, 1080, 1200, 1240, 1440, 1920, 2000,
];

#[derive(Clone, Copy)]
struct Structure {
    space: Space,
    type_scale: TypeScale,
    icons: IconScale,
    controls: ControlScale,
    metrics: MetricScale,
    scrollbar: ScrollbarScale,
    layout: Layout,
    sizes: [Pixels; SIZE_VALUES.len()],
}

const fn default_sizes() -> [Pixels; SIZE_VALUES.len()] {
    let mut sizes = [px(0.0); SIZE_VALUES.len()];
    let mut index = 0;
    while index < SIZE_VALUES.len() {
        sizes[index] = px(SIZE_VALUES[index] as f32);
        index += 1;
    }
    sizes
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
    metrics: MetricScale {
        radius: px(0.0),
        border_width: px(1.0),
    },
    scrollbar: ScrollbarScale {
        width: px(6.0),
        inset: px(0.0),
    },
    layout: Layout {
        wide_min_width: px(1180.0),
        compact_min_width: px(960.0),
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
    sizes: default_sizes(),
};

macro_rules! metric_keys {
    ($($group:ident . $field:ident => $name:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        #[allow(non_camel_case_types)]
        pub(crate) enum MetricKey {
            $($field),*
        }

        impl MetricKey {
            pub(crate) const ALL: &'static [MetricKey] = &[$(MetricKey::$field),*];

            pub(crate) fn from_name(name: &str) -> Option<MetricKey> {
                match name {
                    $($name => Some(MetricKey::$field),)*
                    _ => None,
                }
            }

            pub(crate) fn name(self) -> &'static str {
                match self {
                    $(MetricKey::$field => $name),*
                }
            }

            pub(crate) fn label(self) -> String {
                label(self.name())
            }
        }

        impl Structure {
            fn get(self, key: MetricKey) -> Pixels {
                match key {
                    $(MetricKey::$field => self.$group.$field),*
                }
            }

            fn set(&mut self, key: MetricKey, value: Pixels) {
                match key {
                    $(MetricKey::$field => self.$group.$field = value),*
                }
            }
        }
    };
}

metric_keys!(
    space.xs => "space-xs",
    space.sm => "space-sm",
    space.md => "space-md",
    type_scale.caption => "font-caption",
    type_scale.body_small => "font-body-small",
    type_scale.body => "font-body",
    type_scale.reading => "font-reading",
    type_scale.display => "font-display",
    type_scale.line_body => "line-body",
    type_scale.line_reading => "line-reading",
    type_scale.line_composer => "line-composer",
    icons.inline => "icon-inline",
    icons.control => "icon-control",
    icons.prominent => "icon-prominent",
    controls.icon_button => "control-icon-button",
    controls.utility_row => "control-utility-row",
    controls.archived_preview_row => "control-archived-preview-row",
    metrics.radius => "radius",
    metrics.border_width => "border-width",
    scrollbar.width => "scrollbar-width",
    scrollbar.inset => "scrollbar-inset",
    layout.wide_min_width => "layout-wide-min-width",
    layout.compact_min_width => "layout-compact-min-width",
    layout.window_width => "layout-window-width",
    layout.window_height => "layout-window-height",
    layout.session_rail => "layout-session-rail",
    layout.session_rail_min => "layout-session-rail-min",
    layout.session_rail_max => "layout-session-rail-max",
    layout.run_panel => "layout-run-panel",
    layout.run_panel_min => "layout-run-panel-min",
    layout.run_panel_max => "layout-run-panel-max",
    layout.transcript_overdraw => "layout-transcript-overdraw",
    layout.composer_min => "layout-composer-min",
    layout.conversation_width => "layout-conversation-width",
    layout.dialog_width => "layout-dialog-width",
    layout.dialog_max_height => "layout-dialog-max-height",
    layout.tool_max_height => "layout-tool-max-height",
    layout.session_row_height => "layout-session-row-height",
    layout.status_row_height => "layout-status-row-height",
);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum LengthKey {
    Metric(MetricKey),
    Size(usize),
}

impl LengthKey {
    pub(crate) fn from_name(name: &str) -> Option<LengthKey> {
        if let Some(key) = MetricKey::from_name(name) {
            return Some(LengthKey::Metric(key));
        }
        let value = name.strip_prefix("size-")?.parse::<u16>().ok()?;
        SIZE_VALUES.binary_search(&value).ok().map(LengthKey::Size)
    }

    pub(crate) fn name(self) -> String {
        match self {
            LengthKey::Metric(key) => key.name().to_owned(),
            LengthKey::Size(index) => format!("size-{}", SIZE_VALUES[index]),
        }
    }

    pub(crate) fn label(self) -> String {
        match self {
            LengthKey::Metric(key) => key.label(),
            LengthKey::Size(index) => format!("{}px", SIZE_VALUES[index]),
        }
    }
}

impl Structure {
    fn set_length(&mut self, key: LengthKey, value: Pixels) {
        match key {
            LengthKey::Metric(key) => self.set(key, value),
            LengthKey::Size(index) => self.sizes[index] = value,
        }
    }
}

impl Theme {
    pub(crate) fn from_definition(definition: &ThemeDefinition) -> Self {
        let mut structure = STRUCTURE;
        for (key, value) in &definition.lengths {
            structure.set_length(*key, *value);
        }
        Self::from_structure(definition.colors, structure)
    }

    fn from_structure(colors: Colors, structure: Structure) -> Self {
        Self {
            colors,
            space: structure.space,
            type_scale: structure.type_scale,
            icons: structure.icons,
            controls: structure.controls,
            radius: structure.metrics.radius,
            border: structure.metrics.border_width,
            layout: structure.layout,
            scrollbar: structure.scrollbar,
            sizes: structure.sizes,
        }
    }

    pub(crate) fn size(self, value: f32) -> Pixels {
        let rounded = value.round();
        let clamped = rounded.clamp(0.0, f32::from(u16::MAX)) as u16;
        match SIZE_VALUES.binary_search(&clamped) {
            Ok(index) => self.sizes[index],
            Err(_) => px(rounded),
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
        theme: Theme::from_definition(definition),
        appearance: definition.appearance,
    })
});

static HIGHLIGHT: LazyLock<RwLock<Arc<HighlightTheme>>> = LazyLock::new(|| {
    let definition = builtin::default_definition();
    RwLock::new(highlight::for_theme(
        Theme::from_definition(definition),
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

pub(crate) fn editable_lengths() -> Vec<LengthKey> {
    MetricKey::ALL
        .iter()
        .map(|key| LengthKey::Metric(*key))
        .chain((0..SIZE_VALUES.len()).map(LengthKey::Size))
        .collect()
}

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
    component_colors.accent = colors.highlight.into();
    component_colors.accent_foreground = colors.text.into();
    component_colors.list = colors.panel.into();
    component_colors.list_hover = colors.highlight.into();
    component_colors.list_active = colors.highlight.into();
    component_colors.list_active_border = colors.highlight.into();
    component_colors.table = colors.panel.into();
    component_colors.table_hover = colors.highlight.into();
    component_colors.table_active = colors.highlight.into();
    component_colors.sidebar = colors.panel.into();
    component_colors.sidebar_accent = colors.highlight.into();
    component_colors.sidebar_primary = colors.highlight.into();
    component_colors.tab_bar = colors.panel.into();
    component_colors.tab_active = colors.highlight.into();
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
    component_colors.primary_hover = colors.highlight.into();
    component_colors.primary_active = colors.highlight.into();
    component_colors.primary_foreground = colors.text.into();
    component_colors.secondary = colors.surface.into();
    component_colors.secondary_hover = colors.highlight.into();
    component_colors.secondary_active = colors.highlight.into();
    component_colors.secondary_foreground = colors.text.into();
    component_colors.button = colors.surface.into();
    component_colors.button_hover = colors.highlight.into();
    component_colors.button_active = colors.highlight.into();
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
    component_colors.selection = colors.highlight.into();
    component.tokens = ThemeTokens::from(component.colors);
    install_scrollbar_theme(theme, cx);
    gpui_component::tooltip::set_metrics(gpui_component::tooltip::TooltipMetrics {
        padding_x: theme.space.md,
        padding_y: theme.space.sm,
        gap: theme.space.sm,
        font_size: theme.type_scale.caption,
        max_width: theme.size(360.0),
    });
}

fn install_scrollbar_theme(theme: Theme, cx: &mut App) {
    let base = gpui_base::Theme::global_mut(cx);
    let styles = base.scrollbar.styles.clone();
    let thumb = |style: gpui_base::ScrollbarThumbStyle| {
        style
            .width(theme.scrollbar.width)
            .inset(theme.scrollbar.inset)
    };
    base.scrollbar.styles = styles.thumb(thumb).thumb_hover(thumb).thumb_active(thumb);
}
