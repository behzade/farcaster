use gpui::{App, Font, FontFallbacks, Pixels, Rgba, px};
use gpui_component::{
    highlighter::HighlightTheme,
    theme::{Theme as ComponentTheme, ThemeMode, ThemeTokens},
};
use gpui_libghostty::{TerminalColor, TerminalTheme};

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

#[derive(Clone, Copy)]
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
    pub text: Rgba,
    pub muted: Rgba,
    pub subtle: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_active: Rgba,
    pub link: Rgba,
    pub code: Rgba,
    pub skill: Rgba,
    pub warning: Rgba,
    pub error: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub backdrop: Rgba,
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
    pub agent_marker: Pixels,
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

const fn rgb(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}
const fn rgba(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 24) & 0xff) as f32 / 255.0,
        g: ((hex >> 16) & 0xff) as f32 / 255.0,
        b: ((hex >> 8) & 0xff) as f32 / 255.0,
        a: (hex & 0xff) as f32 / 255.0,
    }
}

pub(crate) const THEME: Theme = Theme {
    colors: Colors {
        canvas: rgb(0x1b1f20),
        panel: rgb(0x222321),
        inspector: rgb(0x202221),
        composer: rgb(0x242522),
        surface: rgb(0x2b2b28),
        hover: rgb(0x383734),
        selection: rgb(0x292a27),
        session_selection: rgb(0x35392d),
        text_selection: rgb(0x4b5538),
        border: rgb(0x53514c),
        text: rgb(0xd6caad),
        muted: rgb(0x9c9280),
        subtle: rgb(0x80786b),
        accent: rgb(0xa3c049),
        accent_hover: rgb(0xbecb2c),
        accent_active: rgb(0x88b53d),
        link: rgb(0xa2ba88),
        code: rgb(0xdca21e),
        skill: rgb(0xc98ba0),
        warning: rgb(0xdca21e),
        error: rgb(0xf54933),
        danger: rgb(0xf54933),
        success: rgb(0xb0c21b),
        backdrop: rgba(0x000000b3),
    },
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
        agent_marker: px(18.0),
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

pub(crate) fn terminal_theme() -> TerminalTheme {
    let colors = THEME.colors;
    TerminalTheme::new(
        terminal_color(colors.canvas),
        terminal_color(colors.text),
        [
            terminal_color(colors.panel),
            TerminalColor::new(0xcc, 0x24, 0x1d),
            TerminalColor::new(0x98, 0x97, 0x1a),
            terminal_color(colors.code),
            TerminalColor::new(0x45, 0x85, 0x88),
            TerminalColor::new(0xb1, 0x62, 0x86),
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
    ComponentTheme::change(ThemeMode::Dark, None, cx);
    let theme = ComponentTheme::global_mut(cx);
    theme.font_family = UI_FONT_FAMILY.into();
    theme.font_size = THEME.type_scale.body;
    theme.mono_font_family = MONO_FONT_FAMILY.into();
    theme.mono_font_size = THEME.type_scale.body_small;
    theme.highlight_theme = HighlightTheme::default_dark();
    theme.radius = THEME.radius;
    theme.radius_lg = THEME.radius;
    theme.shadow = true;
    let colors = &mut theme.colors;
    colors.background = THEME.colors.canvas.into();
    colors.foreground = THEME.colors.text.into();
    colors.accent = THEME.colors.surface.into();
    colors.accent_foreground = THEME.colors.text.into();
    colors.link = THEME.colors.link.into();
    colors.link_active = THEME.colors.link.into();
    colors.link_hover = THEME.colors.link.into();
    colors.border = THEME.colors.border.into();
    colors.input = THEME.colors.border.into();
    colors.muted = THEME.colors.panel.into();
    colors.muted_foreground = THEME.colors.muted.into();
    colors.popover = THEME.colors.panel.into();
    colors.popover_foreground = THEME.colors.text.into();
    colors.primary = THEME.colors.surface.into();
    colors.primary_hover = THEME.colors.hover.into();
    colors.primary_active = THEME.colors.hover.into();
    colors.primary_foreground = THEME.colors.text.into();
    colors.secondary = THEME.colors.surface.into();
    colors.secondary_hover = THEME.colors.hover.into();
    colors.secondary_active = THEME.colors.hover.into();
    colors.secondary_foreground = THEME.colors.text.into();
    colors.button = THEME.colors.surface.into();
    colors.button_hover = THEME.colors.hover.into();
    colors.button_active = THEME.colors.hover.into();
    colors.button_foreground = THEME.colors.text.into();
    colors.button_primary = THEME.colors.accent.into();
    colors.button_primary_hover = THEME.colors.accent_hover.into();
    colors.button_primary_active = THEME.colors.accent_active.into();
    colors.button_primary_foreground = THEME.colors.canvas.into();
    colors.danger = THEME.colors.danger.into();
    colors.danger_foreground = THEME.colors.canvas.into();
    colors.warning = THEME.colors.warning.into();
    colors.warning_foreground = THEME.colors.canvas.into();
    colors.success = THEME.colors.success.into();
    colors.success_foreground = THEME.colors.canvas.into();
    colors.ring = THEME.colors.accent.into();
    colors.caret = THEME.colors.text.into();
    colors.selection = THEME.colors.text_selection.into();
    theme.tokens = ThemeTokens::from(theme.colors);
}

#[cfg(test)]
mod tests {
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
            THEME.controls.utility_row
                >= THEME.controls.icon_button + THEME.space.sm + THEME.space.sm
        );
    }
}
